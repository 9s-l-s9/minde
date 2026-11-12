// SPDX-License-Identifier: GPL-3.0-or-later

//! Init diagnostics and selection. Failed personal policy is retried by main
//! in a fresh process, before either backend acquires a display or seat.

use super::*;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

static RECOVERY_MESSAGE: OnceLock<String> = OnceLock::new();

#[derive(Debug)]
pub struct InitError {
    pub message: String,
    pub can_recover: bool,
}

impl std::fmt::Display for InitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for InitError {}

fn same_file(left: &Path, right: &Path) -> bool {
    left == right
        || match (left.metadata(), right.metadata()) {
            (Ok(left), Ok(right)) => left.dev() == right.dev() && left.ino() == right.ino(),
            _ => false,
        }
}

fn selected_init(bundled: &Path) -> PathBuf {
    std::env::var_os("MINDE_INIT")
        .map(PathBuf::from)
        .or_else(|| {
            let user_config = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
                })
                .join("minde/init.scm");
            user_config.exists().then_some(user_config)
        })
        .unwrap_or_else(|| bundled.to_owned())
}

fn init_load_expression(path: &Path, bundled_dir: &Path) -> String {
    if path == bundled_dir.join("init.scm") {
        // boot() prepends bundled_dir to %load-path. load-from-path finds
        // installed init.go; absolute load searches for PATH/init.scm.go.
        r#"(load-from-path "init")"#.to_owned()
    } else {
        // Exact-path semantics for a user init, even when named init.scm.
        // load uses Guile's autocompiler; primitive-load would interpret it.
        format!("(load {})", scheme_string(&path.to_string_lossy()))
    }
}

/// Boot once on the main thread, register primitives and load policy. Capture
/// the stack before unwinding so unbound variables retain their source lines.
pub fn init(loop_signal: LoopSignal, recovery: Option<String>) -> Result<(), InitError> {
    set_loop_signal(loop_signal);
    boot();
    register_primitives();

    let bundled_dir = scheme_dir();
    let bundled = bundled_dir.join("init.scm");
    let path = if let Some(message) = recovery {
        let _ = RECOVERY_MESSAGE.set(message);
        bundled.clone()
    } else {
        selected_init(&bundled)
    };
    let can_recover = RECOVERY_MESSAGE.get().is_none() && !same_file(&path, &bundled);
    // Catch here rather than in protected_call: the handler produces both a
    // short on-screen diagnostic and the exception key/arguments/backtrace.
    // It does not depend on the policy's own exception-handling helpers.
    let code = format!(
        r#"(let ((stack #f) (source #f))
             (catch #t
               (lambda () {} #t)
               (lambda (key . args)
                 (list
                   (call-with-output-string
                     (lambda (port)
                       (when source (format port "~a: " source))
                       (print-exception port #f key args)))
                   (call-with-output-string
                     (lambda (port)
                       (format port "~s ~s~%" key args)
                       (when stack (display-backtrace stack port))))))
               (lambda _
                 (let ((port (current-load-port)))
                   (when port
                     (set! source
                       (format #f "~a:~a:~a" (port-filename port)
                               (1+ (port-line port)) (1+ (port-column port))))))
                 (set! stack (make-stack #t)))))"#,
        init_load_expression(&path, &bundled_dir)
    );
    tracing::info!(path = %path.display(), "loading scheme init file");
    let result = eval_string(&code);
    if result == Some(ffi::SCM_BOOL_T) {
        return Ok(());
    }
    let (summary, details) = result
        .and_then(string_list)
        .and_then(|parts| {
            let [summary, details]: [String; 2] = parts.try_into().ok()?;
            Some((summary, details))
        })
        .unwrap_or_else(|| ("could not report Scheme exception".into(), String::new()));
    tracing::error!(path = %path.display(), error = %summary.trim(), %details,
                    "failed to load scheme init file");
    // Keep the restart environment and overlay bounded; the log retains the
    // full diagnostic. Strip NULs, which cannot be passed through exec's envp.
    let location = format!("{}:", path.display());
    let message = if summary.starts_with(&location) {
        summary.trim().to_owned()
    } else {
        format!("{location} {}", summary.trim())
    };
    let message = message.chars().filter(|c| *c != '\0').take(500).collect();
    Err(InitError {
        message,
        can_recover,
    })
}

pub(super) fn show_recovery_message() {
    if let Some(error) = RECOVERY_MESSAGE.get() {
        tracing::warn!(%error, "recovered policy using bundled init");
        send_command(WmCommand::Message {
            text: format!(
                "Init failed; using bundled defaults.\n{error}\nSee session.log. Ctrl+Alt+Backspace exits."
            ),
            timeout_ms: 30_000,
        });
    }
}
