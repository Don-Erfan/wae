use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use wae_config::ResolutionMode;
use wae_core::domain::ModulePath;

mod conditions;
mod package;
mod package_scope;
mod relative;
mod request;
mod resolution_kind;
pub use conditions::{
    BundlerConditions, ConditionSet, ConditionSetProvider, Node16Conditions, NodeNextConditions,
};
pub use package::{
    LegacyEntrypoints, PackageModuleType, PackageRelativePath, WorkspacePackage,
    WorkspacePackageIndex,
};
pub use package_scope::{PackageScope, PackageScopeIndex};
use relative::{resolution_candidates, resolve_relative};
pub use relative::{resolve_file, resolve_file_with_mode};
pub use request::{
    ModuleFormat, ModuleResolver, Resolution, ResolutionHandler, ResolutionKind, ResolutionRequest,
};
pub use resolution_kind::{
    BundlerResolutionKindProvider, DependencySyntax, Node10ResolutionKindProvider,
    Node16ResolutionKindProvider, NodeNextResolutionKindProvider, ResolutionKindProvider,
    resolution_kind_for,
};

#[derive(Default)]
pub struct ResolverPipeline {
    handlers: Vec<Box<dyn ResolutionHandler>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolutionAttempt {
    pub specifier: String,
    pub handler: &'static str,
    pub outcome: Option<Resolution>,
}

impl ResolverPipeline {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_handler<H: ResolutionHandler + 'static>(mut self, handler: H) -> Self {
        self.handlers.push(Box::new(handler));
        self
    }

    pub fn candidate_paths(&self, request: &ResolutionRequest<'_>) -> Vec<ModulePath> {
        let mut candidates = self
            .handlers
            .iter()
            .flat_map(|handler| handler.candidate_paths(request))
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| left.0.cmp(&right.0));
        candidates.dedup();
        candidates
    }

    pub fn resolve_with_trace(
        &self,
        request: &ResolutionRequest<'_>,
    ) -> (Resolution, Vec<ResolutionAttempt>) {
        let mut specifier = request.specifier.to_string();
        let mut visited = BTreeSet::new();
        let mut attempts = Vec::new();
        loop {
            if !visited.insert(specifier.clone()) {
                return (
                    Resolution::Invalid(format!(
                        "package resolution redirect loop at `{specifier}`"
                    )),
                    attempts,
                );
            }
            let redirected = ResolutionRequest {
                importer: request.importer,
                specifier: &specifier,
                dependency_kind: request.dependency_kind.clone(),
                resolution_kind: request.resolution_kind,
                importer_format: request.importer_format,
                mode: request.mode,
                custom_conditions: request.custom_conditions,
            };
            let mut result = Resolution::Unresolved;
            for handler in &self.handlers {
                let outcome = handler.try_resolve(&redirected);
                attempts.push(ResolutionAttempt {
                    specifier: specifier.clone(),
                    handler: handler.name(),
                    outcome: outcome.clone(),
                });
                if let Some(outcome) = outcome {
                    result = outcome;
                    break;
                }
            }
            match result {
                Resolution::Redirect(target) => specifier = target,
                result => return (result, attempts),
            }
        }
    }
    pub fn node_defaults(root: impl Into<PathBuf>, aliases: Vec<PathAlias>) -> Self {
        Self::node_defaults_with_mode(root, aliases, ResolutionMode::NodeNext)
    }

    pub fn node_defaults_with_mode(
        root: impl Into<PathBuf>,
        aliases: Vec<PathAlias>,
        mode: ResolutionMode,
    ) -> Self {
        Self::new()
            .with_handler(RelativeResolver { mode })
            .with_handler(AliasResolver { root: root.into(), aliases, mode })
            .with_handler(PackageResolver)
    }

    pub fn node_with_workspaces(
        root: impl Into<PathBuf>,
        aliases: Vec<PathAlias>,
        workspaces: WorkspacePackageIndex,
        mode: ResolutionMode,
    ) -> Self {
        Self::new()
            .with_handler(RelativeResolver { mode })
            .with_handler(AliasResolver { root: root.into(), aliases, mode })
            .with_handler(workspaces)
            .with_handler(PackageResolver)
    }

    pub fn indexed_node_with_workspaces(
        tsconfigs: TsConfigIndex,
        workspaces: WorkspacePackageIndex,
        mode: ResolutionMode,
    ) -> Self {
        Self::new()
            .with_handler(RelativeResolver { mode })
            .with_handler(IndexedAliasResolver::new(tsconfigs))
            .with_handler(workspaces)
            .with_handler(PackageResolver)
    }
}

