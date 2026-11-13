// SPDX-License-Identifier: GPL-3.0-or-later
//! Validate real archives through the release gate and Cargo's offline resolver.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write(root: &Path, name: &str, data: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}

fn command(program: &str, root: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .current_dir(root)
        .env_remove("LD_LIBRARY_PATH")
        .env("CARGO_HOME", root.join("cargo-home"))
        .env("CARGO_NET_OFFLINE", "true");
    command
}

fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn dependency(root: &Path, name: &str, version: &str, dependencies: &str) {
    write(
        root,
        &format!("vendor/{name}/Cargo.toml"),
        &format!(
            "[package]\nname = '{name}'\nversion = '{version}'\nedition = '2021'\n{dependencies}"
        ),
    );
    write(root, &format!("vendor/{name}/src/lib.rs"), "");
    write(
        root,
        &format!("vendor/{name}/.cargo-checksum.json"),
        r#"{"files":{},"package":null}"#,
    );
}

fn pack(scratch: &Path, name: &str, vendored: bool) -> PathBuf {
    let archive = scratch.join(name);
    let mut tar = command("tar", scratch);
    tar.args(["--exclude=.cargo", "--exclude=cargo-home"]);
    if !vendored {
        tar.arg("--exclude=vendor");
    }
    success(
        tar.arg("-czf")
            .arg(&archive)
            .arg("minde-0.0.1")
            .output()
            .unwrap(),
    );
    archive
}

fn check(scratch: &Path, source: &Path, archive: &Path, error: Option<&str>) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/check-release-archives.sh");
    let output = command("sh", scratch)
        .arg(script)
        .arg("0.0.1")
        .arg(source)
        .arg(archive)
        .output()
        .unwrap();
    if let Some(error) = error {
        assert!(
            !output.status.success(),
            "accepted invalid archive: {error}"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(error), "{stderr}");
        assert!(
            !String::from_utf8_lossy(&output.stdout).contains("passed"),
            "printed success for an invalid archive"
        );
    } else {
        success(output);
    }
}

#[test]
fn archives_must_satisfy_the_complete_locked_graph_offline() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("minde-archive-test-{}-{nonce}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let scratch = Scratch(directory);
    let root = scratch.0.join("minde-0.0.1");
    write(
        &root,
        "Cargo.toml",
        "[package]\nname = 'archive-fixture'\nversion = '0.0.1'\nedition = '2021'\n\
         [dependencies]\nsmithay = '=0.0.1'\n",
    );
    write(&root, "src/lib.rs", "");
    let config = "[source.crates-io]\nreplace-with = 'vendored'\n\
                  [source.vendored]\ndirectory = 'vendor'\n";
    write(&root, "guix/cargo-config.toml", config);
    write(&root, ".cargo/config.toml", config);
    dependency(
        &root,
        "smithay",
        "0.0.1",
        "[dependencies]\nhelper = '=0.0.1'\n",
    );
    dependency(&root, "helper", "0.0.1", "");
    success(
        command(env!("CARGO"), &root)
            .args(["generate-lockfile", "--offline"])
            .output()
            .unwrap(),
    );
    let source = pack(&scratch.0, "source.tar.gz", false);
    let good = pack(&scratch.0, "complete.tar.gz", true);
    check(&scratch.0, &source, &good, None);

    fs::remove_dir_all(root.join("vendor/helper")).unwrap();
    let missing = pack(&scratch.0, "missing.tar.gz", true);
    check(
        &scratch.0,
        &source,
        &missing,
        Some("no matching package named `helper`"),
    );

    dependency(&root, "helper", "0.0.2", "");
    let wrong = pack(&scratch.0, "wrong-version.tar.gz", true);
    check(
        &scratch.0,
        &source,
        &wrong,
        Some("failed to select a version"),
    );

    dependency(&root, "helper", "0.0.1", "");
    dependency(&root, "extra", "0.0.1", "");
    let manifest = root.join("Cargo.toml");
    let updated = fs::read_to_string(&manifest).unwrap() + "extra = '=0.0.1'\n";
    fs::write(&manifest, updated).unwrap();
    let unlocked = pack(&scratch.0, "unlocked.tar.gz", true);
    check(&scratch.0, &source, &unlocked, Some("--locked"));
}
