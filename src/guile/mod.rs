//! Embedded Guile layer: init, safe-ish wrappers around `ffi`, and the
//! Rust-side subrs exposed to Scheme (`wm-spawn`, `wm-quit`, `wm-log`).
//!
//! Thread affinity: `scm_init_guile` binds this OS thread as "the" Guile
//! thread for the simple (non-`scm_with_guile`) embedding API used here.
//! All calls into libguile from this process must happen on that same
//! thread, except that Guile's own REPL server (started from Scheme, see
//! `scheme/init.scm`) spawns its own internal thread that Guile itself
//! manages -- we never touch libguile from other Rust threads.

pub mod ffi;

use ffi::Scm;
use smithay::reexports::calloop::LoopSignal;
use smithay::reexports::calloop::channel::Sender;
use std::ffi::{CStr, CString};
use std::os::raw::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Set once, before `scm_init_guile`, so `wm-quit` can reach the event loop.
static LOOP_SIGNAL: OnceLock<LoopSignal> = OnceLock::new();
static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn set_loop_signal(signal: LoopSignal) {
    let _ = LOOP_SIGNAL.set(signal);
}

/// Commands enqueued from Scheme (possibly from the REPL thread) to be
/// applied against `&mut MindeState` on the compositor's main thread via
/// a calloop channel.
#[derive(Debug, Clone)]
pub enum WmCommand {
    Place { id: u64, x: i32, y: i32, w: i32, h: i32 },
    Focus { id: u64 },
    ClearFocus,
    Close { id: u64 },
    /// Rectangle of the currently-selected frame; the render pass draws
    /// the focus border around this (not around the focused window), so an
    /// empty frame is still visibly selected.
    FocusRect { x: i32, y: i32, w: i32, h: i32 },
    /// Show text in the centered message overlay (StumpWM's message
    /// window); auto-hides after `timeout_ms` (0 = sticky until replaced
    /// or cleared).
    Message { text: String, timeout_ms: u64 },
    ClearMessage,
}

/// The sending half of the command channel. Set once from `main`/`state.rs`
/// after the channel and its calloop source are created. Reachable from any
/// thread (including the Guile REPL's own thread), unlike direct access to
/// `MindeState`.
static COMMAND_SENDER: OnceLock<Sender<WmCommand>> = OnceLock::new();

/// Last known output size, updated by the winit backend on init/resize.
/// Stored outside `MindeState` so `(wm-output-geometry)` is callable from
/// any thread, including the REPL.
static OUTPUT_W: AtomicU32 = AtomicU32::new(0);
static OUTPUT_H: AtomicU32 = AtomicU32::new(0);

pub fn set_command_sender(sender: Sender<WmCommand>) {
    let _ = COMMAND_SENDER.set(sender);
}

pub fn set_output_geometry(width: u32, height: u32) {
    OUTPUT_W.store(width, Ordering::SeqCst);
    OUTPUT_H.store(height, Ordering::SeqCst);
}

fn send_command(cmd: WmCommand) -> bool {
    match COMMAND_SENDER.get() {
        Some(sender) => sender.send(cmd).is_ok(),
        None => {
            tracing::warn!("wm command sender not initialized yet");
            false
        }
    }
}

/// Runs `f` inside `scm_internal_catch` with a catch-all tag, so a Scheme
/// error is logged and swallowed instead of aborting the compositor.
/// Returns `None` if `f` raised.
fn protected_call<F: FnOnce() -> Scm>(f: F) -> Option<Scm> {
    unsafe extern "C" fn body_trampoline<F: FnOnce() -> Scm>(data: *mut c_void) -> Scm {
        let boxed: Box<F> = unsafe { Box::from_raw(data as *mut F) };
        (*boxed)()
    }
    unsafe extern "C" fn handler_trampoline(data: *mut c_void, _key: Scm, _args: Scm) -> Scm {
        unsafe {
            *(data as *mut bool) = true;
        }
        ffi::SCM_BOOL_F
    }

    let mut errored = false;
    let data = Box::into_raw(Box::new(f)) as *mut c_void;
    let tag = ffi::SCM_BOOL_T; // #t tag means "catch everything"
    let result = unsafe {
        ffi::scm_internal_catch(
            tag,
            body_trampoline::<F>,
            data,
            handler_trampoline,
            (&mut errored) as *mut bool as *mut c_void,
        )
    };

    if errored {
        tracing::warn!("scheme: error caught during evaluation");
        None
    } else {
        Some(result)
    }
}

