use std::collections::{HashMap, HashSet};

use globset::{GlobBuilder, GlobMatcher};
use wae_config::Config;
use wae_core::domain::{
    ArchitectureOwnershipIndex, Diagnostic, FeatureId, ModuleId, PackageName, Project,
    SourceLocation,
};
use wae_core::rule_registry::{self, RuleDescriptor};
use wae_graph::{ModuleGraph, PackageGraph, RuntimeGraph};

mod architecture_metrics;
mod package_rules;
mod runtime_rules;
use architecture_metrics::{
    ArchitectureCoverageRule, DependencyDepthRule, IncomingCouplingRule, OrphanModuleRule,
    OutgoingCouplingRule, UnassignedLayerRule,
};
use package_rules::{
    CrossPackageRelativeImportRule, ForbiddenPackageDependencyRule, PackageCycleRule,
    UndeclaredWorkspaceDependencyRule,
};
use runtime_rules::{
    AmbiguousUniversalRuntimeRule, BrowserIncompatiblePackageRule, BrowserToNodeRule,
    BrowserToServerRule, EdgeIncompatibleDependencyRule, IncompatibleRuntimeCycleRule,
};

pub struct RuleContext<'a> {
    pub project: &'a Project,
    pub graph: &'a ModuleGraph,
    pub package_graph: &'a PackageGraph,
    pub runtime_graph: &'a RuntimeGraph,
    pub config: &'a Config,
    pub module_layers: &'a HashMap<ModuleId, String>,
    pub ownership: &'a ArchitectureOwnershipIndex,
    pub module_features: &'a HashMap<ModuleId, FeatureId>,
    pub module_feature_roots: &'a HashMap<ModuleId, String>,
    pub policies: &'a CompiledRulePolicies,
    pub declared_package_dependencies: &'a HashMap<PackageName, HashSet<PackageName>>,
}

pub struct CompiledRulePolicies {
    forbidden_dependencies: Vec<(GlobMatcher, GlobMatcher)>,
    public_entrypoints: Vec<GlobMatcher>,
}

