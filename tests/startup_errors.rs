// SPDX-License-Identifier: GPL-3.0-or-later
//! Run the real compositor without a host display. Socket failures must stop
//! startup before Guile/backend setup and leave existing listeners untouched.

use std::{
    fs::{self, File},
    os::{fd::AsRawFd, unix::net::UnixListener},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

struct Scratch(std::path::PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_startup_error(root: &Path, runtime: Option<&Path>, expected: &str) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_minde"));
    command
        .arg("--winit")
        .env_remove("XDG_RUNTIME_DIR")
        .env_remove("_MINDE_INIT_RECOVERY")
        .env("XDG_STATE_HOME", root.join("state"))
        .env("DISPLAY", "")
        .env("WAYLAND_DISPLAY", "")
        .env("XKB_DEFAULT_LAYOUT", "us")
        .env("XKB_DEFAULT_VARIANT", "")
        .env("XKB_DEFAULT_OPTIONS", "")
        .env("RUST_LOG", "minde=info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(runtime) = runtime {
        command.env("XDG_RUNTIME_DIR", runtime);
    }
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("startup did not reject {expected} within ten seconds");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(1), "{diagnostic}");
    assert!(
        diagnostic.contains("cannot bind a Wayland socket"),
        "{diagnostic}"
    );
    assert!(diagnostic.contains(expected), "{diagnostic}");
    assert!(!diagnostic.contains("panicked"), "{diagnostic}");
    assert!(!root.join("state/minde/crash.log").exists(), "{diagnostic}");
}

#[test]
fn missing_and_exhausted_runtime_sockets_are_startup_errors() {
    let root = Scratch(std::env::temp_dir().join(format!("minde-startup-{}", std::process::id())));
    fs::create_dir(&root.0).unwrap();
    assert_startup_error(&root.0, None, "XDG_RUNTIME_DIR");

    let runtime = root.0.join("runtime");
    fs::create_dir(&runtime).unwrap();
    let mut held = Vec::new();
    for number in 1..=32 {
        let path = runtime.join(format!("wayland-{number}"));
        let lock = File::create(path.with_extension("lock")).unwrap();
        // Match libwayland's advisory socket lock while keeping a real
        // listener alive. Child processes must not inherit the lock FD.
        assert_eq!(
            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        let listener = UnixListener::bind(&path).unwrap();
        held.push((lock, listener, path));
    }
    assert_startup_error(&root.0, Some(&runtime), "already in use");
    for (_, listener, path) in &held {
        let _client = std::os::unix::net::UnixStream::connect(path).unwrap();
        listener.set_nonblocking(true).unwrap();
        listener
            .accept()
            .expect("startup replaced an occupied socket");
    }
}
