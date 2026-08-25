use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use wae_config::ResolutionMode;

use super::workspace_index::WorkspacePackage;
use crate::{Resolution, ResolutionRequest, conditions, resolve_file_with_mode};

/// Target grammar for modern `exports` and `imports`. Legacy package-relative entrypoints use the
/// separate `PackageRelativePath` value object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PackageTarget {
    InternalPath(String),
    ExternalSpecifier(String),
    Blocked,
}

pub(crate) fn manifest_imports(
    manifest: &serde_json::Value,
) -> BTreeMap<String, serde_json::Value> {
    manifest
        .get("imports")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flatten()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

pub(crate) fn manifest_entrypoints(
    manifest: &serde_json::Value,
) -> BTreeMap<String, serde_json::Value> {
    let mut entries = BTreeMap::new();
    match manifest.get("exports") {
        Some(
            value @ (serde_json::Value::String(_)
            | serde_json::Value::Array(_)
            | serde_json::Value::Null),
        ) => {
            entries.insert(".".into(), value.clone());
        }
        Some(serde_json::Value::Object(exports)) => {
            if exports.keys().any(|key| key.starts_with('.')) {
                entries.extend(
                    exports
                        .iter()
                        .filter(|(key, _)| key.starts_with('.'))
                        .map(|(key, value)| (key.clone(), value.clone())),
                );
            } else {
                entries.insert(".".into(), serde_json::Value::Object(exports.clone()));
            }
        }
        _ => {}
    }
    entries
}

fn export_target(
    value: &serde_json::Value,
    request: &ResolutionRequest<'_>,
) -> Option<PackageTarget> {
    match value {
        serde_json::Value::String(value) if value.starts_with("./") => {
            Some(PackageTarget::InternalPath(value.clone()))
        }
        serde_json::Value::String(value) => Some(PackageTarget::ExternalSpecifier(value.clone())),
        serde_json::Value::Null => Some(PackageTarget::Blocked),
        serde_json::Value::Array(targets) => {
            let mut blocked = false;
            for target in targets {
                match export_target(target, request) {
                    Some(
                        path @ (PackageTarget::InternalPath(_)
                        | PackageTarget::ExternalSpecifier(_)),
                    ) => return Some(path),
                    Some(PackageTarget::Blocked) => blocked = true,
                    None => {}
                }
            }
            blocked.then_some(PackageTarget::Blocked)
        }
        serde_json::Value::Object(conditions_map) => {
            conditions_map.iter().find_map(|(condition, value)| {
                conditions::active_conditions(request)
                    .contains(condition)
                    .then(|| export_target(value, request))
                    .flatten()
            })
        }
        _ => None,
    }
}

pub(crate) fn resolve_export(
    entries: &BTreeMap<String, serde_json::Value>,
    key: &str,
    request: &ResolutionRequest<'_>,
) -> Option<PackageTarget> {
    if let Some(target) = entries.get(key) {
        return export_target(target, request);
    }
    entries
        .iter()
        .filter_map(|(pattern, target)| {
            let (prefix, suffix) = pattern.split_once('*')?;
            if !key.starts_with(prefix) || !key.ends_with(suffix) {
                return None;
            }
            Some((prefix.len() + suffix.len(), prefix.len(), prefix, suffix, target))
        })
        .max_by_key(|(specificity, prefix_len, ..)| (*specificity, *prefix_len))
        .and_then(|(_, _, prefix, suffix, target)| {
            let capture = &key[prefix.len()..key.len() - suffix.len()];
            export_target(target, request).map(|target| match target {
                PackageTarget::InternalPath(path) => {
                    PackageTarget::InternalPath(path.replace('*', capture))
                }
                PackageTarget::ExternalSpecifier(path) => {
                    PackageTarget::ExternalSpecifier(path.replace('*', capture))
                }
                PackageTarget::Blocked => PackageTarget::Blocked,
            })
        })
}

pub(crate) fn resolve_package_target(
    package: &WorkspacePackage,
    target: Option<PackageTarget>,
    mode: ResolutionMode,
    allow_external: bool,
) -> Resolution {
    let Some(target) = target else { return Resolution::Unresolved };
    let target = match target {
        PackageTarget::InternalPath(target) => target,
        PackageTarget::ExternalSpecifier(target) if allow_external => {
            return Resolution::Redirect(target);
        }
        PackageTarget::ExternalSpecifier(_) | PackageTarget::Blocked => {
            return Resolution::Unresolved;
        }
    };
    let candidate = package.root.join(target.trim_start_matches("./"));
    if !lexically_within(&candidate, &package.root) {
        return Resolution::Unresolved;
    }
    resolve_file_with_mode(&candidate, mode).map_or(Resolution::Unresolved, Resolution::Module)
}

pub(crate) fn lexically_within(path: &Path, directory: &Path) -> bool {
    lexical_path(path).starts_with(lexical_path(directory))
}

fn lexical_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}
