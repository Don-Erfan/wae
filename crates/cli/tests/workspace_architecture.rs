//! Phase 35 dogfooding for the Rust workspace: WAE analyzes JavaScript and TypeScript, so its own
//! crate boundaries are enforced here from the Cargo manifests. Analysis crates must never depend
//! on delivery adapters (CLI, LSP, MCP, reporters), and core depends on nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const ADAPTERS: &[&str] = &["cli", "lsp", "mcp", "reporters"];

/// The approved direction of every internal dependency. Adding a crate or an edge requires an
/// explicit change here, reviewed together with docs/ARCHITECTURE.md.
const ALLOWED: &[(&str, &[&str])] = &[
    ("core", &[]),
    ("config", &["core"]),
    ("parser", &["core"]),
    ("graph", &["core"]),
    ("framework", &["core"]),
    ("resolver", &["core", "config"]),
    ("rules", &["core", "config", "graph"]),
    ("discovery", &["core", "config", "framework"]),
    ("engine", &["core", "config", "graph", "framework", "parser", "resolver", "rules"]),
    ("reporters", &["core", "config", "engine"]),
    ("cli", &["core", "config", "engine", "reporters", "discovery"]),
    ("lsp", &["core", "engine"]),
    ("mcp", &["core", "engine"]),
];

fn internal_dependencies(manifest: &str) -> BTreeSet<String> {
    let mut section = String::new();
    let mut dependencies = BTreeSet::new();
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line.trim_matches(|c| c == '[' || c == ']').to_string();
            continue;
        }
        let runtime_section = section == "dependencies"
            || (section.starts_with("target.") && section.ends_with(".dependencies"));
        if runtime_section {
            if let Some(name) =
                line.split(['=', ' ']).next().and_then(|key| key.strip_prefix("wae-"))
            {
                dependencies.insert(name.to_string());
            }
        }
    }
    dependencies
}

#[test]
fn crate_dependencies_follow_the_approved_architecture() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let allowed = ALLOWED
        .iter()
        .map(|(name, deps)| (*name, deps.iter().copied().collect::<BTreeSet<_>>()))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    for entry in std::fs::read_dir(&crates).unwrap() {
        let path = entry.unwrap().path().join("Cargo.toml");
        if !path.exists() {
            continue;
        }
        let name = path.parent().unwrap().file_name().unwrap().to_string_lossy().to_string();
        let policy = allowed
            .get(name.as_str())
            .unwrap_or_else(|| panic!("crate `{name}` has no approved dependency policy"));
        let actual = internal_dependencies(&std::fs::read_to_string(&path).unwrap());
        for dependency in &actual {
            assert!(
                policy.contains(dependency.as_str()),
                "`wae-{name}` must not depend on `wae-{dependency}`"
            );
            if !ADAPTERS.contains(&name.as_str()) {
                assert!(
                    !ADAPTERS.contains(&dependency.as_str()),
                    "analysis crate `wae-{name}` leaks into adapter `wae-{dependency}`"
                );
            }
        }
        seen.insert(name);
    }
    assert_eq!(seen, allowed.keys().map(|name| name.to_string()).collect());
}

#[test]
fn manifest_parser_ignores_dev_dependencies() {
    let manifest = "[dependencies]\nwae-core = { path = \"../core\" }\nserde = \"1\"\n[dev-dependencies]\nwae-mcp = { path = \"../mcp\" }\n[target.'cfg(windows)'.dependencies]\nwae-graph = { path = \"../graph\" }\n";
    assert_eq!(
        internal_dependencies(manifest),
        ["core", "graph"].into_iter().map(String::from).collect()
    );
}
