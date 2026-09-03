use std::collections::HashSet;

use globset::{GlobBuilder, GlobMatcher};
use wae_core::domain::{Diagnostic, LayerOwnership, ModuleId, ModuleKind, SourceLocation};
use wae_core::rule_registry::{self, RuleDescriptor};

use crate::{DiagnosticSink, Rule, RuleContext};

pub struct DependencyDepthRule;
impl Rule for DependencyDepthRule {
    fn metadata(&self) -> &'static RuleDescriptor {
        rule_registry::descriptor("ARCH-006").expect("registered built-in rule")
    }
    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        let Some(options) = context.config.rules.get("ARCH-006").and_then(|rule| rule.options())
        else {
            return Ok(());
        };
        let Some(max_depth) = options.max_depth else { return Ok(()) };
        let entrypoints = compile_entrypoints(&options.entrypoints)?;
        let roots = matching_nodes(context, &entrypoints);
        let paths = context.graph.shortest_paths_from_any(&roots);
        for target in context.graph.nodes() {
            let Some(depth) = paths.depth(target) else { continue };
            if depth > max_depth {
                let mut diagnostic = module_diagnostic(
                    "ARCH-006",
                    target,
                    format!("Dependency depth {depth} exceeds configured maximum {max_depth}"),
                    "Introduce a facade or split the dependency chain at a stable boundary.",
                );
                diagnostic.dependency_path = paths.path(target).unwrap_or_default();
                diagnostic.metadata.insert("depth".into(), depth.to_string());
                diagnostic.metadata.insert("maximum".into(), max_depth.to_string());
                sink.emit(diagnostic);
            }
        }
        Ok(())
    }
}

pub struct OutgoingCouplingRule;
impl Rule for OutgoingCouplingRule {
    fn metadata(&self) -> &'static RuleDescriptor {
        rule_registry::descriptor("ARCH-007").expect("registered built-in rule")
    }
    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        let Some(maximum) = context
            .config
            .rules
            .get("ARCH-007")
            .and_then(|rule| rule.options())
            .and_then(|options| options.max_fan_out)
        else {
            return Ok(());
        };
        for module in source_modules(context) {
            let actual = context.graph.out_degree(&module.id);
            if actual > maximum {
                sink.emit(metric_diagnostic(
                    "ARCH-007",
                    &module.id,
                    "Outgoing coupling",
                    actual,
                    maximum,
                ));
            }
        }
        Ok(())
    }
}

pub struct IncomingCouplingRule;
impl Rule for IncomingCouplingRule {
    fn metadata(&self) -> &'static RuleDescriptor {
        rule_registry::descriptor("ARCH-008").expect("registered built-in rule")
    }
    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        let Some(maximum) = context
            .config
            .rules
            .get("ARCH-008")
            .and_then(|rule| rule.options())
            .and_then(|options| options.max_fan_in)
        else {
            return Ok(());
        };
        for module in source_modules(context) {
            let actual = context.graph.in_degree(&module.id);
            if actual > maximum {
                sink.emit(metric_diagnostic(
                    "ARCH-008",
                    &module.id,
                    "Incoming coupling",
                    actual,
                    maximum,
                ));
            }
        }
        Ok(())
    }
}

pub struct OrphanModuleRule;
impl Rule for OrphanModuleRule {
    fn metadata(&self) -> &'static RuleDescriptor {
        rule_registry::descriptor("ARCH-009").expect("registered built-in rule")
    }
    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        let Some(options) = context.config.rules.get("ARCH-009").and_then(|rule| rule.options())
        else {
            return Ok(());
        };
        if options.entrypoints.is_empty() {
            return Ok(());
        }
        let entrypoints = compile_entrypoints(&options.entrypoints)?;
        let roots = matching_nodes(context, &entrypoints);
        let mut reachable = roots.iter().cloned().collect::<HashSet<_>>();
        for root in roots {
            reachable.extend(context.graph.reachable_from(&root));
        }
        for module in source_modules(context) {
            if !reachable.contains(&module.id) {
                sink.emit(module_diagnostic(
                    "ARCH-009",
                    &module.id,
                    "Source module is unreachable from every configured architecture entrypoint".into(),
                    "Remove the orphan, export it from a reachable module, or add an intentional entrypoint.",
                ));
            }
        }
        Ok(())
    }
}

