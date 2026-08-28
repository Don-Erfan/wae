use globset::GlobBuilder;
use wae_core::domain::{DependencyTarget, Diagnostic, ModuleId, PackageName, SourceLocation};
use wae_core::rule_registry::{self, RuleDescriptor};

use crate::{DiagnosticSink, Rule, RuleContext};

pub struct PackageCycleRule;
impl Rule for PackageCycleRule {
    fn metadata(&self) -> &'static RuleDescriptor {
        rule_registry::descriptor("PACKAGE-001").expect("registered built-in rule")
    }
    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        for cycle in context.package_graph.cycles() {
            let mut diagnostic =
                Diagnostic::new("PACKAGE-001", "Circular workspace package dependency detected");
            diagnostic.dependency_path =
                cycle.iter().map(|package| ModuleId(format!("package:{}", package.0))).collect();
            diagnostic.primary_location = package_edge_location(context, &cycle[0], &cycle[1]);
            diagnostic.metadata.insert(
                "packages".into(),
                cycle.iter().map(|p| p.0.as_str()).collect::<Vec<_>>().join(" -> "),
            );
            diagnostic.suggestion = Some("Move the shared contract into a lower-level package or invert one dependency through a port.".into());
            sink.emit(diagnostic);
        }
        Ok(())
    }
}

pub struct ForbiddenPackageDependencyRule;
impl Rule for ForbiddenPackageDependencyRule {
    fn metadata(&self) -> &'static RuleDescriptor {
        rule_registry::descriptor("PACKAGE-002").expect("registered built-in rule")
    }
    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        let policies = context
            .config
            .architecture
            .forbidden_package_dependencies
            .iter()
            .map(|policy| {
                let from = GlobBuilder::new(&policy.from)
                    .literal_separator(true)
                    .build()
                    .map_err(|e| e.to_string())?
                    .compile_matcher();
                let to = GlobBuilder::new(&policy.to)
                    .literal_separator(true)
                    .build()
                    .map_err(|e| e.to_string())?
                    .compile_matcher();
                Ok::<_, String>((from, to))
            })
            .collect::<Result<Vec<_>, _>>()?;
        for edge in context.package_graph.edges() {
            if policies
                .iter()
                .any(|(from, to)| from.is_match(&edge.from.0) && to.is_match(&edge.to.0))
            {
                sink.emit(package_diagnostic(
                    "PACKAGE-002",
                    context,
                    &edge.from,
                    &edge.to,
                    "Package dependency is forbidden by architecture policy",
                    "Remove the dependency or revise the explicit package policy.",
                ));
            }
        }
        Ok(())
    }
}

pub struct UndeclaredWorkspaceDependencyRule;
impl Rule for UndeclaredWorkspaceDependencyRule {
    fn metadata(&self) -> &'static RuleDescriptor {
        rule_registry::descriptor("PACKAGE-003").expect("registered built-in rule")
    }
    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        for dependency in &context.project.resolved_dependencies {
            let DependencyTarget::WorkspacePackage { package: target, module } = &dependency.target
            else {
                continue;
            };
            let Some(importer) =
                context.project.modules.iter().find(|module| module.id == dependency.from)
            else {
                continue;
            };
            if importer.package == *target {
                continue;
            }
            let Some(declared) = context.declared_package_dependencies.get(&importer.package)
            else {
                continue;
            };
            if !declared.contains(target) {
                let mut diagnostic = dependency_diagnostic(
                    "PACKAGE-003",
                    &dependency.from,
                    module,
                    &dependency.location,
                    format!(
                        "Workspace package `{}` is not declared by `{}`",
                        target.0, importer.package.0
                    ),
                    "Add the workspace package to dependencies, devDependencies, peerDependencies, or optionalDependencies.",
                );
                diagnostic.metadata.insert("importerPackage".into(), importer.package.0.clone());
                diagnostic.metadata.insert("targetPackage".into(), target.0.clone());
                sink.emit(diagnostic);
            }
        }
        Ok(())
    }
}

pub struct CrossPackageRelativeImportRule;
impl Rule for CrossPackageRelativeImportRule {
    fn metadata(&self) -> &'static RuleDescriptor {
        rule_registry::descriptor("PACKAGE-004").expect("registered built-in rule")
    }
    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        for dependency in &context.project.resolved_dependencies {
            if !dependency.specifier.starts_with('.') {
                continue;
            }
            let target_id = match &dependency.target {
                DependencyTarget::Internal(module)
                | DependencyTarget::WorkspacePackage { module, .. } => module,
                _ => continue,
            };
            let Some(importer) =
                context.project.modules.iter().find(|module| module.id == dependency.from)
            else {
                continue;
            };
            let Some(target) =
                context.project.modules.iter().find(|module| module.id == *target_id)
            else {
                continue;
            };
            if importer.package != target.package {
                sink.emit(dependency_diagnostic(
                    "PACKAGE-004",
                    &dependency.from,
                    target_id,
                    &dependency.location,
                    format!(
                        "Relative import crosses package boundary from `{}` to `{}`",
                        importer.package.0, target.package.0
                    ),
                    "Import the target package by its declared package name and public entrypoint.",
                ));
            }
        }
        Ok(())
    }
}

fn package_edge_location(
    context: &RuleContext<'_>,
    from: &PackageName,
    to: &PackageName,
) -> Option<SourceLocation> {
    context.project.dependencies.iter().find_map(|dependency| {
        let source = context.project.modules.iter().find(|module| module.id == dependency.from)?;
        let target = context.project.modules.iter().find(|module| module.id == dependency.to)?;
        (&source.package == from && &target.package == to).then(|| dependency.location.clone())
    })
}

fn package_diagnostic(
    rule: &str,
    context: &RuleContext<'_>,
    from: &PackageName,
    to: &PackageName,
    message: &str,
    suggestion: &str,
) -> Diagnostic {
    let location = package_edge_location(context, from, to).unwrap_or_default();
    let mut diagnostic = Diagnostic::new(rule, message);
    diagnostic.primary_location = Some(location);
    diagnostic.dependency_path =
        vec![ModuleId(format!("package:{}", from.0)), ModuleId(format!("package:{}", to.0))];
    diagnostic.suggestion = Some(suggestion.into());
    diagnostic
}

fn dependency_diagnostic(
    rule: &str,
    from: &ModuleId,
    to: &ModuleId,
    location: &SourceLocation,
    message: String,
    suggestion: &str,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(rule, message);
    diagnostic.primary_location = Some(location.clone());
    diagnostic.dependency_path = vec![from.clone(), to.clone()];
    diagnostic.suggestion = Some(suggestion.into());
    diagnostic
}
