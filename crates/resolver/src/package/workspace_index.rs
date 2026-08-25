use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobMatcher};
use serde::Deserialize;
use wae_config::ResolutionMode;
use wae_core::domain::ModulePath;

use super::{
    LegacyEntrypoints, PackageTarget, lexically_within, manifest_entrypoints, manifest_imports,
    resolve_export, resolve_package_target,
};
use crate::{
    Resolution, ResolutionHandler, ResolutionRequest, normalize, package_name,
    resolution_candidates, resolve_file_with_mode,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspacePackage {
    pub name: String,
    pub root: PathBuf,
    has_exports: bool,
    entrypoints: BTreeMap<String, serde_json::Value>,
    imports: BTreeMap<String, serde_json::Value>,
    legacy_entrypoints: LegacyEntrypoints,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PackageModuleType {
    Module,
    CommonJs,
    #[default]
    Unspecified,
}

/// Repository/index for named packages declared by npm, Yarn or pnpm workspace configuration.
/// Package-format scopes are intentionally owned by the separate `PackageScopeIndex`.
#[derive(Clone, Debug, Default)]
pub struct WorkspacePackageIndex {
    packages: Vec<WorkspacePackage>,
}

impl WorkspacePackageIndex {
    pub fn discover(project_root: &Path) -> Result<Self, String> {
        let patterns = workspace_patterns(project_root)?;
        let (includes, excludes) = compile_workspace_patterns(&patterns)?;
        let mut packages = Vec::new();
        let mut builder = ignore::WalkBuilder::new(project_root);
        builder.hidden(false).git_ignore(true).git_global(true).git_exclude(true);
        for entry in builder.build() {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry.file_name() != "package.json"
                || entry.path().components().any(|part| part.as_os_str() == "node_modules")
            {
                continue;
            }
            let package_root = entry.path().parent().unwrap_or(project_root);
            let relative =
                normalize(package_root.strip_prefix(project_root).unwrap_or(package_root));
            let is_project_root = package_root == project_root;
            let declared = includes.iter().any(|pattern| pattern.is_match(&relative))
                && !excludes.iter().any(|pattern| pattern.is_match(&relative));
            if !is_project_root && !declared {
                continue;
            }
            let source = fs::read_to_string(entry.path())
                .map_err(|error| format!("cannot read `{}`: {error}", entry.path().display()))?;
            let manifest: serde_json::Value = serde_json::from_str(&source).map_err(|error| {
                format!("invalid package manifest `{}`: {error}", entry.path().display())
            })?;
            let Some(name) = manifest.get("name").and_then(|value| value.as_str()) else {
                continue;
            };
            let root = entry.path().parent().unwrap_or(project_root).to_path_buf();
            packages.push(WorkspacePackage {
                name: name.into(),
                root,
                has_exports: manifest.get("exports").is_some(),
                entrypoints: manifest_entrypoints(&manifest),
                imports: manifest_imports(&manifest),
                legacy_entrypoints: LegacyEntrypoints::from_manifest(&manifest),
            });
        }
        packages.sort_by(|left, right| left.name.cmp(&right.name));
        for duplicate in packages.windows(2) {
            if duplicate[0].name == duplicate[1].name {
                return Err(format!(
                    "duplicate workspace package name `{}` in `{}` and `{}`",
                    duplicate[0].name,
                    duplicate[0].root.display(),
                    duplicate[1].root.display()
                ));
            }
        }
        Ok(Self { packages })
    }

    pub fn packages(&self) -> &[WorkspacePackage] {
        &self.packages
    }
}

impl ResolutionHandler for WorkspacePackageIndex {
    fn try_resolve(&self, request: &ResolutionRequest<'_>) -> Option<Resolution> {
        let importer = Path::new(&request.importer.0);
        let specifier = request.specifier;
        if specifier.starts_with('.') || specifier.starts_with('/') {
            return None;
        }
        if specifier.starts_with('#') {
            let package = self
                .packages
                .iter()
                .filter(|package| importer.starts_with(&package.root))
                .max_by_key(|package| package.root.components().count())?;
            if request.mode == ResolutionMode::Node10 {
                return Some(Resolution::Unresolved);
            }
            let target = resolve_export(&package.imports, specifier, request);
            return Some(resolve_package_target(package, target, request.mode, true));
        }
        let name = package_name(specifier);
        let package = self.packages.iter().find(|package| package.name == name)?;
        let subpath = specifier.strip_prefix(&name).unwrap_or_default().trim_start_matches('/');
        let key = if subpath.is_empty() { ".".into() } else { format!("./{subpath}") };
        let configured = (request.mode != ResolutionMode::Node10)
            .then(|| resolve_export(&package.entrypoints, &key, request))
            .flatten();
        if package.has_exports && request.mode != ResolutionMode::Node10 {
            return Some(resolve_package_target(package, configured, request.mode, false));
        }
        if subpath.is_empty() {
            if let Some(entrypoint) = package.legacy_entrypoints.select(&request.dependency_kind) {
                let candidate = package.root.join(entrypoint.as_str());
                return Some(
                    resolve_file_with_mode(&candidate, request.mode)
                        .map_or(Resolution::Unresolved, Resolution::Module),
                );
            }
        }
        let candidate = if subpath.is_empty() {
            package.root.join("src/index")
        } else {
            package.root.join(subpath)
        };
        Some(
            resolve_file_with_mode(&candidate, request.mode)
                .map_or(Resolution::Unresolved, Resolution::Module),
        )
    }

    fn candidate_paths(&self, request: &ResolutionRequest<'_>) -> Vec<ModulePath> {
        let importer = Path::new(&request.importer.0);
        let (package, target) = if request.specifier.starts_with('#') {
            if request.mode == ResolutionMode::Node10 {
                return Vec::new();
            }
            let Some(package) = self
                .packages
                .iter()
                .filter(|package| importer.starts_with(&package.root))
                .max_by_key(|package| package.root.components().count())
            else {
                return Vec::new();
            };
            (package, resolve_export(&package.imports, request.specifier, request))
        } else {
            let name = package_name(request.specifier);
            let Some(package) = self.packages.iter().find(|package| package.name == name) else {
                return Vec::new();
            };
            let subpath =
                request.specifier.strip_prefix(&name).unwrap_or_default().trim_start_matches('/');
            let key = if subpath.is_empty() { ".".into() } else { format!("./{subpath}") };
            let target = (request.mode != ResolutionMode::Node10)
                .then(|| resolve_export(&package.entrypoints, &key, request))
                .flatten()
                .or_else(|| {
                    ((request.mode == ResolutionMode::Node10 || !package.has_exports)
                        && subpath.is_empty())
                    .then(|| package.legacy_entrypoints.select(&request.dependency_kind))
                    .flatten()
                    .map(|path| PackageTarget::InternalPath(path.as_str().to_owned()))
                })
                .or_else(|| {
                    (request.mode == ResolutionMode::Node10 || !package.has_exports).then(|| {
                        PackageTarget::InternalPath(if subpath.is_empty() {
                            "./src/index".into()
                        } else {
                            format!("./{subpath}")
                        })
                    })
                });
            (package, target)
        };
        match target {
            Some(PackageTarget::InternalPath(target)) => {
                let candidate = package.root.join(target.trim_start_matches("./"));
                if lexically_within(&candidate, &package.root) {
                    resolution_candidates(&candidate, request.mode)
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }
}

#[derive(Deserialize)]
struct PnpmWorkspace {
    #[serde(default)]
    packages: Vec<String>,
}

fn workspace_patterns(project_root: &Path) -> Result<Vec<String>, String> {
    let mut patterns = Vec::new();
    let manifest_path = project_root.join("package.json");
    if manifest_path.exists() {
        let source = fs::read_to_string(&manifest_path)
            .map_err(|error| format!("cannot read `{}`: {error}", manifest_path.display()))?;
        let manifest: serde_json::Value = serde_json::from_str(&source).map_err(|error| {
            format!("invalid package manifest `{}`: {error}", manifest_path.display())
        })?;
        match manifest.get("workspaces") {
            Some(serde_json::Value::Array(values)) => {
                patterns.extend(values.iter().filter_map(|value| value.as_str().map(str::to_owned)))
            }
            Some(serde_json::Value::Object(value)) => {
                if let Some(values) = value.get("packages").and_then(|value| value.as_array()) {
                    patterns.extend(
                        values.iter().filter_map(|value| value.as_str().map(str::to_owned)),
                    );
                }
            }
            _ => {}
        }
    }
    let pnpm_path = project_root.join("pnpm-workspace.yaml");
    if pnpm_path.exists() {
        let source = fs::read_to_string(&pnpm_path)
            .map_err(|error| format!("cannot read `{}`: {error}", pnpm_path.display()))?;
        let workspace: PnpmWorkspace = yaml_serde::from_str(&source).map_err(|error| {
            format!("invalid pnpm workspace `{}`: {error}", pnpm_path.display())
        })?;
        patterns.extend(workspace.packages);
    }
    Ok(patterns)
}

fn compile_workspace_patterns(
    patterns: &[String],
) -> Result<(Vec<GlobMatcher>, Vec<GlobMatcher>), String> {
    let mut includes = Vec::new();
    let mut excludes = Vec::new();
    for pattern in patterns {
        let (target, pattern) = if let Some(pattern) = pattern.strip_prefix('!') {
            (&mut excludes, pattern)
        } else {
            (&mut includes, pattern.as_str())
        };
        let matcher = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .map_err(|error| format!("invalid workspace pattern `{pattern}`: {error}"))?
            .compile_matcher();
        target.push(matcher);
    }
    Ok((includes, excludes))
}