impl ModuleResolver for ResolverPipeline {
    fn resolve(&self, request: &ResolutionRequest<'_>) -> Resolution {
        self.resolve_with_trace(request).0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathAlias {
    pub pattern: String,
    pub targets: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct RelativeResolver {
    pub mode: ResolutionMode,
}
impl ResolutionHandler for RelativeResolver {
    fn name(&self) -> &'static str {
        "relative"
    }
    fn try_resolve(&self, request: &ResolutionRequest<'_>) -> Option<Resolution> {
        let importer = Path::new(&request.importer.0);
        let specifier = request.specifier;
        if !specifier.starts_with('.') && !specifier.starts_with('/') {
            return None;
        }
        if specifier.starts_with('/') {
            return Some(Resolution::Invalid(
                "absolute module specifiers are outside the project analysis boundary".into(),
            ));
        }
        let base = importer.parent()?.join(specifier);
        Some(resolve_relative(&base, request).map_or(Resolution::Unresolved, Resolution::Module))
    }

    fn candidate_paths(&self, request: &ResolutionRequest<'_>) -> Vec<ModulePath> {
        if !request.specifier.starts_with('.') {
            return Vec::new();
        }
        Path::new(&request.importer.0)
            .parent()
            .map(|parent| resolution_candidates(&parent.join(request.specifier), request.mode))
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
pub struct AliasResolver {
    pub root: PathBuf,
    pub aliases: Vec<PathAlias>,
    pub mode: ResolutionMode,
}
impl ResolutionHandler for AliasResolver {
    fn name(&self) -> &'static str {
        "alias"
    }
    fn try_resolve(&self, request: &ResolutionRequest<'_>) -> Option<Resolution> {
        let specifier = request.specifier;
        let mut matched = false;
        for alias in &self.aliases {
            let capture = match alias.pattern.split_once('*') {
                Some((prefix, suffix))
                    if specifier.starts_with(prefix) && specifier.ends_with(suffix) =>
                {
                    Some(&specifier[prefix.len()..specifier.len() - suffix.len()])
                }
                None if alias.pattern == specifier => Some(""),
                _ => None,
            };
            let Some(capture) = capture else { continue };
            matched = true;
            for target in &alias.targets {
                let candidate = self.root.join(target.replace('*', capture));
                if let Some(path) = resolve_file_with_mode(&candidate, request.mode) {
                    return Some(Resolution::Module(path));
                }
            }
        }
        matched.then_some(Resolution::Unresolved)
    }

    fn candidate_paths(&self, request: &ResolutionRequest<'_>) -> Vec<ModulePath> {
        let mut candidates = Vec::new();
        for alias in &self.aliases {
            let capture = match alias.pattern.split_once('*') {
                Some((prefix, suffix))
                    if request.specifier.starts_with(prefix)
                        && request.specifier.ends_with(suffix) =>
                {
                    Some(&request.specifier[prefix.len()..request.specifier.len() - suffix.len()])
                }
                None if alias.pattern == request.specifier => Some(""),
                _ => None,
            };
            if let Some(capture) = capture {
                for target in &alias.targets {
                    candidates.extend(resolution_candidates(
                        &self.root.join(target.replace('*', capture)),
                        request.mode,
                    ));
                }
            }
        }
        candidates
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TsConfigPaths {
    pub base_url: PathBuf,
    pub aliases: Vec<PathAlias>,
}

pub struct TsConfigLoader;

impl TsConfigLoader {
    pub fn load(project_root: &Path) -> Result<TsConfigPaths, String> {
        let path = ["tsconfig.json", "jsconfig.json"]
            .into_iter()
            .map(|name| project_root.join(name))
            .find(|path| path.is_file());
        let Some(path) = path else {
            return Ok(TsConfigPaths { base_url: project_root.to_path_buf(), aliases: Vec::new() });
        };
        let mut visited = BTreeSet::new();
        let resolved = load_tsconfig(&path, &mut visited)?;
        let mut aliases = resolved
            .aliases
            .into_iter()
            .map(|(pattern, targets)| PathAlias { pattern, targets })
            .collect::<Vec<_>>();
        aliases.sort_by(|left, right| {
            alias_specificity(&right.pattern).cmp(&alias_specificity(&left.pattern))
        });
        Ok(TsConfigPaths { base_url: PathBuf::new(), aliases })
    }
}

/// Immutable index of TypeScript project configurations. The nearest ancestor
/// configuration owns an importer, matching TypeScript's configured-project model.
#[derive(Clone, Debug, Default)]
pub struct TsConfigIndex {
    configs: Vec<ScopedTsConfig>,
}

#[derive(Clone, Debug)]
struct ScopedTsConfig {
    directory: PathBuf,
    paths: TsConfigPaths,
}

impl TsConfigIndex {
    pub fn discover(project_root: &Path) -> Result<Self, String> {
        let mut configs = Vec::new();
        let mut directories = BTreeSet::new();
        let mut builder = ignore::WalkBuilder::new(project_root);
        builder.hidden(false).git_ignore(true).git_global(true).git_exclude(true);
        builder.filter_entry(|entry| entry.file_name() != "node_modules");
        for entry in builder.build() {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry.file_type().is_some_and(|kind| kind.is_file())
                && matches!(entry.file_name().to_str(), Some("tsconfig.json" | "jsconfig.json"))
            {
                let directory = entry.path().parent().unwrap_or(project_root).to_path_buf();
                directories.insert(directory);
            }
        }
        for directory in directories {
            configs.push(ScopedTsConfig { paths: TsConfigLoader::load(&directory)?, directory });
        }
        if configs.is_empty() {
            configs.push(ScopedTsConfig {
                directory: project_root.to_path_buf(),
                paths: TsConfigPaths { base_url: project_root.to_path_buf(), aliases: Vec::new() },
            });
        }
        configs.sort_by(|left, right| {
            right
                .directory
                .components()
                .count()
                .cmp(&left.directory.components().count())
                .then_with(|| left.directory.cmp(&right.directory))
        });
        Ok(Self { configs })
    }

    pub fn single(directory: PathBuf, paths: TsConfigPaths) -> Self {
        Self { configs: vec![ScopedTsConfig { directory, paths }] }
    }

    fn paths_for(&self, importer: &Path) -> Option<&TsConfigPaths> {
        self.configs
            .iter()
            .find(|config| importer.starts_with(&config.directory))
            .map(|config| &config.paths)
    }
}

#[derive(Clone, Debug)]
pub struct IndexedAliasResolver {
    index: TsConfigIndex,
}

impl IndexedAliasResolver {
    pub fn new(index: TsConfigIndex) -> Self {
        Self { index }
    }
}

impl ResolutionHandler for IndexedAliasResolver {
    fn name(&self) -> &'static str {
        "tsconfig-alias"
    }
    fn try_resolve(&self, request: &ResolutionRequest<'_>) -> Option<Resolution> {
        let importer = Path::new(&request.importer.0);
        let paths = self.index.paths_for(importer)?;
        AliasResolver {
            root: paths.base_url.clone(),
            aliases: paths.aliases.clone(),
            mode: request.mode,
        }
        .try_resolve(request)
    }

    fn candidate_paths(&self, request: &ResolutionRequest<'_>) -> Vec<ModulePath> {
        let importer = Path::new(&request.importer.0);
        let Some(paths) = self.index.paths_for(importer) else { return Vec::new() };
        AliasResolver {
            root: paths.base_url.clone(),
            aliases: paths.aliases.clone(),
            mode: request.mode,
        }
        .candidate_paths(request)
    }
}

#[derive(Default)]
struct ResolvedTsConfig {
    base_url: PathBuf,
    aliases: BTreeMap<String, Vec<String>>,
}

fn load_tsconfig(path: &Path, visited: &mut BTreeSet<PathBuf>) -> Result<ResolvedTsConfig, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("cannot open tsconfig `{}`: {error}", path.display()))?;
    if !visited.insert(canonical.clone()) {
        return Err(format!("circular tsconfig extends chain at `{}`", canonical.display()));
    }
    let source = fs::read_to_string(&canonical)
        .map_err(|error| format!("cannot read tsconfig `{}`: {error}", canonical.display()))?;
    let json: serde_json::Value = serde_json::from_str(&strip_jsonc(&source))
        .map_err(|error| format!("invalid JSONC in tsconfig `{}`: {error}", canonical.display()))?;
    let directory = canonical.parent().unwrap_or(Path::new("."));
    let mut resolved = match json.get("extends") {
        Some(serde_json::Value::String(parent)) => {
            let parent_path = resolve_extends(directory, parent)?;
            load_tsconfig(&parent_path, visited)?
        }
        Some(_) => {
            return Err(format!(
                "tsconfig `extends` in `{}` must be a single string; arrays are not supported",
                canonical.display()
            ));
        }
        None => ResolvedTsConfig { base_url: directory.to_path_buf(), aliases: BTreeMap::new() },
    };
    let compiler = &json["compilerOptions"];
    if let Some(base_url) = compiler.get("baseUrl").and_then(|value| value.as_str()) {
        resolved.base_url = directory.join(base_url);
    }
    if let Some(paths) = compiler.get("paths").and_then(|value| value.as_object()) {
        for (pattern, targets) in paths {
            let targets = targets
                .as_array()
                .ok_or_else(|| format!("tsconfig path `{pattern}` must be an array"))?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(|target| normalize(&resolved.base_url.join(target)))
                        .ok_or_else(|| {
                            format!("tsconfig path `{pattern}` contains a non-string target")
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            resolved.aliases.insert(pattern.clone(), targets);
        }
    }
    visited.remove(&canonical);
    Ok(resolved)
}

fn resolve_extends(directory: &Path, value: &str) -> Result<PathBuf, String> {
    if value.starts_with('.') || value.starts_with('/') {
        return finalize_extended_config(directory.join(value), value);
    }
    for ancestor in directory.ancestors() {
        let candidate = ancestor.join("node_modules").join(value);
        if let Ok(path) = finalize_extended_config(candidate, value) {
            return Ok(path);
        }
    }
    Err(format!(
        "cannot resolve extended tsconfig `{value}` from `{}` or an ancestor node_modules directory",
        directory.display()
    ))
}

fn finalize_extended_config(mut candidate: PathBuf, value: &str) -> Result<PathBuf, String> {
    if candidate.is_dir() {
        let manifest = candidate.join("package.json");
        candidate = fs::read_to_string(&manifest)
            .ok()
            .and_then(|source| serde_json::from_str::<serde_json::Value>(&source).ok())
            .and_then(|json| {
                json.get("tsconfig").and_then(|value| value.as_str()).map(str::to_owned)
            })
            .map_or_else(|| candidate.join("tsconfig.json"), |path| candidate.join(path));
    } else if !candidate.exists() {
        let with_json = PathBuf::from(format!("{}.json", candidate.to_string_lossy()));
        if with_json.exists() {
            candidate = with_json;
        }
    }
    candidate
        .exists()
        .then_some(candidate)
        .ok_or_else(|| format!("cannot resolve extended tsconfig `{value}`"))
}

fn alias_specificity(pattern: &str) -> (bool, usize, usize) {
    match pattern.split_once('*') {
        Some((prefix, suffix)) => (false, prefix.len(), suffix.len()),
        None => (true, pattern.len(), 0),
    }
}

fn strip_jsonc(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut clean = Vec::with_capacity(bytes.len());
    let mut index = 0;
    let mut in_string = false;
    while index < bytes.len() {
        if in_string {
            clean.push(bytes[index]);
            if bytes[index] == b'\\' && index + 1 < bytes.len() {
                index += 1;
                clean.push(bytes[index]);
            } else if bytes[index] == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if bytes[index] == b'"' {
            in_string = true;
            clean.push(bytes[index]);
            index += 1;
        } else if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            clean.extend_from_slice(b"  ");
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                clean.push(b' ');
                index += 1;
            }
        } else if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            clean.extend_from_slice(b"  ");
            index += 2;
            while index < bytes.len() {
                if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
                    clean.extend_from_slice(b"  ");
                    index += 2;
                    break;
                }
                clean.push(if bytes[index] == b'\n' { b'\n' } else { b' ' });
                index += 1;
            }
        } else {
            clean.push(bytes[index]);
            index += 1;
        }
    }
    remove_trailing_commas(String::from_utf8(clean).unwrap_or_default())
}

fn remove_trailing_commas(source: String) -> String {
    let bytes = source.as_bytes();
    let mut output = String::with_capacity(source.len());
    let mut index = 0;
    let mut in_string = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'"' && (index == 0 || bytes[index - 1] != b'\\') {
            in_string = !in_string;
        }
        if !in_string && byte == b',' {
            let next = bytes[index + 1..].iter().copied().find(|byte| !byte.is_ascii_whitespace());
            if matches!(next, Some(b'}' | b']')) {
                index += 1;
                continue;
            }
        }
        output.push(char::from(byte));
        index += 1;
    }
    output
}

