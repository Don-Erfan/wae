use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use wae_config::Config;
use wae_core::domain::{Dependency, Diagnostic, Import, ModuleSemantics, ResolvedDependency};
use wae_parser::PARSER_CACHE_VERSION;

use crate::AnalysisError;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct CachedModuleAnalysis {
    pub(crate) hash: u64,
    pub(crate) environment_hash: u64,
    pub(crate) imports: Vec<Import>,
    pub(crate) dependencies: Vec<Dependency>,
    pub(crate) resolved_dependencies: Vec<ResolvedDependency>,
    pub(crate) diagnostics: Vec<Diagnostic>,
    #[serde(default)]
    pub(crate) semantics: ModuleSemantics,
    #[serde(default)]
    pub(crate) resolved_paths: Vec<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct CacheFile {
    schema_version: u32,
    #[serde(default)]
    parser_version: String,
    files: BTreeMap<String, CachedModuleAnalysis>,
    #[serde(default)]
    rules: Option<CachedRuleAnalysis>,
}

#[derive(Default, Serialize, Deserialize)]
struct CacheManifest {
    schema_version: u32,
    #[serde(default)]
    parser_version: String,
    #[serde(default)]
    rules: Option<CachedRuleAnalysis>,
    #[serde(default)]
    modules: BTreeSet<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedRuleAnalysis {
    graph_hash: u64,
    diagnostics: Vec<Diagnostic>,
}

pub(crate) struct AnalysisCache {
    enabled: bool,
    path: PathBuf,
    root: PathBuf,
    file: CacheFile,
    live_files: BTreeSet<String>,
    dirty_files: BTreeMap<String, CachedModuleAnalysis>,
    dirty_rules: Option<CachedRuleAnalysis>,
    needs_prune: bool,
    stale_files: BTreeSet<String>,
    loaded_shards: BTreeSet<u8>,
}

impl AnalysisCache {
    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn load(
        root: &Path,
        config: &Config,
        live_files: BTreeSet<String>,
    ) -> Result<Self, AnalysisError> {
        let path = root.join(&config.cache.directory).join("analysis-v3/manifest.json");
        if !config.cache.enabled {
            return Ok(Self {
                enabled: false,
                path,
                root: root.to_path_buf(),
                file: fresh_cache(),
                live_files,
                dirty_files: BTreeMap::new(),
                dirty_rules: None,
                needs_prune: false,
                stale_files: BTreeSet::new(),
                loaded_shards: BTreeSet::new(),
            });
        }
        let parent = path
            .parent()
            .ok_or_else(|| AnalysisError::Project("cache path has no parent directory".into()))?;
        fs::create_dir_all(parent).map_err(|error| {
            AnalysisError::Project(format!("cannot create cache directory: {error}"))
        })?;
        let manifest = read_manifest(&path).unwrap_or_default();
        let stale_files =
            manifest.modules.difference(&live_files).cloned().collect::<BTreeSet<_>>();
        let needs_prune = manifest.modules != live_files;
        let file = CacheFile {
            schema_version: manifest.schema_version,
            parser_version: manifest.parser_version,
            files: BTreeMap::new(),
            rules: manifest.rules,
        };
        Ok(Self {
            enabled: true,
            path,
            root: root.to_path_buf(),
            file,
            live_files,
            dirty_files: BTreeMap::new(),
            dirty_rules: None,
            needs_prune,
            stale_files,
            loaded_shards: BTreeSet::new(),
        })
    }

    pub(crate) fn get(
        &mut self,
        module: &str,
        hash: u64,
        environment_hash: u64,
    ) -> Option<CachedModuleAnalysis> {
        if self.enabled {
            let shard = shard_id(module);
            if self.loaded_shards.insert(shard) {
                self.file.files.extend(read_shard(&shard_path(&self.path, shard)));
            }
        }
        self.enabled
            .then(|| self.file.files.get(module))
            .flatten()
            .filter(|cached| cached.hash == hash && cached.environment_hash == environment_hash)
            .filter(|cached| {
                cached
                    .resolved_paths
                    .iter()
                    .all(|path| self.live_files.contains(path) || self.root.join(path).is_file())
            })
            .filter(|cached| !unresolved_candidate_became_live(cached, &self.live_files))
            .cloned()
    }

    pub(crate) fn insert(
        &mut self,
        module: String,
        hash: u64,
        environment_hash: u64,
        analysis: CachedModuleAnalysis,
    ) {
        if self.enabled {
            let cached = CachedModuleAnalysis {
                hash,
                environment_hash,
                resolved_paths: analysis
                    .dependencies
                    .iter()
                    .filter(|dependency| !dependency.to.0.starts_with("external:"))
                    .map(|dependency| dependency.to.0.clone())
                    .collect(),
                ..analysis
            };
            self.file.files.insert(module.clone(), cached.clone());
            self.dirty_files.insert(module, cached);
        }
    }

    pub(crate) fn rule_diagnostics(&self, graph_hash: u64) -> Option<Vec<Diagnostic>> {
        self.enabled
            .then_some(self.file.rules.as_ref())
            .flatten()
            .filter(|cached| cached.graph_hash == graph_hash)
            .map(|cached| cached.diagnostics.clone())
    }

    pub(crate) fn set_rule_diagnostics(&mut self, graph_hash: u64, diagnostics: Vec<Diagnostic>) {
        if self.enabled {
            let cached = CachedRuleAnalysis { graph_hash, diagnostics };
            self.file.rules = Some(cached.clone());
            self.dirty_rules = Some(cached);
        }
    }

    pub(crate) fn save(&self) -> Result<(), AnalysisError> {
        if !self.enabled {
            return Ok(());
        }
        if self.dirty_files.is_empty() && self.dirty_rules.is_none() && !self.needs_prune {
            return Ok(());
        }
        // Parsing and graph analysis happen without a global writer lock. The short transaction
        // below rewrites only affected content-addressed shards and the small rule manifest.
        let _lock = acquire_advisory_lock(&self.path.with_extension("lock"))?;
        let mut affected_shards =
            self.dirty_files.keys().map(|module| shard_id(module)).collect::<BTreeSet<_>>();
        affected_shards.extend(self.stale_files.iter().map(|module| shard_id(module)));
        for shard in affected_shards {
            let path = shard_path(&self.path, shard);
            let mut records = read_shard(&path);
            records.retain(|module, _| self.live_files.contains(module));
            for (module, cached) in
                self.dirty_files.iter().filter(|(module, _)| shard_id(module) == shard)
            {
                if self.live_files.contains(module) {
                    records.insert(module.clone(), cached.clone());
                }
            }
            write_json_atomic(&path, &records)?;
        }
        let mut manifest = read_manifest(&self.path).unwrap_or(CacheManifest {
            schema_version: 3,
            parser_version: PARSER_CACHE_VERSION.into(),
            rules: self.file.rules.clone(),
            modules: BTreeSet::new(),
        });
        if self.dirty_rules.is_some() {
            manifest.rules.clone_from(&self.dirty_rules);
        }
        manifest.schema_version = 3;
        manifest.parser_version = PARSER_CACHE_VERSION.into();
        manifest.modules.clone_from(&self.live_files);
        write_json_atomic(&self.path, &manifest)?;
        Ok(())
    }
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), AnalysisError> {
    let parent = path
        .parent()
        .ok_or_else(|| AnalysisError::Project("cache path has no parent directory".into()))?;
    fs::create_dir_all(parent)
        .map_err(|error| AnalysisError::Project(format!("cannot create cache shard: {error}")))?;
    static CACHE_WRITE_ID: AtomicU64 = AtomicU64::new(0);
    let write_id = CACHE_WRITE_ID.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_extension(format!("tmp-{}-{write_id}", std::process::id()));
    let contents =
        serde_json::to_vec(value).map_err(|error| AnalysisError::Internal(error.to_string()))?;
    fs::write(&temporary, contents)
        .map_err(|error| AnalysisError::Project(format!("cannot write cache: {error}")))?;
    replace_file(&temporary, path)
}

#[cfg(not(windows))]
fn replace_file(temporary: &Path, destination: &Path) -> Result<(), AnalysisError> {
    fs::rename(temporary, destination)
        .map_err(|error| AnalysisError::Project(format!("cannot install cache: {error}")))
}

#[cfg(windows)]
fn replace_file(temporary: &Path, destination: &Path) -> Result<(), AnalysisError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source = temporary.as_os_str().encode_wide().chain(std::iter::once(0)).collect::<Vec<_>>();
    let target =
        destination.as_os_str().encode_wide().chain(std::iter::once(0)).collect::<Vec<_>>();
    // SAFETY: both buffers are NUL-terminated and remain alive for the duration of the call.
    let replaced = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        return Err(AnalysisError::Project(format!(
            "cannot atomically install cache: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

fn unresolved_candidate_became_live(
    cached: &CachedModuleAnalysis,
    live_files: &BTreeSet<String>,
) -> bool {
    cached.diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id.0 == "RESOLVE-001"
            && diagnostic
                .metadata
                .get("candidatePaths")
                .and_then(|paths| serde_json::from_str::<Vec<String>>(paths).ok())
                .is_some_and(|paths| paths.iter().any(|path| live_files.contains(path)))
    })
}

#[cfg(test)]
fn read_cache(path: &Path) -> CacheFile {
    let Some(manifest) = read_manifest(path) else { return fresh_cache() };
    let mut files = BTreeMap::new();
    if let Some(directory) = path.parent().map(|parent| parent.join("modules")) {
        if let Ok(entries) = fs::read_dir(directory) {
            for entry in entries.flatten().filter(|entry| {
                entry.path().extension().is_some_and(|extension| extension == "json")
            }) {
                files.extend(read_shard(&entry.path()));
            }
        }
    }
    CacheFile {
        schema_version: manifest.schema_version,
        parser_version: manifest.parser_version,
        files,
        rules: manifest.rules,
    }
}

fn read_manifest(path: &Path) -> Option<CacheManifest> {
    fs::read_to_string(path)
        .ok()
        .and_then(|source| serde_json::from_str::<CacheManifest>(&source).ok())
        .filter(|cache| cache.schema_version == 3 && cache.parser_version == PARSER_CACHE_VERSION)
}

fn read_shard(path: &Path) -> BTreeMap<String, CachedModuleAnalysis> {
    fs::read_to_string(path)
        .ok()
        .and_then(|source| serde_json::from_str(&source).ok())
        .unwrap_or_default()
}

fn shard_id(module: &str) -> u8 {
    (stable_hash(module.as_bytes()) & 0xff) as u8
}

fn shard_path(manifest: &Path, shard: u8) -> PathBuf {
    manifest
        .parent()
        .expect("cache manifest parent")
        .join("modules")
        .join(format!("{shard:02x}.json"))
}

fn fresh_cache() -> CacheFile {
    CacheFile {
        schema_version: 3,
        parser_version: PARSER_CACHE_VERSION.into(),
        ..CacheFile::default()
    }
}

fn acquire_advisory_lock(path: &Path) -> Result<File, AnalysisError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| AnalysisError::Project(format!("cannot open cache lock: {error}")))?;
    for _ in 0..500 {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(file),
            Err(error) if is_lock_contended(&error) => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                return Err(AnalysisError::Project(format!(
                    "cannot acquire cache advisory lock: {error}"
                )));
            }
        }
    }
    Err(AnalysisError::Project(format!(
        "timed out waiting for active cache writer `{}`",
        path.display()
    )))
}

