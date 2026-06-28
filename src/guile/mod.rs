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
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};

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
    /// Focus-border color (prefix-state indicator).
    BorderColor { rgba: [f32; 4] },
    /// One-shot timer: after `ms`, call `(wm-on-timer token)` on the main
    /// thread. The Scheme side keeps the token->thunk table.
    RunAfter { ms: u64, token: i64 },
    /// Set/unset xdg fullscreen state on a window; Scheme re-syncs frame
    /// geometry itself when unsetting.
    Fullscreen { id: u64, on: bool },
    /// Force-kill: drop the window's client connection (StumpWM
    /// kill-window, vs. the polite `Close`).
    Kill { id: u64 },
    /// Warp the pointer to a global logical position (banish/ratwarp).
    WarpPointer { x: i32, y: i32 },
    /// Read the current clipboard selection; delivers the text to Scheme
    /// via `(wm-on-paste text)` when it arrives.
    Paste,
    /// Own the clipboard selection with this text (StumpWM putsel).
    SetClipboard { text: String },
    /// Place a floating window: same as `Place` but without the Tiled*
    /// states (floats keep their CSD shadows/rounded corners) and with a
    /// raise, so a newly-floated window pops above the tiling.
    PlaceFloat { id: u64, x: i32, y: i32, w: i32, h: i32 },
    /// Raise a window to the top of the stacking order without focusing
    /// it (raise is otherwise only a side effect of `Focus`).
    Raise { id: u64 },
    /// Mark/unmark a window as floating on the Rust side; gates the
    /// super+drag move/resize grabs in `input.rs`. Scheme's `%floating`
    /// table remains the authority on float geometry.
    SetFloating { id: u64, on: bool },
    /// Spawn a child process ON THE MAIN THREAD. wm-spawn must not
    /// fork from the calling thread: forking from the Guile REPL
    /// server thread wedged mesa/llvmpipe in the parent (the main
    /// thread froze inside eglSwapBuffers with the software
    /// rasterizer spinning forever).
    Spawn { cmd: String },
}

/// The sending half of the command channel. Set once from `main`/`state.rs`
/// after the channel and its calloop source are created. Reachable from any
/// thread (including the Guile REPL's own thread), unlike direct access to
/// `MindeState`.
static COMMAND_SENDER: OnceLock<Sender<WmCommand>> = OnceLock::new();

/// Last known usable area (union of all outputs minus layer-shell
/// exclusive zones). Stored outside `MindeState` so
/// `(wm-output-geometry)` is callable from any thread, including the REPL.
static OUTPUT_X: AtomicI32 = AtomicI32::new(0);
static OUTPUT_Y: AtomicI32 = AtomicI32::new(0);
static OUTPUT_W: AtomicU32 = AtomicU32::new(0);
static OUTPUT_H: AtomicU32 = AtomicU32::new(0);

/// One head (output/monitor) as reported to Scheme: stable id + usable
/// rect (global coordinates) + connector name.
#[derive(Debug, Clone, PartialEq)]
pub struct HeadInfo {
    pub id: u64,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub name: String,
}

/// Current head list, readable from any thread for `(wm-outputs)`.
static HEADS: std::sync::Mutex<Vec<HeadInfo>> = std::sync::Mutex::new(Vec::new());

pub fn set_command_sender(sender: Sender<WmCommand>) {
    let _ = COMMAND_SENDER.set(sender);
}

