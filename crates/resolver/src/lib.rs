use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use globset::{GlobBuilder, GlobMatcher};
use serde::Deserialize;
use wae_config::ResolutionMode;
use wae_core::domain::ModulePath;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    Module(ModulePath),
    External(String),
    Unresolved,
}

pub trait ModuleResolver: Send + Sync {
    fn resolve(&self, importer: &ModulePath, specifier: &str) -> Resolution;
}

/// A resolution handler is one link in the Node/TypeScript resolution chain.
pub trait ResolutionHandler: Send + Sync {
    fn try_resolve(&self, importer: &Path, specifier: &str) -> Option<Resolution>;
}

#[derive(Default)]
pub struct ResolverPipeline {
    handlers: Vec<Box<dyn ResolutionHandler>>,
}

impl ResolverPipeline {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_handler<H: ResolutionHandler + 'static>(mut self, handler: H) -> Self {
        self.handlers.push(Box::new(handler));
        self
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
        mut workspaces: WorkspaceResolver,
        mode: ResolutionMode,
    ) -> Self {
        workspaces.mode = mode;
        Self::new()
            .with_handler(RelativeResolver { mode })
            .with_handler(AliasResolver { root: root.into(), aliases, mode })
            .with_handler(workspaces)
            .with_handler(PackageResolver)
    }
}

