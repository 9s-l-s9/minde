//! minde: a small Wayland compositor whose policy layer (keybindings,
//! spawning, etc.) lives in embedded Guile Scheme, StumpWM-style.
//!
//! The compositor core (state.rs, handlers/, grabs/, winit.rs, input.rs) is
//! adapted from Smithay's `smallvil` example; see README.md for credit and
//! the exact upstream revision used.

mod automation_dnd;
mod automation_observe;
mod events;
mod guile;
mod handlers;

mod grabs;
mod input;
mod ipc;
mod logging;
mod png;
mod render;
mod runtime_dir;
mod state;
mod timing;
mod udev;
mod udev_input;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackendChoice {
    Auto,
    Explicit(Backend),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliAction {
    Run(BackendChoice),
    Help,
    Version,
    CheckConfig(std::path::PathBuf),
}

const HELP: &str = "minde - a Guile-configurable Wayland compositor

Usage: minde [OPTION]

Options:
      --tty      run directly on a DRM/udev/libinput session
      --winit    run nested in an existing Wayland or X11 session
      --check-config FILE
                  validate configuration without starting a compositor
  -h, --help     print this help and exit
  -V, --version  print version and build revision, then exit
";

fn parse_args<I, S>(args: I) -> Result<CliAction, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut backend = BackendChoice::Auto;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let arg = arg.as_ref();
        match arg {
            "-h" | "--help" => return Ok(CliAction::Help),
            "-V" | "--version" => return Ok(CliAction::Version),
            "--tty" => set_backend(&mut backend, Backend::Udev)?,
            "--winit" => set_backend(&mut backend, Backend::Winit)?,
            "--check-config" => {
                if backend != BackendChoice::Auto {
                    return Err(
                        "--check-config cannot be combined with a backend option".to_owned()
                    );
                }
                let path = args
                    .next()
                    .ok_or_else(|| "--check-config requires a file".to_owned())?;
                if let Some(extra) = args.next() {
                    return Err(format!(
                        "unexpected argument after configuration file: {}",
                        extra.as_ref()
                    ));
                }
                return Ok(CliAction::CheckConfig(std::path::PathBuf::from(
                    path.as_ref(),
                )));
            }
            _ => return Err(format!("unknown option: {arg}")),
        }
    }
    Ok(CliAction::Run(backend))
}

fn set_backend(choice: &mut BackendChoice, requested: Backend) -> Result<(), String> {
    match *choice {
        BackendChoice::Auto => {
            *choice = BackendChoice::Explicit(requested);
            Ok(())
        }
        BackendChoice::Explicit(current) if current == requested => Err(format!(
            "option specified more than once: {}",
            backend_flag(requested)
        )),
        BackendChoice::Explicit(current) => Err(format!(
            "conflicting backend options: {} and {}",
            backend_flag(current),
            backend_flag(requested)
        )),
    }
}

fn backend_flag(backend: Backend) -> &'static str {
    match backend {
        Backend::Winit => "--winit",
        Backend::Udev => "--tty",
    }
}

