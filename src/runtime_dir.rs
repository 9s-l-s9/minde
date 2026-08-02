// SPDX-License-Identifier: GPL-3.0-or-later

//! Secure paths for per-user runtime sockets.

use std::ffi::OsString;
use std::fs::{self, Metadata, Permissions};
use std::io::{Error, ErrorKind, Result};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and does not dereference memory.
    unsafe { libc::geteuid() }
}

fn runtime_directory(runtime: Option<OsString>, temporary: &Path, uid: u32) -> PathBuf {
    runtime
        .map(PathBuf::from)
        .unwrap_or_else(|| temporary.join(format!("minde-{uid}")))
}

fn validate_private_directory(path: &Path, metadata: &Metadata, uid: u32) -> Result<()> {
    if !metadata.file_type().is_dir() {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("runtime path is not a directory: {}", path.display()),
        ));
    }
    if metadata.uid() != uid {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "runtime directory is not owned by uid {uid}: {}",
                path.display()
            ),
        ));
    }
    if metadata.mode() & 0o077 != 0 {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "runtime directory is accessible by other users: {}",
                path.display()
            ),
        ));
    }
    Ok(())
}

/// Returns a socket path beneath a private, current-user runtime directory.
/// When XDG_RUNTIME_DIR is absent, creates `/tmp/minde-UID` with mode 0700
/// instead of placing every user's sockets directly in the shared `/tmp`.
pub fn socket_path(name: &str) -> Result<PathBuf> {
    let uid = effective_uid();
    let configured = std::env::var_os("XDG_RUNTIME_DIR");
    let fallback = configured.is_none();
    let directory = runtime_directory(configured, &std::env::temp_dir(), uid);
    if fallback {
        match fs::create_dir(&directory) {
            Ok(()) => fs::set_permissions(&directory, Permissions::from_mode(0o700))?,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    let metadata = fs::symlink_metadata(&directory)?;
    validate_private_directory(&directory, &metadata, uid)?;
    Ok(directory.join(name))
}

/// Removes only an abandoned socket owned by this process's user. An active
/// listener is reported as `AddrInUse`; regular files, symlinks, directories,
/// and another user's socket are never unlinked.
pub fn remove_stale_socket(path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_socket() || metadata.uid() != effective_uid() {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "refusing to replace non-owned socket entry: {}",
                path.display()
            ),
        ));
    }
    match UnixStream::connect(path) {
        Ok(_) => Err(Error::new(
            ErrorKind::AddrInUse,
            format!("another Minde instance owns {}", path.display()),
        )),
        Err(error) if error.kind() == ErrorKind::ConnectionRefused => fs::remove_file(path),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_directory_is_scoped_by_uid() {
        assert_eq!(
            runtime_directory(None, Path::new("/tmp"), 1234),
            PathBuf::from("/tmp/minde-1234")
        );
    }

    #[test]
    fn configured_runtime_directory_is_preserved() {
        assert_eq!(
            runtime_directory(Some(OsString::from("/run/user/42")), Path::new("/tmp"), 42),
            PathBuf::from("/run/user/42")
        );
    }

    #[test]
    fn rejects_world_accessible_directory() {
        let root = std::env::temp_dir().join(format!(
            "minde-runtime-test-{}-{}",
            std::process::id(),
            effective_uid()
        ));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, Permissions::from_mode(0o755)).unwrap();
        let metadata = fs::symlink_metadata(&root).unwrap();
        assert_eq!(
            validate_private_directory(&root, &metadata, effective_uid())
                .unwrap_err()
                .kind(),
            ErrorKind::PermissionDenied
        );
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn refuses_to_remove_a_regular_file() {
        let path =
            std::env::temp_dir().join(format!("minde-runtime-file-test-{}", std::process::id()));
        fs::write(&path, b"keep me").unwrap();
        assert_eq!(
            remove_stale_socket(&path).unwrap_err().kind(),
            ErrorKind::PermissionDenied
        );
        assert_eq!(fs::read(&path).unwrap(), b"keep me");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn refuses_to_remove_an_active_socket() {
        let path = std::env::temp_dir().join(format!(
            "minde-runtime-socket-test-{}",
            std::process::id()
        ));
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert_eq!(
            remove_stale_socket(&path).unwrap_err().kind(),
            ErrorKind::AddrInUse
        );
        assert!(path.exists());
        drop(listener);
        fs::remove_file(path).unwrap();
    }
}