impl ModuleResolver for ResolverPipeline {
    fn resolve(&self, importer: &ModulePath, specifier: &str) -> Resolution {
        let importer = Path::new(&importer.0);
        self.handlers
            .iter()
            .find_map(|handler| handler.try_resolve(importer, specifier))
            .unwrap_or(Resolution::Unresolved)
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
    fn try_resolve(&self, importer: &Path, specifier: &str) -> Option<Resolution> {
        if !specifier.starts_with('.') && !specifier.starts_with('/') {
            return None;
        }
        let base = if specifier.starts_with('/') {
            PathBuf::from(specifier)
        } else {
            importer.parent()?.join(specifier)
        };
        Some(
            resolve_file_with_mode(&base, self.mode)
                .map_or(Resolution::Unresolved, Resolution::Module),
        )
    }
}

#[derive(Clone, Debug)]
pub struct AliasResolver {
    pub root: PathBuf,
    pub aliases: Vec<PathAlias>,
    pub mode: ResolutionMode,
}
impl ResolutionHandler for AliasResolver {
    fn try_resolve(&self, _importer: &Path, specifier: &str) -> Option<Resolution> {
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
                if let Some(path) = resolve_file_with_mode(&candidate, self.mode) {
                    return Some(Resolution::Module(path));
                }
            }
        }
        matched.then_some(Resolution::Unresolved)
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
        let path = project_root.join("tsconfig.json");
        if !path.exists() {
            return Ok(TsConfigPaths { base_url: project_root.to_path_buf(), aliases: Vec::new() });
        }
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
    let mut resolved = if let Some(parent) = json.get("extends").and_then(|value| value.as_str()) {
        let parent_path = resolve_extends(directory, parent)?;
        load_tsconfig(&parent_path, visited)?
    } else {
        ResolvedTsConfig { base_url: directory.to_path_buf(), aliases: BTreeMap::new() }
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
    let mut candidate = if value.starts_with('.') || value.starts_with('/') {
        directory.join(value)
    } else {
        directory.join("node_modules").join(value)
    };
    if candidate.is_dir() {
        candidate = candidate.join("tsconfig.json");
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
    fn try_resolve(&self, _importer: &Path, specifier: &str) -> Option<Resolution> {
        (!specifier.starts_with('.') && !specifier.starts_with('/'))
            .then(|| Resolution::External(package_name(specifier)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspacePackage {
    pub name: String,
    pub root: PathBuf,
    has_exports: bool,
    entrypoints: BTreeMap<String, serde_json::Value>,
    imports: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug)]
pub struct WorkspaceResolver {
    packages: Vec<WorkspacePackage>,
    mode: ResolutionMode,
}

impl Default for WorkspaceResolver {
    fn default() -> Self {
        Self { packages: Vec::new(), mode: ResolutionMode::NodeNext }
    }
}

impl WorkspaceResolver {
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
            });
        }
        packages.sort_by(|left, right| left.name.cmp(&right.name));
        packages.dedup_by(|left, right| left.name == right.name);
        Ok(Self { packages, mode: ResolutionMode::NodeNext })
    }

    pub fn packages(&self) -> &[WorkspacePackage] {
        &self.packages
    }
}

impl ResolutionHandler for WorkspaceResolver {
    fn try_resolve(&self, importer: &Path, specifier: &str) -> Option<Resolution> {
        if specifier.starts_with('.') || specifier.starts_with('/') {
            return None;
        }
        if specifier.starts_with('#') {
            let package = self
                .packages
                .iter()
                .filter(|package| importer.starts_with(&package.root))
                .max_by_key(|package| package.root.components().count())?;
            let target = resolve_export(&package.imports, specifier, self.mode);
            return Some(resolve_package_target(package, target, self.mode));
        }
        let name = package_name(specifier);
        let package = self.packages.iter().find(|package| package.name == name)?;
        let subpath = specifier.strip_prefix(&name).unwrap_or_default().trim_start_matches('/');
        let key = if subpath.is_empty() { ".".into() } else { format!("./{subpath}") };
        let configured = resolve_export(&package.entrypoints, &key, self.mode);
        if package.has_exports {
            return Some(resolve_package_target(package, configured, self.mode));
        }
        if configured.is_some() {
            return Some(resolve_package_target(package, configured, self.mode));
        }
        let candidate = if subpath.is_empty() {
            package.root.join("src/index")
        } else {
            package.root.join(subpath)
        };
        Some(
            resolve_file_with_mode(&candidate, self.mode)
                .map_or(Resolution::Unresolved, Resolution::Module),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PackageTarget {
    Path(String),
    Blocked,
}

fn manifest_imports(manifest: &serde_json::Value) -> BTreeMap<String, serde_json::Value> {
    manifest
        .get("imports")
        .and_then(|value| value.as_object())
        .into_iter()
        .flatten()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn manifest_entrypoints(manifest: &serde_json::Value) -> BTreeMap<String, serde_json::Value> {
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
                for (key, value) in exports {
                    if key.starts_with('.') {
                        entries.insert(key.clone(), value.clone());
                    }
                }
            } else {
                entries.insert(".".into(), serde_json::Value::Object(exports.clone()));
            }
        }
        _ => {}
    }
    if !entries.contains_key(".") {
        for field in ["module", "main", "types"] {
            if let Some(target) = manifest.get(field).and_then(|value| value.as_str()) {
                entries.insert(".".into(), serde_json::Value::String(target.into()));
                break;
            }
        }
    }
    entries
}

fn export_target(value: &serde_json::Value, mode: ResolutionMode) -> Option<PackageTarget> {
    match value {
        serde_json::Value::String(value) => Some(PackageTarget::Path(value.clone())),
        serde_json::Value::Null => Some(PackageTarget::Blocked),
        serde_json::Value::Array(targets) => {
            let mut blocked = false;
            for target in targets {
                match export_target(target, mode) {
                    Some(path @ PackageTarget::Path(_)) => return Some(path),
                    Some(PackageTarget::Blocked) => blocked = true,
                    None => {}
                }
            }
            blocked.then_some(PackageTarget::Blocked)
        }
        serde_json::Value::Object(conditions) => {
            let active = resolution_conditions(mode);
            conditions.iter().find_map(|(condition, value)| {
                (condition == "default" || active.contains(&condition.as_str()))
                    .then(|| export_target(value, mode))
                    .flatten()
            })
        }
        _ => None,
    }
}

fn resolution_conditions(mode: ResolutionMode) -> [&'static str; 5] {
    match mode {
        ResolutionMode::Node | ResolutionMode::Node16 | ResolutionMode::NodeNext => {
            ["types", "import", "node", "require", "default"]
        }
        ResolutionMode::Bundler => ["types", "import", "browser", "default", "require"],
    }
}

fn resolve_export(
    entries: &BTreeMap<String, serde_json::Value>,
    key: &str,
    mode: ResolutionMode,
) -> Option<PackageTarget> {
    if let Some(target) = entries.get(key) {
        return export_target(target, mode);
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
            export_target(target, mode).map(|target| match target {
                PackageTarget::Path(path) => PackageTarget::Path(path.replace('*', capture)),
                PackageTarget::Blocked => PackageTarget::Blocked,
            })
        })
}

fn resolve_package_target(
    package: &WorkspacePackage,
    target: Option<PackageTarget>,
    mode: ResolutionMode,
) -> Resolution {
    let Some(PackageTarget::Path(target)) = target else { return Resolution::Unresolved };
    if !target.starts_with("./") {
        return Resolution::Unresolved;
    }
    let candidate = package.root.join(target.trim_start_matches("./"));
    if !lexically_within(&candidate, &package.root) {
        return Resolution::Unresolved;
    }
    resolve_file_with_mode(&candidate, mode).map_or(Resolution::Unresolved, Resolution::Module)
}

pub fn resolve_file(base: &Path) -> Option<ModulePath> {
    resolve_file_with_mode(base, ResolutionMode::NodeNext)
}

pub fn resolve_file_with_mode(base: &Path, mode: ResolutionMode) -> Option<ModulePath> {
    const EXTENSIONS: [&str; 8] = ["ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs"];
    if base.is_file() {
        return Some(ModulePath(normalize(base)));
    }

    let source_extensions: &[&str] = match (mode, base.extension().and_then(|value| value.to_str()))
    {
        (
            ResolutionMode::Node16 | ResolutionMode::NodeNext | ResolutionMode::Bundler,
            Some("js"),
        ) => &["ts", "tsx", "js", "jsx"],
        (
            ResolutionMode::Node16 | ResolutionMode::NodeNext | ResolutionMode::Bundler,
            Some("mjs"),
        ) => &["mts", "mjs"],
        (
            ResolutionMode::Node16 | ResolutionMode::NodeNext | ResolutionMode::Bundler,
            Some("cjs"),
        ) => &["cts", "cjs"],
        _ => &[],
    };
    for extension in source_extensions {
        let candidate = base.with_extension(extension);
        if candidate.is_file() {
            return Some(ModulePath(normalize(&candidate)));
        }
    }

    if source_extensions.is_empty() {
        for extension in EXTENSIONS {
            let candidate = PathBuf::from(format!("{}.{}", base.to_string_lossy(), extension));
            if candidate.is_file() {
                return Some(ModulePath(normalize(&candidate)));
            }
        }
    }
    if base.is_dir() {
        for extension in EXTENSIONS {
            let candidate = base.join(format!("index.{extension}"));
            if candidate.is_file() {
                return Some(ModulePath(normalize(&candidate)));
            }
        }
    }
    None
}

fn lexically_within(path: &Path, directory: &Path) -> bool {
    let path = lexical_path(path);
    let directory = lexical_path(directory);
    path.starts_with(directory)
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

    #[test]
    fn resolves_extensions_and_index_modules() {
        let root = std::env::temp_dir().join(format!("wae-resolver-{}", std::process::id()));
        fs::create_dir_all(root.join("folder")).unwrap();
        fs::write(root.join("folder/index.ts"), "").unwrap();
        let importer = ModulePath(root.join("a.ts").to_string_lossy().into_owned());
        assert!(matches!(
            RelativeResolver { mode: ResolutionMode::NodeNext }
                .try_resolve(Path::new(&importer.0), "./folder"),
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
            resolver.try_resolve(Path::new("src/a.ts"), "@/shared/util"),
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
            resolver.try_resolve(Path::new("src/app.ts"), "@/auth/index"),
            Some(Resolution::Module(path)) if path.0.ends_with("src/features/auth/index.ts")
        ));
        assert!(matches!(
            resolver.try_resolve(Path::new("src/app.ts"), "@/shared/auth"),
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
        let resolver = WorkspaceResolver::discover(&root).unwrap();
        assert!(matches!(
            resolver.try_resolve(Path::new("apps/web/src/app.ts"), "@acme/ui"),
            Some(Resolution::Module(path)) if path.0.ends_with("packages/ui/src/index.ts")
        ));
        assert!(matches!(
            resolver.try_resolve(&package.join("src/index.ts"), "#internal/button"),
            Some(Resolution::Module(path)) if path.0.ends_with("packages/ui/src/components/button.ts")
        ));
        assert!(matches!(
            resolver.try_resolve(Path::new("apps/web/src/app.ts"), "@acme/ui/components/button"),
            Some(Resolution::Module(path)) if path.0.ends_with("packages/ui/src/components/button.ts")
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
            resolver.try_resolve(&root.join("a.ts"), "./b.js"),
            Some(Resolution::Module(path)) if path.0.ends_with("b.ts")
        ));
        assert!(matches!(
            resolver.try_resolve(&root.join("a.ts"), "./config.prod"),
            Some(Resolution::Module(path)) if path.0.ends_with("config.prod.ts")
        ));
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
        let resolver = WorkspaceResolver::discover(&root).unwrap();
        assert!(matches!(
            resolver.try_resolve(&root.join("app.ts"), "@acme/ui"),
            Some(Resolution::Module(_))
        ));
        assert!(matches!(
            resolver.try_resolve(&root.join("app.ts"), "@acme/ui/conditional"),
            Some(Resolution::Module(path)) if path.0.ends_with("src/import.ts")
        ));
        assert!(matches!(
            resolver.try_resolve(&root.join("app.ts"), "@acme/ui/feature/special/button"),
            Some(Resolution::Module(path)) if path.0.ends_with("src/special/button.ts")
        ));
        for specifier in ["@acme/ui/internal/secret", "@acme/ui/escape", "@acme/ui/private/secret"]
        {
            assert_eq!(
                resolver.try_resolve(&root.join("app.ts"), specifier),
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
        let resolver = WorkspaceResolver::discover(&root).unwrap();
        assert!(resolver.packages().iter().any(|package| package.name == "@acme/ui"));
        assert!(!resolver.packages().iter().any(|package| package.name == "@acme/demo"));
        fs::remove_dir_all(root).unwrap();
    }
}
