//! minde: a small Wayland compositor whose policy layer (keybindings,
//! spawning, etc.) lives in embedded Guile Scheme, StumpWM-style.
//!
//! The compositor core (state.rs, handlers/, grabs/, winit.rs, input.rs) is
//! adapted from Smithay's `smallvil` example; see README.md for credit and
//! the exact upstream revision used.

mod events;
mod guile;
mod handlers;

mod grabs;
mod input;
mod ipc;
mod logging;
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

    // Recorded for wm-spawn so children get WAYLAND_DISPLAY even when
    // spawned (via handle-startup!) before the env export further down.
    let _ = guile::SOCKET_NAME.set(state.socket_name.to_string_lossy().into_owned());

    // Guile must be initialized on the main thread, and after this call
    // every libguile function must be called from this same thread (except
    // through Guile's own REPL server, which manages its own thread).
    //
    // This must happen before backend init, which calls into Scheme (e.g.
    // `handle-output-geometry!`) as soon as it knows the output size.
    guile::init(state.loop_signal.clone());
    ipc::init(&mut event_loop)?;
    events::init(&mut event_loop)?;

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

    event_loop.run(None, &mut state, move |_state| {
        // minde is running; nothing extra to do per-iteration here, the
        // event sources (wayland socket, winit/udev, calloop) drive
        // everything.
    })?;

    Ok(())
}

fn scheme_module_dir() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    let repository_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scheme");
    if repository_dir.is_dir() {
        return Ok(repository_dir);
    }
    if let Some(path) = std::env::var_os("MINDE_SCHEME_DIR") {
        return Ok(path.into());
    }
    let executable = std::env::current_exe()?;
    let prefix = executable
        .parent()
        .and_then(std::path::Path::parent)
        .ok_or_else(|| std::io::Error::other("cannot determine minde installation prefix"))?;
    Ok(prefix.join("share/minde/scheme"))
}

fn validate_config(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let modules = scheme_module_dir()?;
    let expression = "(use-modules (minde command-catalog) (minde config)) \
                      (register-builtin-command-schemas!) \
                      (validate-configuration-file (cadr (command-line)))";
    let status = std::process::Command::new("guile")
        .arg("--no-auto-compile")
        .arg("-L")
        .arg(&modules)
        .arg("-c")
        .arg(expression)
        .arg(path)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("configuration validation failed: {}", path.display()).into())
    }
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
