use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

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
        Self::new()
            .with_handler(RelativeResolver)
            .with_handler(AliasResolver { root: root.into(), aliases })
            .with_handler(PackageResolver)
    }

    pub fn node_with_workspaces(
        root: impl Into<PathBuf>,
        aliases: Vec<PathAlias>,
        workspaces: WorkspaceResolver,
    ) -> Self {
        Self::new()
            .with_handler(RelativeResolver)
            .with_handler(AliasResolver { root: root.into(), aliases })
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
pub struct RelativeResolver;
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
        Some(resolve_file(&base).map_or(Resolution::Unresolved, Resolution::Module))
    }
}

#[derive(Clone, Debug)]
pub struct AliasResolver {
    pub root: PathBuf,
    pub aliases: Vec<PathAlias>,
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
                if let Some(path) = resolve_file(&candidate) {
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
    entrypoints: BTreeMap<String, String>,
    imports: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default)]
pub struct WorkspaceResolver {
    packages: Vec<WorkspacePackage>,
}

impl WorkspaceResolver {
    pub fn discover(project_root: &Path) -> Result<Self, String> {
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
                entrypoints: manifest_entrypoints(&manifest),
                imports: manifest_imports(&manifest),
            });
        }
        packages.sort_by(|left, right| left.name.cmp(&right.name));
        packages.dedup_by(|left, right| left.name == right.name);
        Ok(Self { packages })
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
            let target = resolve_export(&package.imports, specifier)?;
            let candidate = package.root.join(target.trim_start_matches("./"));
            return Some(
                resolve_file(&candidate).map_or(Resolution::Unresolved, Resolution::Module),
            );
        }
        let name = package_name(specifier);
        let package = self.packages.iter().find(|package| package.name == name)?;
        let subpath = specifier.strip_prefix(&name).unwrap_or_default().trim_start_matches('/');
        let key = if subpath.is_empty() { ".".into() } else { format!("./{subpath}") };
        let configured = resolve_export(&package.entrypoints, &key);
        let candidate = configured
            .map(|target| package.root.join(target.trim_start_matches("./")))
            .unwrap_or_else(|| {
                if subpath.is_empty() {
                    package.root.join("src/index")
                } else {
                    package.root.join(subpath)
                }
            });
        Some(resolve_file(&candidate).map_or(Resolution::Unresolved, Resolution::Module))
    }
}

fn manifest_imports(manifest: &serde_json::Value) -> BTreeMap<String, String> {
    manifest
        .get("imports")
        .and_then(|value| value.as_object())
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| export_target(value).map(|target| (key.clone(), target.into())))
        .collect()
}

fn manifest_entrypoints(manifest: &serde_json::Value) -> BTreeMap<String, String> {
    let mut entries = BTreeMap::new();
    match manifest.get("exports") {
        Some(serde_json::Value::String(target)) => {
            entries.insert(".".into(), target.clone());
        }
        Some(serde_json::Value::Object(exports)) => {
            for (key, value) in exports {
                if key.starts_with('.') {
                    if let Some(target) = export_target(value) {
                        entries.insert(key.clone(), target.into());
                    }
                }
            }
        }
        _ => {}
    }
    if !entries.contains_key(".") {
        for field in ["module", "main", "types"] {
            if let Some(target) = manifest.get(field).and_then(|value| value.as_str()) {
                entries.insert(".".into(), target.into());
                break;
            }
        }
    }
    entries
}

fn export_target(value: &serde_json::Value) -> Option<&str> {
    match value {
        serde_json::Value::String(value) => Some(value),
        serde_json::Value::Object(conditions) => ["import", "default", "types", "require"]
            .into_iter()
            .find_map(|condition| conditions.get(condition).and_then(export_target)),
        _ => None,
    }
}

fn resolve_export(entries: &BTreeMap<String, String>, key: &str) -> Option<String> {
    if let Some(target) = entries.get(key) {
        return Some(target.clone());
    }
    entries.iter().find_map(|(pattern, target)| {
        let (prefix, suffix) = pattern.split_once('*')?;
        (key.starts_with(prefix) && key.ends_with(suffix)).then(|| {
            let capture = &key[prefix.len()..key.len() - suffix.len()];
            target.replace('*', capture)
        })
    })
}

pub fn resolve_file(base: &Path) -> Option<ModulePath> {
    const EXTENSIONS: [&str; 8] = ["ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs"];
    if base.is_file() {
        return Some(ModulePath(normalize(base)));
    }
    if base.extension().is_none() {
        for extension in EXTENSIONS {
            let candidate = base.with_extension(extension);
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
            RelativeResolver.try_resolve(Path::new(&importer.0), "./folder"),
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
        let resolver = AliasResolver { root: loaded.base_url, aliases: loaded.aliases };
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
}