pub struct UnassignedLayerRule;
impl Rule for UnassignedLayerRule {
    fn metadata(&self) -> &'static RuleDescriptor {
        rule_registry::descriptor("ARCH-010").expect("registered built-in rule")
    }
    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        if context.config.architecture.layers.is_empty() {
            return Ok(());
        }
        for module in source_modules(context) {
            if matches!(context.ownership.get(&module.id), Some(LayerOwnership::Unassigned)) {
                let mut diagnostic = module_diagnostic(
                    "ARCH-010",
                    &module.id,
                    "Source module is not assigned to any architecture layer".into(),
                    "Anchor the module under a configured layer pattern or explicitly exempt it in architecture.coverage.allow_unassigned.",
                );
                diagnostic.metadata.insert("ownership".into(), "unassigned".into());
                sink.emit(diagnostic);
            }
        }
        Ok(())
    }
}

pub struct ArchitectureCoverageRule;
impl Rule for ArchitectureCoverageRule {
    fn metadata(&self) -> &'static RuleDescriptor {
        rule_registry::descriptor("ARCH-011").expect("registered built-in rule")
    }

    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        let Some(minimum) = context.config.architecture.coverage.minimum else {
            return Ok(());
        };
        let actual = context.ownership.coverage_percent();
        if actual >= minimum {
            return Ok(());
        }
        let unassigned = context.ownership.unassigned_modules();
        let mut diagnostic = Diagnostic::new(
            "ARCH-011",
            format!(
                "Architecture layer coverage {actual}% is below the configured minimum {minimum}%"
            ),
        );
        diagnostic.suggestion = Some(
            "Assign unowned modules to layers, lower the intentional threshold, or explicitly exempt support paths."
                .into(),
        );
        diagnostic.primary_location = unassigned.first().map(|module| SourceLocation {
            file: module.0.clone(),
            line: 1,
            column: 1,
        });
        diagnostic.dependency_path = unassigned.into_iter().cloned().collect();
        diagnostic.metadata.insert("coverageStatus".into(), "belowMinimum".into());
        diagnostic.metadata.insert("actualPercent".into(), actual.to_string());
        diagnostic.metadata.insert("minimumPercent".into(), minimum.to_string());
        diagnostic
            .metadata
            .insert("assignedModules".into(), context.ownership.assigned_modules().to_string());
        diagnostic
            .metadata
            .insert("exemptedModules".into(), context.ownership.exempted_modules().to_string());
        diagnostic.metadata.insert(
            "unassignedModules".into(),
            context.ownership.unassigned_modules().len().to_string(),
        );
        sink.emit(diagnostic);
        Ok(())
    }
}

fn source_modules<'a>(
    context: &'a RuleContext<'a>,
) -> impl Iterator<Item = &'a wae_core::domain::Module> {
    context.project.modules.iter().filter(|module| module.kind == ModuleKind::Source)
}

fn compile_entrypoints(patterns: &[String]) -> Result<Vec<GlobMatcher>, String> {
    patterns
        .iter()
        .map(|pattern| {
            GlobBuilder::new(pattern)
                .literal_separator(true)
                .build()
                .map(|glob| glob.compile_matcher())
                .map_err(|error| error.to_string())
        })
        .collect()
}

fn matching_nodes(context: &RuleContext<'_>, patterns: &[GlobMatcher]) -> Vec<ModuleId> {
    context
        .project
        .modules
        .iter()
        .filter(|module| module.kind == ModuleKind::Source)
        .filter(|module| patterns.iter().any(|pattern| pattern.is_match(&module.id.0)))
        .map(|module| module.id.clone())
        .collect()
}

fn metric_diagnostic(
    rule: &str,
    module: &ModuleId,
    label: &str,
    actual: usize,
    maximum: usize,
) -> Diagnostic {
    let mut diagnostic = module_diagnostic(
        rule,
        module,
        format!("{label} {actual} exceeds configured maximum {maximum}"),
        "Split responsibilities or introduce a stable facade to reduce direct coupling.",
    );
    diagnostic.metadata.insert("actual".into(), actual.to_string());
    diagnostic.metadata.insert("maximum".into(), maximum.to_string());
    diagnostic
}

fn module_diagnostic(
    rule: &str,
    module: &ModuleId,
    message: String,
    suggestion: &str,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(rule, message);
    diagnostic.primary_location =
        Some(SourceLocation { file: module.0.clone(), line: 1, column: 1 });
    diagnostic.dependency_path = vec![module.clone()];
    diagnostic.suggestion = Some(suggestion.into());
    diagnostic
}
