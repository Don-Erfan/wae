use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::PackageModuleType;

/// A Node package scope. A scope does not require a package name: the presence of package.json is
/// sufficient to establish module-format semantics for its descendants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageScope {
    pub root: PathBuf,
    pub module_type: PackageModuleType,
}

/// Nearest-ancestor index of every package boundary inside the analyzed project.
#[derive(Clone, Debug, Default)]
pub struct PackageScopeIndex {
    scopes: Vec<PackageScope>,
}

impl PackageScopeIndex {
    /// Builds the index only from package boundaries that can affect discovered source importers.
    /// Generated, excluded and out-of-root manifests are never parsed.
    pub fn from_importers(project_root: &Path, importers: &[PathBuf]) -> Result<Self, String> {
        let mut manifests = BTreeSet::new();
        for importer in importers {
            let Some(mut directory) = importer.parent() else { continue };
            loop {
                if !directory.starts_with(project_root) {
                    break;
                }
                let manifest = directory.join("package.json");
                if manifest.is_file() {
                    manifests.insert(manifest);
                }
                if directory == project_root {
                    break;
                }
                let Some(parent) = directory.parent() else { break };
                directory = parent;
            }
        }
        Self::from_manifest_paths(project_root, manifests)
    }

    /// Full-repository discovery is retained for resolver API compatibility. Analysis engines
    /// should prefer `from_importers` so excluded manifests cannot affect a scoped analysis.
    pub fn discover(project_root: &Path) -> Result<Self, String> {
        let mut manifests = BTreeSet::new();
        let mut builder = ignore::WalkBuilder::new(project_root);
        builder.hidden(false).git_ignore(true).git_global(true).git_exclude(true);
        builder.filter_entry(|entry| entry.file_name() != "node_modules");
        for entry in builder.build() {
            let entry = entry.map_err(|error| error.to_string())?;
            if !entry.file_type().is_some_and(|kind| kind.is_file())
                || entry.file_name() != "package.json"
            {
                continue;
            }
            manifests.insert(entry.into_path());
        }
        Self::from_manifest_paths(project_root, manifests)
    }

    fn from_manifest_paths(
        project_root: &Path,
        manifests: BTreeSet<PathBuf>,
    ) -> Result<Self, String> {
        let mut scopes = Vec::new();
        for path in manifests {
            let source = fs::read_to_string(&path)
                .map_err(|error| format!("cannot read `{}`: {error}", path.display()))?;
            let manifest: serde_json::Value = serde_json::from_str(&source).map_err(|error| {
                format!("invalid package manifest `{}`: {error}", path.display())
            })?;
            scopes.push(PackageScope {
                root: path.parent().unwrap_or(project_root).to_path_buf(),
                module_type: match manifest.get("type").and_then(|value| value.as_str()) {
                    Some("module") => PackageModuleType::Module,
                    Some("commonjs") => PackageModuleType::CommonJs,
                    _ => PackageModuleType::Unspecified,
                },
            });
        }
        scopes.sort_by(|left, right| {
            right
                .root
                .components()
                .count()
                .cmp(&left.root.components().count())
                .then_with(|| left.root.cmp(&right.root))
        });
        Ok(Self { scopes })
    }

    pub fn nearest(&self, importer: &Path) -> Option<&PackageScope> {
        self.scopes.iter().find(|scope| importer.starts_with(&scope.root))
    }

    pub fn scopes(&self) -> &[PackageScope] {
        &self.scopes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_unnamed_and_nested_package_boundaries() {
        let root = std::env::temp_dir().join(format!("wae-package-scopes-{}", std::process::id()));
        let nested = root.join("src/esm-zone");
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join("package.json"), r#"{"private":true,"type":"commonjs"}"#).unwrap();
        fs::write(nested.join("package.json"), r#"{"type":"module"}"#).unwrap();
        let importers = [nested.join("app.ts"), root.join("src/app.ts")];
        let index = PackageScopeIndex::from_importers(&root, &importers).unwrap();
        assert_eq!(index.scopes().len(), 2);
        assert_eq!(
            index.nearest(&nested.join("app.ts")).unwrap().module_type,
            PackageModuleType::Module
        );
        assert_eq!(
            index.nearest(&root.join("src/app.ts")).unwrap().module_type,
            PackageModuleType::CommonJs
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn importer_scopes_ignore_invalid_manifests_outside_analysis() {
        let root = std::env::temp_dir().join(format!("wae-package-scoped-{}", std::process::id()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("dist")).unwrap();
        fs::write(root.join("package.json"), r#"{"type":"module"}"#).unwrap();
        fs::write(root.join("src/app.ts"), "").unwrap();
        fs::write(root.join("dist/package.json"), "not-json").unwrap();
        let index = PackageScopeIndex::from_importers(&root, &[root.join("src/app.ts")]).unwrap();
        assert_eq!(index.scopes().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
