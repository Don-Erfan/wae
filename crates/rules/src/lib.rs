use std::collections::HashMap;

use wae_config::Config;
use wae_core::domain::{Diagnostic, ModuleId, Project, SourceLocation};
use wae_graph::ModuleGraph;

pub struct RuleMetadata {
    pub id: &'static str,
    pub title: &'static str,
}

pub struct RuleContext<'a> {
    pub project: &'a Project,
    pub graph: &'a ModuleGraph,
    pub config: &'a Config,
    pub module_layers: &'a HashMap<ModuleId, String>,
    pub module_features: &'a HashMap<ModuleId, String>,
}

pub trait DiagnosticSink {
    fn emit(&mut self, diagnostic: Diagnostic);
}
impl DiagnosticSink for Vec<Diagnostic> {
    fn emit(&mut self, diagnostic: Diagnostic) {
        self.push(diagnostic);
    }
}

pub trait Rule: Send + Sync {
    fn metadata(&self) -> &'static RuleMetadata;
    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String>;
}

#[derive(Default)]
pub struct RuleSet {
    rules: Vec<Box<dyn Rule>>,
}

impl RuleSet {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn defaults() -> Self {
        Self::new()
            .with_rule(CircularDependencyRule)
            .with_rule(ForbiddenDependencyRule)
            .with_rule(LayerBoundaryRule)
            .with_rule(FeatureBoundaryRule)
            .with_rule(PrivateImportRule)
    }
    pub fn with_rule<R: Rule + 'static>(mut self, rule: R) -> Self {
        self.rules.push(Box::new(rule));
        self
    }
    pub fn evaluate(&self, context: &RuleContext<'_>) -> Result<Vec<Diagnostic>, String> {
        let mut diagnostics = Vec::new();
        for rule in &self.rules {
            let severity =
                context.config.rules.get(rule.metadata().id).and_then(|value| value.severity());
            let Some(severity) = severity else { continue };
            let before = diagnostics.len();
            rule.evaluate(context, &mut diagnostics)?;
            for diagnostic in &mut diagnostics[before..] {
                diagnostic.severity = severity.clone();
                diagnostic.refresh_fingerprint();
            }
        }
        Ok(diagnostics)
    }
}

static CIRCULAR: RuleMetadata = RuleMetadata { id: "ARCH-001", title: "Circular dependency" };
pub struct CircularDependencyRule;
impl Rule for CircularDependencyRule {
    fn metadata(&self) -> &'static RuleMetadata {
        &CIRCULAR
    }
    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        for cycle in context.graph.cycles() {
            let primary = edge_location(context.project, &cycle[0], &cycle[1]);
            let mut diagnostic = Diagnostic::new(CIRCULAR.id, "Circular dependency detected");
            diagnostic.primary_location = primary;
            diagnostic.dependency_path = cycle;
            diagnostic.suggestion = Some(
                "Break the cycle by extracting a lower-level module or dependency port.".into(),
            );
            sink.emit(diagnostic);
        }
        Ok(())
    }
}

static FORBIDDEN: RuleMetadata = RuleMetadata { id: "ARCH-002", title: "Forbidden dependency" };
pub struct ForbiddenDependencyRule;
impl Rule for ForbiddenDependencyRule {
    fn metadata(&self) -> &'static RuleMetadata {
        &FORBIDDEN
    }
    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        for dependency in &context.project.dependencies {
            let from = dependency.from.0.replace('\\', "/");
            let to = dependency.to.0.replace('\\', "/");
            let configured =
                context.config.architecture.forbidden_dependencies.iter().any(|policy| {
                    path_matches(&from, &policy.from) && path_matches(&to, &policy.to)
                });
            let package_to_app = (from.starts_with("packages/") || from.contains("/packages/"))
                && (to.starts_with("apps/") || to.contains("/apps/"));
            if configured || package_to_app {
                sink.emit(dependency_diagnostic(
                    FORBIDDEN.id,
                    "Dependency is forbidden by architecture policy",
                    dependency,
                    "Remove the dependency or explicitly revise the policy.",
                ));
            }
        }
        Ok(())
    }
}

