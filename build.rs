fn main() {
    // Locate libguile-3.0 via pkg-config and emit the required link flags.
    pkg_config::Config::new()
        .atleast_version("3.0")
        .probe("guile-3.0")
        .expect("guile-3.0 not found via pkg-config; is it in the guix shell manifest?");

    println!("cargo:rerun-if-env-changed=MINDE_BUILD_REVISION");
    let revision = std::env::var("MINDE_BUILD_REVISION")
        .ok()
        .filter(|revision| !revision.trim().is_empty())
        .or_else(git_revision)
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=MINDE_BUILD_REVISION={revision}");
}

fn git_revision() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["describe", "--always", "--dirty", "--abbrev=12"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|revision| revision.trim().to_owned())
        .filter(|revision| !revision.is_empty())
}
