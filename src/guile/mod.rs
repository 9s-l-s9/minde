//! Embedded Guile layer: init, safe-ish wrappers around `ffi`, and the
//! Rust-side subrs exposed to Scheme (`wm-spawn`, `wm-quit`, `wm-log`).
//!
//! Thread affinity: `scm_init_guile` binds this OS thread as "the" Guile
//! thread for the simple (non-`scm_with_guile`) embedding API used here.
//! All calls into libguile from this process must happen on that same
//! thread, except that Guile's own REPL server (started from Scheme, see
//! `scheme/init.scm`) spawns its own internal thread that Guile itself
//! manages -- we never touch libguile from other Rust threads.

mod command;
pub mod ffi;

pub use command::WmCommand;

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

// Small process facts exposed through `(wm-runtime-info)`. Numeric atomics
// avoid sharing compositor objects with Scheme and remain safe if the opt-in
// unsafe REPL is enabled.
static RUNTIME_BACKEND: AtomicU32 = AtomicU32::new(0); // 1=winit, 2=udev
static XWAYLAND_STATUS: AtomicU32 = AtomicU32::new(0);
static XWAYLAND_DISPLAY: AtomicI32 = AtomicI32::new(-1);
static RUNTIME_STARTED: OnceLock<std::time::Instant> = OnceLock::new();

/// Whether the session is locked (ext-session-lock). Mirrored out of
/// `MindeState` so `(wm-session-locked?)` is callable from any thread,
/// including the REPL. Set from the session-lock handler's lock/unlock
/// transitions (see `src/handlers/session_lock.rs`).
static SESSION_LOCKED: AtomicBool = AtomicBool::new(false);

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

/// A stored libinput configuration rule (see `wm-configure-input!`).
///
/// Backend-agnostic: this layer only stores normalized values. The udev
/// backend (`src/udev.rs`) maps them onto the libinput `input`-crate enums
/// and applies them to matching devices as they arrive (and re-applies to
/// devices already present). Under winit there is no libinput context, so
/// rules are stored and never applied.
#[derive(Debug, Clone)]
pub struct InputRule {
    /// Substring matched against the device name; the empty string matches
    /// every device (`#t` in Scheme).
    pub match_name: String,
    /// tap-to-click: `Some(true/false)` to set, `None` to leave unchanged.
    pub tap: Option<bool>,
    /// natural scrolling: `Some(true/false)` to set, `None` to leave.
    pub natural_scroll: Option<bool>,
    /// acceleration profile: `"flat"` | `"adaptive"`, or `None` to leave.
    pub accel_profile: Option<String>,
    /// click method: `"button-areas"` | `"clickfinger"`, or `None` to leave.
    pub click_method: Option<String>,
}

/// All rules registered through `wm-configure-input!`, applied in order
/// (later rules win). De-duplicated by `match_name` so repeated config
/// reloads cannot grow the list without bound.
static INPUT_RULES: std::sync::Mutex<Vec<InputRule>> = std::sync::Mutex::new(Vec::new());

/// One input device as reported to Scheme by `(wm-input-devices)`.
#[derive(Debug, Clone)]
pub struct InputDeviceInfo {
    pub name: String,
    pub capabilities: Vec<String>,
}

/// Currently-present libinput devices, maintained by the udev backend on
/// device add/remove. Empty under winit (no libinput context).
static INPUT_DEVICES: std::sync::Mutex<Vec<InputDeviceInfo>> = std::sync::Mutex::new(Vec::new());

/// Snapshot of the stored input rules, for the udev backend to apply.
pub fn input_rules() -> Vec<InputRule> {
    INPUT_RULES.lock().unwrap().clone()
}

/// Records a present input device for `(wm-input-devices)`. Called by the
/// udev backend on `InputEvent::DeviceAdded`.
pub fn register_input_device(name: String, capabilities: Vec<String>) {
    INPUT_DEVICES
        .lock()
        .unwrap()
        .push(InputDeviceInfo { name, capabilities });
}

