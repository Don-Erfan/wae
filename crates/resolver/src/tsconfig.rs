use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use wae_core::domain::ModulePath;

use super::{
    AliasResolver, PathAlias, Resolution, ResolutionHandler, ResolutionRequest, normalize,
    normalized_path_is_within,
};

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

    pub(crate) fn paths_for(&self, importer: &Path) -> Option<&TsConfigPaths> {
        self.configs
            .iter()
            .find(|config| normalized_path_is_within(importer, &config.directory))
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
