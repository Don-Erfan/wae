use std::path::{Component, Path};

use wae_core::domain::DependencyKind;

/// A package-relative legacy entrypoint (`main`, `module`, `types`, or `typings`). Unlike an
/// `exports` target, Node manifests do not require this value to start with `./`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageRelativePath(String);

impl PackageRelativePath {
    pub fn parse(value: &str) -> Option<Self> {
        let path = Path::new(value);
        if value.trim().is_empty() || path.is_absolute() {
            return None;
        }
        let mut depth = 0_usize;
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::Normal(_) => depth += 1,
                Component::ParentDir if depth > 0 => depth -= 1,
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
            }
        }
        Some(Self(value.trim_start_matches("./").replace('\\', "/")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Legacy package entrypoints are intentionally modeled separately from `exports`/`imports` so
/// their path grammar and dependency-kind precedence cannot be conflated.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LegacyEntrypoints {
    runtime: Option<PackageRelativePath>,
    types: Option<PackageRelativePath>,
}

impl LegacyEntrypoints {
    pub fn from_manifest(manifest: &serde_json::Value) -> Self {
        let string_field = |name: &str| {
            manifest
                .get(name)
                .and_then(serde_json::Value::as_str)
                .and_then(PackageRelativePath::parse)
        };
        Self {
            runtime: string_field("module").or_else(|| string_field("main")),
            types: string_field("types").or_else(|| string_field("typings")),
        }
    }

    pub fn select(&self, dependency_kind: &DependencyKind) -> Option<&PackageRelativePath> {
        if *dependency_kind == DependencyKind::TypeOnly {
            self.types.as_ref().or(self.runtime.as_ref())
        } else {
            self.runtime.as_ref()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_paths_do_not_require_export_target_prefixes() {
        let manifest: serde_json::Value = serde_json::from_str(
            r#"{"module":"esm/index.js","main":"dist/index.js","types":"dist/index.d.ts"}"#,
        )
        .unwrap();
        let entrypoints = LegacyEntrypoints::from_manifest(&manifest);
        assert_eq!(entrypoints.select(&DependencyKind::Static).unwrap().as_str(), "esm/index.js");
        assert_eq!(
            entrypoints.select(&DependencyKind::TypeOnly).unwrap().as_str(),
            "dist/index.d.ts"
        );
    }

    #[test]
    fn rejects_absolute_and_escaping_legacy_paths() {
        assert!(PackageRelativePath::parse("../outside.js").is_none());
        assert!(PackageRelativePath::parse("/outside.js").is_none());
    }
}