/// Drops the first device registered under `name` (udev
/// `InputEvent::DeviceRemoved`).
pub fn unregister_input_device(name: &str) {
    let mut devices = INPUT_DEVICES.lock().unwrap();
    if let Some(pos) = devices.iter().position(|d| d.name == name) {
        devices.remove(pos);
    }
}

pub fn set_command_sender(sender: Sender<WmCommand>) {
    let _ = COMMAND_SENDER.set(sender);
}

pub fn set_output_geometry(x: i32, y: i32, width: u32, height: u32) {
    OUTPUT_X.store(x, Ordering::SeqCst);
    OUTPUT_Y.store(y, Ordering::SeqCst);
    OUTPUT_W.store(width, Ordering::SeqCst);
    OUTPUT_H.store(height, Ordering::SeqCst);
}

pub fn set_runtime_backend(name: &str) {
    let code = match name {
        "winit" => 1,
        "udev" => 2,
        _ => 0,
    };
    RUNTIME_BACKEND.store(code, Ordering::SeqCst);
    let _ = RUNTIME_STARTED.set(std::time::Instant::now());
}

pub fn set_xwayland_status(status: &str, display: Option<u32>) {
    let code = match status {
        "disabled" => 1,
        "starting" => 2,
        "ready" => 3,
        "failed" => 4,
        _ => 0,
    };
    XWAYLAND_STATUS.store(code, Ordering::SeqCst);
    XWAYLAND_DISPLAY.store(display.map_or(-1, |number| number as i32), Ordering::SeqCst);
}

