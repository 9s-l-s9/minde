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

fn init_logging() {
    if let Ok(env_filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    } else {
        tracing_subscriber::fmt().init();
    }
}