impl CompiledRulePolicies {
    pub fn compile(config: &Config) -> Result<Self, String> {
        Ok(Self {
            forbidden_dependencies: config
                .architecture
                .forbidden_dependencies
                .iter()
                .map(|policy| {
                    Ok::<_, String>((compile_matcher(&policy.from)?, compile_matcher(&policy.to)?))
                })
                .collect::<Result<Vec<_>, _>>()?,
            public_entrypoints: config
                .architecture
                .features
                .public_entrypoints
                .iter()
                .map(|entry| compile_matcher(entry))
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
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
    fn metadata(&self) -> &'static RuleDescriptor;
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleProfile {
    pub rule_id: &'static str,
    pub elapsed_ns: u128,
    pub diagnostics: usize,
}

pub struct RuleEvaluation {
    pub diagnostics: Vec<Diagnostic>,
    pub profiles: Vec<RuleProfile>,
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
            .with_rule(DependencyDepthRule)
            .with_rule(OutgoingCouplingRule)
            .with_rule(IncomingCouplingRule)
            .with_rule(OrphanModuleRule)
            .with_rule(UnassignedLayerRule)
            .with_rule(ArchitectureCoverageRule)
            .with_rule(PackageCycleRule)
            .with_rule(ForbiddenPackageDependencyRule)
            .with_rule(UndeclaredWorkspaceDependencyRule)
            .with_rule(CrossPackageRelativeImportRule)
            .with_rule(BrowserToServerRule)
            .with_rule(BrowserToNodeRule)
            .with_rule(BrowserIncompatiblePackageRule)
            .with_rule(EdgeIncompatibleDependencyRule)
            .with_rule(AmbiguousUniversalRuntimeRule)
            .with_rule(IncompatibleRuntimeCycleRule)
    }
    pub fn with_rule<R: Rule + 'static>(mut self, rule: R) -> Self {
        self.rules.push(Box::new(rule));
        self
    }
    pub fn evaluate(&self, context: &RuleContext<'_>) -> Result<Vec<Diagnostic>, String> {
        Ok(self.evaluate_profiled(context)?.diagnostics)
    }

    pub fn evaluate_profiled(&self, context: &RuleContext<'_>) -> Result<RuleEvaluation, String> {
        self.evaluate_profiled_selected(context, None)
    }

    pub fn enabled_rule_ids(&self, context: &RuleContext<'_>) -> Vec<&'static str> {
        self.rules
            .iter()
            .filter(|rule| context.config.configured || rule.metadata().id == "ARCH-001")
            .filter(|rule| context.config.rule_enabled_anywhere(rule.metadata().id))
            .map(|rule| rule.metadata().id)
            .collect()
    }

    pub fn evaluate_profiled_rules(
        &self,
        context: &RuleContext<'_>,
        rule_ids: &HashSet<String>,
    ) -> Result<RuleEvaluation, String> {
        self.evaluate_profiled_selected(context, Some(rule_ids))
    }

    fn evaluate_profiled_selected(
        &self,
        context: &RuleContext<'_>,
        selected: Option<&HashSet<String>>,
    ) -> Result<RuleEvaluation, String> {
        let enabled = self
            .rules
            .iter()
            .filter(|rule| context.config.configured || rule.metadata().id == "ARCH-001")
            .filter(|rule| selected.is_none_or(|ids| ids.contains(rule.metadata().id)))
            .filter(|rule| context.config.rule_enabled_anywhere(rule.metadata().id))
            .map(|rule| rule.as_ref())
            .collect::<Vec<_>>();
        let parallel = context.project.modules.len() >= 100
            && enabled.len() >= 4
            && std::thread::available_parallelism().is_ok_and(|workers| workers.get() > 1);
        let batches = if parallel {
            std::thread::scope(|scope| {
                let handles = enabled
                    .iter()
                    .map(|rule| scope.spawn(move || evaluate_one(*rule, context)))
                    .collect::<Vec<_>>();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle
                            .join()
                            .map_err(|_| "architecture rule worker panicked".to_string())?
                    })
                    .collect::<Result<Vec<_>, String>>()
            })?
        } else {
            enabled
                .into_iter()
                .map(|rule| evaluate_one(rule, context))
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut diagnostics = Vec::new();
        let mut profiles = Vec::new();
        for (batch, profile) in batches {
            diagnostics.extend(batch);
            profiles.push(profile);
        }
        profiles.sort_by_key(|profile| profile.rule_id);
        Ok(RuleEvaluation { diagnostics, profiles })
    }
}

fn evaluate_one(
    rule: &dyn Rule,
    context: &RuleContext<'_>,
) -> Result<(Vec<Diagnostic>, RuleProfile), String> {
    let started = std::time::Instant::now();
    let mut diagnostics = Vec::new();
    rule.evaluate(context, &mut diagnostics)?;
    diagnostics.retain_mut(|diagnostic| {
        let path =
            diagnostic.primary_location.as_ref().map_or("", |location| location.file.as_str());
        let Some(severity) = context.config.rule_severity_for_path(rule.metadata().id, path) else {
            return false;
        };
        diagnostic.severity = severity;
        diagnostic.refresh_fingerprint();
        true
    });
    let profile = RuleProfile {
        rule_id: rule.metadata().id,
        elapsed_ns: started.elapsed().as_nanos(),
        diagnostics: diagnostics.len(),
    };
    Ok((diagnostics, profile))
}

pub struct CircularDependencyRule;
impl Rule for CircularDependencyRule {
    fn metadata(&self) -> &'static RuleDescriptor {
        rule_registry::descriptor("ARCH-001").expect("registered built-in rule")
    }
    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        for cycle in context.graph.cycles() {
            let primary = edge_location(context.project, &cycle[0], &cycle[1]);
            let mut diagnostic = Diagnostic::new("ARCH-001", "Circular dependency detected");
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

pub struct ForbiddenDependencyRule;
impl Rule for ForbiddenDependencyRule {
    fn metadata(&self) -> &'static RuleDescriptor {
        rule_registry::descriptor("ARCH-002").expect("registered built-in rule")
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
                context.policies.forbidden_dependencies.iter().any(|(from_matcher, to_matcher)| {
                    from_matcher.is_match(&from) && to_matcher.is_match(&to)
                });
            let package_to_app = context.config.architecture.presets.monorepo_boundaries
                && (from.starts_with("packages/") || from.contains("/packages/"))
                && (to.starts_with("apps/") || to.contains("/apps/"));
            if configured || package_to_app {
                sink.emit(dependency_diagnostic(
                    "ARCH-002",
                    "Dependency is forbidden by architecture policy",
                    dependency,
                    "Remove the dependency or explicitly revise the policy.",
                ));
            }
        }
        Ok(())
    }
}

pub struct LayerBoundaryRule;
impl Rule for LayerBoundaryRule {
    fn metadata(&self) -> &'static RuleDescriptor {
        rule_registry::descriptor("ARCH-003").expect("registered built-in rule")
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
                    "ARCH-003",
                    &format!("Layer `{from}` cannot import `{to}`"),
                    dependency,
                    "Depend on an allowed lower layer or introduce an application facade.",
                ));
            }
        }
        Ok(())
    }
}