pub fn set_session_locked(locked: bool) {
    SESSION_LOCKED.store(locked, Ordering::SeqCst);
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

pub fn call0(proc: Scm) -> Option<Scm> {
    protected_call(move || unsafe { ffi::scm_call_0(proc) })
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

/// Evaluate one IPC request through the Scheme-side envelope. This is called
/// only by the calloop IPC source, so Guile and all policy mutation stay on
/// the compositor thread.
pub fn eval_ipc(code: &str) -> Option<String> {
    let result = call_named_1("minde-ipc-eval", from_str(code))?;
    to_string_lossy(result)
}

/// Asks the public status module to publish after Rust-only state changes
/// such as Xwayland becoming ready.
pub fn publish_status() {
    let Some(proc) = lookup("publish-status!") else {
        return;
    };
    let _ = protected_call(move || unsafe { ffi::scm_call_0(proc) });
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

fn from_symbol(s: &str) -> Scm {
    let c = to_cstring(s);
    unsafe { ffi::scm_from_utf8_symbol(c.as_ptr()) }
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
/// spawned from `handle-startup!` would otherwise inherit a stale/absent
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

/// Unix-epoch millis of the last user input event (key or pointer),
/// updated from `process_input_event`; `(wm-idle-ms)` reads it so
/// Scheme can implement idle timers (StumpWM *idle-hook* style).
static LAST_ACTIVITY_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn note_activity() {
    LAST_ACTIVITY_MS.store(now_ms(), Ordering::Relaxed);
}

unsafe extern "C" fn wm_idle_ms() -> Scm {
    let last = LAST_ACTIVITY_MS.load(Ordering::Relaxed);
    from_i64(if last == 0 {
        0
    } else {
        now_ms().saturating_sub(last) as i64
    })
}

/// `(wm-session-locked?)` -> boolean: whether the session is currently
/// locked via ext-session-lock (swaylock &c.).
unsafe extern "C" fn wm_session_locked() -> Scm {
    from_bool(SESSION_LOCKED.load(Ordering::SeqCst))
}

/// `(wm-publish-event line)` -- mirror one already-serialized event LINE (an
/// s-expression string) to every event-socket subscriber. The Scheme hook glue
/// (event-stream.scm) does the serialization, sanitization and lock-privacy
/// filtering; this just hands the finished line to the fan-out delivery path.
unsafe extern "C" fn wm_publish_event(line: Scm) -> Scm {
    if let Some(line) = to_string_lossy(line) {
        crate::events::publish_line(&line);
    }
    from_bool(true)
}

unsafe extern "C" fn wm_send_string(text: Scm, delay: Scm) -> Scm {
    let Some(text) = to_string_lossy(text) else {
        return from_bool(false);
    };
    let delay_ms = if delay == ffi::SCM_UNDEFINED {
        20
    } else {
        to_i64(delay).max(0) as u64
    };
    from_bool(send_command(WmCommand::SendString { text, delay_ms }))
}

fn button_number(value: Scm) -> Option<u32> {
    if to_bool(unsafe { ffi::scm_integer_p(value) }) {
        return match to_i64(value) {
            1 | 272 => Some(1),
            2 | 274 => Some(2),
            3 | 273 => Some(3),
            _ => None,
        };
    }
    if to_bool(unsafe { ffi::scm_symbol_p(value) }) {
        return match to_string_lossy(unsafe { ffi::scm_symbol_to_string(value) })?.as_str() {
            "left" => Some(1),
            "middle" => Some(2),
            "right" => Some(3),
            _ => None,
        };
    }
    None
}

unsafe extern "C" fn wm_click(button: Scm, count: Scm) -> Scm {
    let Some(button) = button_number(button) else {
        return from_bool(false);
    };
    let count = if count == ffi::SCM_UNDEFINED {
        1
    } else {
        to_i64(count)
    };
    if !(1..=32).contains(&count) {
        return from_bool(false);
    }
    from_bool(send_command(WmCommand::Click {
        button,
        count: count as u32,
    }))
}

unsafe extern "C" fn wm_paste_key() -> Scm {
    from_bool(send_command(WmCommand::PasteKey))
}

unsafe extern "C" fn wm_scroll(dx: Scm, dy: Scm) -> Scm {
    from_bool(send_command(WmCommand::Scroll {
        dx: to_i64(dx) as f64,
        dy: to_i64(dy) as f64,
    }))
}

/// `(wm-send-key mods keysym-name)` -- synthesizes one key press/release
/// (wrapped in the requested modifiers) into the focused window.
unsafe extern "C" fn wm_send_key(mods: Scm, keysym: Scm) -> Scm {
    let mods = to_i64(mods).max(0) as u32;
    let Some(keysym) = to_string_lossy(keysym) else {
        return from_bool(false);
    };
    from_bool(send_command(WmCommand::SendKey { mods, keysym }))
}

unsafe extern "C" fn wm_warp_pointer_relative(dx: Scm, dy: Scm) -> Scm {
    let dx = to_i64(dx) as i32;
    let dy = to_i64(dy) as i32;
    from_bool(send_command(WmCommand::WarpPointerRel { dx, dy }))
}

unsafe extern "C" fn wm_set_key_repeat(on: Scm) -> Scm {
    let on = to_bool(on);
    from_bool(send_command(WmCommand::SetKeyRepeat { on }))
}

/// `(wm-add-overlay x y text)` -- adds a positioned text overlay at a
/// global logical position (fselect/expose frame labels).
unsafe extern "C" fn wm_add_overlay(x: Scm, y: Scm, text: Scm) -> Scm {
    let x = to_i64(x) as i32;
    let y = to_i64(y) as i32;
    let Some(text) = to_string_lossy(text) else {
        return from_bool(false);
    };
    from_bool(send_command(WmCommand::AddOverlay { x, y, text }))
}

unsafe extern "C" fn wm_clear_overlays() -> Scm {
    from_bool(send_command(WmCommand::ClearOverlays))
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

unsafe extern "C" fn wm_pointer_position() -> Scm {
    let (x, y) = crate::automation_observe::pointer_position();
    scm_list(&[from_i64(x as i64), from_i64(y as i64)])
}

unsafe extern "C" fn wm_window_geometry(id: Scm) -> Scm {
    let id = to_i64(id) as u64;
    match crate::automation_observe::window_geometry(id) {
        Some([x, y, w, h]) => scm_list(&[
            from_i64(x as i64),
            from_i64(y as i64),
            from_i64(w as i64),
            from_i64(h as i64),
        ]),
        None => from_bool(false),
    }
}

fn string_list(mut list: Scm) -> Option<Vec<String>> {
    let mut strings = Vec::new();
    while !to_bool(unsafe { ffi::scm_null_p(list) }) {
        if strings.len() >= 256 || !to_bool(unsafe { ffi::scm_pair_p(list) }) {
            return None;
        }
        let item = unsafe { ffi::scm_car(list) };
        if !to_bool(unsafe { ffi::scm_string_p(item) }) {
            return None;
        }
        strings.push(to_string_lossy(item)?);
        list = unsafe { ffi::scm_cdr(list) };
    }
    Some(strings)
}

unsafe extern "C" fn wm_drop_files(x: Scm, y: Scm, paths: Scm) -> Scm {
    let Some(paths) = string_list(paths) else {
        return from_bool(false);
    };
    // Validate before allocating a public token: malformed requests return #f.
    if crate::automation_dnd::build_uri_list(&paths).is_err() {
        return from_bool(false);
    }
    let results = crate::automation_dnd::automation_results().clone();
    let token = results.allocate(crate::automation_dnd::AutomationOperation::DropFiles);
    let source = match crate::automation_dnd::AutomationDndSource::files(paths, token, results) {
        Ok(source) => source,
        Err(_) => return from_bool(false),
    };
    if send_command(WmCommand::Drop {
        x: to_i64(x) as i32,
        y: to_i64(y) as i32,
        source,
    }) {
        from_i64(token as i64)
    } else {
        from_bool(false)
    }
}

unsafe extern "C" fn wm_drop_text(x: Scm, y: Scm, text: Scm) -> Scm {
    if !to_bool(unsafe { ffi::scm_string_p(text) }) {
        return from_bool(false);
    }
    let Some(text) = to_string_lossy(text) else {
        return from_bool(false);
    };
    let results = crate::automation_dnd::automation_results().clone();
    let token = results.allocate(crate::automation_dnd::AutomationOperation::DropText);
    let source = crate::automation_dnd::AutomationDndSource::text(text, token, results);
    if send_command(WmCommand::Drop {
        x: to_i64(x) as i32,
        y: to_i64(y) as i32,
        source,
    }) {
        from_i64(token as i64)
    } else {
        from_bool(false)
    }
}

unsafe extern "C" fn wm_automation_status(token: Scm) -> Scm {
    let token = to_i64(token);
    if token <= 0 {
        return from_bool(false);
    }
    let Some(result) = crate::automation_dnd::automation_results().get(token as u64) else {
        return from_bool(false);
    };
    let operation = match result.operation {
        crate::automation_dnd::AutomationOperation::DropFiles => "drop-files",
        crate::automation_dnd::AutomationOperation::DropText => "drop-text",
    };
    let status = match result.status {
        crate::automation_dnd::AutomationStatus::Pending => "pending",
        crate::automation_dnd::AutomationStatus::Accepted => "accepted",
        crate::automation_dnd::AutomationStatus::Rejected => "rejected",
        crate::automation_dnd::AutomationStatus::NoTarget => "no-target",
        crate::automation_dnd::AutomationStatus::Cancelled => "cancelled",
        crate::automation_dnd::AutomationStatus::UnsupportedTarget => "unsupported-target",
    };
    scm_list(&[from_symbol(operation), from_symbol(status)])
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

unsafe extern "C" fn wm_set_primary(text: Scm) -> Scm {
    let Some(text) = to_string_lossy(text) else {
        return from_bool(false);
    };
    from_bool(send_command(WmCommand::SetPrimary { text }))
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
    items.iter().rev().fold(ffi::SCM_EOL, |tail, &head| unsafe {
        ffi::scm_cons(head, tail)
    })
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

/// `(wm-input-devices)` -> `((name cap ...) ...)`: the libinput devices
/// present on the seat, each with its capability names ("keyboard",
/// "pointer", "touch", ...). Empty under the winit backend (no libinput).
unsafe extern "C" fn wm_input_devices() -> Scm {
    let devices = INPUT_DEVICES.lock().unwrap().clone();
    let entries: Vec<Scm> = devices
        .iter()
        .map(|device| {
            let mut items = vec![from_str(&device.name)];
            items.extend(device.capabilities.iter().map(|c| from_str(c)));
            scm_list(&items)
        })
        .collect();
    scm_list(&entries)
}

/// Low-level primitive behind the Scheme `wm-configure-input!` wrapper
/// (see `scheme/init.scm`). All arguments are pre-normalized scalars so
/// the FFI stays simple:
/// - `match_`: device-name substring; the empty string matches every device.
/// - `tap`, `natural`: `1` = enable, `0` = disable, anything else = leave.
/// - `accel`, `click`: the profile/method string, or `""` to leave unchanged.
///
/// Stores the rule (replacing any earlier rule with the same match) and
/// asks the main thread to re-apply to devices already present.
unsafe extern "C" fn wm_configure_input_rule(
    match_: Scm,
    tap: Scm,
    natural: Scm,
    accel: Scm,
    click: Scm,
) -> Scm {
    let tri = |v: Scm| match to_i64(v) {
        1 => Some(true),
        0 => Some(false),
        _ => None,
    };
    let opt = |v: Scm| {
        to_string_lossy(v)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    };
    let rule = InputRule {
        match_name: to_string_lossy(match_).unwrap_or_default(),
        tap: tri(tap),
        natural_scroll: tri(natural),
        accel_profile: opt(accel),
        click_method: opt(click),
    };
    {
        let mut rules = INPUT_RULES.lock().unwrap();
        rules.retain(|r| r.match_name != rule.match_name);
        rules.push(rule);
    }
    send_command(WmCommand::ReapplyInputConfig);
    from_bool(true)
}

/// `(wm-runtime-info)` -> `(backend xwayland-status xdisplay uptime-ms)`.
unsafe extern "C" fn wm_runtime_info() -> Scm {
    let backend = match RUNTIME_BACKEND.load(Ordering::SeqCst) {
        1 => "winit",
        2 => "udev",
        _ => "unknown",
    };
    let xwayland = match XWAYLAND_STATUS.load(Ordering::SeqCst) {
        1 => "disabled",
        2 => "starting",
        3 => "ready",
        4 => "failed",
        _ => "unknown",
    };
    let display = XWAYLAND_DISPLAY.load(Ordering::SeqCst) as i64;
    let uptime = RUNTIME_STARTED
        .get()
        .map_or(0, |started| started.elapsed().as_millis() as i64);
    scm_list(&[
        from_str(backend),
        from_str(xwayland),
        from_i64(display),
        from_i64(uptime),
    ])
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

        register_gsubr(
            "wm-spawn",
            1,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm) -> Scm, ffi::Gsubr>(wm_spawn),
        );
        register_gsubr("wm-quit", 0, 0, 0, wm_quit);
        register_gsubr(
            "wm-log",
            1,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm) -> Scm, ffi::Gsubr>(wm_log),
        );
        register_gsubr(
            "wm-place-window",
            5,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm, Scm, Scm, Scm, Scm) -> Scm, ffi::Gsubr>(
                wm_place_window,
            ),
        );
        register_gsubr(
            "wm-focus-window",
            1,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm) -> Scm, ffi::Gsubr>(wm_focus_window),
        );
        register_gsubr(
            "wm-close-window",
            1,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm) -> Scm, ffi::Gsubr>(wm_close_window),
        );
        register_gsubr("wm-clear-focus", 0, 0, 0, wm_clear_focus);
        register_gsubr(
            "wm-message",
            1,
            1,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm, Scm) -> Scm, ffi::Gsubr>(wm_message),
        );
        register_gsubr("wm-clear-message", 0, 0, 0, wm_clear_message);
        register_gsubr(
            "wm-add-overlay",
            3,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm, Scm, Scm) -> Scm, ffi::Gsubr>(
                wm_add_overlay,
            ),
        );
        register_gsubr("wm-clear-overlays", 0, 0, 0, wm_clear_overlays);
        register_gsubr(
            "wm-border-color",
            1,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm) -> Scm, ffi::Gsubr>(wm_border_color),
        );
        register_gsubr(
            "wm-focus-rect",
            4,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm, Scm, Scm, Scm) -> Scm, ffi::Gsubr>(
                wm_focus_rect,
            ),
        );
        register_gsubr("wm-output-geometry", 0, 0, 0, wm_output_geometry);
        register_gsubr(
            "wm-run-after-ms",
            2,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm, Scm) -> Scm, ffi::Gsubr>(
                wm_run_after_ms,
            ),
        );
        register_gsubr(
            "wm-set-fullscreen",
            2,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm, Scm) -> Scm, ffi::Gsubr>(
                wm_set_fullscreen,
            ),
        );
        register_gsubr(
            "wm-kill-window",
            1,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm) -> Scm, ffi::Gsubr>(wm_kill_window),
        );
        register_gsubr(
            "wm-warp-pointer",
            2,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm, Scm) -> Scm, ffi::Gsubr>(
                wm_warp_pointer,
            ),
        );
        register_gsubr("wm-pointer-position", 0, 0, 0, wm_pointer_position);
        register_gsubr(
            "wm-window-geometry",
            1,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm) -> Scm, ffi::Gsubr>(wm_window_geometry),
        );
        register_gsubr(
            "wm-drop-files",
            3,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm, Scm, Scm) -> Scm, ffi::Gsubr>(
                wm_drop_files,
            ),
        );
        register_gsubr(
            "wm-drop-text",
            3,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm, Scm, Scm) -> Scm, ffi::Gsubr>(
                wm_drop_text,
            ),
        );
        register_gsubr(
            "wm-automation-status",
            1,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm) -> Scm, ffi::Gsubr>(
                wm_automation_status,
            ),
        );
        // Gsubr is exactly the zero-arg signature; no transmute needed.
        register_gsubr("wm-request-paste", 0, 0, 0, wm_request_paste);
        register_gsubr("wm-outputs", 0, 0, 0, wm_outputs);
        register_gsubr("wm-runtime-info", 0, 0, 0, wm_runtime_info);
        register_gsubr(
            "wm-set-clipboard",
            1,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm) -> Scm, ffi::Gsubr>(wm_set_clipboard),
        );
        register_gsubr(
            "wm-set-primary",
            1,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm) -> Scm, ffi::Gsubr>(wm_set_primary),
        );
        register_gsubr(
            "wm-place-float",
            5,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm, Scm, Scm, Scm, Scm) -> Scm, ffi::Gsubr>(
                wm_place_float,
            ),
        );
        register_gsubr(
            "wm-raise-window",
            1,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm) -> Scm, ffi::Gsubr>(wm_raise_window),
        );
        register_gsubr(
            "wm-set-floating",
            2,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm, Scm) -> Scm, ffi::Gsubr>(
                wm_set_floating,
            ),
        );
        register_gsubr(
            "wm-send-string",
            1,
            1,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm, Scm) -> Scm, ffi::Gsubr>(
                wm_send_string,
            ),
        );
        register_gsubr(
            "wm-type",
            1,
            1,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm, Scm) -> Scm, ffi::Gsubr>(
                wm_send_string,
            ),
        );
        register_gsubr(
            "wm-click",
            1,
            1,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm, Scm) -> Scm, ffi::Gsubr>(wm_click),
        );
        register_gsubr(
            "wm-send-key",
            2,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm, Scm) -> Scm, ffi::Gsubr>(wm_send_key),
        );
        register_gsubr(
            "wm-warp-pointer-relative",
            2,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm, Scm) -> Scm, ffi::Gsubr>(
                wm_warp_pointer_relative,
            ),
        );
        register_gsubr("wm-paste", 0, 0, 0, wm_paste_key);
        register_gsubr(
            "wm-scroll",
            2,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm, Scm) -> Scm, ffi::Gsubr>(wm_scroll),
        );
        register_gsubr(
            "wm-set-key-repeat",
            1,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm) -> Scm, ffi::Gsubr>(wm_set_key_repeat),
        );
        register_gsubr("wm-idle-ms", 0, 0, 0, wm_idle_ms);
        // libinput device query + low-level configuration primitive. The
        // friendly keyword-argument `wm-configure-input!` wraps the latter
        // in scheme/init.scm. Neither is part of a frozen public module.
        register_gsubr("wm-input-devices", 0, 0, 0, wm_input_devices);
        register_gsubr(
            "wm-configure-input-rule!",
            5,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm, Scm, Scm, Scm, Scm) -> Scm, ffi::Gsubr>(
                wm_configure_input_rule,
            ),
        );
        // Zero-arg, boolean return: matches Gsubr exactly, no transmute.
        register_gsubr("wm-session-locked?", 0, 0, 0, wm_session_locked);
        register_gsubr(
            "wm-publish-event",
            1,
            0,
            0,
            std::mem::transmute::<unsafe extern "C" fn(Scm) -> Scm, ffi::Gsubr>(wm_publish_event),
        );
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
    let module_dir = std::env::var("MINDE_SCHEME_DIR")
        .unwrap_or_else(|_| format!("{}/scheme", env!("CARGO_MANIFEST_DIR")));
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
    tracing::debug!(
        mods,
        keysym_name,
        consumed,
        errored = result.is_none(),
        "handle_key"
    );
    consumed
}

