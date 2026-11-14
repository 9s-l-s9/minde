// SPDX-License-Identifier: GPL-3.0-or-later
use super::{lock::HostPixels, wire::*};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};
fn arm(fault: &Path, stage: &str, kind: &str, repeat: &str) {
    let next = fault.with_extension("new");
    fs::write(
        &next,
        format!(
            "{stage} {kind} {repeat} {}\n",
            env::var("MINDE_TEST_TARGET_PID").unwrap()
        ),
    )
    .unwrap();
    fs::rename(next, fault).unwrap();
}
fn color_is(pixels: &HostPixels, color: u32) -> bool {
    pixels.samples().iter().all(|p| *p == color)
}
fn paint(c: &mut Client, color: u32, size: Option<(u32, u32)>) -> u32 {
    let callback = c.alloc();
    c.send(c.surface, 3, &words(&[callback]));
    let size = size.unwrap_or(c.configured_size);
    c.paint(c.surface, size.0, size.1, color);
    callback
}
fn presented(c: &mut Client, callback: u32, pixels: &HostPixels, color: u32) {
    c.event(callback, 0);
    until(|| color_is(pixels, color), "retry did not present frame");
}
fn output_command(args: &[&str]) {
    let out = Command::new("timeout")
        .arg("5")
        .args(args)
        .env("DISPLAY", "")
        .env(
            "WAYLAND_DISPLAY",
            env::var("NESTED_WAYLAND_DISPLAY").unwrap(),
        )
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
pub fn run(stage: &str) {
    assert!(["render", "swap"].contains(&stage));
    struct Fault(PathBuf);
    impl Drop for Fault {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    let fault = Fault(env::var("MINDE_TEST_EGL_FAULT").unwrap().into());
    let log = PathBuf::from(env::var("MINDE_TEST_COMPOSITOR_LOG").unwrap());
    let injected = |marker: &str| fs::read_to_string(&log).unwrap().matches(marker).count();
    let pixels = HostPixels::new();
    // The lock renderer draws its cursor. Keep it away from the sample grid
    // so every sample still has to match the submitted surface color.
    command(
        "xdotool",
        &[
            "mousemove",
            "--window",
            &env::var("MINDE_SESSION_LOCK_TEST_WINDOW").unwrap(),
            "16",
            "16",
        ],
    );
    let mut desktop = Client::new("desktop for rendering failures");
    ipc("(wm-clear-message)");
    desktop.toplevel(true);
    desktop.map_on_configure = false;
    until(
        || color_is(&pixels, 0xff00ff),
        "initial desktop not visible",
    );
    let mut owner = None;
    let mut lock_size = None;
    if stage == "render" {
        for (phase, color) in [("render", 0x004488), ("swap", 0x884400)] {
            arm(&fault.0, phase, "temporary", "once");
            let callback = paint(&mut desktop, color, None);
            until(|| !fault.0.exists(), "failure not injected");
            presented(&mut desktop, callback, &pixels, color);
            println!("ok - desktop recovers from {phase} failure without another commit");
        }
        let before = injected("egl-fault: injected render");
        arm(&fault.0, "render", "temporary", "hold");
        let callback = paint(&mut desktop, 0x228844, None);
        until(
            || injected("egl-fault: injected render") >= before + 3,
            "failures not retried",
        );
        desktop.roundtrip();
        assert!(!desktop.events.iter().any(|e| e.is(callback, 0)));
        assert!(color_is(&pixels, 0x884400));
        fs::remove_file(&fault.0).unwrap();
        presented(&mut desktop, callback, &pixels, 0x228844);
        println!("ok - repeated failures withhold frame callbacks and retry automatically");
        for phase in ["render", "swap"] {
            for (off, on) in [
                (
                    vec!["wlopm", "--off", "winit"],
                    vec!["wlopm", "--on", "winit"],
                ),
                (
                    vec!["wlr-randr", "--output", "winit", "--off"],
                    vec!["wlr-randr", "--output", "winit", "--on"],
                ),
            ] {
                arm(&fault.0, phase, "temporary", "once");
                output_command(&off);
                until(|| !fault.0.exists(), "blackout failure not injected");
                until(|| color_is(&pixels, 0), "blackout did not recover");
                output_command(&on);
                desktop.roundtrip();
                let callback = paint(&mut desktop, 0x228844, None);
                presented(&mut desktop, callback, &pixels, 0x228844);
                println!("ok - {} blackout recovers from {phase} failure", off[0]);
            }
        }
        for phase in ["render", "swap"] {
            let marker = format!("egl-fault: injected {phase}");
            let before = injected(&marker);
            arm(&fault.0, phase, "temporary", "hold");
            let mut c = Client::new("lock owner during rendering failures");
            let lock = c.request_lock();
            until(
                || injected(&marker) >= before + 3,
                "lock blackout not retried",
            );
            c.roundtrip();
            assert!(
                !c.events.iter().any(|e| e.is(lock, 0)),
                "failed frame confirmed lock"
            );
            fs::remove_file(&fault.0).unwrap();
            c.event(lock, 0);
            assert!(color_is(&pixels, 0), "confirmed before blackout visible");
            println!("ok - lock confirmation waits through repeated {phase} failures");
            if phase == "render" {
                c.send(lock, 2, &[]);
                c.roundtrip();
                drop(c);
                let callback = paint(&mut desktop, 0x228844, None);
                presented(&mut desktop, callback, &pixels, 0x228844);
            } else {
                owner = Some((c, lock));
            }
        }
        let (c, lock) = owner.as_mut().unwrap();
        let surface = c.lock_surface(*lock, true);
        let configure = c.event(surface, 0);
        lock_size = Some((configure.word(1), configure.word(2)));
        until(
            || color_is(&pixels, 0x004080),
            "initial lock surface not visible",
        );
        for (phase, color) in [("render", 0x224466), ("swap", 0x446622)] {
            arm(&fault.0, phase, "temporary", "once");
            let callback = paint(c, color, lock_size);
            until(|| !fault.0.exists(), "lock failure not injected");
            presented(c, callback, &pixels, color);
            println!("ok - lock surface recovers from {phase} failure");
        }
    }
    arm(&fault.0, stage, "fatal", "once");
    let c = owner.as_mut().map(|(c, _)| c).unwrap_or(&mut desktop);
    paint(c, 0x123456, lock_size);
    until(|| !fault.0.exists(), "context loss not injected");
    let end = Instant::now() + TIMEOUT;
    loop {
        match c.read(end) {
            Ok(_) => {}
            Err(Error::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset
                ) =>
            {
                break;
            }
            result => panic!("context loss did not close connection: {result:?}"),
        }
    }
    println!("ok - {stage} context loss disconnects clients cleanly");
}