fn to_cstring(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| CString::new("").unwrap())
}

/// Evaluates a snippet of Scheme source, catching errors.
pub fn eval_string(code: &str) -> Option<Scm> {
    let c = to_cstring(code);
    let ptr = c.as_ptr();
    protected_call(move || unsafe { ffi::scm_c_eval_string(ptr) })
}

/// Loads a Scheme file, catching errors.
pub fn load_file(path: &std::path::Path) -> Option<Scm> {
    let c = to_cstring(&path.to_string_lossy());
    let ptr = c.as_ptr();
    protected_call(move || unsafe { ffi::scm_c_primitive_load(ptr) })
}

/// Looks up a top-level variable's value by name, returning `None` if it is
/// unbound (uses `scm_c_lookup` + `scm_variable_ref`, wrapped in a catch
/// since `scm_c_lookup` throws on an unbound name).
pub fn lookup(name: &str) -> Option<Scm> {
    let c = to_cstring(name);
    let ptr = c.as_ptr();
    protected_call(move || unsafe {
        let var = ffi::scm_c_lookup(ptr);
        ffi::scm_variable_ref(var)
    })
}

pub fn call1(proc: Scm, a: Scm) -> Option<Scm> {
    protected_call(move || unsafe { ffi::scm_call_1(proc, a) })
}

pub fn call2(proc: Scm, a: Scm, b: Scm) -> Option<Scm> {
    protected_call(move || unsafe { ffi::scm_call_2(proc, a, b) })
}

pub fn call3(proc: Scm, a: Scm, b: Scm, c: Scm) -> Option<Scm> {
    protected_call(move || unsafe { ffi::scm_call_3(proc, a, b, c) })
}

/// Looks up NAME fresh and calls it with one argument, if bound. Mirrors
/// `handle_key`'s "missing definition = no-op" behavior.
fn call_named_1(name: &str, a: Scm) -> Option<Scm> {
    call1(lookup(name)?, a)
}

fn call_named_2(name: &str, a: Scm, b: Scm) -> Option<Scm> {
    call2(lookup(name)?, a, b)
}

fn call_named_3(name: &str, a: Scm, b: Scm, c: Scm) -> Option<Scm> {
    call3(lookup(name)?, a, b, c)
}

/// Converts a Scheme integer to `i64`. Only call this on values expected to
/// be integers (e.g. subr arguments); on a non-integer Guile will raise,
/// unwinding through this call -- acceptable since we're still on the
/// Guile-owned call stack at that point.
pub fn to_i64(v: Scm) -> i64 {
    unsafe { ffi::scm_to_int64(v) }
}

pub fn to_bool(v: Scm) -> bool {
    unsafe { ffi::scm_to_bool(v) != 0 }
}

pub fn from_bool(b: bool) -> Scm {
    ffi::scm_from_bool_inline(b)
}

pub fn from_i64(v: i64) -> Scm {
    unsafe { ffi::scm_from_int64(v) }
}

pub fn from_str(s: &str) -> Scm {
    let c = to_cstring(s);
    unsafe { ffi::scm_from_utf8_string(c.as_ptr()) }
}

/// Converts a Scheme string SCM to a Rust `String`. Returns `None` if `v`
/// isn't a string (Guile will raise inside `scm_to_utf8_stringn`, which we
/// don't want to crash on, so callers should only pass values known to be
/// strings, e.g. from `(symbol->string ...)`).
pub fn to_string_lossy(v: Scm) -> Option<String> {
    unsafe {
        let mut len: usize = 0;
        let ptr = ffi::scm_to_utf8_stringn(v, &mut len as *mut usize);
        if ptr.is_null() {
            return None;
        }
        let s = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        ffi::free(ptr as *mut c_void);
        Some(s)
    }
}

// ---------------------------------------------------------------------
// Subrs exposed to Scheme
// ---------------------------------------------------------------------

unsafe extern "C" fn wm_spawn(cmd: Scm) -> Scm {
    if let Some(cmd) = to_string_lossy(cmd) {
        tracing::info!(%cmd, "wm-spawn");
        let result = std::process::Command::new("sh").arg("-c").arg(&cmd).spawn();
        if let Err(e) = result {
            tracing::warn!(%cmd, error = %e, "wm-spawn failed");
            return from_bool(false);
        }
        from_bool(true)
    } else {
        from_bool(false)
    }
}