/// Calls `(handle-window-map! id title app-id)` if bound. `title`/`app_id` may
/// be empty strings if the client hasn't set them (yet).
pub fn on_window_map(id: u64, title: &str, app_id: &str) {
    call_named_3(
        "handle-window-map!",
        from_i64(id as i64),
        from_str(title),
        from_str(app_id),
    );
}

/// Calls `(handle-window-title-change! id title app-id)` if bound: a mapped
/// toplevel's title or app-id changed. Wayland clients set both only
/// after the initial configure, so `on_window_map` usually reports
/// empty strings and the real values arrive through here.
pub fn on_window_title(id: u64, title: &str, app_id: &str) {
    call_named_3(
        "handle-window-title-change!",
        from_i64(id as i64),
        from_str(title),
        from_str(app_id),
    );
}

/// Calls `(handle-window-unmap! id)` if bound.
pub fn on_window_unmap(id: u64) {
    call_named_1("handle-window-unmap!", from_i64(id as i64));
}

/// Reports the full head list (usable rects) to Scheme:
/// `(handle-heads-change! ((id x y w h) ...))`. Also refreshes the
/// `(wm-output-geometry)` union and the `(wm-outputs)` registry. Falls
/// back to the legacy single-head `handle-output-geometry!` when the new
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

    if let Some(proc) = lookup("handle-heads-change!") {
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
            "handle-output-geometry!",
            from_i64(first.x as i64),
            from_i64(first.y as i64),
            from_i64(first.w as i64),
            from_i64(first.h as i64),
        );
    }
}

