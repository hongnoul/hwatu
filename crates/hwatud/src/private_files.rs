// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Private daemon-side files for remote payloads.
//!
//! Each upload gets an exclusive mode-0700 directory containing a mode-0600
//! file with the caller's original basename. The directory indirection keeps
//! the page-visible filename intact while `create_new` prevents replacement.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

static NEXT_FILE: AtomicU64 = AtomicU64::new(1);
static CLEANED_ABANDONED: OnceLock<()> = OnceLock::new();

#[cfg(unix)]
const O_NOFOLLOW: i32 = 0o400000;

#[derive(Debug)]
pub(crate) struct StagedUpload {
    path: PathBuf,
    dir: PathBuf,
    size: usize,
}

impl StagedUpload {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn size(&self) -> usize {
        self.size
    }
}

impl Drop for StagedUpload {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.dir);
    }
}

pub(crate) fn stage_upload(name: &str, bytes: &[u8]) -> Result<StagedUpload, String> {
    if bytes.len() > hwatu_ipc::INLINE_MAX_BYTES {
        return Err(format!(
            "upload is {} bytes; the inline limit is {} bytes",
            bytes.len(),
            hwatu_ipc::INLINE_MAX_BYTES
        ));
    }
    let root = private_root()?;
    let basename = safe_basename(name);

    for _ in 0..32 {
        let sequence = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        let dir = root.join(format!("upload-{}-{sequence}", std::process::id()));
        match create_private_dir(&dir) {
            Ok(()) => {
                let path = dir.join(&basename);
                let result = write_private_file(&path, bytes);
                return match result {
                    Ok(()) => Ok(StagedUpload {
                        path,
                        dir,
                        size: bytes.len(),
                    }),
                    Err(error) => {
                        let _ = std::fs::remove_file(&path);
                        let _ = std::fs::remove_dir(&dir);
                        Err(error)
                    }
                };
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "cannot create private upload directory {}: {error}",
                    dir.display()
                ));
            }
        }
    }
    Err("could not allocate a unique private upload directory".to_string())
}

fn private_root() -> Result<PathBuf, String> {
    let root = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("hwatu-{}-private", effective_uid()));

    match std::fs::symlink_metadata(&root) {
        Ok(metadata) => {
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(format!(
                    "private runtime path {} is not a real directory",
                    root.display()
                ));
            }
            #[cfg(unix)]
            {
                if metadata.uid() != effective_uid() {
                    return Err(format!(
                        "private runtime directory {} is owned by another user",
                        root.display()
                    ));
                }
                if metadata.permissions().mode() & 0o077 != 0 {
                    return Err(format!(
                        "private runtime directory {} has unsafe permissions",
                        root.display()
                    ));
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            create_private_dir(&root).map_err(|error| {
                format!(
                    "cannot create private runtime directory {}: {error}",
                    root.display()
                )
            })?;
        }
        Err(error) => {
            return Err(format!(
                "cannot inspect private runtime directory {}: {error}",
                root.display()
            ));
        }
    }
    if CLEANED_ABANDONED.set(()).is_ok() {
        cleanup_abandoned_uploads(&root);
    }
    Ok(root)
}

fn create_private_dir(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(path)
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
        options.custom_flags(O_NOFOLLOW);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("cannot create staged upload {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("cannot write staged upload {}: {error}", path.display()))
}

fn cleanup_abandoned_uploads(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.strip_prefix("upload-"))
            .and_then(|rest| rest.split_once('-'))
            .and_then(|(pid, _)| pid.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == std::process::id() || process_is_alive(pid) {
            continue;
        }
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        #[cfg(unix)]
        if metadata.uid() != effective_uid() {
            continue;
        }
        let _ = std::fs::remove_dir_all(path);
    }
}

#[cfg(target_os = "linux")]
fn process_is_alive(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

#[cfg(not(target_os = "linux"))]
fn process_is_alive(_pid: u32) -> bool {
    true
}

fn safe_basename(name: &str) -> String {
    Path::new(name)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .map(str::to_string)
        .unwrap_or_else(|| "upload.bin".to_string())
}

#[cfg(unix)]
fn effective_uid() -> u32 {
    unsafe { geteuid() }
}

#[cfg(unix)]
extern "C" {
    fn geteuid() -> u32;
}

#[cfg(not(unix))]
fn effective_uid() -> u32 {
    std::process::id()
}

#[cfg(test)]
mod tests {
    use super::{safe_basename, stage_upload};

    #[test]
    fn basename_never_escapes_the_private_directory() {
        assert_eq!(safe_basename("../../report.pdf"), "report.pdf");
        assert_eq!(safe_basename(""), "upload.bin");
        assert_eq!(safe_basename(".."), "upload.bin");
    }

    #[cfg(unix)]
    #[test]
    fn staged_upload_is_private_and_removed_on_drop() {
        use std::os::unix::fs::PermissionsExt;

        let staged = stage_upload("../fixture.txt", b"secret fixture").unwrap();
        let path = staged.path().to_path_buf();
        let dir = path.parent().unwrap().to_path_buf();
        assert_eq!(path.file_name().unwrap(), "fixture.txt");
        assert_eq!(std::fs::read(&path).unwrap(), b"secret fixture");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        drop(staged);
        assert!(!path.exists());
        assert!(!dir.exists());
    }
}