unsafe extern "C" fn wm_quit() -> Scm {
    tracing::info!("wm-quit");
    QUIT_REQUESTED.store(true, Ordering::SeqCst);
    if let Some(signal) = LOOP_SIGNAL.get() {
        signal.stop();
    }
    from_bool(true)
}

unsafe extern "C" fn wm_log(msg: Scm) -> Scm {
    if let Some(msg) = to_string_lossy(msg) {
        tracing::info!(target: "scheme", "{msg}");
    }
    from_bool(true)
}

unsafe extern "C" fn wm_place_window(id: Scm, x: Scm, y: Scm, w: Scm, h: Scm) -> Scm {
    let id = to_i64(id) as u64;
    let x = to_i64(x) as i32;
    let y = to_i64(y) as i32;
    let w = to_i64(w) as i32;
    let h = to_i64(h) as i32;
    from_bool(send_command(WmCommand::Place { id, x, y, w, h }))
}

unsafe extern "C" fn wm_focus_window(id: Scm) -> Scm {
    let id = to_i64(id) as u64;
    from_bool(send_command(WmCommand::Focus { id }))
}

unsafe extern "C" fn wm_close_window(id: Scm) -> Scm {
    let id = to_i64(id) as u64;
    from_bool(send_command(WmCommand::Close { id }))
}

unsafe extern "C" fn wm_clear_focus() -> Scm {
    from_bool(send_command(WmCommand::ClearFocus))
}

/// `(wm-message text)` or `(wm-message text timeout-ms)`. Guile passes
/// SCM_UNDEFINED for a missing optional argument.
unsafe extern "C" fn wm_message(text: Scm, timeout: Scm) -> Scm {
    let Some(text) = to_string_lossy(text) else {
        return from_bool(false);
    };
    let timeout_ms = if timeout.0 == ffi::SCM_UNDEFINED.0 {
        5000
    } else {
        to_i64(timeout).max(0) as u64
    };
    from_bool(send_command(WmCommand::Message { text, timeout_ms }))
}

unsafe extern "C" fn wm_clear_message() -> Scm {
    from_bool(send_command(WmCommand::ClearMessage))
}

unsafe extern "C" fn wm_focus_rect(x: Scm, y: Scm, w: Scm, h: Scm) -> Scm {
    let x = to_i64(x) as i32;
    let y = to_i64(y) as i32;
    let w = to_i64(w) as i32;
    let h = to_i64(h) as i32;
    from_bool(send_command(WmCommand::FocusRect { x, y, w, h }))
}

unsafe extern "C" fn wm_output_geometry() -> Scm {
    let w = OUTPUT_W.load(Ordering::SeqCst) as i64;
    let h = OUTPUT_H.load(Ordering::SeqCst) as i64;
    unsafe { ffi::scm_list_2(from_i64(w), from_i64(h)) }
}

fn register_gsubr(name: &str, req: i32, opt: i32, rst: i32, f: ffi::Gsubr) {
    let c = to_cstring(name);
    unsafe {
        ffi::scm_c_define_gsubr(c.as_ptr(), req, opt, rst, f);
    }
}