/// Calls `(handle-timer! token)` if bound; fired by `WmCommand::RunAfter`'s
/// calloop timer on the main (Guile) thread.
pub fn on_timer(token: i64) {
    call_named_1("handle-timer!", from_i64(token));
}

/// Calls `(handle-paste! text)` if bound, delivering clipboard contents
/// requested via `wm-request-paste`.
pub fn on_paste(text: &str) {
    call_named_1("handle-paste!", from_str(text));
}

/// Calls `(handle-window-move! id x y w h)` if bound; fired when a
/// super+drag move/resize grab releases, so Scheme's `%floating` table
/// tracks the user-dragged geometry.
pub fn on_window_moved(id: u64, x: i32, y: i32, w: i32, h: i32) {
    call_named_5(
        "handle-window-move!",
        from_i64(id as i64),
        from_i64(x as i64),
        from_i64(y as i64),
        from_i64(w as i64),
        from_i64(h as i64),
    );
}

/// Calls `(handle-urgent-window! id)` if bound (xdg-activation request for a
/// mapped toplevel; StumpWM urgency).
pub fn on_urgent(id: u64) {
    call_named_1("handle-urgent-window!", from_i64(id as i64));
}

/// Calls `(handle-foreign-activate! id)` if bound: an external taskbar or
/// switcher (wlr-foreign-toplevel-management) asked to activate a window.
/// Routed through Scheme so the group/frame focus model stays authoritative.
pub fn on_foreign_activate(id: u64) {
    call_named_1("handle-foreign-activate!", from_i64(id as i64));
}

