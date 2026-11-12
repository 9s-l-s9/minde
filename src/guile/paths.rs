// SPDX-License-Identifier: GPL-3.0-or-later

//! Locate trusted, bundled Scheme code without reviving a release build tree.

use std::path::{Path, PathBuf};

pub fn scheme_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("MINDE_SCHEME_DIR") {
        return dir.into();
    }
    // A release executable must never load code from its former build path,
    // even if somebody recreates that directory after installation.
    let checkout = if cfg!(debug_assertions) {
        Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("scheme"))
    } else {
        None
    };
    let executable = std::env::current_exe()
        .expect("cannot locate bundled Scheme code; set MINDE_SCHEME_DIR explicitly");
    bundled_scheme_dir(&executable, checkout.as_deref())
}

fn bundled_scheme_dir(executable: &Path, checkout: Option<&Path>) -> PathBuf {
    let installed = executable
        .parent()
        .and_then(Path::parent)
        .expect("executable has no installation prefix; set MINDE_SCHEME_DIR explicitly")
        .join("share/minde/scheme");
    if !installed.is_dir()
        && let Some(checkout) = checkout.filter(|path| path.is_dir())
    {
        return checkout.to_path_buf();
    }
    // Keep the installed path even if it is missing. Startup will report the
    // missing policy instead of executing a recreated release build tree.
    installed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_policy_takes_precedence_and_release_never_uses_checkout() {
        let root = std::env::temp_dir().join(format!("minde-policy-path-{}", std::process::id()));
        let checkout = root.join("build/source/scheme");
        let executable = root.join("prefix/bin/minde");
        let installed = root.join("prefix/share/minde/scheme");
        std::fs::create_dir_all(&checkout).unwrap();

        // A development checkout remains usable without an installation.
        assert_eq!(bundled_scheme_dir(&executable, Some(&checkout)), checkout);
        // A recreated build directory is irrelevant to release binaries.
        assert_eq!(bundled_scheme_dir(&executable, None), installed);

        std::fs::create_dir_all(&installed).unwrap();
        assert_eq!(bundled_scheme_dir(&executable, Some(&checkout)), installed);
        assert_eq!(bundled_scheme_dir(&executable, None), installed);

        std::fs::remove_dir_all(root).unwrap();
    }
}
