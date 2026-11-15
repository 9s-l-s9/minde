// SPDX-License-Identifier: GPL-3.0-or-later

//! Run a unit test that needs the real compositor state in its own process.

use std::{
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const CHILD: &str = "MINDE_ISOLATED_TEST";

/// Guile and the compositor globals belong to one thread and process, and
/// the Wayland socket needs a private runtime directory. Returns `true` in
/// the child, which then runs the test body. The parent runs `test` (its
/// full path) in that child, requires it to pass and returns `false`.
pub(crate) fn in_child(test: &str) -> bool {
    if std::env::var_os(CHILD).is_some() {
        return true;
    }
    let dir = std::env::temp_dir().join(format!(
        "minde-isolated-{}-{}",
        std::process::id(),
        test.rsplit("::").next().unwrap()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture"])
        .env(CHILD, "1")
        .env("XDG_RUNTIME_DIR", &dir)
        .env("WAYLAND_DISPLAY", "wayland-1")
        .env_remove("NESTED_WAYLAND_DISPLAY")
        .env_remove("MINDE_SCREENSHOT_DIR")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            child.kill().unwrap();
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let result = child.wait_with_output().unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    let output = String::from_utf8_lossy(&result.stdout);
    // A misspelled path selects no test, which the harness reports as success.
    assert!(
        result.status.success() && output.contains("1 passed"),
        "{output}\n{}",
        String::from_utf8_lossy(&result.stderr)
    );
    false
}