/// Calls `(handle-foreign-fullscreen! id on)` if bound: a foreign-toplevel
/// client requested (un)fullscreen. Scheme applies it via the same path as
/// the interactive fullscreen command, keeping its state model in sync.
pub fn on_foreign_fullscreen(id: u64, on: bool) {
    call_named_2(
        "handle-foreign-fullscreen!",
        from_i64(id as i64),
        from_bool(on),
    );
}

/// Calls `(handle-foreign-minimize! id on)` if bound: a foreign-toplevel
/// client requested (un)minimize. minde maps this onto hide/show.
pub fn on_foreign_minimize(id: u64, on: bool) {
    call_named_2(
        "handle-foreign-minimize!",
        from_i64(id as i64),
        from_bool(on),
    );
}

/// Policy gate for `wlr-output-management` apply requests: an external
/// tool (wlr-randr, kanshi, wdisplays) asked to change the output layout.
/// Returns whether the compositor should accept it. Consults the optional
/// Scheme predicate `(output-configuration-allowed?)`; if it is unbound
/// (the default) or errors, external configuration is accepted. A user can
/// define it to return `#f` to refuse all external output changes.
pub fn output_config_allowed() -> bool {
    match lookup("output-configuration-allowed?") {
        Some(proc) => call0(proc).map(to_bool).unwrap_or(true),
        None => true,
    }
}