static LAYER: RuleMetadata = RuleMetadata { id: "ARCH-003", title: "Layer boundary violation" };
pub struct LayerBoundaryRule;
impl Rule for LayerBoundaryRule {
    fn metadata(&self) -> &'static RuleMetadata {
        &LAYER
    }
    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        for dependency in &context.project.dependencies {
            let (Some(from), Some(to)) = (
                context.module_layers.get(&dependency.from),
                context.module_layers.get(&dependency.to),
            ) else {
                continue;
            };
            if from == to {
                continue;
            }
            let allowed = context
                .config
                .architecture
                .layers
                .get(from)
                .is_none_or(|layer| layer.can_import.contains(to));
            if !allowed {
                sink.emit(dependency_diagnostic(
                    LAYER.id,
                    &format!("Layer `{from}` cannot import `{to}`"),
                    dependency,
                    "Depend on an allowed lower layer or introduce an application facade.",
                ));
            }
        }
        Ok(())
    }
}

static FEATURE: RuleMetadata = RuleMetadata { id: "ARCH-004", title: "Feature boundary violation" };
pub struct FeatureBoundaryRule;
impl Rule for FeatureBoundaryRule {
    fn metadata(&self) -> &'static RuleMetadata {
        &FEATURE
    }
    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        for dependency in &context.project.dependencies {
            let (Some(from), Some(to)) = (
                context.module_features.get(&dependency.from),
                context.module_features.get(&dependency.to),
            ) else {
                continue;
            };
            if from != to && !is_public_entrypoint(&dependency.to.0, context.config) {
                sink.emit(dependency_diagnostic(
                    FEATURE.id,
                    &format!("Feature `{from}` imports the internals of feature `{to}`"),
                    dependency,
                    "Import the target feature's public index entrypoint.",
                ));
            }
        }
        Ok(())
    }
}

static PRIVATE: RuleMetadata = RuleMetadata { id: "ARCH-005", title: "Private module import" };
pub struct PrivateImportRule;
impl Rule for PrivateImportRule {
    fn metadata(&self) -> &'static RuleMetadata {
        &PRIVATE
    }
    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        for dependency in &context.project.dependencies {
            let target = dependency.to.0.replace('\\', "/");
            let private = context
                .config
                .architecture
                .features
                .private_segments
                .iter()
                .any(|segment| target.split('/').any(|part| part == segment));
            let cross_feature = matches!((context.module_features.get(&dependency.from), context.module_features.get(&dependency.to)), (Some(from), Some(to)) if from != to);
            if private && !cross_feature {
                sink.emit(dependency_diagnostic(
                    PRIVATE.id,
                    "Private module imported outside its public API",
                    dependency,
                    "Import from the module's public entrypoint.",
                ));
            }
        }
        Ok(())
    }
}

fn dependency_diagnostic(
    rule: &str,
    message: &str,
    dependency: &wae_core::domain::Dependency,
    suggestion: &str,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(rule, message);
    diagnostic.primary_location = Some(dependency.location.clone());
    diagnostic.dependency_path = vec![dependency.from.clone(), dependency.to.clone()];
    diagnostic.suggestion = Some(suggestion.into());
    diagnostic
}

fn edge_location(project: &Project, from: &ModuleId, to: &ModuleId) -> Option<SourceLocation> {
    project
        .dependencies
        .iter()
        .find(|edge| &edge.from == from && &edge.to == to)
        .map(|edge| edge.location.clone())
}
fn path_matches(path: &str, pattern: &str) -> bool {
    let needle = pattern.trim_matches('*');
    needle.is_empty() || path.contains(needle)
}
fn is_public_entrypoint(path: &str, config: &Config) -> bool {
    let normalized = path.replace('\\', "/");
    let name = normalized.rsplit('/').next().unwrap_or(path);
    config.architecture.features.public_entrypoints.iter().any(|entry| entry == name)
}
