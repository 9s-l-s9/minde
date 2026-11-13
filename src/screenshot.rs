// SPDX-License-Identifier: GPL-3.0-or-later

//! Confined, non-overwriting screenshot publication. Directory descriptors
//! keep deferred writes from following a replaced path or a symlink.

use smithay::reexports::rustix::{
    fs::{
        AtFlags, Mode, OFlags, RenameFlags, mkdirat, open, openat, renameat_with, statat, unlinkat,
    },
    io::Errno,
};
use std::{
    ffi::OsString,
    fs::File,
    io::{self, Write},
    os::unix::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

pub(crate) fn directory() -> io::Result<PathBuf> {
    let path = match std::env::var_os("MINDE_SCREENSHOT_DIR").filter(|p| !p.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => crate::runtime_dir::socket_path("minde/screenshots")?,
    };
    if !path.is_absolute() || path.components().any(|c| c == Component::ParentDir) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "screenshot directory must be absolute without '..'",
        ));
    }
    Ok(path)
}

const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

fn descend(parent: &File, name: &std::ffi::OsStr, create: bool) -> io::Result<File> {
    if create {
        match mkdirat(parent, name, Mode::from_raw_mode(0o700)) {
            Ok(()) | Err(Errno::EXIST) => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(openat(parent, name, DIRECTORY_FLAGS, Mode::empty())?.into())
}

#[derive(Debug, Clone)]
pub struct Destination {
    parent: Arc<File>,
    name: OsString,
    path: PathBuf,
}

impl Destination {
    pub(crate) fn prepare(path: &Path) -> io::Result<Self> {
        Self::prepare_at(&directory()?, path)
    }

    fn prepare_at(root: &Path, path: &Path) -> io::Result<Self> {
        let invalid = || {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "screenshot path must stay below its configured directory",
            )
        };
        if !root.is_absolute()
            || path.as_os_str().as_bytes().ends_with(b"/")
            || path.components().any(|c| c == Component::ParentDir)
        {
            return Err(invalid());
        }
        let relative = path.strip_prefix(root).map_err(|_| invalid())?;
        let mut parts: Vec<_> = relative
            .components()
            .map(|c| match c {
                Component::Normal(name) => Ok(name),
                _ => Err(invalid()),
            })
            .collect::<io::Result<_>>()?;
        let name = parts.pop().ok_or_else(invalid)?.to_os_string();
        let mut parent: File = open("/", DIRECTORY_FLAGS, Mode::empty())?.into();
        for component in root.components() {
            match component {
                Component::RootDir => (),
                Component::Normal(name) => parent = descend(&parent, name, true)?,
                _ => return Err(invalid()),
            }
        }
        for name in parts {
            parent = descend(&parent, name, false)?;
        }
        match statat(&parent, &name, AtFlags::SYMLINK_NOFOLLOW) {
            Err(Errno::NOENT) => (),
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "screenshot destination already exists",
                ));
            }
            Err(error) => return Err(error.into()),
        }
        Ok(Self {
            parent: Arc::new(parent),
            name,
            path: path.to_path_buf(),
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn write_rgba(&self, width: u32, height: u32, pixels: &[u8]) -> io::Result<()> {
        let bytes = crate::png::encode_rgba(width, height, pixels)?;
        self.publish(&bytes)
    }

    fn publish(&self, bytes: &[u8]) -> io::Result<()> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..16 {
            let name = format!(
                ".minde-screenshot-{}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let fd = match openat(
                &*self.parent,
                &name,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            ) {
                Ok(fd) => fd,
                Err(Errno::EXIST) => continue,
                Err(error) => return Err(error.into()),
            };
            let mut file = File::from(fd);
            let result = (|| {
                file.write_all(bytes)?;
                file.flush()?;
                // Unlike rename(), NOREPLACE also rejects a destination
                // created after prepare(), including a dangling symlink.
                renameat_with(
                    &*self.parent,
                    &name,
                    &*self.parent,
                    &self.name,
                    RenameFlags::NOREPLACE,
                )?;
                Ok(())
            })();
            let _ = unlinkat(&*self.parent, &name, AtFlags::empty());
            return result;
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "cannot allocate screenshot temporary file",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };

    struct Directory(PathBuf);
    impl Directory {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("minde-screenshot-{}-{label}", std::process::id()));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn prepare(&self, name: &str) -> io::Result<Destination> {
            Destination::prepare_at(&self.0, &self.0.join(name))
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn publishes_a_complete_private_png_without_temporary_files() {
        let root = Directory::new("png");
        let target = root.prepare("image.png").unwrap();
        assert!(!target.path().exists());
        target.write_rgba(1, 1, &[255, 0, 0, 255]).unwrap();
        let bytes = fs::read(target.path()).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        assert!(bytes.windows(4).any(|v| v == b"IEND"));
        assert_eq!(
            fs::metadata(target.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
        assert_eq!(
            root.prepare("image.png").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
    }

    #[test]
    fn rejects_escape_paths_and_symlinks_without_touching_targets() {
        let root = Directory::new("paths");
        let outside = Directory::new("outside");
        fs::write(outside.0.join("keep"), b"keep").unwrap();
        symlink(&outside.0, root.0.join("link")).unwrap();
        symlink(outside.0.join("keep"), root.0.join("file")).unwrap();
        symlink(outside.0.join("missing"), root.0.join("dangling")).unwrap();
        for path in [
            outside.0.join("new"),
            root.0.join("../new"),
            root.0.join("link/new"),
            root.0.join("file"),
            root.0.join("dangling"),
            root.0.clone(),
        ] {
            assert!(
                Destination::prepare_at(&root.0, &path).is_err(),
                "{}",
                path.display()
            );
        }
        assert!(
            Destination::prepare_at(
                &root.0,
                &PathBuf::from(format!("{}-other/new", root.0.display()))
            )
            .is_err()
        );
        assert_eq!(fs::read(outside.0.join("keep")).unwrap(), b"keep");
        assert_eq!(fs::read_dir(&outside.0).unwrap().count(), 1);
    }

    #[test]
    fn deferred_write_does_not_follow_a_replaced_parent() {
        let root = Directory::new("parent");
        let outside = Directory::new("replacement");
        fs::create_dir(root.0.join("child")).unwrap();
        let target = root.prepare("child/image.png").unwrap();
        fs::rename(root.0.join("child"), root.0.join("original")).unwrap();
        symlink(&outside.0, root.0.join("child")).unwrap();
        target.write_rgba(1, 1, &[0; 4]).unwrap();
        assert!(root.0.join("original/image.png").is_file());
        assert_eq!(fs::read_dir(&outside.0).unwrap().count(), 0);
    }

    #[test]
    fn concurrent_publications_never_replace_the_winner() {
        let root = Directory::new("race");
        let first = root.prepare("image.png").unwrap();
        let second = root.prepare("image.png").unwrap();
        let a = std::thread::spawn(move || first.publish(b"first"));
        let b = std::thread::spawn(move || second.publish(b"second"));
        let (a, b) = (a.join().unwrap(), b.join().unwrap());
        assert_ne!(a.is_ok(), b.is_ok());
        assert_eq!(
            fs::read(root.0.join("image.png")).unwrap(),
            if a.is_ok() {
                b"first".as_slice()
            } else {
                b"second".as_slice()
            }
        );
        assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
    }
}
