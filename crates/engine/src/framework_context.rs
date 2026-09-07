use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use wae_framework::{FrameworkAdapter, FrameworkRegistry, ProjectEvidence};

use crate::{AnalysisError, normalize_text_path, relative_resolved_path};

/// Package-scoped framework evidence. A monorepo root is not assumed to describe every package.
#[derive(Clone, Debug, Default)]
pub(crate) struct FrameworkContextIndex {
    by_package_root: BTreeMap<String, ProjectEvidence>,
}

impl FrameworkContextIndex {
    pub(crate) fn discover(
        workspace_root: &Path,
        package_roots: impl IntoIterator<Item = PathBuf>,
    ) -> Result<Self, AnalysisError> {
        let mut roots = package_roots.into_iter().collect::<Vec<_>>();
        roots.push(workspace_root.to_path_buf());
        roots.sort();
        roots.dedup();

        let mut by_package_root = BTreeMap::new();
        for root in roots {
            let key = relative_resolved_path(
                workspace_root,
                &normalize_text_path(&root.to_string_lossy()),
            );
            by_package_root.insert(key, read_package_evidence(&root)?);
        }
        Ok(Self { by_package_root })
    }

    pub(crate) fn adapter_for<'a>(
        &self,
        registry: &'a FrameworkRegistry,
        package_root: &str,
        enabled: &[String],
        auto_detect: bool,
    ) -> Option<&'a dyn FrameworkAdapter> {
        let evidence = self
            .by_package_root
            .get(package_root.trim_matches('/'))
            .or_else(|| self.by_package_root.get(""))?;
        registry.select(evidence, enabled, auto_detect)
    }
}

fn read_package_evidence(root: &Path) -> Result<ProjectEvidence, AnalysisError> {
    let manifest_path = root.join("package.json");
    let package_manifest = manifest_path
        .is_file()
        .then(|| {
            fs::read_to_string(&manifest_path)
                .map_err(|error| {
                    AnalysisError::Project(format!(
                        "cannot read framework manifest `{}`: {error}",
                        manifest_path.display()
                    ))
                })
                .and_then(|source| {
                    serde_json::from_str(&source).map_err(|error| {
                        AnalysisError::Project(format!(
                            "invalid framework manifest `{}`: {error}",
                            manifest_path.display()
                        ))
                    })
                })
        })
        .transpose()?;
    let config_files = ["next.config.js", "next.config.mjs", "next.config.cjs", "next.config.ts"]
        .into_iter()
        .filter(|name| root.join(name).is_file())
        .map(str::to_owned)
        .collect();
    Ok(ProjectEvidence { package_manifest, config_files })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_framework_evidence_per_package_in_a_non_next_workspace() {
        let root =
            std::env::temp_dir().join(format!("wae-framework-context-{}", std::process::id()));
        let package = root.join("services/store");
        fs::create_dir_all(&package).unwrap();
        fs::write(root.join("package.json"), r#"{"private":true}"#).unwrap();
        fs::write(
            package.join("package.json"),
            r#"{"name":"store","dependencies":{"next":"16.3.4"}}"#,
        )
        .unwrap();

        let index = FrameworkContextIndex::discover(&root, [package]).unwrap();
        let registry = FrameworkRegistry::default();
        assert!(index.adapter_for(&registry, "", &[], true).is_none());
        assert_eq!(
            index.adapter_for(&registry, "services/store", &[], true).map(FrameworkAdapter::id),
            Some("nextjs")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
