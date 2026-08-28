use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use wae_config::{Config, ConfigPreset, LayerConfig};
use wae_framework::{FrameworkAdapter, NextJsAdapter, ProjectEvidence};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectKind {
    Generic,
    FeatureSliced,
    NextJs,
    Nx,
    Turborepo,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Evidence {
    pub signal: String,
    pub path: String,
    pub weight: u8,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryReport {
    pub project_kind: ProjectKind,
    pub confidence: u8,
    pub evidence: Vec<Evidence>,
    pub config_files: Vec<String>,
    pub feature_clusters: Vec<String>,
    pub suggested_config: Config,
}

pub fn discover(root: &Path) -> Result<DiscoveryReport, String> {
    if !root.is_dir() {
        return Err(format!("project root `{}` is not a directory", root.display()));
    }
    let package_manifest = read_json(root.join("package.json"))?;
    let config_files = KNOWN_CONFIGS
        .iter()
        .filter(|name| root.join(name).is_file())
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    let framework_evidence = ProjectEvidence {
        package_manifest: package_manifest.clone(),
        config_files: config_files.clone(),
    };
    let next_score = NextJsAdapter.detection_score(&framework_evidence);
    let mut evidence = Vec::new();
    if root.join("nx.json").is_file() {
        evidence.push(Evidence {
            signal: "nx-workspace".into(),
            path: "nx.json".into(),
            weight: 100,
        });
    }
    if root.join("turbo.json").is_file() {
        evidence.push(Evidence {
            signal: "turborepo-workspace".into(),
            path: "turbo.json".into(),
            weight: 100,
        });
    }
    if next_score > 0 {
        let path = if manifest_has_dependency(package_manifest.as_ref(), "next") {
            "package.json#dependencies.next"
        } else {
            config_files
                .iter()
                .find(|path| path.starts_with("next.config."))
                .map(String::as_str)
                .unwrap_or("package.json")
        };
        evidence.push(Evidence {
            signal: "nextjs-framework".into(),
            path: path.into(),
            weight: next_score,
        });
    }
    let fsd_segments = ["app", "features", "entities", "shared"]
        .into_iter()
        .filter_map(|segment| {
            [format!("src/{segment}"), segment.to_string()]
                .into_iter()
                .find(|candidate| root.join(candidate).is_dir())
        })
        .collect::<Vec<_>>();
    for path in &fsd_segments {
        evidence.push(Evidence {
            signal: "architecture-segment".into(),
            path: path.clone(),
            weight: 20,
        });
    }

    let project_kind = if root.join("nx.json").is_file() {
        ProjectKind::Nx
    } else if root.join("turbo.json").is_file() {
        ProjectKind::Turborepo
    } else if next_score > 0 {
        ProjectKind::NextJs
    } else if fsd_segments.len() >= 2 {
        ProjectKind::FeatureSliced
    } else {
        ProjectKind::Generic
    };
    let confidence = match project_kind {
        ProjectKind::Nx | ProjectKind::Turborepo => 100,
        ProjectKind::NextJs => next_score,
        ProjectKind::FeatureSliced => (fsd_segments.len() as u8 * 20).min(80),
        ProjectKind::Generic => 0,
    };
    let feature_roots = ["src/features", "features"]
        .into_iter()
        .filter(|path| root.join(path).is_dir())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let feature_clusters = feature_roots
        .iter()
        .flat_map(|feature_root| immediate_directories(&root.join(feature_root), feature_root))
        .collect::<Vec<_>>();
    let mut suggested_config = suggested_config(&project_kind);
    if next_score > 0 {
        suggested_config.framework.auto_detect = true;
        suggested_config.framework.enabled = vec!["nextjs".into()];
    }
    if !feature_roots.is_empty() {
        suggested_config.architecture.features.roots = feature_roots;
    }
    Ok(DiscoveryReport {
        project_kind,
        confidence,
        evidence,
        config_files,
        feature_clusters,
        suggested_config,
    })
}

fn suggested_config(kind: &ProjectKind) -> Config {
    match kind {
        ProjectKind::FeatureSliced => Config::for_preset(ConfigPreset::Fsd),
        ProjectKind::NextJs => Config::for_preset(ConfigPreset::Next),
        ProjectKind::Nx => Config::for_preset(ConfigPreset::Nx),
        ProjectKind::Turborepo => {
            let mut config = Config::default();
            config.architecture.layers = BTreeMap::from([
                (
                    "apps".into(),
                    LayerConfig {
                        patterns: vec!["apps/*/**".into()],
                        can_import: vec!["packages".into()],
                    },
                ),
                (
                    "packages".into(),
                    LayerConfig { patterns: vec!["packages/*/**".into()], can_import: vec![] },
                ),
            ]);
            config.architecture.presets.monorepo_boundaries = true;
            config
        }
        ProjectKind::Generic => Config::for_preset(ConfigPreset::Blank),
    }
}

fn immediate_directories(path: &Path, prefix: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(path) else { return Vec::new() };
    let mut values = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .map(|name| format!("{prefix}/{name}"))
        .collect::<Vec<_>>();
    values.sort();
    values
}

fn read_json(path: impl AsRef<Path>) -> Result<Option<serde_json::Value>, String> {
    let path = path.as_ref();
    if !path.is_file() {
        return Ok(None);
    }
    let source = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read `{}`: {error}", path.display()))?;
    serde_json::from_str(&source)
        .map(Some)
        .map_err(|error| format!("invalid JSON in `{}`: {error}", path.display()))
}

fn manifest_has_dependency(manifest: Option<&serde_json::Value>, dependency: &str) -> bool {
    manifest.is_some_and(|manifest| {
        ["dependencies", "devDependencies", "peerDependencies"]
            .into_iter()
            .filter_map(|field| manifest.get(field).and_then(serde_json::Value::as_object))
            .any(|dependencies| dependencies.contains_key(dependency))
    })
}

const KNOWN_CONFIGS: &[&str] = &[
    "tsconfig.json",
    "jsconfig.json",
    "nx.json",
    "turbo.json",
    "pnpm-workspace.yaml",
    "next.config.js",
    "next.config.mjs",
    "next.config.cjs",
    "next.config.ts",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_next_features_from_authoritative_evidence() {
        let root = std::env::temp_dir().join(format!("wae-discover-next-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src/features/cart")).unwrap();
        std::fs::write(root.join("package.json"), r#"{"dependencies":{"next":"15"}}"#).unwrap();
        std::fs::write(root.join("jsconfig.json"), "{}").unwrap();
        let report = discover(&root).unwrap();
        assert_eq!(report.project_kind, ProjectKind::NextJs);
        assert_eq!(report.confidence, 100);
        assert_eq!(report.feature_clusters, ["src/features/cart"]);
        assert_eq!(report.suggested_config.framework.enabled, ["nextjs"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn turborepo_layout_wins_over_nested_framework_evidence() {
        let root = std::env::temp_dir().join(format!("wae-discover-turbo-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("package.json"), r#"{"devDependencies":{"next":"15"}}"#).unwrap();
        std::fs::write(root.join("turbo.json"), "{}").unwrap();
        let report = discover(&root).unwrap();
        assert_eq!(report.project_kind, ProjectKind::Turborepo);
        assert!(report.suggested_config.architecture.layers.contains_key("packages"));
        assert_eq!(report.suggested_config.framework.enabled, ["nextjs"]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