#[derive(Clone, Copy, Debug)]
pub struct PackageResolver;
impl ResolutionHandler for PackageResolver {
    fn name(&self) -> &'static str {
        "external-package"
    }
    fn try_resolve(&self, request: &ResolutionRequest<'_>) -> Option<Resolution> {
        let specifier = request.specifier;
        (!specifier.starts_with('.') && !specifier.starts_with('/'))
            .then(|| Resolution::External(package_name(specifier)))
    }
}

/// Compatibility alias retained for integrations compiled against the pre-0.0.11 name.
#[deprecated(note = "use WorkspacePackageIndex; package scopes are indexed separately")]
pub type WorkspaceResolver = WorkspacePackageIndex;

fn normalize(path: &Path) -> String {
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
    result.to_string_lossy().replace('\\', "/")
}

fn package_name(specifier: &str) -> String {
    if specifier.starts_with('@') {
        specifier.split('/').take(2).collect::<Vec<_>>().join("/")
    } else {
        specifier.split('/').next().unwrap_or(specifier).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use wae_core::domain::DependencyKind;

    fn resolve_with(
        resolver: &dyn ResolutionHandler,
        importer: &Path,
        specifier: &str,
    ) -> Option<Resolution> {
        resolve_kind_with(resolver, importer, specifier, DependencyKind::Static)
    }

    fn resolve_kind_with(
        resolver: &dyn ResolutionHandler,
        importer: &Path,
        specifier: &str,
        dependency_kind: DependencyKind,
    ) -> Option<Resolution> {
        let resolution_kind = match dependency_kind {
            DependencyKind::Require => ResolutionKind::Require,
            _ => ResolutionKind::Import,
        };
        resolve_request_with(
            resolver,
            importer,
            specifier,
            dependency_kind,
            resolution_kind,
            ModuleFormat::CommonJs,
            ResolutionMode::NodeNext,
        )
    }

    fn resolve_request_with(
        resolver: &dyn ResolutionHandler,
        importer: &Path,
        specifier: &str,
        dependency_kind: DependencyKind,
        resolution_kind: ResolutionKind,
        importer_format: ModuleFormat,
        mode: ResolutionMode,
    ) -> Option<Resolution> {
        let importer = ModulePath(importer.to_string_lossy().into_owned());
        resolver.try_resolve(&ResolutionRequest {
            importer: &importer,
            specifier,
            dependency_kind,
            resolution_kind,
            importer_format,
            mode,
            custom_conditions: &[],
        })
    }

    #[test]
    fn resolves_extensions_and_index_modules() {
        let root = std::env::temp_dir().join(format!("wae-resolver-{}", std::process::id()));
        fs::create_dir_all(root.join("folder")).unwrap();
        fs::write(root.join("folder/index.ts"), "").unwrap();
        let importer = ModulePath(root.join("a.ts").to_string_lossy().into_owned());
        assert!(matches!(
            resolve_with(
                &RelativeResolver { mode: ResolutionMode::NodeNext },
                Path::new(&importer.0),
                "./folder"
            ),
            Some(Resolution::Module(_))
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn alias_handler_substitutes_wildcards() {
        let root = std::env::temp_dir().join(format!("wae-alias-{}", std::process::id()));
        fs::create_dir_all(root.join("src/shared")).unwrap();
        fs::write(root.join("src/shared/util.ts"), "").unwrap();
        let resolver = AliasResolver {
            root: root.clone(),
            aliases: vec![PathAlias { pattern: "@/*".into(), targets: vec!["src/*".into()] }],
            mode: ResolutionMode::NodeNext,
        };
        assert!(matches!(
            resolve_with(&resolver, Path::new("src/a.ts"), "@/shared/util"),
            Some(Resolution::Module(_))
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tsconfig_loader_supports_jsonc_extends_and_specific_aliases() {
        let root = std::env::temp_dir().join(format!("wae-tsconfig-{}", std::process::id()));
        fs::create_dir_all(root.join("src/features/auth")).unwrap();
        fs::create_dir_all(root.join("src/shared")).unwrap();
        fs::write(root.join("src/features/auth/index.ts"), "").unwrap();
        fs::write(root.join("src/shared/auth.ts"), "").unwrap();
        fs::write(
            root.join("tsconfig.base.json"),
            r##"{
              // inherited compiler settings
              "compilerOptions": {
                "baseUrl": ".",
                "paths": { "@/*": ["src/*"], },
              }
            }"##,
        )
        .unwrap();
        fs::write(
            root.join("tsconfig.json"),
            r#"{
              "extends": "./tsconfig.base",
              "compilerOptions": {
                "paths": { "@/auth/*": ["src/features/auth/*"] }
              }
            }"#,
        )
        .unwrap();

        let loaded = TsConfigLoader::load(&root).unwrap();
        assert_eq!(loaded.aliases[0].pattern, "@/auth/*");
        let resolver = AliasResolver {
            root: loaded.base_url,
            aliases: loaded.aliases,
            mode: ResolutionMode::NodeNext,
        };
        assert!(matches!(
            resolve_with(&resolver, Path::new("src/app.ts"), "@/auth/index"),
            Some(Resolution::Module(path)) if path.0.ends_with("src/features/auth/index.ts")
        ));
        assert!(matches!(
            resolve_with(&resolver, Path::new("src/app.ts"), "@/shared/auth"),
            Some(Resolution::Module(path)) if path.0.ends_with("src/shared/auth.ts")
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_tsconfig_is_an_explicit_error() {
        let root = std::env::temp_dir().join(format!("wae-bad-tsconfig-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("tsconfig.json"), "{ invalid }").unwrap();
        let error = TsConfigLoader::load(&root).unwrap_err();
        assert!(error.contains("invalid JSONC"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn jsconfig_aliases_are_discovered_and_tsconfig_wins_in_the_same_directory() {
        let root = std::env::temp_dir().join(format!("wae-jsconfig-{}", std::process::id()));
        fs::create_dir_all(root.join("web")).unwrap();
        fs::create_dir_all(root.join("src/js")).unwrap();
        fs::create_dir_all(root.join("src/ts")).unwrap();
        fs::write(root.join("src/js/value.js"), "").unwrap();
        fs::write(root.join("src/ts/value.ts"), "").unwrap();
        fs::write(
            root.join("jsconfig.json"),
            r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/js/*"]}}}"#,
        )
        .unwrap();
        let loaded = TsConfigLoader::load(&root).unwrap();
        assert_eq!(loaded.aliases[0].targets[0], normalize(&root.join("src/js/*")));

        fs::write(
            root.join("tsconfig.json"),
            r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/ts/*"]}}}"#,
        )
        .unwrap();
        let index = TsConfigIndex::discover(&root).unwrap();
        let resolver = IndexedAliasResolver::new(index);
        assert!(matches!(
            resolve_with(&resolver, &root.join("web/app.ts"), "@/value"),
            Some(Resolution::Module(path)) if path.0.ends_with("src/ts/value.ts")
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn package_tsconfig_extends_resolves_from_ancestor_node_modules() {
        let root =
            std::env::temp_dir().join(format!("wae-hoisted-tsconfig-{}", std::process::id()));
        let config_package = root.join("node_modules/@acme/tsconfig");
        let app = root.join("packages/app");
        fs::create_dir_all(&config_package).unwrap();
        fs::create_dir_all(app.join("src")).unwrap();
        fs::write(
            config_package.join("package.json"),
            r#"{"name":"@acme/tsconfig","tsconfig":"base.json"}"#,
        )
        .unwrap();
        fs::write(
            config_package.join("base.json"),
            r#"{"compilerOptions":{"baseUrl":"../../../","paths":{"@shared/*":["shared/*"]}}}"#,
        )
        .unwrap();
        fs::write(app.join("tsconfig.json"), r#"{"extends":"@acme/tsconfig"}"#).unwrap();
        let loaded = TsConfigLoader::load(&app).unwrap();
        assert_eq!(loaded.aliases[0].pattern, "@shared/*");
        assert!(loaded.aliases[0].targets[0].ends_with("shared/*"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tsconfig_extends_arrays_are_explicitly_rejected() {
        let root = std::env::temp_dir().join(format!("wae-array-tsconfig-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("tsconfig.json"), r#"{"extends":["./base.json"]}"#).unwrap();
        let error = TsConfigLoader::load(&root).unwrap_err();
        assert!(error.contains("must be a single string"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolves_workspace_exports_and_subpaths_before_external_packages() {
        let root = std::env::temp_dir().join(format!("wae-workspace-{}", std::process::id()));
        let package = root.join("packages/ui");
        fs::create_dir_all(package.join("src/components")).unwrap();
        fs::write(
            root.join("package.json"),
            r#"{"name":"repo","private":true,"workspaces":["packages/*"]}"#,
        )
        .unwrap();
        fs::write(package.join("src/index.ts"), "").unwrap();
        fs::write(package.join("src/components/button.ts"), "").unwrap();
        fs::write(
            package.join("package.json"),
            r##"{
              "name": "@acme/ui",
              "imports": { "#internal/*": "./src/components/*.ts" },
              "exports": {
                ".": "./src/index.ts",
                "./components/*": "./src/components/*.ts"
              }
            }"##,
        )
        .unwrap();
        let resolver = WorkspacePackageIndex::discover(&root).unwrap();
        assert!(matches!(
            resolve_with(&resolver, Path::new("apps/web/src/app.ts"), "@acme/ui"),
            Some(Resolution::Module(path)) if path.0.ends_with("packages/ui/src/index.ts")
        ));
        assert!(matches!(
            resolve_with(&resolver, &package.join("src/index.ts"), "#internal/button"),
            Some(Resolution::Module(path)) if path.0.ends_with("packages/ui/src/components/button.ts")
        ));
        assert!(matches!(
            resolve_with(&resolver, Path::new("apps/web/src/app.ts"), "@acme/ui/components/button"),
            Some(Resolution::Module(path)) if path.0.ends_with("packages/ui/src/components/button.ts")
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_main_and_types_use_package_relative_paths_and_kind_precedence() {
        let root =
            std::env::temp_dir().join(format!("wae-legacy-entrypoints-{}", std::process::id()));
        let package = root.join("packages/pkg");
        fs::create_dir_all(package.join("dist")).unwrap();
        fs::write(root.join("package.json"), r#"{"workspaces":["packages/*"]}"#).unwrap();
        fs::write(
            package.join("package.json"),
            r#"{"name":"@fixture/pkg","main":"dist/index.js","types":"dist/index.d.ts"}"#,
        )
        .unwrap();
        fs::write(package.join("dist/index.js"), "").unwrap();
        fs::write(package.join("dist/index.d.ts"), "").unwrap();
        let resolver = WorkspacePackageIndex::discover(&root).unwrap();

        assert!(matches!(
            resolve_kind_with(
                &resolver,
                &root.join("src/app.ts"),
                "@fixture/pkg",
                DependencyKind::Static,
            ),
            Some(Resolution::Module(path)) if path.0.ends_with("packages/pkg/dist/index.js")
        ));
        assert!(matches!(
            resolve_kind_with(
                &resolver,
                &root.join("src/app.ts"),
                "@fixture/pkg",
                DependencyKind::TypeOnly,
            ),
            Some(Resolution::Module(path)) if path.0.ends_with("packages/pkg/dist/index.d.ts")
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn node10_ignores_exports_and_maps_javascript_extensions() {
        let root = std::env::temp_dir().join(format!("wae-node10-{}", std::process::id()));
        let package = root.join("packages/pkg");
        fs::create_dir_all(package.join("dist")).unwrap();
        fs::write(root.join("package.json"), r#"{"workspaces":["packages/*"]}"#).unwrap();
        fs::write(
            package.join("package.json"),
            r#"{"name":"pkg","exports":"./dist/modern.js","main":"dist/legacy.js"}"#,
        )
        .unwrap();
        fs::write(package.join("dist/modern.ts"), "").unwrap();
        fs::write(package.join("dist/legacy.ts"), "").unwrap();
        let resolver = WorkspacePackageIndex::discover(&root).unwrap();
        assert!(matches!(
            resolve_request_with(
                &resolver,
                &root.join("src/app.ts"),
                "pkg",
                DependencyKind::Static,
                ResolutionKind::Import,
                ModuleFormat::CommonJs,
                ResolutionMode::Node10,
            ),
            Some(Resolution::Module(path)) if path.0.ends_with("packages/pkg/dist/legacy.ts")
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn nodenext_maps_javascript_specifiers_and_preserves_dotted_basenames() {
        let root = std::env::temp_dir().join(format!("wae-nodenext-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("b.ts"), "").unwrap();
        fs::write(root.join("config.prod.ts"), "").unwrap();
        let resolver = RelativeResolver { mode: ResolutionMode::NodeNext };
        assert!(matches!(
            resolve_with(&resolver, &root.join("a.ts"), "./b.js"),
            Some(Resolution::Module(path)) if path.0.ends_with("b.ts")
        ));
        assert!(matches!(
            resolve_with(&resolver, &root.join("a.ts"), "./config.prod"),
            Some(Resolution::Module(path)) if path.0.ends_with("config.prod.ts")
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn nodenext_esm_requires_explicit_relative_extensions() {
        let root = std::env::temp_dir().join(format!("wae-nodenext-esm-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("value.ts"), "").unwrap();
        let resolver = RelativeResolver { mode: ResolutionMode::NodeNext };
        assert_eq!(
            resolve_request_with(
                &resolver,
                &root.join("app.mts"),
                "./value",
                DependencyKind::Static,
                ResolutionKind::Import,
                ModuleFormat::Esm,
                ResolutionMode::NodeNext,
            ),
            Some(Resolution::Unresolved)
        );
        assert!(matches!(
            resolve_request_with(
                &resolver,
                &root.join("app.mts"),
                "./value.js",
                DependencyKind::Static,
                ResolutionKind::Import,
                ModuleFormat::Esm,
                ResolutionMode::NodeNext,
            ),
            Some(Resolution::Module(path)) if path.0.ends_with("value.ts")
        ));
        for (mode, format) in [
            (ResolutionMode::NodeNext, ModuleFormat::CommonJs),
            (ResolutionMode::Bundler, ModuleFormat::Esm),
        ] {
            let resolver = RelativeResolver { mode };
            assert!(matches!(
                resolve_request_with(
                    &resolver,
                    &root.join("app.ts"),
                    "./value",
                    DependencyKind::Static,
                    ResolutionKind::Import,
                    format,
                    mode,
                ),
                Some(Resolution::Module(_))
            ));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn package_exports_block_unexported_and_escaping_subpaths() {
        let root = std::env::temp_dir().join(format!("wae-exports-{}", std::process::id()));
        let package = root.join("packages/ui");
        fs::create_dir_all(package.join("src/internal")).unwrap();
        fs::create_dir_all(package.join("src/special")).unwrap();
        fs::write(root.join("outside.ts"), "").unwrap();
        fs::write(package.join("src/index.ts"), "").unwrap();
        fs::write(package.join("src/import.ts"), "").unwrap();
        fs::write(package.join("src/types.ts"), "").unwrap();
        fs::write(package.join("src/special/button.ts"), "").unwrap();
        fs::write(package.join("src/internal/secret.ts"), "").unwrap();
        fs::write(
            root.join("package.json"),
            r#"{"name":"repo","private":true,"workspaces":["packages/*"]}"#,
        )
        .unwrap();
        fs::write(
            package.join("package.json"),
            r#"{"name":"@acme/ui","exports":{".":[null,{"types":"./src/index.ts"}],"./conditional":{"import":"./src/import.ts","types":"./src/types.ts"},"./feature/*":"./src/general/*.ts","./feature/special/*":"./src/special/*.ts","./escape":"../outside.ts","./private/*":null}}"#,
        )
        .unwrap();
        let resolver = WorkspacePackageIndex::discover(&root).unwrap();
        assert!(matches!(
            resolve_kind_with(
                &resolver,
                &root.join("app.ts"),
                "@acme/ui",
                DependencyKind::TypeOnly
            ),
            Some(Resolution::Module(_))
        ));
        assert!(matches!(
            resolve_with(&resolver, &root.join("app.ts"), "@acme/ui/conditional"),
            Some(Resolution::Module(path)) if path.0.ends_with("src/import.ts")
        ));
        assert!(matches!(
            resolve_with(&resolver, &root.join("app.ts"), "@acme/ui/feature/special/button"),
            Some(Resolution::Module(path)) if path.0.ends_with("src/special/button.ts")
        ));
        for specifier in ["@acme/ui/internal/secret", "@acme/ui/escape", "@acme/ui/private/secret"]
        {
            assert_eq!(
                resolve_with(&resolver, &root.join("app.ts"), specifier),
                Some(Resolution::Unresolved)
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workspace_index_ignores_packages_outside_declared_patterns() {
        let root = std::env::temp_dir().join(format!("wae-workspace-scope-{}", std::process::id()));
        fs::create_dir_all(root.join("packages/ui/src")).unwrap();
        fs::create_dir_all(root.join("examples/demo/src")).unwrap();
        fs::write(
            root.join("package.json"),
            r#"{"name":"repo","private":true,"workspaces":["packages/*"]}"#,
        )
        .unwrap();
        fs::write(root.join("packages/ui/package.json"), r#"{"name":"@acme/ui"}"#).unwrap();
        fs::write(root.join("examples/demo/package.json"), r#"{"name":"@acme/demo"}"#).unwrap();
        let resolver = WorkspacePackageIndex::discover(&root).unwrap();
        assert!(resolver.packages().iter().any(|package| package.name == "@acme/ui"));
        assert!(!resolver.packages().iter().any(|package| package.name == "@acme/demo"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dependency_kind_selects_only_the_matching_package_condition() {
        let root = std::env::temp_dir().join(format!("wae-conditions-{}", std::process::id()));
        let package = root.join("packages/lib");
        fs::create_dir_all(package.join("src")).unwrap();
        fs::write(root.join("package.json"), r#"{"name":"repo","workspaces":["packages/*"]}"#)
            .unwrap();
        fs::write(
            package.join("package.json"),
            r#"{"name":"pkg","exports":{".":{"types":"./src/types.ts","import":"./src/import.ts","require":"./src/require.ts"}}}"#,
        )
        .unwrap();
        for file in ["types.ts", "import.ts", "require.ts"] {
            fs::write(package.join("src").join(file), "").unwrap();
        }
        let resolver = WorkspacePackageIndex::discover(&root).unwrap();
        for (kind, suffix) in [
            (DependencyKind::TypeOnly, "types.ts"),
            (DependencyKind::Static, "import.ts"),
            (DependencyKind::Require, "require.ts"),
        ] {
            assert!(matches!(
                resolve_kind_with(&resolver, &root.join("app.ts"), "pkg", kind),
                Some(Resolution::Module(path)) if path.0.ends_with(suffix)
            ));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn package_conditions_combine_module_format_and_type_only_resolution() {
        let root =
            std::env::temp_dir().join(format!("wae-nested-conditions-{}", std::process::id()));
        let package = root.join("packages/lib");
        fs::create_dir_all(package.join("src")).unwrap();
        fs::write(root.join("package.json"), r#"{"name":"repo","workspaces":["packages/*"]}"#)
            .unwrap();
        fs::write(
            package.join("package.json"),
            r#"{"name":"pkg","exports":{".":{"import":{"types":"./src/import.d.ts","default":"./src/import.js"},"require":{"types":"./src/require.d.ts","default":"./src/require.cjs"}}}}"#,
        )
        .unwrap();
        for file in ["import.d.ts", "import.js", "require.d.ts", "require.cjs"] {
            fs::write(package.join("src").join(file), "").unwrap();
        }
        let resolver = WorkspacePackageIndex::discover(&root).unwrap();
        for (kind, format, suffix) in [
            (ResolutionKind::Import, ModuleFormat::Esm, "import.d.ts"),
            (ResolutionKind::Require, ModuleFormat::CommonJs, "require.d.ts"),
        ] {
            assert!(matches!(
                resolve_request_with(
                    &resolver,
                    &root.join("app.ts"),
                    "pkg",
                    DependencyKind::TypeOnly,
                    kind,
                    format,
                    ResolutionMode::NodeNext,
                ),
                Some(Resolution::Module(path)) if path.0.ends_with(suffix)
            ));
        }
        assert!(matches!(
            resolve_request_with(
                &resolver,
                &root.join("app.cts"),
                "pkg",
                DependencyKind::Static,
                ResolutionKind::Require,
                ModuleFormat::CommonJs,
                ResolutionMode::NodeNext,
            ),
            Some(Resolution::Module(path)) if path.0.ends_with("require.cjs")
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bundler_does_not_activate_browser_without_an_explicit_custom_condition() {
        let root =
            std::env::temp_dir().join(format!("wae-bundler-conditions-{}", std::process::id()));
        let package = root.join("packages/lib");
        fs::create_dir_all(package.join("src")).unwrap();
        fs::write(root.join("package.json"), r#"{"name":"repo","workspaces":["packages/*"]}"#)
            .unwrap();
        fs::write(
            package.join("package.json"),
            r#"{"name":"pkg","exports":{".":{"browser":"./src/browser.js","import":"./src/import.js","default":"./src/default.js"}}}"#,
        ).unwrap();
        for file in ["browser.js", "import.js", "default.js"] {
            fs::write(package.join("src").join(file), "").unwrap();
        }
        let resolver = WorkspacePackageIndex::discover(&root).unwrap();
        let importer = ModulePath(root.join("app.ts").to_string_lossy().into_owned());
        let no_custom = ResolutionRequest {
            importer: &importer,
            specifier: "pkg",
            dependency_kind: DependencyKind::Static,
            resolution_kind: ResolutionKind::Import,
            importer_format: ModuleFormat::Esm,
            mode: ResolutionMode::Bundler,
            custom_conditions: &[],
        };
        assert!(matches!(
            resolver.try_resolve(&no_custom),
            Some(Resolution::Module(path)) if path.0.ends_with("import.js")
        ));
        let browser = vec!["browser".to_owned()];
        let custom_browser = ResolutionRequest { custom_conditions: &browser, ..no_custom };
        assert!(matches!(
            resolver.try_resolve(&custom_browser),
            Some(Resolution::Module(path)) if path.0.ends_with("browser.js")
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn package_imports_can_redirect_to_external_packages_and_detect_loops() {
        let root = std::env::temp_dir().join(format!("wae-import-map-{}", std::process::id()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("package.json"),
            r##"{"name":"repo","imports":{"#dep":"dep-node-native","#a":"#b","#b":"#a"}}"##,
        )
        .unwrap();
        let workspaces = WorkspacePackageIndex::discover(&root).unwrap();
        let pipeline =
            ResolverPipeline::new().with_handler(workspaces).with_handler(PackageResolver);
        let importer = ModulePath(normalize(&root.join("src/app.ts")));
        let request = |specifier| ResolutionRequest {
            importer: &importer,
            specifier,
            dependency_kind: DependencyKind::Static,
            resolution_kind: ResolutionKind::Import,
            importer_format: ModuleFormat::Esm,
            mode: ResolutionMode::NodeNext,
            custom_conditions: &[],
        };
        assert_eq!(
            pipeline.resolve(&request("#dep")),
            Resolution::External("dep-node-native".into())
        );
        assert!(
            matches!(pipeline.resolve(&request("#a")), Resolution::Invalid(message) if message.contains("loop"))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn nearest_tsconfig_owns_each_workspace_importer() {
        let root = std::env::temp_dir().join(format!("wae-ts-index-{}", std::process::id()));
        for package in ["web", "admin"] {
            fs::create_dir_all(root.join("apps").join(package).join("src")).unwrap();
            fs::write(
                root.join("apps").join(package).join("tsconfig.json"),
                format!(r#"{{"compilerOptions":{{"baseUrl":".","paths":{{"@local/*":["src/{package}/*"]}}}}}}"#),
            )
            .unwrap();
            fs::create_dir_all(root.join("apps").join(package).join("src").join(package)).unwrap();
            fs::write(
                root.join("apps").join(package).join("src").join(package).join("value.ts"),
                "",
            )
            .unwrap();
        }
        let resolver = IndexedAliasResolver::new(TsConfigIndex::discover(&root).unwrap());
        for package in ["web", "admin"] {
            let importer = root.join("apps").join(package).join("src/app.ts");
            assert!(matches!(
                resolve_with(&resolver, &importer, "@local/value"),
                Some(Resolution::Module(path)) if path.0.contains(&format!("/{package}/src/{package}/value.ts"))
            ));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn duplicate_workspace_names_are_configuration_errors() {
        let root = std::env::temp_dir().join(format!("wae-workspace-dupe-{}", std::process::id()));
        for package in ["a", "b"] {
            fs::create_dir_all(root.join("packages").join(package)).unwrap();
            fs::write(
                root.join("packages").join(package).join("package.json"),
                r#"{"name":"same"}"#,
            )
            .unwrap();
        }
        fs::write(root.join("package.json"), r#"{"name":"repo","workspaces":["packages/*"]}"#)
            .unwrap();
        let error = WorkspacePackageIndex::discover(&root).unwrap_err();
        assert!(error.contains("duplicate workspace package name `same`"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn absolute_module_specifiers_are_rejected() {
        let importer = ModulePath("/project/src/app.ts".into());
        let request = ResolutionRequest {
            importer: &importer,
            specifier: "/outside/secret.ts",
            dependency_kind: DependencyKind::Static,
            resolution_kind: ResolutionKind::Import,
            importer_format: ModuleFormat::Esm,
            mode: ResolutionMode::NodeNext,
            custom_conditions: &[],
        };
        assert!(matches!(
            RelativeResolver { mode: ResolutionMode::NodeNext }.try_resolve(&request),
            Some(Resolution::Invalid(message)) if message.contains("outside")
        ));
    }

    #[test]
    fn workspace_index_scales_across_named_packages() {
        let root =
            std::env::temp_dir().join(format!("wae-workspace-matrix-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("package.json"), r#"{"name":"repo","workspaces":["packages/*"]}"#)
            .unwrap();
        for index in 0..12 {
            let package = root.join(format!("packages/p{index}"));
            fs::create_dir_all(package.join("src")).unwrap();
            fs::write(package.join("src/index.ts"), "").unwrap();
            fs::write(
                package.join("package.json"),
                format!(
                    r#"{{"name":"@matrix/p{index}","type":"{}","exports":"./src/index.ts"}}"#,
                    if index % 2 == 0 { "module" } else { "commonjs" }
                ),
            )
            .unwrap();
        }
        let resolver = WorkspacePackageIndex::discover(&root).unwrap();
        assert_eq!(resolver.packages().len(), 13); // project root plus twelve workspaces
        for index in 0..12 {
            assert!(matches!(
                resolve_with(&resolver, &root.join("app.ts"), &format!("@matrix/p{index}")),
                Some(Resolution::Module(path)) if path.0.ends_with(&format!("packages/p{index}/src/index.ts"))
            ));
        }
        fs::remove_dir_all(root).unwrap();
    }
}
