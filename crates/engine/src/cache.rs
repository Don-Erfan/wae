use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use wae_config::Config;
use wae_core::domain::Import;
use wae_parser::PARSER_CACHE_VERSION;

use crate::AnalysisError;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedImports {
    hash: u64,
    imports: Vec<Import>,
}

#[derive(Default, Serialize, Deserialize)]
struct CacheFile {
    schema_version: u32,
    #[serde(default)]
    parser_version: String,
    files: BTreeMap<String, CachedImports>,
}

pub(crate) struct AnalysisCache {
    enabled: bool,
    path: PathBuf,
    file: CacheFile,
    // The advisory lock spans read -> update -> atomic write. The OS releases it on process exit,
    // so a stale physical `.lock` file never blocks later analyses.
    _lock: Option<File>,
}

impl AnalysisCache {
    pub(crate) fn load(root: &Path, config: &Config) -> Result<Self, AnalysisError> {
        let path = root.join(&config.cache.directory).join("imports-v1.json");
        if !config.cache.enabled {
            return Ok(Self { enabled: false, path, file: fresh_cache(), _lock: None });
        }
        let parent = path
            .parent()
            .ok_or_else(|| AnalysisError::Project("cache path has no parent directory".into()))?;
        fs::create_dir_all(parent).map_err(|error| {
            AnalysisError::Project(format!("cannot create cache directory: {error}"))
        })?;
        let lock = acquire_advisory_lock(&path.with_extension("lock"))?;
        let file = fs::read_to_string(&path)
            .ok()
            .and_then(|source| serde_json::from_str::<CacheFile>(&source).ok())
            .filter(|cache| {
                cache.schema_version == 1 && cache.parser_version == PARSER_CACHE_VERSION
            })
            .unwrap_or_else(fresh_cache);
        Ok(Self { enabled: true, path, file, _lock: Some(lock) })
    }

    pub(crate) fn get(&self, module: &str, hash: u64) -> Option<Vec<Import>> {
        self.enabled
            .then(|| self.file.files.get(module))
            .flatten()
            .filter(|cached| cached.hash == hash)
            .map(|cached| cached.imports.clone())
    }

    pub(crate) fn insert(&mut self, module: String, hash: u64, imports: Vec<Import>) {
        if self.enabled {
            self.file.files.insert(module, CachedImports { hash, imports });
        }
    }

    pub(crate) fn save(&self) -> Result<(), AnalysisError> {
        if !self.enabled {
            return Ok(());
        }
        static CACHE_WRITE_ID: AtomicU64 = AtomicU64::new(0);
        let write_id = CACHE_WRITE_ID.fetch_add(1, Ordering::Relaxed);
        let temporary = self.path.with_extension(format!("tmp-{}-{write_id}", std::process::id()));
        let contents = serde_json::to_vec(&self.file)
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

fn fresh_cache() -> CacheFile {
    CacheFile {
        schema_version: 1,
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
