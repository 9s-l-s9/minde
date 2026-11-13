// SPDX-License-Identifier: GPL-3.0-or-later

//! Secure paths for per-user runtime sockets and the state directory.

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

fn runtime_directory(runtime: Option<OsString>, temporary: Option<OsString>, uid: u32) -> PathBuf {
    runtime
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            temporary
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(format!("minde-{uid}"))
        })
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

/// The per-user state directory, `$XDG_STATE_HOME/minde` or
/// `~/.local/state/minde`: crash log, session logs. Mirrors the resolution
/// in `scripts/mindectl`; keep the two in step.
pub fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        })
        .join("minde")
}

/// Returns a socket path beneath a private, current-user runtime directory.
/// When XDG_RUNTIME_DIR is absent or empty, creates `${TMPDIR:-/tmp}/minde-UID`
/// with mode 0700. Empty TMPDIR also falls back to `/tmp`.
pub fn socket_path(name: &str) -> Result<PathBuf> {
    socket_path_for(
        name,
        std::env::var_os("XDG_RUNTIME_DIR"),
        std::env::var_os("TMPDIR"),
        effective_uid(),
    )
}

fn socket_path_for(
    name: &str,
    configured: Option<OsString>,
    temporary: Option<OsString>,
    uid: u32,
) -> Result<PathBuf> {
    let fallback = configured.as_ref().is_none_or(|path| path.is_empty());
    let directory = runtime_directory(configured, temporary, uid);
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
    fn state_directory_ends_in_minde() {
        assert_eq!(state_dir().file_name().unwrap(), "minde");
    }

    #[test]
    fn fallback_directory_is_scoped_by_uid() {
        for runtime in [None, Some(OsString::new())] {
            for temporary in [None, Some(OsString::new()), Some(OsString::from("/tmp"))] {
                assert_eq!(
                    runtime_directory(runtime.clone(), temporary, 1234),
                    PathBuf::from("/tmp/minde-1234")
                );
            }
            assert_eq!(
                runtime_directory(runtime, Some(OsString::from("/custom tmp")), 1234),
                PathBuf::from("/custom tmp/minde-1234")
            );
        }
    }

    #[test]
    fn configured_runtime_directory_is_preserved() {
        assert_eq!(
            runtime_directory(
                Some(OsString::from("/run/user/42")),
                Some(OsString::from("/custom tmp")),
                42
            ),
            PathBuf::from("/run/user/42")
        );
    }

    #[test]
    fn fallback_socket_paths_create_and_validate_a_private_directory() {
        let root = std::env::temp_dir().join(format!("minde-runtime-paths-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let temporary = root.join("temporary files");
        fs::create_dir(&temporary).unwrap();
        let uid = effective_uid();
        let private = temporary.join(format!("minde-{uid}"));
        for runtime in [None, Some(OsString::new())] {
            for name in ["minde-ipc.sock", "minde-events.sock"] {
                assert_eq!(
                    socket_path_for(name, runtime.clone(), Some(temporary.clone().into()), uid)
                        .unwrap(),
                    private.join(name)
                );
            }
        }
        assert_eq!(
            fs::metadata(&private).unwrap().permissions().mode() & 0o777,
            0o700
        );
        fs::set_permissions(&private, Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            socket_path_for(
                "test.sock",
                Some(OsString::new()),
                Some(temporary.into()),
                uid
            )
            .unwrap_err()
            .kind(),
            ErrorKind::PermissionDenied
        );
        let configured = root.join("missing-configured");
        assert_eq!(
            socket_path_for("test.sock", Some(configured.clone().into()), None, uid)
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );
        assert!(!configured.exists());
        fs::remove_dir_all(root).unwrap();
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
        let path =
            std::env::temp_dir().join(format!("minde-runtime-socket-test-{}", std::process::id()));
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
