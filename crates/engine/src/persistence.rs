use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

static WRITE_ID: AtomicU64 = AtomicU64::new(0);

/// Format-agnostic transactional file writer.
pub struct AtomicFileWriter;

impl AtomicFileWriter {
    pub fn write(path: &Path, contents: &[u8]) -> Result<(), String> {
        Self::write_with(path, contents, &SystemFileReplacer)
    }

    pub fn write_with(
        path: &Path,
        contents: &[u8],
        replacer: &impl AtomicFileReplacer,
    ) -> Result<(), String> {
        let parent = path.parent().ok_or_else(|| "file path has no parent directory".to_owned())?;
        fs::create_dir_all(parent).map_err(|error| {
            format!("cannot create repository directory `{}`: {error}", parent.display())
        })?;
        let write_id = WRITE_ID.fetch_add(1, Ordering::Relaxed);
        let temporary = path.with_extension(format!("tmp-{}-{write_id}", std::process::id()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .map_err(|error| format!("cannot create `{}`: {error}", temporary.display()))?;
            file.write_all(contents)
                .map_err(|error| format!("cannot write `{}`: {error}", temporary.display()))?;
            file.sync_all()
                .map_err(|error| format!("cannot flush `{}`: {error}", temporary.display()))?;
            replacer.replace(&temporary, path)?;
            sync_parent(parent)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

/// JSON serialization policy backed by [`AtomicFileWriter`].
pub struct JsonRepository;

impl JsonRepository {
    pub fn write(path: &Path, value: &impl Serialize) -> Result<(), String> {
        let mut contents = serde_json::to_vec_pretty(value)
            .map_err(|error| format!("cannot serialize `{}`: {error}", path.display()))?;
        contents.push(b'\n');
        AtomicFileWriter::write(path, &contents)
    }
}

/// Replacement seam used by fault-injection tests and alternative storage adapters.
pub trait AtomicFileReplacer {
    fn replace(&self, temporary: &Path, destination: &Path) -> Result<(), String>;
}

struct SystemFileReplacer;

impl AtomicFileReplacer for SystemFileReplacer {
    fn replace(&self, temporary: &Path, destination: &Path) -> Result<(), String> {
        replace_file(temporary, destination)
    }
}

#[cfg(not(windows))]
fn replace_file(temporary: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(temporary, destination)
        .map_err(|error| format!("cannot atomically replace `{}`: {error}", destination.display()))
}

#[cfg(windows)]
fn replace_file(temporary: &Path, destination: &Path) -> Result<(), String> {
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
        return Err(format!(
            "cannot atomically replace `{}`: {}",
            destination.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> Result<(), String> {
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("cannot flush directory `{}`: {error}", parent.display()))
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailedReplace;

    impl AtomicFileReplacer for FailedReplace {
        fn replace(&self, _temporary: &Path, _destination: &Path) -> Result<(), String> {
            Err("injected replacement failure".into())
        }
    }

    #[test]
    fn failed_replacement_keeps_the_previous_file_and_cleans_up_the_temporary_file() {
        let root = std::env::temp_dir().join(format!(
            "wae-atomic-writer-{}-{}",
            std::process::id(),
            WRITE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("wae.yaml");
        fs::write(&path, b"old").unwrap();
        let error = AtomicFileWriter::write_with(&path, b"new", &FailedReplace).unwrap_err();
        assert!(error.contains("injected replacement failure"));
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
