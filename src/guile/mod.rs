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

/// Children launched by `wm-spawn` are owned by one polling waiter. Dropping a
/// `std::process::Child` does not reap it on Unix, which previously left every
/// closed application as a zombie owned by the compositor.
static CHILD_REAPER: OnceLock<std::sync::mpsc::Sender<std::process::Child>> = OnceLock::new();

fn child_reaper() -> &'static std::sync::mpsc::Sender<std::process::Child> {
    CHILD_REAPER.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel::<std::process::Child>();
        std::thread::Builder::new()
            .name("minde-child-reaper".into())
            .spawn(move || {
                use std::sync::mpsc::{RecvTimeoutError, TryRecvError};
                use std::time::Duration;

                let mut children = Vec::new();
                loop {
                    match receiver.recv_timeout(Duration::from_millis(250)) {
                        Ok(child) => children.push(child),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                    loop {
                        match receiver.try_recv() {
                            Ok(child) => children.push(child),
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => return,
                        }
                    }
                    children.retain_mut(|child| match child.try_wait() {
                        Ok(None) => true,
                        Ok(Some(status)) => {
                            tracing::debug!(pid = child.id(), %status, "wm-spawn child exited");
                            false
                        }
                        Err(error) => {
                            tracing::warn!(pid = child.id(), %error, "failed to reap wm-spawn child");
                            false
                        }
                    });
                }
            })
            .expect("failed to start child reaper");
        sender
    })
}

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

