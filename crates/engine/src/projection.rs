//! Read-only views over a completed [`Analysis`].
//!
//! CLI, language server and other adapters render these projections instead of re-deriving
//! architecture facts, so every surface explains a module identically.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use serde::Serialize;
use wae_core::domain::{Diagnostic, LayerOwnership, ModuleId, ModuleKind, Runtime};

use crate::Analysis;

/// Everything WAE knows about one module, including why it received its runtime.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleInspection {
    pub id: String,
    pub kind: String,
    pub package: String,
    pub layer: Option<String>,
    pub feature: Option<String>,
    pub ownership: Option<LayerOwnership>,
    pub runtime: RuntimeExplanation,
    pub framework: Option<String>,
    pub framework_attributes: BTreeMap<String, String>,
    pub dependencies: Vec<EdgeView>,
    pub dependents: Vec<EdgeView>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeExplanation {
    pub runtime: String,
    /// `explicit`, `marker-package`, `directive`, `convention`, `propagated`, `default`,
    /// `builtin`, or `none` when no framework adapter classified the module.
    pub source: String,
    pub reason: String,
    /// For propagated browser modules: the shortest path from the module that declared the
    /// browser boundary (for example a `"use client"` file) to this module.
    pub propagation_path: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeView {
    pub module: String,
    pub kind: String,
    pub file: String,
    pub line: usize,
    pub column: usize,
}

/// Grouped project overview for IDE architecture explorers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchitectureOverview {
    pub schema_version: u32,
    pub source_modules: usize,
    pub dependencies: usize,
    pub packages: Vec<ModuleGroup>,
    pub layers: Vec<ModuleGroup>,
    pub features: Vec<ModuleGroup>,
    pub runtimes: Vec<ModuleGroup>,
    pub violations: Vec<RuleGroup>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleGroup {
    pub name: String,
    pub modules: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleGroup {
    pub rule_id: String,
    pub title: String,
    pub category: String,
    pub diagnostics: Vec<Diagnostic>,
}

/// Groups source modules by package, layer, feature and runtime, and active (unsuppressed)
/// diagnostics by rule. Every list is sorted, so the overview is deterministic.
pub fn architecture_overview(analysis: &Analysis) -> ArchitectureOverview {
    fn grouped(entries: impl Iterator<Item = (String, String)>) -> Vec<ModuleGroup> {
        let mut groups = BTreeMap::<String, Vec<String>>::new();
        for (group, module) in entries {
            groups.entry(group).or_default().push(module);
        }
        groups
            .into_iter()
            .map(|(name, mut modules)| {
                modules.sort();
                ModuleGroup { name, modules }
            })
            .collect()
    }
    let sources = analysis
        .project
        .modules
        .iter()
        .filter(|module| module.kind == ModuleKind::Source)
        .collect::<Vec<_>>();
    let mut violations = BTreeMap::<String, Vec<Diagnostic>>::new();
    for diagnostic in analysis.diagnostics.iter().filter(|diagnostic| !diagnostic.suppressed) {
        violations.entry(diagnostic.rule_id.0.clone()).or_default().push(diagnostic.clone());
    }
    ArchitectureOverview {
        schema_version: analysis.schema_version,
        source_modules: sources.len(),
        dependencies: analysis.project.dependencies.len(),
        packages: grouped(
            sources.iter().map(|module| (module.package.0.clone(), module.id.0.clone())),
        ),
        layers: grouped(sources.iter().map(|module| {
            let layer = module.layer.as_ref().map_or("(unassigned)", |layer| layer.0.as_str());
            (layer.to_string(), module.id.0.clone())
        })),
        features: grouped(analysis.features.iter().map(|(module, feature)| {
            (format!("{} ({})", feature.name, feature.package.0), module.0.clone())
        })),
        runtimes: grouped(
            sources.iter().map(|module| (runtime_name(module.runtime).into(), module.id.0.clone())),
        ),
        violations: violations
            .into_iter()
            .map(|(rule_id, diagnostics)| {
                let descriptor = wae_core::rule_registry::descriptor(&rule_id);
                RuleGroup {
                    title: descriptor.map_or(rule_id.clone(), |rule| rule.title.to_string()),
                    category: descriptor.map_or("unknown", |rule| rule.category).to_string(),
                    rule_id,
                    diagnostics,
                }
            })
            .collect(),
    }
}

pub fn runtime_name(runtime: Runtime) -> &'static str {
    match runtime {
        Runtime::Browser => "browser",
        Runtime::Server => "server",
        Runtime::Edge => "edge",
        Runtime::Node => "node",
        Runtime::Universal => "universal",
        Runtime::Unknown => "unknown",
    }
}

/// Resolves a user-supplied path (`./src/a.ts`, `src\\a.ts`, `src/a.ts`) to a module id.
pub fn find_module(analysis: &Analysis, query: &str) -> Option<ModuleId> {
    let normalized = query.replace('\\', "/");
    let normalized = normalized.trim_start_matches("./");
    analysis
        .project
        .modules
        .iter()
        .find(|module| module.id.0 == normalized || module.id.0 == query)
        .map(|module| module.id.clone())
}

pub fn inspect_module(analysis: &Analysis, id: &ModuleId) -> Option<ModuleInspection> {
    let module = analysis.project.modules.iter().find(|module| &module.id == id)?;
    let edge = |module_id: &ModuleId, dependency: &wae_core::domain::Dependency| EdgeView {
        module: module_id.0.clone(),
        kind: format!("{:?}", dependency.kind),
        file: dependency.location.file.clone(),
        line: dependency.location.line,
        column: dependency.location.column,
    };
    let mut dependencies = analysis
        .project
        .dependencies
        .iter()
        .filter(|dependency| &dependency.from == id)
        .map(|dependency| edge(&dependency.to, dependency))
        .collect::<Vec<_>>();
    let mut dependents = analysis
        .project
        .dependencies
        .iter()
        .filter(|dependency| &dependency.to == id)
        .map(|dependency| edge(&dependency.from, dependency))
        .collect::<Vec<_>>();
    let order = |edge: &EdgeView| (edge.module.clone(), edge.line, edge.column, edge.kind.clone());
    dependencies.sort_by_key(order);
    dependents.sort_by_key(order);
    let diagnostics = analysis
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.primary_location.as_ref().is_some_and(|location| location.file == id.0)
                || diagnostic.dependency_path.contains(id)
        })
        .cloned()
        .collect();
    Some(ModuleInspection {
        id: id.0.clone(),
        kind: format!("{:?}", module.kind),
        package: module.package.0.clone(),
        layer: module.layer.as_ref().map(|layer| layer.0.clone()),
        feature: analysis.features.get(id).map(|feature| feature.name.clone()),
        ownership: analysis.ownership.get(id).cloned(),
        runtime: explain_runtime(analysis, id),
        framework: module.framework_metadata.adapter_id.clone(),
        framework_attributes: module.framework_metadata.attributes.clone(),
        dependencies,
        dependents,
        diagnostics,
    })
}