/// Initializes Guile on the calling (main) thread, registers all Rust
/// subrs, then loads `scheme/init.scm`.
///
/// Must be called from the compositor's main thread, before any other
/// libguile call, and only once.
pub fn init(loop_signal: LoopSignal) {
    set_loop_signal(loop_signal);

    unsafe {
        ffi::scm_init_guile();

        register_gsubr("wm-spawn", 1, 0, 0, std::mem::transmute::<
            unsafe extern "C" fn(Scm) -> Scm,
            ffi::Gsubr,
        >(wm_spawn));
        register_gsubr("wm-quit", 0, 0, 0, std::mem::transmute::<
            unsafe extern "C" fn() -> Scm,
            ffi::Gsubr,
        >(wm_quit));
        register_gsubr("wm-log", 1, 0, 0, std::mem::transmute::<
            unsafe extern "C" fn(Scm) -> Scm,
            ffi::Gsubr,
        >(wm_log));
        register_gsubr("wm-place-window", 5, 0, 0, std::mem::transmute::<
            unsafe extern "C" fn(Scm, Scm, Scm, Scm, Scm) -> Scm,
            ffi::Gsubr,
        >(wm_place_window));
        register_gsubr("wm-focus-window", 1, 0, 0, std::mem::transmute::<
            unsafe extern "C" fn(Scm) -> Scm,
            ffi::Gsubr,
        >(wm_focus_window));
        register_gsubr("wm-close-window", 1, 0, 0, std::mem::transmute::<
            unsafe extern "C" fn(Scm) -> Scm,
            ffi::Gsubr,
        >(wm_close_window));
        register_gsubr("wm-clear-focus", 0, 0, 0, std::mem::transmute::<
            unsafe extern "C" fn() -> Scm,
            ffi::Gsubr,
        >(wm_clear_focus));
        register_gsubr("wm-message", 1, 1, 0, std::mem::transmute::<
            unsafe extern "C" fn(Scm, Scm) -> Scm,
            ffi::Gsubr,
        >(wm_message));
        register_gsubr("wm-clear-message", 0, 0, 0, std::mem::transmute::<
            unsafe extern "C" fn() -> Scm,
            ffi::Gsubr,
        >(wm_clear_message));
        register_gsubr("wm-focus-rect", 4, 0, 0, std::mem::transmute::<
            unsafe extern "C" fn(Scm, Scm, Scm, Scm) -> Scm,
            ffi::Gsubr,
        >(wm_focus_rect));
        register_gsubr("wm-output-geometry", 0, 0, 0, std::mem::transmute::<
            unsafe extern "C" fn() -> Scm,
            ffi::Gsubr,
        >(wm_output_geometry));
    }

    // Init file resolution: $MINDE_INIT > ~/.config/minde/init.scm >
    // the repo's scheme/init.scm (the tested default).
    let init_path = std::env::var("MINDE_INIT")
        .map(std::path::PathBuf::from)
        .ok()
        .or_else(|| {
            let user_config = std::env::var("XDG_CONFIG_HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| {
                    std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
                        .join(".config")
                })
                .join("minde/init.scm");
            user_config.exists().then_some(user_config)
        })
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scheme/init.scm")
        });

    // Make the bundled modules ((minde frames) &c.) importable from any
    // init file location, e.g. a user config in ~/.config/minde/.
    let module_dir = std::env::var("MINDE_SCHEME_DIR").unwrap_or_else(|_| {
        format!("{}/scheme", env!("CARGO_MANIFEST_DIR"))
    });
    eval_string(&format!("(add-to-load-path {:?})", module_dir));

    tracing::info!(path = %init_path.display(), "loading scheme init file");
    if load_file(&init_path).is_none() {
        tracing::error!("failed to load scheme init file; wm-handle-key etc. will be unavailable");
    }
}

/// Calls `(wm-handle-key mods keysym keysym-name)` if that variable is
/// currently bound, looking it up fresh each time so it can be redefined
/// live from the REPL. Returns `true` if the key was consumed.
pub fn handle_key(mods: u32, keysym: u32, keysym_name: &str) -> bool {
    let Some(proc) = lookup("wm-handle-key") else {
        return false;
    };
    let a = from_i64(mods as i64);
    let b = from_i64(keysym as i64);
    let c = from_str(keysym_name);
    let result = call3(proc, a, b, c);
    let consumed = match result {
        Some(r) => to_bool(r),
        None => false,
    };
    tracing::debug!(mods, keysym_name, consumed, errored = result.is_none(), "handle_key");
    consumed
}

/// Calls `(wm-on-window-map id title app-id)` if bound. `title`/`app_id` may
/// be empty strings if the client hasn't set them (yet).
pub fn on_window_map(id: u64, title: &str, app_id: &str) {
    call_named_3("wm-on-window-map", from_i64(id as i64), from_str(title), from_str(app_id));
}

/// Calls `(wm-on-window-unmap id)` if bound.
pub fn on_window_unmap(id: u64) {
    call_named_1("wm-on-window-unmap", from_i64(id as i64));
}

/// Records the new output size and calls `(wm-on-output-geometry width
/// height)` if bound. Call this whenever the output size changes (init and
/// resize).
pub fn on_output_geometry(width: u32, height: u32) {
    set_output_geometry(width, height);
    call_named_2(
        "wm-on-output-geometry",
        from_i64(width as i64),
        from_i64(height as i64),
    );
}

/// Calls `(wm-on-startup)` if bound, once the first output is up and
/// synced. Missing definition is a no-op, same as the other `on_*` hooks.
/// Called from both backends (winit and udev) so autostart works whether
/// nested or standalone.
pub fn on_startup() {
    let Some(proc) = lookup("wm-on-startup") else {
        return;
    };
    protected_call(move || unsafe { ffi::scm_call_0(proc) });
}
