// SPDX-License-Identifier: GPL-3.0-or-later
//! Exercise the real build-stamp code through Cargo's incremental cache, in
//! isolated repositories. No compositor build or developer Git state is changed.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn command(program: &str, root: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Minde test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Minde test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid");
    command
}

fn output(command: &mut Command) -> String {
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{command:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap().trim().to_owned()
}

fn git(root: &Path, arguments: &[&str]) -> String {
    output(command("git", root).args(arguments))
}

fn fixture(root: &Path) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = 'stamp-fixture'\nversion = '0.0.0'\nedition = '2024'\n",
    )
    .unwrap();
    fs::write(
        root.join("src/main.rs"),
        "fn main() { println!(\"{}\", env!(\"MINDE_BUILD_REVISION\")); }\n",
    )
    .unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("build_revision.rs"),
        root.join("build_revision.rs"),
    )
    .unwrap();
    fs::write(
        root.join("build.rs"),
        r#"
mod build_revision;
fn main() {
    use std::io::Write;
    build_revision::emit();
    let target = std::env::var_os("CARGO_TARGET_DIR").unwrap();
    writeln!(std::fs::OpenOptions::new().create(true).append(true)
        .open(std::path::Path::new(&target).join("stamp-runs")).unwrap(), "run").unwrap();
}
"#,
    )
    .unwrap();
    fs::write(root.join(".gitignore"), "/target\n").unwrap();
    fs::write(root.join("policy.scm"), "; original\n").unwrap();
    output(command(env!("CARGO"), root).args(["generate-lockfile", "--offline"]));
}

fn build(root: &Path, revision: Option<&str>) -> String {
    let mut cargo = command(env!("CARGO"), root);
    cargo
        .args(["build", "--offline", "--quiet"])
        .env("CARGO_TARGET_DIR", root.join("target"))
        .env_remove("MINDE_BUILD_REVISION");
    if let Some(revision) = revision {
        cargo.env("MINDE_BUILD_REVISION", revision);
    }
    output(&mut cargo);
    output(&mut Command::new(root.join("target/debug/stamp-fixture")))
}

fn index_mtime(root: &Path) -> std::time::SystemTime {
    let index = PathBuf::from(git(
        root,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    ));
    fs::metadata(index).unwrap().modified().unwrap()
}

fn check_stamp(root: &Path) {
    // The oracle itself refreshes the Git index; run it before Cargo records
    // timestamps, then assert that the build leaves that index untouched.
    let expected = git(root, &["describe", "--always", "--dirty", "--abbrev=12"]);
    let before = index_mtime(root);
    assert_eq!(build(root, None), expected);
    assert_eq!(index_mtime(root), before, "build rewrote the Git index");
}

fn check_cached(root: &Path, revision: Option<&str>) {
    let before = fs::read(root.join("target/stamp-runs")).unwrap();
    build(root, revision);
    assert_eq!(
        fs::read(root.join("target/stamp-runs")).unwrap(),
        before,
        "unchanged build reran its stamp"
    );
}

#[test]
fn cargo_stamp_tracks_git_and_respects_release_overrides() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("minde-build-stamp-{}-{nonce}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    // Only own cleanup after creating the directory successfully.
    let scratch = Scratch(directory);
    let root = scratch.0.join("checkout with spaces");
    fixture(&root);
    git(&root, &["init", "--initial-branch=main"]);
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "Initial fixture"]);
    check_stamp(&root);
    check_cached(&root, None);
    fs::write(root.join("untracked.txt"), "ignored by the version stamp\n").unwrap();
    check_cached(&root, None);
    fs::remove_file(root.join("untracked.txt")).unwrap();

    // Neither an unstaged Scheme edit nor an empty commit changes Rust input.
    fs::write(root.join("policy.scm"), "; edited\n").unwrap();
    let before = index_mtime(&root);
    assert!(build(&root, None).ends_with("-dirty"));
    assert_eq!(
        index_mtime(&root),
        before,
        "dirty detection rewrote the Git index"
    );
    check_stamp(&root);
    git(&root, &["restore", "policy.scm"]);
    check_stamp(&root);
    fs::write(root.join("policy.scm"), "; staged\n").unwrap();
    git(&root, &["add", "policy.scm"]);
    check_stamp(&root);
    git(&root, &["commit", "-m", "Change policy"]);
    check_stamp(&root);
    git(&root, &["commit", "--allow-empty", "-m", "Advance HEAD"]);
    check_stamp(&root);
    git(&root, &["switch", "--detach", "HEAD~1"]);
    check_stamp(&root);
    git(&root, &["switch", "main"]);
    check_stamp(&root);

    // Shared packed refs and annotated tags influence git describe too.
    git(&root, &["pack-refs", "--all", "--prune"]);
    check_stamp(&root);
    check_cached(&root, None);
    git(
        &root,
        &["commit", "--allow-empty", "-m", "Advance packed branch"],
    );
    check_stamp(&root);
    git(&root, &["tag", "-a", "v-test", "-m", "Test tag"]);
    check_stamp(&root);
    assert_eq!(build(&root, None), "v-test");
    git(&root, &["tag", "-d", "v-test"]);
    check_stamp(&root);

    let linked = scratch.0.join("linked checkout");
    git(
        &root,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    check_stamp(&linked);
    git(
        &linked,
        &["commit", "--allow-empty", "-m", "Advance linked HEAD"],
    );
    check_stamp(&linked);
    fs::write(linked.join("policy.scm"), "; linked edit\n").unwrap();
    check_stamp(&linked);
    git(&linked, &["restore", "policy.scm"]);
    check_stamp(&linked);
    check_cached(&linked, None);

    // An initialized submodule's HEAD can change without file content changes.
    let submodule = scratch.0.join("submodule");
    fs::create_dir(&submodule).unwrap();
    git(&submodule, &["init", "--initial-branch=main"]);
    fs::write(submodule.join("module.scm"), "; module\n").unwrap();
    git(&submodule, &["add", "."]);
    git(&submodule, &["commit", "-m", "Initial module"]);
    git(
        &root,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            submodule.to_str().unwrap(),
            "module",
        ],
    );
    git(&root, &["commit", "-m", "Track module"]);
    check_stamp(&root);
    git(&root, &["submodule", "deinit", "-f", "module"]);
    check_stamp(&root);
    check_cached(&root, None);
    git(
        &root,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "update",
            "--init",
        ],
    );
    check_stamp(&root);
    git(
        &root.join("module"),
        &["commit", "--allow-empty", "-m", "Advance module HEAD"],
    );
    check_stamp(&root);
    assert!(build(&root, None).ends_with("-dirty"));
    git(&root, &["add", "module"]);
    git(&root, &["commit", "-m", "Record module HEAD"]);
    check_stamp(&root);
    fs::write(root.join("module/module.scm"), "; module edit\n").unwrap();
    check_stamp(&root);
    assert!(build(&root, None).ends_with("-dirty"));

    assert_eq!(build(&root, Some("release-one")), "release-one");
    git(
        &root,
        &[
            "commit",
            "--allow-empty",
            "-m",
            "Release override ignores Git",
        ],
    );
    check_cached(&root, Some("release-one"));
    assert_eq!(build(&root, Some("release-two")), "release-two");
    check_stamp(&root);

    let archive = scratch.0.join("source archive");
    fixture(&archive);
    assert_eq!(build(&archive, None), "unknown");
    check_cached(&archive, None);
    assert_eq!(
        build(&archive, Some("archive-revision")),
        "archive-revision"
    );
    assert_eq!(build(&archive, Some(" ")), "unknown");
}