fn select_backend(choice: BackendChoice) -> Backend {
    if let BackendChoice::Explicit(backend) = choice {
        return backend;
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some() {
        Backend::Winit
    } else {
        Backend::Udev
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let action = match parse_args(std::env::args().skip(1)) {
        Ok(action) => action,
        Err(error) => {
            eprintln!("minde: {error}\nTry 'minde --help' for more information.");
            std::process::exit(2);
        }
    };
    let backend_choice = match action {
        CliAction::Help => {
            print!("{HELP}");
            return Ok(());
        }
        CliAction::Version => {
            println!(
                "minde {} ({})",
                env!("CARGO_PKG_VERSION"),
                env!("MINDE_BUILD_REVISION")
            );
            return Ok(());
        }
        CliAction::CheckConfig(path) => {
            validate_config(&path)?;
            println!("configuration valid: {}", path.display());
            return Ok(());
        }
        CliAction::Run(choice) => choice,
    };

    logging::init();
    install_crash_log();

    let backend = select_backend(backend_choice);
    guile::set_runtime_backend(match backend {
        Backend::Winit => "winit",
        Backend::Udev => "udev",
    });
    tracing::info!(component = "runtime", ?backend, "selected backend");

    let mut event_loop: EventLoop<MindeState> = EventLoop::try_new()?;

    let display: Display<MindeState> = Display::new()?;

    let mut state = MindeState::new(&mut event_loop, display);
    // From here on `wm-*` primitives apply directly while Scheme runs on
    // this thread (see `guile::set_state`); cleared again after the loop.
    guile::set_state(&mut state);

    // Recorded for wm-spawn so children get WAYLAND_DISPLAY even when
    // spawned (via handle-startup!) before the env export further down.
    let _ = guile::SOCKET_NAME.set(state.socket_name.to_string_lossy().into_owned());

    // Sockets first: neither needs Scheme, and binding them early means a
    // stale-socket or runtime-dir error surfaces before the (comparatively
    // slow) Guile boot and init.scm load.
    ipc::init(&mut event_loop)?;
    events::init(&mut event_loop)?;

    // Guile must be initialized on the main thread, and after this call
    // every libguile function must be called from this same thread (except
    // through Guile's own REPL server, which manages its own thread).
    //
    // This must happen before backend init: both backends call into Scheme
    // from inside their init (udev enumerates the already-present GPU and
    // connectors synchronously, reaching `handle-heads-change!` and
    // `handle-startup!`; winit calls `handle-startup!` directly), so the
    // policy layer has to be loaded by then.
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

    // Embedded Xwayland: spawns lazily-managed X server; X11 apps become
    // ordinary managed windows (src/handlers/xwayland.rs). Safe after
    // backend init (needs the loop handle + display only).
    state.start_xwayland();

    // Set WAYLAND_DISPLAY to our socket name so child processes (spawned
    // via `wm-spawn`) connect to us. This must happen only AFTER the
    // backend has been picked: if winit sees this variable, it opens our
    // nested window as a client of *ourselves* and deadlocks before the
    // loop runs.
    unsafe { std::env::set_var("WAYLAND_DISPLAY", &state.socket_name) };

    let run = event_loop.run(None, &mut state, move |_state| {
        // minde is running; nothing extra to do per-iteration here, the
        // event sources (wayland socket, winit/udev, calloop) drive
        // everything.
    });
    guile::clear_state();
    run?;

    Ok(())
}

fn validate_config(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    // In-process through the linked libguile (the bundled modules resolve
    // via `guile::scheme_dir`), so `--check-config` needs no `guile` on PATH.
    if guile::check_config(path) {
        Ok(())
    } else {
        Err(format!("configuration validation failed: {}", path.display()).into())
    }
}

/// A panic on the TTY backend takes the whole session down with no
/// visible output (frozen VT). Leave evidence: append panic message and
/// backtrace to `crash.log` in the per-user state directory (see
/// `runtime_dir::state_dir`).
fn install_crash_log() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let state_dir = runtime_dir::state_dir();
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

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn parses_empty_arguments_as_auto_backend() {
        assert_eq!(
            parse_args([] as [&str; 0]),
            Ok(CliAction::Run(BackendChoice::Auto))
        );
    }

    #[test]
    fn parses_public_flags() {
        assert_eq!(
            parse_args(["--tty"]),
            Ok(CliAction::Run(BackendChoice::Explicit(Backend::Udev)))
        );
        assert_eq!(
            parse_args(["--winit"]),
            Ok(CliAction::Run(BackendChoice::Explicit(Backend::Winit)))
        );
        assert_eq!(parse_args(["--help"]), Ok(CliAction::Help));
        assert_eq!(parse_args(["-V"]), Ok(CliAction::Version));
        assert_eq!(
            parse_args(["--check-config", "config.scm"]),
            Ok(CliAction::CheckConfig("config.scm".into()))
        );
    }

    #[test]
    fn rejects_unknown_duplicate_and_conflicting_flags() {
        assert!(
            parse_args(["--unknown"])
                .unwrap_err()
                .contains("unknown option")
        );
        assert!(
            parse_args(["--check-config"])
                .unwrap_err()
                .contains("requires a file")
        );
        assert!(
            parse_args(["--tty", "--tty"])
                .unwrap_err()
                .contains("more than once")
        );
        assert!(
            parse_args(["--tty", "--winit"])
                .unwrap_err()
                .contains("conflicting")
        );
    }
}