thread_local! {
    /// The compositor state, reachable from `wm-*` primitives while Scheme
    /// runs on the main thread. Set once by `main` after construction and
    /// cleared before the state is dropped; null on every other thread.
    static STATE: std::cell::Cell<*mut crate::MindeState> =
        const { std::cell::Cell::new(std::ptr::null_mut()) };
    /// True while a command is being applied through `STATE`, so a command
    /// issued by Scheme code that Rust calls back into from *inside* that
    /// application is queued instead of re-entering the half-updated state.
    static APPLYING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Publishes `state` as the target for direct command application. Every
/// Rust->Scheme hook runs beneath a calloop callback that already holds
/// `&mut MindeState`; the pointer lets a `wm-*` primitive apply its command
/// while that callback is suspended in Scheme, the same re-entrancy shape
/// as any C callback. The hook call sites therefore must not keep references
/// into the state alive across the call (they pass ids and clones, which is
/// what the compiler enforces for the queued path as well).
pub fn set_state(state: &mut crate::MindeState) {
    STATE.with(|cell| cell.set(state as *mut _));
}

/// Forgets the pointer published by [`set_state`]; later commands queue.
pub fn clear_state() {
    STATE.with(|cell| cell.set(std::ptr::null_mut()));
}

/// Runs `f` against the compositor state when Scheme is executing on the
/// main thread outside a command application, so a `wm-*` query reads the
/// live state instead of a mirror. `None` from the REPL thread, before
/// `set_state`, or re-entrantly from inside `apply_wm_command`.
fn with_state<R>(f: impl FnOnce(&mut crate::MindeState) -> R) -> Option<R> {
    if !on_guile_thread() || APPLYING.with(|cell| cell.get()) {
        return None;
    }
    let state = STATE.with(|cell| cell.get());
    if state.is_null() {
        return None;
    }
    // SAFETY: see `send_command`.
    Some(f(unsafe { &mut *state }))
}

struct ApplyingGuard;

impl Drop for ApplyingGuard {
    fn drop(&mut self) {
        APPLYING.with(|cell| cell.set(false));
    }
}

/// Applies `cmd` to the compositor state. On the Guile thread (every hook,
/// timer, key and IPC evaluation) it runs immediately and reports whether
/// the command found its target, so Scheme observes the result -- geometry,
/// focus, status -- within the same call. From the REPL thread, or while a
/// command is already being applied, it is queued on the calloop channel
/// and `true` only means "queued".
fn send_command(cmd: WmCommand) -> bool {
    if on_guile_thread() && !APPLYING.with(|cell| cell.get()) {
        let state = STATE.with(|cell| cell.get());
        if !state.is_null() {
            APPLYING.with(|cell| cell.set(true));
            let _guard = ApplyingGuard;
            // SAFETY: `state` was published by `set_state` from the main
            // thread, is only read on that thread, and is cleared before the
            // state is dropped. The calloop callback that owns the `&mut`
            // is suspended in a Scheme call for the whole duration.
            let state = unsafe { &mut *state };
            let started = std::time::Instant::now();
            let applied = state.apply_wm_command(cmd);
            crate::timing::record(crate::timing::Probe::ApplyCommand, started);
            return applied;
        }
    }
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
///
/// `scm_internal_catch` is synchronous, so the closure lives on this stack
/// frame; a Scheme non-local exit simply leaves it unconsumed and it is
/// dropped on return like any other local.
fn protected_call<F: FnOnce() -> Scm>(f: F) -> Option<Scm> {
    unsafe extern "C" fn body_trampoline<F: FnOnce() -> Scm>(data: *mut c_void) -> Scm {
        let slot = unsafe { &mut *(data as *mut Option<F>) };
        (slot.take().expect("protected_call body entered twice"))()
    }
    unsafe extern "C" fn handler_trampoline(data: *mut c_void, _key: Scm, _args: Scm) -> Scm {
        unsafe {
            *(data as *mut bool) = true;
        }
        ffi::SCM_BOOL_F
    }

    let mut errored = false;
    let mut slot = Some(f);
    let tag = ffi::SCM_BOOL_T; // #t tag means "catch everything"
    let result = unsafe {
        ffi::scm_internal_catch(
            tag,
            body_trampoline::<F>,
            (&mut slot) as *mut Option<F> as *mut c_void,
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

/// Quotes `s` as a Scheme string literal (only `\` and `"` need escaping;
/// Guile reads every other byte of a path verbatim).
fn scheme_string(s: &str) -> String {
    let mut quoted = String::with_capacity(s.len() + 2);
    quoted.push('"');
    for c in s.chars() {
        if matches!(c, '\\' | '"') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

/// Evaluates a snippet of Scheme source, catching errors.
pub fn eval_string(code: &str) -> Option<Scm> {
    let c = to_cstring(code);
    let ptr = c.as_ptr();
    protected_call(move || unsafe { ffi::scm_c_eval_string(ptr) })
}

/// Loads a Scheme file with `load` semantics, catching errors.
///
/// `load` (rather than `scm_c_primitive_load`) goes through Guile's
/// autocompiler: the file is compiled to a cached `.go` on first use (or when
/// the source changes) and runs in the VM from then on, exactly like the
/// bundled modules. `scm_c_primitive_load` would hand the whole policy layer
/// to the tree-walking evaluator on every start. Top-level definitions still
/// land in the current module, so re-loading the same file redefines in
/// place.
pub fn load_file(path: &std::path::Path) -> Option<Scm> {
    eval_string(&format!(
        "(load {})",
        scheme_string(&path.to_string_lossy())
    ))
}

/// A Scheme top-level name Rust calls into, with its symbol interned once.
///
/// Every call resolves the *current* binding through
/// `scm_module_variable`, so a live `define` from the REPL or a config
/// reload takes effect immediately, while the per-call cost drops to one
/// obarray lookup: no `CString`, no symbol interning and no
/// `scm_internal_catch` frame just to find out whether the name is bound.
struct Hook {
    name: &'static str,
    symbol: OnceLock<Scm>,
}

impl Hook {
    const fn new(name: &'static str) -> Self {
        Hook {
            name,
            symbol: OnceLock::new(),
        }
    }

    fn symbol(&self) -> Scm {
        *self.symbol.get_or_init(|| {
            let symbol = from_symbol(self.name);
            // Guile's symbol table is weak; pin the symbol so this cache
            // can never dangle.
            unsafe { ffi::scm_gc_protect_object(symbol) };
            symbol
        })
    }

    /// The current value bound to the name, or `None` when it is unbound.
    fn value(&self) -> Option<Scm> {
        unsafe {
            let var = ffi::scm_module_variable(ffi::scm_current_module(), self.symbol());
            if var == ffi::SCM_BOOL_F || !to_bool(ffi::scm_variable_bound_p(var)) {
                return None;
            }
            Some(ffi::scm_variable_ref(var))
        }
    }

    /// Calls the bound procedure with `args` under `protected_call`. An
    /// unbound name is a no-op returning `None`, the "missing definition =
    /// no-op" contract shared by every Rust->Scheme hook.
    fn call(&self, args: &[Scm]) -> Option<Scm> {
        let proc = self.value()?;
        protected_call(|| unsafe { ffi::scm_call_n(proc, args.as_ptr(), args.len()) })
    }
}

static IPC_EVAL: Hook = Hook::new("minde-ipc-eval");
static PUBLISH_STATUS: Hook = Hook::new("publish-status!");
static HANDLE_KEY: Hook = Hook::new("wm-handle-key");
static WINDOW_MAP: Hook = Hook::new("handle-window-map!");
static WINDOW_TITLE: Hook = Hook::new("handle-window-title-change!");
static WINDOW_UNMAP: Hook = Hook::new("handle-window-unmap!");
static HEADS_CHANGE: Hook = Hook::new("handle-heads-change!");
static OUTPUT_GEOMETRY: Hook = Hook::new("handle-output-geometry!");
static TIMER: Hook = Hook::new("handle-timer!");
static PASTE: Hook = Hook::new("handle-paste!");
static WINDOW_MOVE: Hook = Hook::new("handle-window-move!");
static URGENT: Hook = Hook::new("handle-urgent-window!");
static FOREIGN_ACTIVATE: Hook = Hook::new("handle-foreign-activate!");
static FOREIGN_FULLSCREEN: Hook = Hook::new("handle-foreign-fullscreen!");
static FOREIGN_MINIMIZE: Hook = Hook::new("handle-foreign-minimize!");
static OUTPUT_CONFIG_ALLOWED: Hook = Hook::new("output-configuration-allowed?");
static OUTPUT_CONFIGURED: Hook = Hook::new("handle-output-configured!");
static INPUT_DEVICE_ADDED: Hook = Hook::new("handle-input-device-added!");
static STARTUP: Hook = Hook::new("handle-startup!");
static SESSION_LOCK: Hook = Hook::new("wm-on-session-lock");
static SESSION_UNLOCK: Hook = Hook::new("wm-on-session-unlock");

/// Evaluate one IPC request through the Scheme-side envelope. This is called
/// only by the calloop IPC source, so Guile and all policy mutation stay on
/// the compositor thread.
pub fn eval_ipc(code: &str) -> Option<String> {
    let result = IPC_EVAL.call(&[from_str(code)])?;
    to_string_lossy(result)
}

/// Asks the public status module to publish after Rust-only state changes
/// such as Xwayland becoming ready.
pub fn publish_status() {
    PUBLISH_STATUS.call(&[]);
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

/// The thread that ran `scm_init_guile`, i.e. the compositor main thread.
static GUILE_THREAD: OnceLock<std::thread::ThreadId> = OnceLock::new();

fn on_guile_thread() -> bool {
    GUILE_THREAD.get() == Some(&std::thread::current().id())
}

unsafe extern "C" fn wm_spawn(cmd: Scm) -> Scm {
    if let Some(cmd) = to_string_lossy(cmd) {
        tracing::info!(%cmd, "wm-spawn");
        if on_guile_thread() {
            // Already on the main thread (key binding, IPC, timer): spawn
            // right away instead of bouncing through the command channel.
            spawn_on_main_thread(&cmd);
            from_bool(true)
        } else {
            // Enqueue instead of spawning right here: this subr may run on
            // the REPL server thread, and forking from there deadlocked the
            // main thread's GL swap (see WmCommand::Spawn).
            from_bool(send_command(WmCommand::Spawn { cmd }))
        }
    } else {
        from_bool(false)
    }
}

/// Actually spawns a child; runs on the main thread only (directly from
/// `wm-spawn`, or via `apply_wm_command` for REPL-thread callers).
/// `std::process::Command` uses `posix_spawn` when it can (no `pre_exec`,
/// cwd or credential changes -- true here), so this never pays for a full
/// `fork` of a process with GPU mappings.
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
    match command.spawn() {
        Ok(child) => {
            if let Err(error) = child_reaper().send(child) {
                tracing::warn!(%cmd, %error, "failed to hand wm-spawn child to reaper");
            }
        }
        Err(e) => tracing::warn!(%cmd, error = %e, "wm-spawn failed"),
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

/// `(wm-window-title ID)` -> `(title . app-id)` as the client set them, or
/// `#f` for an unknown window or off the main thread. Rust owns this
/// fact; Scheme's `window-title` asks here first and keeps its own copy
/// only for rename overrides and for tests without a compositor.
unsafe extern "C" fn wm_window_title(id: Scm) -> Scm {
    let id = to_i64(id) as u64;
    match with_state(|state| state.window_title(id)).flatten() {
        Some((title, app_id)) => unsafe { ffi::scm_cons(from_str(&title), from_str(&app_id)) },
        None => from_bool(false),
    }
}

/// `(wm-floating-ids)` -> the ids Rust currently treats as floating (set
/// through `wm-set-floating`), for `mirror-drift`.
unsafe extern "C" fn wm_floating_ids() -> Scm {
    let ids = with_state(|state| {
        let mut ids: Vec<u64> = state.floating_ids.iter().copied().collect();
        ids.sort_unstable();
        ids
    })
    .unwrap_or_default();
    let items: Vec<Scm> = ids.into_iter().map(|id| from_i64(id as i64)).collect();
    scm_list(&items)
}

/// `(wm-timing-stats)` -> one `(name count total-us max-us (b1 b2 b3 b4 b5))`
/// entry per probe in `crate::timing`; the buckets are counts at or below
/// 100 us, 1 ms, 4 ms, 16.6 ms and above.
unsafe extern "C" fn wm_timing_stats() -> Scm {
    let entries: Vec<Scm> = crate::timing::snapshot()
        .into_iter()
        .map(|probe| {
            let buckets: Vec<Scm> = probe.buckets.iter().map(|b| from_i64(*b as i64)).collect();
            scm_list(&[
                from_symbol(probe.name),
                from_i64(probe.count as i64),
                from_i64(probe.total_us as i64),
                from_i64(probe.max_us as i64),
                scm_list(&buckets),
            ])
        })
        .collect();
    scm_list(&entries)
}

/// `(wm-place-windows PLACEMENTS)`: applies a whole layout in one call.
/// PLACEMENTS is a list of `(id x y w h)` (tiled) or `(id x y w h #f)`
/// (float placement) entries. Returns the list of ids whose placement
/// failed (unknown window), so an empty list means every entry applied.
/// A malformed list returns `#f` without applying anything.
unsafe extern "C" fn wm_place_windows(placements: Scm) -> Scm {
    let mut commands = Vec::new();
    let mut list = placements;
    while !to_bool(unsafe { ffi::scm_null_p(list) }) {
        if commands.len() >= 4096 || !to_bool(unsafe { ffi::scm_pair_p(list) }) {
            return from_bool(false);
        }
        let entry = unsafe { ffi::scm_car(list) };
        let mut fields = [0i64; 5];
        let mut tiled = true;
        let mut rest = entry;
        for (index, field) in fields.iter_mut().enumerate() {
            if !to_bool(unsafe { ffi::scm_pair_p(rest) }) {
                return from_bool(false);
            }
            let item = unsafe { ffi::scm_car(rest) };
            if !to_bool(unsafe { ffi::scm_integer_p(item) }) {
                return from_bool(false);
            }
            *field = to_i64(item);
            rest = unsafe { ffi::scm_cdr(rest) };
            if index == 4 && to_bool(unsafe { ffi::scm_pair_p(rest) }) {
                tiled = to_bool(unsafe { ffi::scm_car(rest) });
            }
        }
        let [id, x, y, w, h] = fields;
        let (id, x, y, w, h) = (id as u64, x as i32, y as i32, w as i32, h as i32);
        commands.push((
            id,
            if tiled {
                WmCommand::Place { id, x, y, w, h }
            } else {
                WmCommand::PlaceFloat { id, x, y, w, h }
            },
        ));
        list = unsafe { ffi::scm_cdr(list) };
    }
    let failed: Vec<Scm> = commands
        .into_iter()
        .filter(|(_, command)| !send_command(command.clone()))
        .map(|(id, _)| from_i64(id as i64))
        .collect();
    scm_list(&failed)
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

/// `(wm-events-active?)` -> boolean: whether any event-socket subscriber is
/// connected. The Scheme mirror consults it before serialising an event, so
/// with nobody listening a hook firing costs no `write` at all.
unsafe extern "C" fn wm_events_active() -> Scm {
    from_bool(crate::events::has_subscribers())
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

/// `(wm-screenshot path [window-id])` -- deferred PNG screenshot of the
/// output under the pointer (or the region of `window-id`). Returns an
/// automation token; completion via `(wm-automation-status token)` ->
/// `(screenshot done|failed)` plus an `automation-result` event line.
unsafe extern "C" fn wm_screenshot(path: Scm, window_id: Scm) -> Scm {
    let Some(path) = to_string_lossy(path) else {
        return from_bool(false);
    };
    if !path.starts_with('/') {
        return from_bool(false);
    }
    let window_id = if window_id == ffi::SCM_UNDEFINED {
        None
    } else {
        let id = to_i64(window_id);
        if id <= 0 {
            return from_bool(false);
        }
        Some(id as u64)
    };
    let token = crate::automation_dnd::automation_results()
        .allocate(crate::automation_dnd::AutomationOperation::Screenshot);
    if send_command(WmCommand::Screenshot {
        path,
        window_id,
        token,
    }) {
        from_i64(token as i64)
    } else {
        from_bool(false)
    }
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
    scm_list(&[
        from_symbol(crate::automation_dnd::operation_name(result.operation)),
        from_symbol(crate::automation_dnd::status_name(result.status)),
    ])
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
    scm_list(&[from_i64(x), from_i64(y), from_i64(w), from_i64(h)])
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

/// Where the bundled Scheme modules ((minde frames) &c.) live.
///
/// `MINDE_SCHEME_DIR` wins when set (packages and the nested test harness
/// point it at the installed or checked-out tree); then the repository's
/// `scheme/` directory when this binary was built from a checkout that is
/// still around; otherwise `share/minde/scheme` relative to the executable's
/// installation prefix. `scripts/mindectl` resolves the same three sources.
pub fn scheme_dir() -> std::path::PathBuf {
    if let Some(dir) = std::env::var_os("MINDE_SCHEME_DIR") {
        return dir.into();
    }
    let repository_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scheme");
    if repository_dir.is_dir() {
        return repository_dir;
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent()?.parent().map(std::path::Path::to_path_buf))
        .map(|prefix| prefix.join("share/minde/scheme"))
        .unwrap_or(repository_dir)
}

/// Boots libguile on the calling thread and makes the bundled modules
/// importable. Shared by the compositor (`init`) and `--check-config`, which
/// validates in-process instead of spawning an external `guile`.
///
/// Must be called from the thread that will make every later libguile
/// call, and only once.
pub fn boot() {
    let _ = GUILE_THREAD.set(std::thread::current().id());
    unsafe { ffi::scm_init_guile() };
    // Make the bundled modules ((minde frames) &c.) importable from any
    // init file location, e.g. a user config in ~/.config/minde/.
    let module_dir = scheme_dir();
    eval_string(&format!(
        "(add-to-load-path {})",
        scheme_string(&module_dir.to_string_lossy())
    ));
}

/// Casts a concrete `unsafe extern "C" fn(Scm, ..) -> Scm` of the given
/// arity to the untyped `Gsubr` pointer `scm_c_define_gsubr` takes; Guile
/// dispatches on the arity it was told, so the pointer cast is the whole
/// ABI story.
macro_rules! gsubr {
    ($f:ident, 0) => {
        $f as ffi::Gsubr
    };
    ($f:ident, 1) => {
        std::mem::transmute::<unsafe extern "C" fn(Scm) -> Scm, ffi::Gsubr>($f)
    };
    ($f:ident, 2) => {
        std::mem::transmute::<unsafe extern "C" fn(Scm, Scm) -> Scm, ffi::Gsubr>($f)
    };
    ($f:ident, 3) => {
        std::mem::transmute::<unsafe extern "C" fn(Scm, Scm, Scm) -> Scm, ffi::Gsubr>($f)
    };
    ($f:ident, 4) => {
        std::mem::transmute::<unsafe extern "C" fn(Scm, Scm, Scm, Scm) -> Scm, ffi::Gsubr>($f)
    };
    ($f:ident, 5) => {
        std::mem::transmute::<unsafe extern "C" fn(Scm, Scm, Scm, Scm, Scm) -> Scm, ffi::Gsubr>($f)
    };
}

/// Defines one gsubr with `req` required and `opt` optional arguments (no
/// rest list). Keep every call site one literal name per call:
/// tests/api-introspect-test.scm parses them to check that `describe-api`
/// documents every primitive.
fn register_gsubr(name: &str, req: i32, opt: i32, f: ffi::Gsubr) {
    let c = to_cstring(name);
    unsafe {
        ffi::scm_c_define_gsubr(c.as_ptr(), req, opt, 0, f);
    }
}

/// Initializes Guile on the calling (main) thread, registers all Rust
/// subrs, then loads `scheme/init.scm`.
///
/// Must be called from the compositor's main thread, before any other
/// libguile call, and only once.
pub fn init(loop_signal: LoopSignal) {
    set_loop_signal(loop_signal);
    boot();

    unsafe {
        register_gsubr("wm-spawn", 1, 0, gsubr!(wm_spawn, 1));
        register_gsubr("wm-quit", 0, 0, gsubr!(wm_quit, 0));
        register_gsubr("wm-log", 1, 0, gsubr!(wm_log, 1));
        register_gsubr("wm-place-window", 5, 0, gsubr!(wm_place_window, 5));
        register_gsubr("wm-place-windows", 1, 0, gsubr!(wm_place_windows, 1));
        register_gsubr("wm-window-title", 1, 0, gsubr!(wm_window_title, 1));
        register_gsubr("wm-floating-ids", 0, 0, gsubr!(wm_floating_ids, 0));
        register_gsubr("wm-timing-stats", 0, 0, gsubr!(wm_timing_stats, 0));
        register_gsubr("wm-focus-window", 1, 0, gsubr!(wm_focus_window, 1));
        register_gsubr("wm-close-window", 1, 0, gsubr!(wm_close_window, 1));
        register_gsubr("wm-clear-focus", 0, 0, gsubr!(wm_clear_focus, 0));
        register_gsubr("wm-message", 1, 1, gsubr!(wm_message, 2));
        register_gsubr("wm-clear-message", 0, 0, gsubr!(wm_clear_message, 0));
        register_gsubr("wm-add-overlay", 3, 0, gsubr!(wm_add_overlay, 3));
        register_gsubr("wm-clear-overlays", 0, 0, gsubr!(wm_clear_overlays, 0));
        register_gsubr("wm-border-color", 1, 0, gsubr!(wm_border_color, 1));
        register_gsubr("wm-focus-rect", 4, 0, gsubr!(wm_focus_rect, 4));
        // Single-head union kept for configs predating `wm-outputs`; see
        // doc/generated/api-reference.md ("handle-output-geometry!").
        register_gsubr("wm-output-geometry", 0, 0, gsubr!(wm_output_geometry, 0));
        register_gsubr("wm-run-after-ms", 2, 0, gsubr!(wm_run_after_ms, 2));
        register_gsubr("wm-set-fullscreen", 2, 0, gsubr!(wm_set_fullscreen, 2));
        register_gsubr("wm-kill-window", 1, 0, gsubr!(wm_kill_window, 1));
        register_gsubr("wm-warp-pointer", 2, 0, gsubr!(wm_warp_pointer, 2));
        register_gsubr("wm-pointer-position", 0, 0, gsubr!(wm_pointer_position, 0));
        register_gsubr("wm-window-geometry", 1, 0, gsubr!(wm_window_geometry, 1));
        register_gsubr("wm-drop-files", 3, 0, gsubr!(wm_drop_files, 3));
        register_gsubr("wm-drop-text", 3, 0, gsubr!(wm_drop_text, 3));
        register_gsubr(
            "wm-automation-status",
            1,
            0,
            gsubr!(wm_automation_status, 1),
        );
        register_gsubr("wm-request-paste", 0, 0, gsubr!(wm_request_paste, 0));
        register_gsubr("wm-outputs", 0, 0, gsubr!(wm_outputs, 0));
        register_gsubr("wm-runtime-info", 0, 0, gsubr!(wm_runtime_info, 0));
        register_gsubr("wm-set-clipboard", 1, 0, gsubr!(wm_set_clipboard, 1));
        register_gsubr("wm-set-primary", 1, 0, gsubr!(wm_set_primary, 1));
        register_gsubr("wm-place-float", 5, 0, gsubr!(wm_place_float, 5));
        register_gsubr("wm-raise-window", 1, 0, gsubr!(wm_raise_window, 1));
        register_gsubr("wm-set-floating", 2, 0, gsubr!(wm_set_floating, 2));
        register_gsubr("wm-send-string", 1, 1, gsubr!(wm_send_string, 2));
        // Documented alias of wm-send-string (doc/api.md, "wm-type").
        register_gsubr("wm-type", 1, 1, gsubr!(wm_send_string, 2));
        register_gsubr("wm-click", 1, 1, gsubr!(wm_click, 2));
        register_gsubr("wm-send-key", 2, 0, gsubr!(wm_send_key, 2));
        register_gsubr(
            "wm-warp-pointer-relative",
            2,
            0,
            gsubr!(wm_warp_pointer_relative, 2),
        );
        register_gsubr("wm-paste", 0, 0, gsubr!(wm_paste_key, 0));
        register_gsubr("wm-screenshot", 1, 1, gsubr!(wm_screenshot, 2));
        register_gsubr("wm-scroll", 2, 0, gsubr!(wm_scroll, 2));
        register_gsubr("wm-set-key-repeat", 1, 0, gsubr!(wm_set_key_repeat, 1));
        register_gsubr("wm-idle-ms", 0, 0, gsubr!(wm_idle_ms, 0));
        // libinput device query + low-level configuration primitive. The
        // friendly keyword-argument `wm-configure-input!` wraps the latter
        // in scheme/init.scm. Neither is part of a frozen public module.
        register_gsubr("wm-input-devices", 0, 0, gsubr!(wm_input_devices, 0));
        register_gsubr(
            "wm-configure-input-rule!",
            5,
            0,
            gsubr!(wm_configure_input_rule, 5),
        );
        register_gsubr("wm-session-locked?", 0, 0, gsubr!(wm_session_locked, 0));
        register_gsubr("wm-publish-event", 1, 0, gsubr!(wm_publish_event, 1));
        register_gsubr("wm-events-active?", 0, 0, gsubr!(wm_events_active, 0));
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

    tracing::info!(path = %init_path.display(), "loading scheme init file");
    if load_file(&init_path).is_none() {
        tracing::error!("failed to load scheme init file; wm-handle-key etc. will be unavailable");
    }
}

/// Validates a configuration file with the bundled `(minde config)` module
/// in-process. Boots Guile on the calling thread (so this is for the
/// `--check-config` entry point, never a running compositor) and prints
/// the validation error, if any, to stderr. Returns whether the file is
/// valid.
pub fn check_config(path: &std::path::Path) -> bool {
    boot();
    let code = format!(
        "(begin \
           (use-modules (minde command-catalog) (minde config)) \
           (register-builtin-command-schemas!) \
           (catch #t \
             (lambda () (validate-configuration-file {}) #t) \
             (lambda (key . args) \
               (let ((port (current-error-port))) \
                 (display \"configuration invalid: \" port) \
                 (print-exception port #f key args) \
                 #f))))",
        scheme_string(&path.to_string_lossy())
    );
    eval_string(&code).is_some_and(to_bool)
}

/// Calls `(wm-handle-key mods keysym keysym-name)` if that variable is
/// currently bound, resolving it on every call so it can be redefined
/// live from the REPL. Returns `true` if the key was consumed.
pub fn handle_key(mods: u32, keysym: u32, keysym_name: &str, utf8: &str) -> bool {
    if HANDLE_KEY.value().is_none() {
        return false;
    }
    let started = std::time::Instant::now();
    let result = HANDLE_KEY.call(&[
        from_i64(mods as i64),
        from_i64(keysym as i64),
        from_str(keysym_name),
        from_str(utf8),
    ]);
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
    crate::timing::record(crate::timing::Probe::HandleKey, started);
    consumed
}

/// Calls `(handle-window-map! id title app-id)` if bound. `title`/`app_id` may
/// be empty strings if the client hasn't set them (yet).
pub fn on_window_map(id: u64, title: &str, app_id: &str) {
    WINDOW_MAP.call(&[from_i64(id as i64), from_str(title), from_str(app_id)]);
}

/// Calls `(handle-window-title-change! id title app-id)` if bound: a mapped
/// toplevel's title or app-id changed. Wayland clients set both only
/// after the initial configure, so `on_window_map` usually reports
/// empty strings and the real values arrive through here.
pub fn on_window_title(id: u64, title: &str, app_id: &str) {
    WINDOW_TITLE.call(&[from_i64(id as i64), from_str(title), from_str(app_id)]);
}

/// Calls `(handle-window-unmap! id)` if bound.
pub fn on_window_unmap(id: u64) {
    WINDOW_UNMAP.call(&[from_i64(id as i64)]);
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

    if HEADS_CHANGE.value().is_some() {
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
        HEADS_CHANGE.call(&[scm_list(&entries)]);
    } else {
        OUTPUT_GEOMETRY.call(&[
            from_i64(first.x as i64),
            from_i64(first.y as i64),
            from_i64(first.w as i64),
            from_i64(first.h as i64),
        ]);
    }
}

/// Calls `(handle-timer! token)` if bound; fired by `WmCommand::RunAfter`'s
/// calloop timer on the main (Guile) thread.
pub fn on_timer(token: i64) {
    TIMER.call(&[from_i64(token)]);
}

/// Calls `(handle-paste! text)` if bound, delivering clipboard contents
/// requested via `wm-request-paste`.
pub fn on_paste(text: &str) {
    PASTE.call(&[from_str(text)]);
}

/// Calls `(handle-window-move! id x y w h)` if bound; fired when a
/// super+drag move/resize grab releases, so Scheme's `%floating` table
/// tracks the user-dragged geometry.
pub fn on_window_moved(id: u64, x: i32, y: i32, w: i32, h: i32) {
    WINDOW_MOVE.call(&[
        from_i64(id as i64),
        from_i64(x as i64),
        from_i64(y as i64),
        from_i64(w as i64),
        from_i64(h as i64),
    ]);
}

/// Calls `(handle-urgent-window! id)` if bound (xdg-activation request for a
/// mapped toplevel; StumpWM urgency).
pub fn on_urgent(id: u64) {
    URGENT.call(&[from_i64(id as i64)]);
}

/// Calls `(handle-foreign-activate! id)` if bound: an external taskbar or
/// switcher (wlr-foreign-toplevel-management) asked to activate a window.
/// Routed through Scheme so the group/frame focus model stays authoritative.
pub fn on_foreign_activate(id: u64) {
    FOREIGN_ACTIVATE.call(&[from_i64(id as i64)]);
}

/// Calls `(handle-foreign-fullscreen! id on)` if bound: a foreign-toplevel
/// client requested (un)fullscreen. Scheme applies it via the same path as
/// the interactive fullscreen command, keeping its state model in sync.
pub fn on_foreign_fullscreen(id: u64, on: bool) {
    FOREIGN_FULLSCREEN.call(&[from_i64(id as i64), from_bool(on)]);
}

/// Calls `(handle-foreign-minimize! id on)` if bound: a foreign-toplevel
/// client requested (un)minimize. minde maps this onto hide/show.
pub fn on_foreign_minimize(id: u64, on: bool) {
    FOREIGN_MINIMIZE.call(&[from_i64(id as i64), from_bool(on)]);
}

/// Policy gate for `wlr-output-management` apply requests: an external
/// tool (wlr-randr, kanshi, wdisplays) asked to change the output layout.
/// Returns whether the compositor should accept it. Consults the optional
/// Scheme predicate `(output-configuration-allowed?)`; if it is unbound
/// (the default) or errors, external configuration is accepted. A user can
/// define it to return `#f` to refuse all external output changes.
pub fn output_config_allowed() -> bool {
    if OUTPUT_CONFIG_ALLOWED.value().is_none() {
        return true;
    }
    OUTPUT_CONFIG_ALLOWED.call(&[]).map(to_bool).unwrap_or(true)
}

/// Notifies Scheme that the output layout was changed by an external
/// `wlr-output-management` client, via `(handle-output-configured!)` if
/// bound, so a config can react (re-tile, persist, log). A no-op otherwise.
pub fn on_output_configured() {
    OUTPUT_CONFIGURED.call(&[]);
}

/// Calls `(handle-input-device-added!)` if bound, once a libinput device
/// arrives (udev backend only) and its stored `wm-configure-input!` rules
/// have been applied. Passes the device name and its capability-name list,
/// letting a config apply imperative per-device policy. Missing definition
/// is a no-op, same as the other hooks.
pub fn on_input_device_added(name: &str, capabilities: &[String]) {
    if INPUT_DEVICE_ADDED.value().is_none() {
        return;
    }
    let caps: Vec<Scm> = capabilities.iter().map(|c| from_str(c)).collect();
    INPUT_DEVICE_ADDED.call(&[from_str(name), scm_list(&caps)]);
}

/// Calls `(handle-startup!)` if bound, once the first output is up and
/// synced. Missing definition is a no-op, same as the other `on_*` hooks.
/// Called from both backends (winit and udev) so autostart works whether
/// nested or standalone.
pub fn on_startup() {
    STARTUP.call(&[]);
}

/// Calls `(wm-on-session-lock)` if bound, once the session becomes locked
/// via ext-session-lock. Missing definition is a no-op, same as the other
/// `on_*` hooks; a Scheme error is caught and never crashes the compositor.
pub fn on_session_lock() {
    SESSION_LOCK.call(&[]);
}

/// Calls `(wm-on-session-unlock)` if bound, once the session is unlocked.
pub fn on_session_unlock() {
    SESSION_UNLOCK.call(&[]);
}

#[cfg(test)]
mod tests {
    use super::scheme_string;

    #[test]
    fn scheme_string_escapes_only_quotes_and_backslashes() {
        assert_eq!(scheme_string("/a/b"), "\"/a/b\"");
        assert_eq!(scheme_string("q\"x\\y"), "\"q\\\"x\\\\y\"");
    }
}
