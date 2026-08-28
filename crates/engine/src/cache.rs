use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use wae_config::Config;
use wae_core::domain::{Dependency, Diagnostic, Import, ResolvedDependency};
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
        let path = root.join(&config.cache.directory).join("analysis-v2.json");
        if !config.cache.enabled {
            return Ok(Self {
                enabled: false,
                path,
                root: root.to_path_buf(),
                file: fresh_cache(),
                live_files,
            });
        }
        let parent = path
            .parent()
            .ok_or_else(|| AnalysisError::Project("cache path has no parent directory".into()))?;
        fs::create_dir_all(parent).map_err(|error| {
            AnalysisError::Project(format!("cannot create cache directory: {error}"))
        })?;
        let file = read_cache(&path);
        Ok(Self { enabled: true, path, root: root.to_path_buf(), file, live_files })
    }

    pub(crate) fn get(
        &self,
        module: &str,
        hash: u64,
        environment_hash: u64,
    ) -> Option<CachedModuleAnalysis> {
        self.enabled
            .then(|| self.file.files.get(module))
            .flatten()
            .filter(|cached| cached.hash == hash && cached.environment_hash == environment_hash)
            .filter(|cached| {
                cached.resolved_paths.iter().all(|path| self.root.join(path).is_file())
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
            self.file.files.insert(
                module,
                CachedModuleAnalysis {
                    hash,
                    environment_hash,
                    resolved_paths: analysis
                        .dependencies
                        .iter()
                        .filter(|dependency| !dependency.to.0.starts_with("external:"))
                        .map(|dependency| dependency.to.0.clone())
                        .collect(),
                    ..analysis
                },
            );
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
            self.file.rules = Some(CachedRuleAnalysis { graph_hash, diagnostics });
        }
    }

    pub(crate) fn save(&self) -> Result<(), AnalysisError> {
        if !self.enabled {
            return Ok(());
        }
        // Parsing and graph analysis happen without a global writer lock. The short transaction
        // below locks, reloads the newest snapshot, merges this analysis, prunes deleted files and
        // atomically replaces the cache.
        let _lock = acquire_advisory_lock(&self.path.with_extension("lock"))?;
        let mut merged = read_cache(&self.path);
        merged.files.retain(|module, _| self.live_files.contains(module));
        for (module, cached) in &self.file.files {
            if self.live_files.contains(module) {
                merged.files.insert(module.clone(), cached.clone());
            }
        }
        if self.file.rules.is_some() {
            merged.rules.clone_from(&self.file.rules);
        }
        static CACHE_WRITE_ID: AtomicU64 = AtomicU64::new(0);
        let write_id = CACHE_WRITE_ID.fetch_add(1, Ordering::Relaxed);
        let temporary = self.path.with_extension(format!("tmp-{}-{write_id}", std::process::id()));
        let contents = serde_json::to_vec(&merged)
            .map_err(|error| AnalysisError::Internal(error.to_string()))?;
        fs::write(&temporary, contents)
            .map_err(|error| AnalysisError::Project(format!("cannot write cache: {error}")))?;
        if let Err(error) = fs::rename(&temporary, &self.path) {
            if self.path.exists() {
                fs::remove_file(&self.path).map_err(|remove_error| {
                    AnalysisError::Project(format!("cannot replace cache: {remove_error}"))
                })?;
                fs::rename(&temporary, &self.path).map_err(|rename_error| {
                    AnalysisError::Project(format!("cannot install cache: {rename_error}"))
                })?;
            } else {
                return Err(AnalysisError::Project(format!("cannot install cache: {error}")));
            }
        }
        Ok(())
    }
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

fn read_cache(path: &Path) -> CacheFile {
    fs::read_to_string(path)
        .ok()
        .and_then(|source| serde_json::from_str::<CacheFile>(&source).ok())
        .filter(|cache| cache.schema_version == 2 && cache.parser_version == PARSER_CACHE_VERSION)
        .unwrap_or_else(fresh_cache)
}

fn fresh_cache() -> CacheFile {
    CacheFile {
        schema_version: 2,
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
    use super::is_lock_contended;

    #[test]
    fn recognizes_the_platform_specific_lock_contention_error() {
        assert!(is_lock_contended(&fs2::lock_contended_error()));
        assert!(!is_lock_contended(&std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "not lock contention",
        )));
    }
}
