// SPDX-License-Identifier: GPL-3.0-or-later
//! Build stamp and Cargo inputs, also exercised by a standalone fixture crate.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub fn emit() {
    println!("cargo:rerun-if-env-changed=MINDE_BUILD_REVISION");
    let revision = std::env::var("MINDE_BUILD_REVISION")
        .ok()
        .filter(|revision| !revision.trim().is_empty())
        .or_else(|| {
            let root = git_text(Path::new("."), &["rev-parse", "--show-toplevel"])?;
            watch_repository(Path::new(&root), &mut BTreeSet::new());
            git_revision(Path::new(&root))
        })
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=MINDE_BUILD_REVISION={revision}");
}

fn git_output(root: &Path, arguments: &[&str]) -> Option<Output> {
    Command::new("git")
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(arguments)
        .output()
        .ok()
}

fn git(root: &Path, arguments: &[&str]) -> Option<Vec<u8>> {
    let output = git_output(root, arguments)?;
    output.status.success().then_some(output.stdout)
}

fn git_revision(root: &Path) -> Option<String> {
    let mut revision = git_text(root, &["describe", "--always", "--abbrev=12"])?;
    // describe --dirty refreshes the index even with GIT_OPTIONAL_LOCKS=0.
    // A read-only diff avoids touching our own Cargo input during the build.
    let diff = git_output(
        root,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--quiet",
            "HEAD",
            "--",
        ],
    )?;
    match diff.status.code() {
        Some(0) => (),
        Some(1) => revision.push_str("-dirty"),
        _ => return None,
    }
    Some(revision)
}

fn git_text(root: &Path, arguments: &[&str]) -> Option<String> {
    String::from_utf8(git(root, arguments)?)
        .ok()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

fn watch(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
}

fn watch_repository(root: &Path, visited: &mut BTreeSet<PathBuf>) {
    if !visited.insert(root.to_owned()) {
        return;
    }
    // Git resolves per-worktree HEAD/index and shared refs itself. Watch refs
    // directories to catch new/deleted tags and loose refs after packing, but
    // never scan the object database, build output or untracked vendor trees.
    for name in [
        "HEAD",
        "index",
        "refs",
        "packed-refs",
        "reftable",
        "shallow",
        "config",
    ] {
        if let Some(path) = git_text(
            root,
            &["rev-parse", "--path-format=absolute", "--git-path", name],
        ) {
            let path = Path::new(&path);
            // Missing optional metadata would make Cargo rerun on every build.
            if path.exists() {
                watch(path);
            }
        }
    }
    if root.join(".git").is_file() {
        watch(&root.join(".git"));
    }
    // Unstaged edits do not change the index. Watch tracked files too, including
    // Scheme/docs and initialized submodules, to follow git describe's dirty bit.
    if let Some(files) = git(root, &["ls-files", "--stage", "-z"]) {
        for entry in files.split(|byte| *byte == 0) {
            if let Some(separator) = entry.iter().position(|byte| *byte == b'\t') {
                let path = root.join(OsStr::from_bytes(&entry[separator + 1..]));
                if entry.starts_with(b"160000 ") {
                    if path.join(".git").exists() {
                        watch_repository(&path, visited);
                    } else {
                        // Normally an empty directory until submodule init.
                        watch(&path);
                    }
                } else {
                    watch(&path);
                }
            }
        }
    }
}