pub fn explain_runtime(analysis: &Analysis, id: &ModuleId) -> RuntimeExplanation {
    let Some(module) = analysis.project.modules.iter().find(|module| &module.id == id) else {
        return RuntimeExplanation {
            runtime: "unknown".into(),
            source: "none".into(),
            reason: "module is not part of the analyzed project".into(),
            propagation_path: None,
        };
    };
    let attributes = &module.framework_metadata.attributes;
    let source = match (&module.kind, attributes.get("runtimeSource")) {
        (_, Some(source)) => source.clone(),
        (ModuleKind::External, None) if module.runtime == Runtime::Node => "builtin".into(),
        _ => "none".into(),
    };
    let reason = match source.as_str() {
        "explicit" => "the module exports an explicit `runtime` value".to_string(),
        "marker-package" if module.runtime == Runtime::Server => {
            "the module imports the `server-only` marker package".into()
        }
        "marker-package" => "the module imports the `client-only` marker package".into(),
        "conflicting-marker" => "the module imports both `server-only` and `client-only`".into(),
        "directive" => "the module starts with the `\"use client\"` directive".into(),
        "convention" => format!(
            "framework convention for role `{}`",
            attributes.get("role").map_or("module", String::as_str)
        ),
        "propagated" => {
            "imported (transitively) by a browser boundary, so it is bundled for the browser".into()
        }
        "default" => "no runtime evidence; the module is treated as universal".into(),
        "builtin" => "Node.js builtin module".into(),
        _ if module.kind == ModuleKind::External => "external package".into(),
        _ => "no framework adapter classified this module".into(),
    };
    let propagation_path =
        (source == "propagated").then(|| browser_propagation_path(analysis, id)).flatten();
    RuntimeExplanation {
        runtime: runtime_name(module.runtime).into(),
        source,
        reason,
        propagation_path,
    }
}