/// Notifies Scheme that the output layout was changed by an external
/// `wlr-output-management` client, via `(handle-output-configured!)` if
/// bound, so a config can react (re-tile, persist, log). A no-op otherwise.
pub fn on_output_configured() {
    if let Some(proc) = lookup("handle-output-configured!") {
        let _ = protected_call(move || unsafe { ffi::scm_call_0(proc) });
    }
}

/// Calls `(handle-input-device-added!)` if bound, once a libinput device
/// arrives (udev backend only) and its stored `wm-configure-input!` rules
/// have been applied. Passes the device name and its capability-name list,
/// letting a config apply imperative per-device policy. Missing definition
/// is a no-op, same as the other hooks.
pub fn on_input_device_added(name: &str, capabilities: &[String]) {
    let Some(proc) = lookup("handle-input-device-added!") else {
        return;
    };
    let caps: Vec<Scm> = capabilities.iter().map(|c| from_str(c)).collect();
    let list = scm_list(&caps);
    let _ = call2(proc, from_str(name), list);
}

/// Calls `(handle-startup!)` if bound, once the first output is up and
/// synced. Missing definition is a no-op, same as the other `on_*` hooks.
/// Called from both backends (winit and udev) so autostart works whether
/// nested or standalone.
pub fn on_startup() {
    let Some(proc) = lookup("handle-startup!") else {
        return;
    };
    protected_call(move || unsafe { ffi::scm_call_0(proc) });
}

/// Calls `(wm-on-session-lock)` if bound, once the session becomes locked
/// via ext-session-lock. Missing definition is a no-op, same as the other
/// `on_*` hooks; a Scheme error is caught and never crashes the compositor.
pub fn on_session_lock() {
    let Some(proc) = lookup("wm-on-session-lock") else {
        return;
    };
    protected_call(move || unsafe { ffi::scm_call_0(proc) });
}

/// Calls `(wm-on-session-unlock)` if bound, once the session is unlocked.
pub fn on_session_unlock() {
    let Some(proc) = lookup("wm-on-session-unlock") else {
        return;
    };
    protected_call(move || unsafe { ffi::scm_call_0(proc) });
}