fn is_lock_contended(error: &std::io::Error) -> bool {
    let expected = fs2::lock_contended_error();
    match (error.raw_os_error(), expected.raw_os_error()) {
        (Some(actual), Some(expected)) => actual == expected,
        _ => error.kind() == expected.kind(),
    }
}

pub(crate) fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;

    use wae_config::Config;
    use wae_core::domain::ModuleSemantics;

    use super::{AnalysisCache, CachedModuleAnalysis, is_lock_contended, read_cache};

    #[test]
    fn recognizes_the_platform_specific_lock_contention_error() {
        assert!(is_lock_contended(&fs2::lock_contended_error()));
        assert!(!is_lock_contended(&std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "not lock contention",
        )));
    }

    #[test]
    fn stale_writers_merge_only_their_dirty_modules() {
        let root = std::env::temp_dir().join(format!(
            "wae-cache-dirty-merge-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&root).unwrap();
        let mut config = Config::default();
        config.cache.enabled = true;
        config.cache.directory = ".wae/cache".into();
        let live = BTreeSet::from(["src/a.ts".into(), "src/b.ts".into()]);
        let mut first = AnalysisCache::load(&root, &config, live.clone()).unwrap();
        let mut stale = AnalysisCache::load(&root, &config, live).unwrap();
        let analysis = |hash| CachedModuleAnalysis {
            hash,
            environment_hash: 7,
            imports: vec![],
            dependencies: vec![],
            resolved_dependencies: vec![],
            diagnostics: vec![],
            semantics: ModuleSemantics::default(),
            resolved_paths: vec![],
        };

        first.insert("src/a.ts".into(), 1, 7, analysis(1));
        stale.insert("src/b.ts".into(), 2, 7, analysis(2));
        first.save().unwrap();
        stale.save().unwrap();

        let cache = read_cache(&root.join(".wae/cache/analysis-v3/manifest.json"));
        assert_eq!(cache.files.len(), 2);
        assert_eq!(cache.files["src/a.ts"].hash, 1);
        assert_eq!(cache.files["src/b.ts"].hash, 2);
        fs::remove_dir_all(root).unwrap();
    }
}
