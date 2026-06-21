//! minde: a small Wayland compositor whose policy layer (keybindings,
//! spawning, etc.) lives in embedded Guile Scheme, StumpWM-style.
//!
//! The compositor core (state.rs, handlers/, grabs/, winit.rs, input.rs) is
//! adapted from Smithay's `smallvil` example; see README.md for credit and
//! the exact upstream revision used.

mod guile;
mod handlers;

mod grabs;
mod input;
mod render;
mod state;
mod udev;
mod winit;

use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
pub use state::MindeState;

/// Which backend to run: a nested Wayland/X11 window (winit) or a
/// standalone DRM/udev/libinput session (tty).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Winit,
    Udev,
}

/// Parses `--tty` / `--winit` from argv; anything else is ignored (no
/// clap dependency, this is intentionally tiny). Default: auto-detect via
/// `WAYLAND_DISPLAY`/`DISPLAY` (nested if set, else standalone tty).
fn parse_backend() -> Backend {
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--tty" => return Backend::Udev,
            "--winit" => return Backend::Winit,
            _ => {}
        }
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some() {
        Backend::Winit
    } else {
        Backend::Udev
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();
    install_crash_log();

    let backend = parse_backend();
    tracing::info!(?backend, "selected backend");

    let mut event_loop: EventLoop<MindeState> = EventLoop::try_new()?;

    let display: Display<MindeState> = Display::new()?;

    let mut state = MindeState::new(&mut event_loop, display);

    // Guile must be initialized on the main thread, and after this call
    // every libguile function must be called from this same thread (except
    // through Guile's own REPL server, which manages its own thread).
    //
    // This must happen before backend init, which calls into Scheme (e.g.
    // `wm-on-output-geometry`) as soon as it knows the output size.
    guile::init(state.loop_signal.clone());

    match backend {
        Backend::Winit => {
            // Open a Wayland/X11 window for our nested compositor.
            crate::winit::init_winit(&mut event_loop, &mut state)?;
        }
        Backend::Udev => {
            // Standalone: own a VT directly via libseat/DRM/libinput.
            crate::udev::init_udev(&mut event_loop, &mut state)?;
        }
    }

    // Set WAYLAND_DISPLAY to our socket name so child processes (spawned
    // via `wm-spawn`) connect to us. This must happen only AFTER the
    // backend has been picked: if winit sees this variable, it opens our
    // nested window as a client of *ourselves* and deadlocks before the
    // loop runs.
    unsafe { std::env::set_var("WAYLAND_DISPLAY", &state.socket_name) };

    event_loop.run(None, &mut state, move |_state| {
        // minde is running; nothing extra to do per-iteration here, the
        // event sources (wayland socket, winit/udev, calloop) drive
        // everything.
    })?;

    Ok(())
}

/// A panic on the TTY backend takes the whole session down with no
/// visible output (frozen VT). Leave evidence: append panic message and
/// backtrace to $XDG_STATE_HOME/minde/crash.log (or ~/.local/state).
fn install_crash_log() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let state_dir = std::env::var("XDG_STATE_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
                    .join(".local/state")
            })
            .join("minde");
        let _ = std::fs::create_dir_all(&state_dir);
        let backtrace = std::backtrace::Backtrace::force_capture();
        let entry = format!(
            "==== {} minde panic ====\n{info}\n{backtrace}\n",
            chrono_free_timestamp()
        );
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(state_dir.join("crash.log"))
        {
            let _ = f.write_all(entry.as_bytes());
        }
        default_hook(info);
    }));
}

/// Seconds since the epoch -- enough to order crash-log entries without
/// pulling in a date/time dependency.
fn chrono_free_timestamp() -> String {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => format!("@{}", d.as_secs()),
        Err(_) => "@unknown".into(),
    }
}

fn init_logging() {
    if let Ok(env_filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    } else {
        tracing_subscriber::fmt().init();
    }
}