/// Browser propagation only marks modules reachable through other browser modules, so the
/// explanation searches the browser-only subgraph from every declared (non-propagated) boundary.
fn browser_propagation_path(analysis: &Analysis, target: &ModuleId) -> Option<Vec<String>> {
    let browser = analysis
        .project
        .modules
        .iter()
        .filter(|module| module.runtime == Runtime::Browser)
        .map(|module| (module.id.clone(), module))
        .collect::<HashMap<_, _>>();
    let mut roots = browser
        .values()
        .filter(|module| {
            module.framework_metadata.attributes.get("runtimeSource").map(String::as_str)
                != Some("propagated")
        })
        .map(|module| module.id.clone())
        .collect::<Vec<_>>();
    roots.sort();
    let mut previous = HashMap::<ModuleId, ModuleId>::new();
    let mut seen = roots.iter().cloned().collect::<HashSet<_>>();
    let mut queue = roots.into_iter().collect::<VecDeque<_>>();
    while let Some(node) = queue.pop_front() {
        if &node == target {
            let mut path = vec![node.0.clone()];
            let mut cursor = node;
            while let Some(parent) = previous.get(&cursor) {
                path.push(parent.0.clone());
                cursor = parent.clone();
            }
            path.reverse();
            return Some(path);
        }
        let mut next = analysis.graph.outgoing(&node);
        next.sort();
        for neighbor in next {
            if browser.contains_key(&neighbor) && seen.insert(neighbor.clone()) {
                previous.insert(neighbor.clone(), node.clone());
                queue.push_back(neighbor);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AnalyzeRequest, Engine};

    fn analyze(root: impl Into<std::path::PathBuf>) -> Analysis {
        Engine::default().analyze(AnalyzeRequest::new(root)).unwrap()
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(name)
    }

    #[test]
    fn inspection_lists_both_edge_directions_and_related_diagnostics() {
        let analysis = analyze(fixture("runtime"));
        let id = find_module(&analysis, "./src/app/server.ts").unwrap();
        let inspection = inspect_module(&analysis, &id).unwrap();
        assert_eq!(inspection.runtime.runtime, "server");
        assert!(inspection.dependents.iter().any(|edge| edge.module == "src/app/client.tsx"));
        assert!(inspection.dependencies.iter().any(|edge| edge.module == "src/app/client.tsx"));
        assert!(
            inspection.diagnostics.iter().any(|diagnostic| diagnostic.rule_id.0 == "RUNTIME-001")
        );
    }

    #[test]
    fn overview_groups_modules_and_active_violations_deterministically() {
        let analysis = analyze(fixture("features"));
        let overview = architecture_overview(&analysis);
        assert_eq!(overview, architecture_overview(&analysis));
        assert_eq!(
            overview.source_modules,
            overview.packages.iter().map(|group| group.modules.len()).sum::<usize>()
        );
        assert!(!overview.features.is_empty(), "{:?}", overview.features);
        assert!(overview.violations.iter().any(|group| group.rule_id == "ARCH-005"));
        assert!(
            overview
                .violations
                .iter()
                .all(|group| { group.diagnostics.iter().all(|diagnostic| !diagnostic.suppressed) })
        );
    }

    #[test]
    fn propagated_browser_runtime_is_explained_with_its_boundary_path() {
        let root = std::env::temp_dir().join(format!("wae-runtime-why-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src/app")).unwrap();
        std::fs::write(root.join("package.json"), r#"{"dependencies":{"next":"15.0.0"}}"#).unwrap();
        std::fs::write(root.join("wae.yaml"), "version: 1\n").unwrap();
        std::fs::write(
            root.join("src/app/widget.tsx"),
            "\"use client\";\nimport { helper } from \"../lib/helper\";\nexport const W = helper;\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("src/lib")).unwrap();
        std::fs::write(
            root.join("src/lib/helper.ts"),
            "import { fmt } from './format';\nexport const helper = fmt;\n",
        )
        .unwrap();
        std::fs::write(root.join("src/lib/format.ts"), "export const fmt = 1;\n").unwrap();
        let analysis = analyze(&root);
        let explanation = explain_runtime(&analysis, &ModuleId("src/lib/format.ts".into()));
        assert_eq!(explanation.runtime, "browser");
        assert_eq!(explanation.source, "propagated");
        assert_eq!(
            explanation.propagation_path.unwrap(),
            ["src/app/widget.tsx", "src/lib/helper.ts", "src/lib/format.ts"]
        );
        let directive = explain_runtime(&analysis, &ModuleId("src/app/widget.tsx".into()));
        assert_eq!(directive.source, "directive");
        assert!(directive.propagation_path.is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
