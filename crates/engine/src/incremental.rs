use std::collections::HashSet;

use wae_core::domain::{
    Dependency, DependencyTarget, Diagnostic, Module, ModuleId, PackageName, Project,
    ResolvedDependency,
};

/// Materialized read view for edge-local rule execution. The vectors are deliberately built once
/// per edit so individual rules never rescan the full immutable project merely to discard
/// unaffected records.
pub(crate) struct AffectedRuleInputs {
    pub(crate) modules: Vec<Module>,
    pub(crate) dependencies: Vec<Dependency>,
    pub(crate) resolved_dependencies: Vec<ResolvedDependency>,
    pub(crate) packages: HashSet<PackageName>,
}

impl AffectedRuleInputs {
    pub(crate) fn from_project(project: &Project, affected: &HashSet<ModuleId>) -> Self {
        let modules = project
            .modules
            .iter()
            .filter(|module| affected.contains(&module.id))
            .cloned()
            .collect::<Vec<_>>();
        let packages = modules.iter().map(|module| module.package.clone()).collect();
        let dependencies = project
            .dependencies
            .iter()
            .filter(|dependency| {
                affected.contains(&dependency.from) || affected.contains(&dependency.to)
            })
            .cloned()
            .collect();
        let resolved_dependencies = project
            .resolved_dependencies
            .iter()
            .filter(|dependency| {
                affected.contains(&dependency.from)
                    || resolved_target_module(&dependency.target)
                        .is_some_and(|target| affected.contains(target))
            })
            .cloned()
            .collect();
        Self { modules, dependencies, resolved_dependencies, packages }
    }
}

fn resolved_target_module(target: &DependencyTarget) -> Option<&ModuleId> {
    match target {
        DependencyTarget::Internal(module) | DependencyTarget::WorkspacePackage { module, .. } => {
            Some(module)
        }
        DependencyTarget::Builtin(_)
        | DependencyTarget::ExternalPackage(_)
        | DependencyTarget::Unresolved { .. } => None,
    }
}

/// The smallest conservative region needed by edge-local rules.
///
/// Seeds include reparsed modules and endpoints from their previous dependency facts. One pass
/// over the current edge set then adds the opposite endpoint of every incident edge. This keeps
/// local rule evaluation bounded without pretending that closure/global rules can use the same
/// locality guarantee.
pub(crate) fn edge_affected_region(
    seeds: HashSet<ModuleId>,
    dependencies: &[Dependency],
) -> HashSet<ModuleId> {
    let mut affected = seeds;
    let directly_affected = affected.clone();
    for dependency in dependencies {
        if directly_affected.contains(&dependency.from)
            || directly_affected.contains(&dependency.to)
        {
            affected.insert(dependency.from.clone());
            affected.insert(dependency.to.clone());
        }
    }
    affected
}

/// Returns whether a previously cached diagnostic belongs to an affected edge/package region.
/// Diagnostics outside this region are safe to retain when an edge-local rule is reevaluated.
pub(crate) fn diagnostic_touches_region(
    diagnostic: &Diagnostic,
    affected_modules: &HashSet<ModuleId>,
    project: &Project,
) -> bool {
    if diagnostic.dependency_path.iter().any(|module| affected_modules.contains(module))
        || diagnostic
            .primary_location
            .as_ref()
            .is_some_and(|location| affected_modules.contains(&ModuleId(location.file.clone())))
    {
        return true;
    }
    let affected_packages = project
        .modules
        .iter()
        .filter(|module| affected_modules.contains(&module.id))
        .map(|module| module.package.0.as_str())
        .collect::<HashSet<_>>();
    diagnostic.dependency_path.iter().any(|module| {
        module.0.strip_prefix("package:").is_some_and(|package| affected_packages.contains(package))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wae_core::domain::{DependencyKind, SourceLocation};

    #[test]
    fn edge_region_adds_only_incident_endpoints() {
        let module = |value: &str| ModuleId(value.into());
        let dependency = |from: &str, to: &str| Dependency {
            from: module(from),
            to: module(to),
            kind: DependencyKind::Static,
            location: SourceLocation { file: from.into(), line: 1, column: 1 },
        };
        let dependencies = [
            dependency("a.ts", "b.ts"),
            dependency("b.ts", "c.ts"),
            dependency("unrelated.ts", "other.ts"),
        ];

        let affected = edge_affected_region(HashSet::from([module("a.ts")]), &dependencies);

        assert_eq!(affected, HashSet::from([module("a.ts"), module("b.ts")]));
    }
}