pub fn set_output_geometry(x: i32, y: i32, width: u32, height: u32) {
    OUTPUT_X.store(x, Ordering::SeqCst);
    OUTPUT_Y.store(y, Ordering::SeqCst);
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

pub fn call4(proc: Scm, a: Scm, b: Scm, c: Scm, d: Scm) -> Option<Scm> {
    protected_call(move || unsafe { ffi::scm_call_4(proc, a, b, c, d) })
}

pub fn call5(proc: Scm, a: Scm, b: Scm, c: Scm, d: Scm, e: Scm) -> Option<Scm> {
    protected_call(move || unsafe { ffi::scm_call_5(proc, a, b, c, d, e) })
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

fn call_named_4(name: &str, a: Scm, b: Scm, c: Scm, d: Scm) -> Option<Scm> {
    call4(lookup(name)?, a, b, c, d)
}

fn call_named_5(name: &str, a: Scm, b: Scm, c: Scm, d: Scm, e: Scm) -> Option<Scm> {
    call5(lookup(name)?, a, b, c, d, e)
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

/// Our Wayland socket name, recorded before the backend (and Scheme)
/// start. `wm-spawn` sets WAYLAND_DISPLAY from this explicitly: the
/// process-wide env var is only exported late in main() (exporting it
/// before winit init would make winit connect to ourselves), so children
/// spawned from `wm-on-startup` would otherwise inherit a stale/absent
/// WAYLAND_DISPLAY and fail to find the compositor (seen as eww's
/// "Failed to initialize GTK" on the TTY).
pub static SOCKET_NAME: OnceLock<String> = OnceLock::new();

/// Xwayland's ":N", set once it reports Ready -- same motivation as
/// SOCKET_NAME: children spawned before/around the env export must still
/// see the right DISPLAY.
pub static X11_DISPLAY: OnceLock<String> = OnceLock::new();

unsafe extern "C" fn wm_spawn(cmd: Scm) -> Scm {
    if let Some(cmd) = to_string_lossy(cmd) {
        tracing::info!(%cmd, "wm-spawn");
        // Enqueue instead of spawning right here: this subr may run on
        // the REPL server thread, and forking from there deadlocked the
        // main thread's GL swap (see WmCommand::Spawn).
        from_bool(send_command(WmCommand::Spawn { cmd }))
    } else {
        from_bool(false)
    }
}

/// Actually spawns a child; called from `apply_wm_command`, i.e. on the
/// main thread only.
pub fn spawn_on_main_thread(cmd: &str) {
    let mut command = std::process::Command::new("sh");
    command.arg("-c").arg(cmd);
    if let Some(socket) = SOCKET_NAME.get() {
        command.env("WAYLAND_DISPLAY", socket);
    }
    if let Some(display) = X11_DISPLAY.get() {
        command.env("DISPLAY", display);
        // With DISPLAY set, dual-stack toolkits (Firefox/zen, GTK apps)
        // would default to X11 -- through Xwayland their CSD shadow
        // margins aren't compensated (the old zen gap bug). Prefer
        // Wayland; X11-only apps ignore these. A user command can still
        // override with its own VAR=... prefix.
        command.env("MOZ_ENABLE_WAYLAND", "1");
        command.env("GDK_BACKEND", "wayland,x11");
    }
    if let Err(e) = command.spawn() {
        tracing::warn!(%cmd, error = %e, "wm-spawn failed");
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

unsafe extern "C" fn wm_place_float(id: Scm, x: Scm, y: Scm, w: Scm, h: Scm) -> Scm {
    let id = to_i64(id) as u64;
    let x = to_i64(x) as i32;
    let y = to_i64(y) as i32;
    let w = to_i64(w) as i32;
    let h = to_i64(h) as i32;
    from_bool(send_command(WmCommand::PlaceFloat { id, x, y, w, h }))
}

unsafe extern "C" fn wm_raise_window(id: Scm) -> Scm {
    let id = to_i64(id) as u64;
    from_bool(send_command(WmCommand::Raise { id }))
}

unsafe extern "C" fn wm_set_floating(id: Scm, on: Scm) -> Scm {
    let id = to_i64(id) as u64;
    let on = to_bool(on);
    from_bool(send_command(WmCommand::SetFloating { id, on }))
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

/// `(wm-border-color "#rrggbb")` -- sets the focus-border color.
unsafe extern "C" fn wm_border_color(hex: Scm) -> Scm {
    let Some(hex) = to_string_lossy(hex) else {
        return from_bool(false);
    };
    let s = hex.trim_start_matches('#');
    if s.len() != 6 {
        tracing::warn!(%hex, "wm-border-color: expected #rrggbb");
        return from_bool(false);
    }
    let Ok(v) = u32::from_str_radix(s, 16) else {
        tracing::warn!(%hex, "wm-border-color: bad hex");
        return from_bool(false);
    };
    let rgba = [
        ((v >> 16) & 0xff) as f32 / 255.0,
        ((v >> 8) & 0xff) as f32 / 255.0,
        (v & 0xff) as f32 / 255.0,
        1.0,
    ];
    from_bool(send_command(WmCommand::BorderColor { rgba }))
}

unsafe extern "C" fn wm_focus_rect(x: Scm, y: Scm, w: Scm, h: Scm) -> Scm {
    let x = to_i64(x) as i32;
    let y = to_i64(y) as i32;
    let w = to_i64(w) as i32;
    let h = to_i64(h) as i32;
    from_bool(send_command(WmCommand::FocusRect { x, y, w, h }))
}

/// `(wm-run-after-ms ms token)` -- see `wm-run-after` in init.scm for the
/// thunk-keeping wrapper.
unsafe extern "C" fn wm_run_after_ms(ms: Scm, token: Scm) -> Scm {
    let ms = to_i64(ms).max(0) as u64;
    let token = to_i64(token);
    from_bool(send_command(WmCommand::RunAfter { ms, token }))
}

unsafe extern "C" fn wm_set_fullscreen(id: Scm, on: Scm) -> Scm {
    let id = to_i64(id) as u64;
    let on = to_bool(on);
    from_bool(send_command(WmCommand::Fullscreen { id, on }))
}

unsafe extern "C" fn wm_kill_window(id: Scm) -> Scm {
    let id = to_i64(id) as u64;
    from_bool(send_command(WmCommand::Kill { id }))
}

unsafe extern "C" fn wm_warp_pointer(x: Scm, y: Scm) -> Scm {
    let x = to_i64(x) as i32;
    let y = to_i64(y) as i32;
    from_bool(send_command(WmCommand::WarpPointer { x, y }))
}

unsafe extern "C" fn wm_request_paste() -> Scm {
    from_bool(send_command(WmCommand::Paste))
}

unsafe extern "C" fn wm_set_clipboard(text: Scm) -> Scm {
    let Some(text) = to_string_lossy(text) else {
        return from_bool(false);
    };
    from_bool(send_command(WmCommand::SetClipboard { text }))
}

unsafe extern "C" fn wm_output_geometry() -> Scm {
    let x = OUTPUT_X.load(Ordering::SeqCst) as i64;
    let y = OUTPUT_Y.load(Ordering::SeqCst) as i64;
    let w = OUTPUT_W.load(Ordering::SeqCst) as i64;
    let h = OUTPUT_H.load(Ordering::SeqCst) as i64;
    unsafe { ffi::scm_list_4(from_i64(x), from_i64(y), from_i64(w), from_i64(h)) }
}

/// Builds a proper list from a slice of SCM values.
fn scm_list(items: &[Scm]) -> Scm {
    items
        .iter()
        .rev()
        .fold(ffi::SCM_EOL, |tail, &head| unsafe { ffi::scm_cons(head, tail) })
}

/// `(wm-outputs)` -> `((id x y w h name) ...)`, usable rects.
unsafe extern "C" fn wm_outputs() -> Scm {
    let heads = HEADS.lock().unwrap().clone();
    let entries: Vec<Scm> = heads
        .iter()
        .map(|head| {
            scm_list(&[
                from_i64(head.id as i64),
                from_i64(head.x as i64),
                from_i64(head.y as i64),
                from_i64(head.w as i64),
                from_i64(head.h as i64),
                from_str(&head.name),
            ])
        })
        .collect();
    scm_list(&entries)
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
        register_gsubr("wm-border-color", 1, 0, 0, std::mem::transmute::<
            unsafe extern "C" fn(Scm) -> Scm,
            ffi::Gsubr,
        >(wm_border_color));
        register_gsubr("wm-focus-rect", 4, 0, 0, std::mem::transmute::<
            unsafe extern "C" fn(Scm, Scm, Scm, Scm) -> Scm,
            ffi::Gsubr,
        >(wm_focus_rect));
        register_gsubr("wm-output-geometry", 0, 0, 0, std::mem::transmute::<
            unsafe extern "C" fn() -> Scm,
            ffi::Gsubr,
        >(wm_output_geometry));
        register_gsubr("wm-run-after-ms", 2, 0, 0, std::mem::transmute::<
            unsafe extern "C" fn(Scm, Scm) -> Scm,
            ffi::Gsubr,
        >(wm_run_after_ms));
        register_gsubr("wm-set-fullscreen", 2, 0, 0, std::mem::transmute::<
            unsafe extern "C" fn(Scm, Scm) -> Scm,
            ffi::Gsubr,
        >(wm_set_fullscreen));
        register_gsubr("wm-kill-window", 1, 0, 0, std::mem::transmute::<
            unsafe extern "C" fn(Scm) -> Scm,
            ffi::Gsubr,
        >(wm_kill_window));
        register_gsubr("wm-warp-pointer", 2, 0, 0, std::mem::transmute::<
            unsafe extern "C" fn(Scm, Scm) -> Scm,
            ffi::Gsubr,
        >(wm_warp_pointer));
        // Gsubr is exactly the zero-arg signature; no transmute needed.
        register_gsubr("wm-request-paste", 0, 0, 0, wm_request_paste);
        register_gsubr("wm-outputs", 0, 0, 0, wm_outputs);
        register_gsubr("wm-set-clipboard", 1, 0, 0, std::mem::transmute::<
            unsafe extern "C" fn(Scm) -> Scm,
            ffi::Gsubr,
        >(wm_set_clipboard));
        register_gsubr("wm-place-float", 5, 0, 0, std::mem::transmute::<
            unsafe extern "C" fn(Scm, Scm, Scm, Scm, Scm) -> Scm,
            ffi::Gsubr,
        >(wm_place_float));
        register_gsubr("wm-raise-window", 1, 0, 0, std::mem::transmute::<
            unsafe extern "C" fn(Scm) -> Scm,
            ffi::Gsubr,
        >(wm_raise_window));
        register_gsubr("wm-set-floating", 2, 0, 0, std::mem::transmute::<
            unsafe extern "C" fn(Scm, Scm) -> Scm,
            ffi::Gsubr,
        >(wm_set_floating));
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
pub fn handle_key(mods: u32, keysym: u32, keysym_name: &str, utf8: &str) -> bool {
    let Some(proc) = lookup("wm-handle-key") else {
        return false;
    };
    let a = from_i64(mods as i64);
    let b = from_i64(keysym as i64);
    let c = from_str(keysym_name);
    let d = from_str(utf8);
    let result = call4(proc, a, b, c, d);
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

/// Records the new usable area and calls `(wm-on-output-geometry x y width
/// height)` if bound. Call this whenever the usable area changes: output
/// init/resize (x/y = 0) or a layer-shell exclusive zone reserving space
/// (x/y = the zone's origin).
pub fn on_output_geometry(x: i32, y: i32, width: u32, height: u32) {
    set_output_geometry(x, y, width, height);
    call_named_4(
        "wm-on-output-geometry",
        from_i64(x as i64),
        from_i64(y as i64),
        from_i64(width as i64),
        from_i64(height as i64),
    );
}

/// Reports the full head list (usable rects) to Scheme:
/// `(wm-on-heads-changed ((id x y w h) ...))`. Also refreshes the
/// `(wm-output-geometry)` union and the `(wm-outputs)` registry. Falls
/// back to the legacy single-head `wm-on-output-geometry` when the new
/// entry point isn't bound (older user configs).
pub fn on_heads_changed(heads: Vec<HeadInfo>) {
    if heads.is_empty() {
        return;
    }
    // Union of the usable rects, for the legacy geometry query.
    let x1 = heads.iter().map(|h| h.x).min().unwrap();
    let y1 = heads.iter().map(|h| h.y).min().unwrap();
    let x2 = heads.iter().map(|h| h.x + h.w as i32).max().unwrap();
    let y2 = heads.iter().map(|h| h.y + h.h as i32).max().unwrap();
    set_output_geometry(x1, y1, (x2 - x1).max(0) as u32, (y2 - y1).max(0) as u32);

    let first = heads[0].clone();
    *HEADS.lock().unwrap() = heads.clone();

    if let Some(proc) = lookup("wm-on-heads-changed") {
        let entries: Vec<Scm> = heads
            .iter()
            .map(|h| {
                scm_list(&[
                    from_i64(h.id as i64),
                    from_i64(h.x as i64),
                    from_i64(h.y as i64),
                    from_i64(h.w as i64),
                    from_i64(h.h as i64),
                ])
            })
            .collect();
        call1(proc, scm_list(&entries));
    } else {
        call_named_4(
            "wm-on-output-geometry",
            from_i64(first.x as i64),
            from_i64(first.y as i64),
            from_i64(first.w as i64),
            from_i64(first.h as i64),
        );
    }
}

/// Calls `(wm-on-timer token)` if bound; fired by `WmCommand::RunAfter`'s
/// calloop timer on the main (Guile) thread.
pub fn on_timer(token: i64) {
    call_named_1("wm-on-timer", from_i64(token));
}

/// Calls `(wm-on-paste text)` if bound, delivering clipboard contents
/// requested via `wm-request-paste`.
pub fn on_paste(text: &str) {
    call_named_1("wm-on-paste", from_str(text));
}

/// Calls `(wm-on-window-moved id x y w h)` if bound; fired when a
/// super+drag move/resize grab releases, so Scheme's `%floating` table
/// tracks the user-dragged geometry.
pub fn on_window_moved(id: u64, x: i32, y: i32, w: i32, h: i32) {
    call_named_5(
        "wm-on-window-moved",
        from_i64(id as i64),
        from_i64(x as i64),
        from_i64(y as i64),
        from_i64(w as i64),
        from_i64(h as i64),
    );
}

/// Calls `(wm-on-urgent id)` if bound (xdg-activation request for a
/// mapped toplevel; StumpWM urgency).
pub fn on_urgent(id: u64) {
    call_named_1("wm-on-urgent", from_i64(id as i64));
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