pub struct FeatureBoundaryRule;
impl Rule for FeatureBoundaryRule {
    fn metadata(&self) -> &'static RuleDescriptor {
        rule_registry::descriptor("ARCH-004").expect("registered built-in rule")
    }
    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        for dependency in &context.project.dependencies {
            let Some(target_feature) = context.module_features.get(&dependency.to) else {
                continue;
            };
            let importer_is_owner =
                context.module_features.get(&dependency.from) == Some(target_feature);
            if !importer_is_owner
                && !is_public_entrypoint(
                    &dependency.to.0,
                    target_feature,
                    context.module_feature_roots.get(&dependency.to),
                    &context.policies.public_entrypoints,
                )
            {
                let importer = context
                    .module_features
                    .get(&dependency.from)
                    .map_or("outside", |feature| feature.name.as_str());
                sink.emit(dependency_diagnostic(
                    "ARCH-004",
                    &format!(
                        "Importer `{importer}` accesses internals of feature `{}` in package `{}`",
                        target_feature.name, target_feature.package.0
                    ),
                    dependency,
                    "Import the target feature's public index entrypoint.",
                ));
            }
        }
        Ok(())
    }
}

pub struct PrivateImportRule;
impl Rule for PrivateImportRule {
    fn metadata(&self) -> &'static RuleDescriptor {
        rule_registry::descriptor("ARCH-005").expect("registered built-in rule")
    }
    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        for dependency in &context.project.dependencies {
            let private = is_private_path(&dependency.to.0, context.config);
            let importer_feature = context.module_features.get(&dependency.from);
            let target_feature = context.module_features.get(&dependency.to);
            let outside_owner = match (importer_feature, target_feature) {
                (Some(from), Some(to)) => from != to,
                (_, Some(_)) => true,
                _ => false,
            };
            if private && outside_owner {
                sink.emit(dependency_diagnostic(
                    "ARCH-005",
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
    diagnostic.metadata.insert("dependencyKind".into(), format!("{:?}", dependency.kind));
    diagnostic.metadata.insert(
        "policy".into(),
        match rule {
            "ARCH-004" => "feature-public-boundary",
            "ARCH-005" => "explicit-private-segment",
            _ => "dependency-policy",
        }
        .into(),
    );
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
fn compile_matcher(pattern: &str) -> Result<GlobMatcher, String> {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .map(|glob| glob.compile_matcher())
        .map_err(|error| error.to_string())
}
fn is_public_entrypoint(
    path: &str,
    feature: &FeatureId,
    feature_root: Option<&String>,
    entries: &[GlobMatcher],
) -> bool {
    let normalized = path.replace('\\', "/");
    let Some(feature_root) = feature_root else { return false };
    let prefix = format!("{}/{}/", feature_root.trim_matches('/'), feature.name);
    normalized
        .strip_prefix(&prefix)
        .is_some_and(|relative| entries.iter().any(|entry| entry.is_match(relative)))
}

fn is_private_path(path: &str, config: &Config) -> bool {
    let target = path.replace('\\', "/");
    config
        .architecture
        .features
        .private_segments
        .iter()
        .any(|segment| target.split('/').any(|part| part == segment))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wae_core::domain::{Dependency, DependencyKind, PackageName, Project, SourceLocation};

    fn dependency(from: &str, to: &str) -> Dependency {
        Dependency {
            from: ModuleId(from.into()),
            to: ModuleId(to.into()),
            kind: DependencyKind::Static,
            location: SourceLocation { file: from.into(), line: 1, column: 1 },
        }
    }

    fn context<'a>(
        project: &'a Project,
        graph: &'a ModuleGraph,
        config: &'a Config,
        features: &'a HashMap<ModuleId, FeatureId>,
    ) -> RuleContext<'a> {
        RuleContext {
            project,
            graph,
            package_graph: Box::leak(Box::new(PackageGraph::from_project(project))),
            runtime_graph: Box::leak(Box::new(RuntimeGraph::from_project(project))),
            config,
            module_layers: Box::leak(Box::new(HashMap::new())),
            ownership: Box::leak(Box::new(ArchitectureOwnershipIndex::default())),
            module_features: features,
            module_feature_roots: Box::leak(Box::new(HashMap::new())),
            policies: Box::leak(Box::new(CompiledRulePolicies::compile(config).unwrap())),
            declared_package_dependencies: Box::leak(Box::new(HashMap::new())),
        }
    }

    #[test]
    fn private_modules_are_allowed_inside_their_own_feature() {
        let edge = dependency("src/features/user/internal/a.ts", "src/features/user/internal/b.ts");
        let project = Project { dependencies: vec![edge.clone()], ..Project::default() };
        let graph = ModuleGraph::from_project(&project);
        let config = Config::default();
        let user = FeatureId { package: PackageName("app".into()), name: "user".into() };
        let features = HashMap::from([(edge.from, user.clone()), (edge.to, user)]);
        let mut diagnostics = Vec::new();
        PrivateImportRule
            .evaluate(&context(&project, &graph, &config, &features), &mut diagnostics)
            .unwrap();
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn private_modules_are_rejected_outside_their_owner() {
        let edge =
            dependency("src/features/payment/service.ts", "src/features/user/internal/token.ts");
        let project = Project { dependencies: vec![edge.clone()], ..Project::default() };
        let graph = ModuleGraph::from_project(&project);
        let config = Config::default();
        let features = HashMap::from([
            (edge.from, FeatureId { package: PackageName("app".into()), name: "payment".into() }),
            (edge.to, FeatureId { package: PackageName("app".into()), name: "user".into() }),
        ]);
        let mut diagnostics = Vec::new();
        PrivateImportRule
            .evaluate(&context(&project, &graph, &config, &features), &mut diagnostics)
            .unwrap();
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn only_the_feature_root_entrypoint_is_public() {
        let config = Config::default();
        let public = config
            .architecture
            .features
            .public_entrypoints
            .iter()
            .map(|entry| compile_matcher(entry).unwrap())
            .collect::<Vec<_>>();
        let feature = FeatureId { package: PackageName("app".into()), name: "user".into() };
        let root = "src/features".to_string();
        assert!(
            is_public_entrypoint("src/features/user/index.ts", &feature, Some(&root), &public,)
        );
        assert!(!is_public_entrypoint(
            "src/features/user/internal/index.ts",
            &feature,
            Some(&root),
            &public,
        ));
    }

    #[test]
    fn importer_outside_features_cannot_access_feature_internals() {
        let edge = dependency("src/app/page.ts", "src/features/user/model.ts");
        let project = Project { dependencies: vec![edge.clone()], ..Project::default() };
        let graph = ModuleGraph::from_project(&project);
        let config = Config::default();
        let features = HashMap::from([(
            edge.to.clone(),
            FeatureId { package: PackageName("app".into()), name: "user".into() },
        )]);
        let roots = HashMap::from([(edge.to.clone(), "src/features".into())]);
        let mut diagnostics = Vec::new();
        FeatureBoundaryRule
            .evaluate(
                &RuleContext {
                    project: &project,
                    graph: &graph,
                    package_graph: &PackageGraph::from_project(&project),
                    runtime_graph: &RuntimeGraph::from_project(&project),
                    config: &config,
                    module_layers: &HashMap::new(),
                    ownership: &ArchitectureOwnershipIndex::default(),
                    module_features: &features,
                    module_feature_roots: &roots,
                    policies: &CompiledRulePolicies::compile(&config).unwrap(),
                    declared_package_dependencies: &HashMap::new(),
                },
                &mut diagnostics,
            )
            .unwrap();
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn forbidden_dependency_patterns_use_real_glob_semantics() {
        let exact = compile_matcher("src/shared/*.ts").unwrap();
        assert!(exact.is_match("src/shared/util.ts"));
        assert!(!exact.is_match("src/shared/deep/util.ts"));
    }

    #[test]
    fn overlapping_feature_and_private_rules_preserve_the_strongest_enforcement() {
        let edge =
            dependency("src/features/payment/service.ts", "src/features/user/internal/token.ts");
        let project = Project { dependencies: vec![edge.clone()], ..Project::default() };
        let graph = ModuleGraph::from_project(&project);
        let mut config = Config::default();
        config.configured = true;
        config.rules.insert(
            "ARCH-004".into(),
            wae_config::RuleConfig::Severity(wae_core::domain::Severity::Info),
        );
        config.rules.insert(
            "ARCH-005".into(),
            wae_config::RuleConfig::Severity(wae_core::domain::Severity::Error),
        );
        let features = HashMap::from([
            (edge.from, FeatureId { package: PackageName("app".into()), name: "payment".into() }),
            (edge.to, FeatureId { package: PackageName("app".into()), name: "user".into() }),
        ]);
        let diagnostics =
            RuleSet::defaults().evaluate(&context(&project, &graph, &config, &features)).unwrap();
        assert_eq!(diagnostics.iter().filter(|d| d.rule_id.0 == "ARCH-004").count(), 1);
        assert_eq!(diagnostics.iter().filter(|d| d.rule_id.0 == "ARCH-005").count(), 1);
        assert!(diagnostics.iter().any(|d| {
            d.rule_id.0 == "ARCH-005" && d.severity == wae_core::domain::Severity::Error
        }));
    }
}
