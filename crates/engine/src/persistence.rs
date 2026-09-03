use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

/// Transactional JSON repository shared by cache and governance files.
pub struct AtomicJsonRepository;

impl AtomicJsonRepository {
    pub fn write(path: &Path, value: &impl Serialize) -> Result<(), String> {
        let mut contents = serde_json::to_vec_pretty(value)
            .map_err(|error| format!("cannot serialize `{}`: {error}", path.display()))?;
        contents.push(b'\n');
        Self::write_bytes(path, &contents)
    }

    pub fn write_bytes(path: &Path, contents: &[u8]) -> Result<(), String> {
        let parent = path.parent().ok_or_else(|| "JSON path has no parent directory".to_owned())?;
        fs::create_dir_all(parent).map_err(|error| {
            format!("cannot create JSON repository directory `{}`: {error}", parent.display())
        })?;
        static WRITE_ID: AtomicU64 = AtomicU64::new(0);
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
            replace_file(&temporary, path)?;
            sync_parent(parent)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
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
