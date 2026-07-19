// SPDX-License-Identifier: MIT

use smithay::wayland::seat::WaylandFocus;
use std::{ffi::OsString, sync::Arc};

use smithay::{
    desktop::{PopupManager, Space, Window, WindowSurfaceType},
    input::{Seat, SeatState},
    reexports::{
        calloop::{
            EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction,
            channel::Event as ChannelEvent, generic::Generic,
        },
        wayland_server::{
            Display, DisplayHandle,
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        fractional_scale::FractionalScaleManagerState,
        idle_inhibit::IdleInhibitManagerState,
        idle_notify::IdleNotifierState,
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        session_lock::{LockSurface, SessionLockManagerState},
        shell::xdg::XdgShellState,
        shm::ShmState,
        socket::ListeningSocketSource,
        viewporter::ViewporterState,
    },
};

use crate::guile::{self, WmCommand};

/// Clamp a point to the nearest mapped output.  Treating outputs as a summed
/// horizontal strip breaks for negative origins, vertical arrangements and
/// gaps, so keep this calculation independent and table-testable.
fn clamp_point_to_rectangles(
    pos: Point<f64, Logical>,
    rectangles: impl IntoIterator<Item = Rectangle<i32, Logical>>,
) -> Point<f64, Logical> {
    let (x, y) = pos.into();
    let mut nearest: Option<(f64, Point<f64, Logical>)> = None;

    for rectangle in rectangles {
        let rectangle = rectangle.to_f64();
        if rectangle.contains(pos) {
            return pos;
        }
        let left = rectangle.loc.x;
        let top = rectangle.loc.y;
        let right = left + rectangle.size.w;
        let bottom = top + rectangle.size.h;
        let candidate: Point<f64, Logical> = (x.clamp(left, right), y.clamp(top, bottom)).into();
        let dx = candidate.x - x;
        let dy = candidate.y - y;
        let distance = dx * dx + dy * dy;
        if nearest
            .as_ref()
            .is_none_or(|(best_distance, _)| distance < *best_distance)
        {
            nearest = Some((distance, candidate));
        }
    }

    nearest.map_or(pos, |(_, point)| point)
}

/// Compositor state. Adapted from Smithay's `smallvil` example
/// (https://github.com/Smithay/Smithay, MIT licensed); see README for details.
pub struct MindeState {
    pub start_time: std::time::Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,

    pub space: Space<Window>,
    pub loop_signal: LoopSignal,
    /// Handle into the compositor's own event loop, used by the udev
    /// backend to register per-device DRM event sources and repaint
    /// timers after startup (see `src/udev.rs`).
    pub handle: LoopHandle<'static, MindeState>,

    // Smithay protocol state
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,
    /// Primary selection (`zwp_primary_selection_device_manager_v1`):
    /// middle-click paste between Wayland clients, mirrored to/from
    /// Xwayland. Registered unconditionally in `new`.
    pub primary_selection_state:
        smithay::wayland::selection::primary_selection::PrimarySelectionState,
    /// `wlr-data-control-unstable-v1` manager state: lets clipboard
    /// managers (cliphist, `wl-paste --watch`) observe and set the
    /// clipboard and primary selection without holding keyboard focus.
    pub data_control_state: smithay::wayland::selection::wlr_data_control::DataControlState,
    /// `ext-data-control-v1` manager state: the standardized successor to
    /// wlr-data-control, for newer clipboard managers. Both are advertised.
    pub ext_data_control_state: smithay::wayland::selection::ext_data_control::DataControlState,
    pub xdg_decoration_state: smithay::wayland::shell::xdg::decoration::XdgDecorationState,
    pub xdg_activation_state: smithay::wayland::xdg_activation::XdgActivationState,
    pub xwayland_shell_state: smithay::wayland::xwayland_shell::XWaylandShellState,
    /// The X11 window manager connection, once Xwayland is up.
    pub xwm: Option<smithay::xwayland::X11Wm>,
    /// The X display number (":N") Xwayland serves, once ready.
    pub xdisplay: Option<u32>,
    pub layer_shell_state: smithay::wayland::shell::wlr_layer::WlrLayerShellState,
    pub popups: PopupManager,

    /// `ext-session-lock-v1` manager global state (registered in `new`).
    pub session_lock_state: SessionLockManagerState,
    /// Whether the session is locked. While set, both backends render ONLY
    /// each output's lock surface (or solid black if none is committed / the
    /// lock client died), and `process_input_event` delivers nothing to
    /// regular clients or the Scheme keybinding layer. This is the security
    /// boundary: no desktop pixel and no stray input while locked.
    pub locked: bool,
    /// Committed lock surfaces, one per output (keyed by the compositor
    /// `Output`). A missing or dead entry for an output means "draw solid
    /// black" -- the spec forbids ever flashing desktop content while
    /// locked. See `src/handlers/session_lock.rs`.
    pub lock_surfaces: Vec<(smithay::output::Output, LockSurface)>,

    pub seat: Seat<Self>,

    /// Registry of mapped toplevels by their stable id, assigned in
    /// `handlers::xdg_shell::new_toplevel`. Scheme addresses windows only by
    /// this id (see `wm-place-window` &c. in `guile::mod`).
    pub windows: Vec<(u64, Window)>,
    pub next_window_id: u64,
    /// Ids Scheme marked floating (`wm-set-floating`); gates the
    /// super+drag move/resize grabs in `input.rs`. Float geometry itself
    /// lives on the Scheme side (`%floating`).
    pub floating_ids: std::collections::HashSet<u64>,
    /// Window last focused via `wm-focus-window`; gets the border drawn
    /// around it in the render pass.
    pub focused_window: Option<Window>,
    /// Rectangle of the currently-selected frame (sent from Scheme's
    /// `sync-frames!` via `wm-focus-rect`); the focus border is drawn
    /// around this so an empty frame is still visibly selected.
    pub focus_rect: Option<Rectangle<i32, Logical>>,
    /// Currently displayed message overlay (StumpWM echo window), if any.
    /// The generation counter lets the hide timer verify it isn't
    /// clearing a newer message.
    pub message: Option<crate::render::MessageState>,
    pub message_generation: u64,
    /// Positioned text overlays (fselect/expose frame-number labels):
    /// global logical position -> rasterized label. Accumulated via
    /// `wm-add-overlay`, dropped via `wm-clear-overlays`.
    pub overlays: Vec<(Point<i32, Logical>, crate::render::MessageState)>,
    /// Last head list (usable rects) sent to Scheme, to avoid
    /// re-announcing unchanged geometry on every commit.
    pub reported_heads: Vec<guile::HeadInfo>,
    /// Monotonic source of stable per-output ids (stored in each
    /// `Output`'s user data as `OutputId`).
    pub next_output_id: u64,
    /// Focus border color; Scheme flips it while the prefix key is armed
    /// (StumpWM's pointer-box equivalent).
    pub border_color: [f32; 4],

    /// Last title/app-id reported to Scheme per window id. Clients set
    /// them only after the initial commit (so the map-time report is
    /// usually empty) and retitle at will; the commit handler diffs
    /// against this and calls `handle-window-title-change!` on change.
    pub reported_titles: std::collections::HashMap<u64, (String, String)>,

    /// Compositor-side auto-repeat for consumed key presses (prompts and
    /// armed keymaps -- clients repeat held keys themselves, but keys the
    /// compositor swallows never come back from libinput as repeats).
    /// Toggled from Scheme via `wm-set-key-repeat`; the active repeat is
    /// the held raw keycode plus its calloop timer's token.
    pub key_repeat_enabled: bool,
    pub key_repeat: Option<(u32, smithay::reexports::calloop::RegistrationToken)>,

    /// Pointer location in the global (logical) coordinate space. Updated
    /// by every pointer-motion input event (absolute in winit, relative in
    /// the udev/libinput backend) and used to render the cursor.
    pub pointer_location: Point<f64, Logical>,
    /// Current pointer cursor image (default fallback vs. client surface),
    /// set via `SeatHandler::cursor_image`; consulted by the udev render
    /// pass.
    pub cursor_state: crate::render::CursorState,

    /// The libseat session handle, set by the udev backend after it takes
    /// over a VT. `None` under winit, which makes VT-switch keysyms a no-op
    /// there (see `input.rs`).
    pub session: Option<smithay::backend::session::libseat::LibSeatSession>,
    /// Backend-private state for the udev/DRM backend; `None` under winit.
    pub udev_data: Option<crate::udev::UdevBackendData>,

    /// Screencopy (`ext-image-copy-capture-v1`) delegate state and the
    /// `ext-image-capture-source-v1` source managers. `capture_sessions`
    /// parks the owned [`Session`](smithay::wayland::image_copy_capture::Session)
    /// handles (dropping one sends `stopped`); `pending_captures` holds frames
    /// queued by clients, satisfied after the next render of their output.
    /// See `handlers::screencopy`.
    pub image_capture_source_state: smithay::wayland::image_capture_source::ImageCaptureSourceState,
    pub output_capture_source_state:
        smithay::wayland::image_capture_source::OutputCaptureSourceState,
    pub image_copy_capture_state: smithay::wayland::image_copy_capture::ImageCopyCaptureState,
    pub capture_sessions: Vec<smithay::wayland::image_copy_capture::Session>,
    pub pending_captures: Vec<crate::handlers::screencopy::PendingCapture>,

    /// Active `wlr-gamma-control-unstable-v1` controls, one per output
    /// (blue-light tools: gammastep, wlsunset). Only ever populated under
    /// the udev backend, which alone advertises the manager global. See
    /// `handlers::gamma_control`.
    pub gamma_controls: std::collections::HashMap<
        smithay::output::Output,
        crate::handlers::gamma_control::GammaControlEntry,
    >,

    /// `wlr-foreign-toplevel-management-unstable-v1` manager state: mirrors
    /// the window registry into taskbar/switcher handles. See
    /// `handlers::foreign_toplevel`.
    pub foreign_toplevel: crate::handlers::foreign_toplevel::ForeignToplevelManagerState,

    /// `wlr-output-management-unstable-v1` manager state: lets wlr-randr,
    /// kanshi and wdisplays query and set the output layout. See
    /// `handlers::output_management`.
    pub output_management: crate::handlers::output_management::OutputManagementState,

    /// `zwp_pointer_constraints_v1` global state (pointer lock/confinement)
    /// and `zwp_relative_pointer_manager_v1` global state (raw relative
    /// motion). Both are per-surface client protocols for games and
    /// pointer-lock clients; the input integration lives in
    /// `handlers::pointer_constraints` and `input.rs`.
    pub pointer_constraints_state: smithay::wayland::pointer_constraints::PointerConstraintsState,
    pub relative_pointer_manager_state:
        smithay::wayland::relative_pointer::RelativePointerManagerState,
    /// `wp-fractional-scale-v1` manager global: lets clients request a
    /// fractional buffer scale and learn each surface's preferred scale.
    /// Advertised on both backends; the preferred scale is (re)sent from
    /// `FractionalScaleHandler::new_fractional_scale` and whenever an
    /// output's scale changes (`update_fractional_scales`).
    pub fractional_scale_manager_state: FractionalScaleManagerState,
    /// `wp-viewporter` global: lets clients crop/scale their buffer to a
    /// logical destination size. Registered unconditionally; Smithay's
    /// commit buffer handler validates and applies the viewport, and its
    /// surface render elements honor the destination size automatically.
    pub viewporter_state: ViewporterState,

    /// Last cursor-position hint committed by a locked-pointer client
    /// (surface-local), honored by warping the cursor there on unlock. See
    /// `PointerConstraintsHandler` in `handlers::pointer_constraints`.
    pub pointer_lock_hint: Option<(WlSurface, Point<f64, Logical>)>,

    /// `ext-idle-notify-v1` notifier state: per-seat idle timers that fire
    /// `idled`/`resumed` to clients (swayidle &c.). Reset on every input
    /// event from `process_input_event`; suppressed while an idle inhibitor
    /// is active. See `handlers::idle`.
    pub idle_notifier_state: IdleNotifierState<Self>,
    /// `zwp_idle_inhibit_manager_v1` global state. Clients (fullscreen
    /// video/calls) create per-surface inhibitors; we track the set of
    /// surfaces with a live inhibitor and, while non-empty, mark the idle
    /// notifier inhibited. See `handlers::idle`.
    pub idle_inhibit_state: IdleInhibitManagerState,
    /// Surfaces that currently hold an idle inhibitor. Non-empty means idle
    /// notifications are suppressed. See `handlers::idle`.
    pub idle_inhibitors: std::collections::HashSet<WlSurface>,
    /// `wp_cursor_shape_manager_v1` global state. Kept alive so the global
    /// stays advertised; requests route to `SeatHandler::cursor_image`.
    pub cursor_shape_manager_state: smithay::wayland::cursor_shape::CursorShapeManagerState,

    /// `zwp_text_input_manager_v3` and `zwp_input_method_manager_v2` global
    /// state (IME: fcitx5/ibus, on-screen keyboards). Kept alive so both
    /// globals stay advertised; per-seat `TextInputHandle`/`InputMethodHandle`
    /// live in the seat user-data. Text-input focus follows keyboard focus via
    /// `set_text_input_focus`; the candidate-window popup is handled in
    /// `handlers::input_method`.
    pub text_input_manager_state: smithay::wayland::text_input::TextInputManagerState,
    pub input_method_manager_state: smithay::wayland::input_method::InputMethodManagerState,

    /// `zwp_virtual_keyboard_manager_v1` global state (wtype/ydotool-style
    /// automation and accessibility tools). Kept alive so the global stays
    /// advertised; Smithay's virtual-keyboard handle injects keys straight
    /// into the focused surface's `wl_keyboard` (see the module docs in
    /// `handlers::virtual_keyboard`). The client filter admits every client:
    /// these are user-session automation tools, the same stance taken for the
    /// input-method manager.
    pub virtual_keyboard_manager_state:
        smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState,
    /// `zwp_keyboard_shortcuts_inhibit_manager_v1` global state
    /// (remote-desktop/VM clients). Inhibitors are auto-granted but only
    /// *active* while their surface holds keyboard focus; an active inhibitor
    /// makes `process_input_event` bypass the prefix grab and every Scheme
    /// shortcut. See `handlers::keyboard_shortcuts_inhibit`.
    pub keyboard_shortcuts_inhibit_state:
        smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitState,
    /// The inhibitor currently activated because its surface holds keyboard
    /// focus, if any. Tracked so it can be deactivated the instant focus
    /// leaves the surface (the protocol forbids an inhibitor surviving focus
    /// loss). See `update_keyboard_shortcuts_inhibitors`.
    pub active_shortcuts_inhibitor:
        Option<smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitor>,
}

impl MindeState {
    pub fn new(event_loop: &mut EventLoop<'static, Self>, display: Display<Self>) -> Self {
        let start_time = std::time::Instant::now();

        let dh = display.handle();

        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let popups = PopupManager::default();

        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        // Primary selection plus both data-control managers. Data control
        // is told about primary selection so clipboard managers can also
        // manage the middle-click selection; the `|_| true` filters admit
        // every client (these are unprivileged desktop tools here).
        let primary_selection_state =
            smithay::wayland::selection::primary_selection::PrimarySelectionState::new::<Self>(&dh);
        let data_control_state =
            smithay::wayland::selection::wlr_data_control::DataControlState::new::<Self, _>(
                &dh,
                Some(&primary_selection_state),
                |_| true,
            );
        let ext_data_control_state =
            smithay::wayland::selection::ext_data_control::DataControlState::new::<Self, _>(
                &dh,
                Some(&primary_selection_state),
                |_| true,
            );
        let xdg_decoration_state =
            smithay::wayland::shell::xdg::decoration::XdgDecorationState::new::<Self>(&dh);
        let xdg_activation_state =
            smithay::wayland::xdg_activation::XdgActivationState::new::<Self>(&dh);
        let xwayland_shell_state =
            smithay::wayland::xwayland_shell::XWaylandShellState::new::<Self>(&dh);
        let layer_shell_state =
            smithay::wayland::shell::wlr_layer::WlrLayerShellState::new::<Self>(&dh);
        // Register the ext-session-lock-v1 manager global. The filter admits
        // every client; a lock client (swaylock &c.) binds it to lock.
        let session_lock_state = SessionLockManagerState::new::<Self, _>(&dh, |_| true);

        // Screencopy: the copy-capture manager plus the output image-capture
        // source manager (both admit every client). We deliberately do NOT
        // register the foreign-toplevel source manager or a cursor session --
        // see `handlers::screencopy`.
        let image_capture_source_state =
            smithay::wayland::image_capture_source::ImageCaptureSourceState::new();
        let output_capture_source_state =
            smithay::wayland::image_capture_source::OutputCaptureSourceState::new::<Self>(&dh);
        let image_copy_capture_state =
            smithay::wayland::image_copy_capture::ImageCopyCaptureState::new::<Self>(&dh);
        // Also serve the legacy wlr-screencopy protocol (wf-recorder,
        // xdg-desktop-portal-wlr) on both backends; see handlers::wlr_screencopy.
        let _ = crate::handlers::wlr_screencopy::init_wlr_screencopy_manager(&dh);
        // wlr-foreign-toplevel-management: advertised on both backends so
        // external bars/switchers can enumerate and control windows.
        let foreign_toplevel =
            crate::handlers::foreign_toplevel::init_foreign_toplevel_manager(&dh);
        // wlr-output-management: advertised on both backends. Under winit the
        // output size is fixed, so mode changes fail rather than lie; scale,
        // transform and position still apply.
        let output_management = crate::handlers::output_management::init_output_management(&dh);

        // Pointer constraints (lock/confine) and relative pointer. Both are
        // advertised on both backends; the relative-motion source differs
        // (true libinput deltas under udev, deltas synthesized from absolute
        // motion under winit -- see input.rs).
        let pointer_constraints_state =
            smithay::wayland::pointer_constraints::PointerConstraintsState::new::<Self>(&dh);
        let relative_pointer_manager_state =
            smithay::wayland::relative_pointer::RelativePointerManagerState::new::<Self>(&dh);

        // Fractional scale and viewporter: advertised on both backends.
        // Fractional scale reports each surface's preferred scale (following
        // its output); viewporter lets clients render at a logical
        // destination size independent of buffer scale.
        let fractional_scale_manager_state = FractionalScaleManagerState::new::<Self>(&dh);
        let viewporter_state = ViewporterState::new::<Self>(&dh);

        // ext-idle-notify-v1 and zwp_idle_inhibit_manager_v1: both advertised
        // on both backends. The notifier owns per-seat idle timers on the
        // event loop; the inhibit manager lets clients suppress them. See
        // handlers::idle for the input/inhibit wiring.
        let idle_notifier_state = IdleNotifierState::<Self>::new(&dh, event_loop.handle());
        let idle_inhibit_state = IdleInhibitManagerState::new::<Self>(&dh);

        // wp_cursor_shape_manager_v1: clients request a named cursor shape
        // instead of attaching their own surface; Smithay routes the request
        // to `SeatHandler::cursor_image` as `CursorImageStatus::Named`, which
        // the render path resolves through the Xcursor theme. See
        // `crate::render::CursorState`.
        let cursor_shape_manager_state =
            smithay::wayland::cursor_shape::CursorShapeManagerState::new::<Self>(&dh);

        // text-input-v3 + input-method-v2: both advertised on both backends.
        // The input-method manager admits every client (privileged; the
        // protocol enforces one active IME per seat). See handlers::input_method.
        let (text_input_manager_state, input_method_manager_state) =
            crate::handlers::input_method::init_input_method(&dh);

        // virtual-keyboard + virtual-pointer: automation/accessibility tools
        // (wtype, ydotool, wlrctl). The virtual-keyboard filter admits every
        // client -- these are user-session tools, and their injected keys go
        // to the focused surface honestly (gated by focus, never bypassing the
        // session lock; a locked session's focus is its lock surface). The
        // virtual-pointer manager is hand-rolled (Smithay ships no server
        // module); its global id is discarded like the other wlr globals.
        let virtual_keyboard_manager_state =
            smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState::new::<Self, _>(
                &dh,
                |_client| true,
            );
        let _ = crate::handlers::virtual_pointer::init_virtual_pointer_manager(&dh);

        // wp-tearing-control-v1: advertised on both backends so tearing-aware
        // clients (games) find the protocol. Advisory only -- the DRM backend
        // cannot perform async page flips at this Smithay revision, so the hint
        // is recorded but never acted upon (see handlers::tearing_control and
        // the capability matrix). wp-presentation-time and linux-drm-syncobj
        // are udev-only and registered in `init_udev` (they need real vblank
        // timestamps and a syncobj-capable DRM device respectively).
        let _ = crate::handlers::tearing_control::init_tearing_control_manager(&dh);

        // keyboard-shortcuts-inhibit: remote-desktop/VM clients. Inhibitors
        // are auto-granted, but only take effect while their surface has
        // keyboard focus (see handlers::keyboard_shortcuts_inhibit).
        let keyboard_shortcuts_inhibit_state =
            smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitState::new::<Self>(
                &dh,
            );

        let mut seat_state = SeatState::new();
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "winit");

        // Keymap from the standard XKB_DEFAULT_* environment variables
        // (e.g. XKB_DEFAULT_LAYOUT=de XKB_DEFAULT_VARIANT=bone); empty
        // strings mean xkbcommon defaults ("us").
        let env = |k: &str| std::env::var(k).unwrap_or_default();
        let (rules, model, layout, variant) = (
            env("XKB_DEFAULT_RULES"),
            env("XKB_DEFAULT_MODEL"),
            env("XKB_DEFAULT_LAYOUT"),
            env("XKB_DEFAULT_VARIANT"),
        );
        let xkb_config = smithay::input::keyboard::XkbConfig {
            rules: &rules,
            model: &model,
            layout: &layout,
            variant: &variant,
            options: std::env::var("XKB_DEFAULT_OPTIONS").ok(),
        };
        seat.add_keyboard(xkb_config, 200, 25).unwrap();
        seat.add_pointer();

        let space = Space::default();

        let socket_name = Self::init_wayland_listener(display, event_loop);
        let loop_signal = event_loop.get_signal();
        let handle = event_loop.handle();

        Self::init_command_channel(event_loop);

        Self {
            start_time,
            display_handle: dh,

            space,
            loop_signal,
            handle,
            socket_name,

            compositor_state,
            xdg_shell_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            primary_selection_state,
            data_control_state,
            ext_data_control_state,
            xdg_decoration_state,
            xdg_activation_state,
            xwayland_shell_state,
            xwm: None,
            xdisplay: None,
            layer_shell_state,
            popups,
            session_lock_state,
            locked: false,
            lock_surfaces: Vec::new(),
            seat,

            windows: Vec::new(),
            next_window_id: 0,
            floating_ids: std::collections::HashSet::new(),
            focused_window: None,
            focus_rect: None,
            message: None,
            message_generation: 0,
            overlays: Vec::new(),
            reported_heads: Vec::new(),
            next_output_id: 0,
            border_color: crate::render::BORDER_COLOR,
            reported_titles: std::collections::HashMap::new(),
            key_repeat_enabled: false,
            key_repeat: None,

            pointer_location: (0.0, 0.0).into(),
            cursor_state: crate::render::CursorState::default(),
            session: None,
            udev_data: None,
            image_capture_source_state,
            output_capture_source_state,
            image_copy_capture_state,
            capture_sessions: Vec::new(),
            pending_captures: Vec::new(),
            gamma_controls: std::collections::HashMap::new(),
            foreign_toplevel,
            output_management,
            pointer_constraints_state,
            relative_pointer_manager_state,
            fractional_scale_manager_state,
            viewporter_state,
            pointer_lock_hint: None,
            idle_notifier_state,
            idle_inhibit_state,
            idle_inhibitors: std::collections::HashSet::new(),
            cursor_shape_manager_state,
            text_input_manager_state,
            input_method_manager_state,
            virtual_keyboard_manager_state,
            keyboard_shortcuts_inhibit_state,
            active_shortcuts_inhibitor: None,
        }
    }

    /// Clamps `pos` to the union of all mapped outputs' geometry, mirroring
    /// anvil's `clamp_coords`. Used by the udev backend's relative pointer
    /// motion (winit only ever gets absolute motion, already in range).
    pub fn clamp_to_outputs(&self, pos: Point<f64, Logical>) -> Point<f64, Logical> {
        clamp_point_to_rectangles(
            pos,
            self.space
                .outputs()
                .filter_map(|output| self.space.output_geometry(output)),
        )
    }

    /// Sets up the calloop channel used to carry `WmCommand`s from Scheme
    /// (running on the main Guile thread, or the REPL's own thread) into the
    /// compositor's event loop, where they're applied against
    /// `&mut MindeState`.
    fn init_command_channel(event_loop: &mut EventLoop<Self>) {
        let (sender, channel) = smithay::reexports::calloop::channel::channel::<WmCommand>();
        guile::set_command_sender(sender);

        event_loop
            .handle()
            .insert_source(channel, |event, _, state| {
                if let ChannelEvent::Msg(cmd) = event {
                    state.apply_wm_command(cmd);
                }
            })
            .expect("Failed to init the wm command channel source.");
    }

    /// Applies a single `WmCommand` enqueued from Scheme.
    fn apply_wm_command(&mut self, cmd: WmCommand) {
        match cmd {
            WmCommand::Place { id, x, y, w, h } => {
                let Some(window) = self.window_by_id(id) else {
                    tracing::warn!(id, "wm-place-window: unknown window id");
                    return;
                };
                if let Some(toplevel) = window.toplevel() {
                    toplevel.with_pending_state(|state| {
                        state.size = Some((w, h).into());
                        // Mark the window tiled on all edges: Firefox-family
                        // clients (zen) only obey exact configure sizes and
                        // drop their CSD shadow margins when tiled.
                        use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState;
                        state.states.set(XdgState::TiledLeft);
                        state.states.set(XdgState::TiledRight);
                        state.states.set(XdgState::TiledTop);
                        state.states.set(XdgState::TiledBottom);
                    });
                    toplevel.send_pending_configure();
                } else if let Some(x11) = window.x11_surface() {
                    tracing::debug!(id, x, y, w, h, "x11 place");
                    let _ = x11.configure(Rectangle::new((x, y).into(), (w, h).into()));
                }
                self.space.map_element(window, (x, y), false);
                self.refresh_foreign_toplevel_outputs();
            }
            WmCommand::PlaceFloat { id, x, y, w, h } => {
                let Some(window) = self.window_by_id(id) else {
                    tracing::warn!(id, "wm-place-float: unknown window id");
                    return;
                };
                // No Tiled* states: a floating window keeps its CSD
                // shadows/rounding; exact-size obedience matters less
                // since nothing tiles around it.
                if let Some(toplevel) = window.toplevel() {
                    toplevel.with_pending_state(|state| {
                        state.size = Some((w, h).into());
                        use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState;
                        state.states.unset(XdgState::TiledLeft);
                        state.states.unset(XdgState::TiledRight);
                        state.states.unset(XdgState::TiledTop);
                        state.states.unset(XdgState::TiledBottom);
                    });
                    toplevel.send_pending_configure();
                } else if let Some(x11) = window.x11_surface() {
                    let _ = x11.configure(Rectangle::new((x, y).into(), (w, h).into()));
                }
                self.space.map_element(window, (x, y), false);
            }
            WmCommand::Raise { id } => {
                let Some(window) = self.window_by_id(id) else {
                    tracing::warn!(id, "wm-raise-window: unknown window id");
                    return;
                };
                self.space.raise_element(&window, false);
            }
            WmCommand::SetFloating { id, on } => {
                if on {
                    self.floating_ids.insert(id);
                } else {
                    self.floating_ids.remove(&id);
                }
            }
            WmCommand::Focus { id } => {
                let Some(window) = self.window_by_id(id) else {
                    tracing::warn!(id, "wm-focus-window: unknown window id");
                    return;
                };
                // Both window kinds expose a wl_surface -- X11 ones only
                // once the xwayland-shell association happened, which can
                // lag the map. Still record/raise/activate in that case;
                // the keyboard focus is applied retroactively in
                // `surface_associated` (handlers/xwayland.rs), or emacs &
                // co. think they're unfocused (hollow cursor) forever.
                if let Some(surface) = window.wl_surface().map(|s| s.into_owned()) {
                    let serial = SERIAL_COUNTER.next_serial();
                    if let Some(keyboard) = self.seat.get_keyboard() {
                        keyboard.set_focus(self, Some(surface), serial);
                    }
                }
                self.space.raise_element(&window, true);
                // Let clients render their focused/unfocused state.
                for (_, w) in &self.windows {
                    w.set_activated(w == &window);
                    if let Some(t) = w.toplevel() {
                        t.send_pending_configure();
                    }
                }
                self.focused_window = Some(window);
                self.foreign_toplevel_focus(Some(id));
            }
            WmCommand::ClearFocus => {
                let serial = SERIAL_COUNTER.next_serial();
                if let Some(keyboard) = self.seat.get_keyboard() {
                    keyboard.set_focus(self, Option::<WlSurface>::None, serial);
                }
                // Clearing keyboard focus doesn't invoke `focus_changed`, so
                // drop text-input focus and deactivate any shortcuts inhibitor
                // explicitly (IME leaves the surface; the inhibitor must not
                // survive focus loss).
                self.set_text_input_focus(None);
                self.update_keyboard_shortcuts_inhibitors(None);
                for (_, w) in &self.windows {
                    w.set_activated(false);
                    if let Some(t) = w.toplevel() {
                        t.send_pending_configure();
                    }
                }
                self.focused_window = None;
                self.foreign_toplevel_focus(None);
            }
            WmCommand::FocusRect { x, y, w, h } => {
                self.focus_rect = Some(Rectangle::new((x, y).into(), (w, h).into()));
            }
            WmCommand::Message { text, timeout_ms } => {
                self.message_generation += 1;
                let generation = self.message_generation;
                let (max_w, max_h) = self
                    .space
                    .outputs()
                    .next()
                    .and_then(|o| self.space.output_geometry(o))
                    .map(|g| (g.size.w, g.size.h))
                    .unwrap_or((1280, 720));
                self.message = Some(crate::render::render_message(
                    &text, generation, max_w, max_h,
                ));
                if timeout_ms > 0 {
                    let timer = smithay::reexports::calloop::timer::Timer::from_duration(
                        std::time::Duration::from_millis(timeout_ms),
                    );
                    let _ = self.handle.insert_source(timer, move |_, _, state| {
                        if state.message.as_ref().map(|m| m.generation) == Some(generation) {
                            state.message = None;
                        }
                        smithay::reexports::calloop::timer::TimeoutAction::Drop
                    });
                }
            }
            WmCommand::ClearMessage => {
                self.message = None;
            }
            WmCommand::AddOverlay { x, y, text } => {
                // Labels are a couple of characters; a small budget keeps
                // render_message's wrap math trivial.
                self.overlays.push((
                    Point::from((x, y)),
                    crate::render::render_message(&text, 0, 400, 200),
                ));
            }
            WmCommand::ClearOverlays => {
                self.overlays.clear();
            }
            WmCommand::BorderColor { rgba } => {
                self.border_color = rgba;
            }
            WmCommand::Close { id } => {
                let Some(window) = self.window_by_id(id) else {
                    tracing::warn!(id, "wm-close-window: unknown window id");
                    return;
                };
                if let Some(toplevel) = window.toplevel() {
                    toplevel.send_close();
                } else if let Some(x11) = window.x11_surface() {
                    let _ = x11.close();
                }
            }
            WmCommand::RunAfter { ms, token } => {
                let timer = smithay::reexports::calloop::timer::Timer::from_duration(
                    std::time::Duration::from_millis(ms),
                );
                let _ = self.handle.insert_source(timer, move |_, _, _| {
                    guile::on_timer(token);
                    smithay::reexports::calloop::timer::TimeoutAction::Drop
                });
            }
            WmCommand::Fullscreen { id, on } => {
                let Some(window) = self.window_by_id(id) else {
                    tracing::warn!(id, "wm-set-fullscreen: unknown window id");
                    return;
                };
                self.foreign_toplevel_fullscreen(id, on);
                use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState;
                // X11 windows: set the fullscreen hint and let the shared
                // full-rect placement below apply through configure.
                if let Some(x11) = window.x11_surface() {
                    let _ = x11.set_fullscreen(on);
                    if on {
                        let geo = self
                            .space
                            .outputs()
                            .next()
                            .and_then(|o| self.space.output_geometry(o))
                            .unwrap_or_else(|| Rectangle::new((0, 0).into(), (1280, 720).into()));
                        let _ = x11.configure(geo);
                        self.space.map_element(window.clone(), geo.loc, false);
                        self.space.raise_element(&window, true);
                    }
                    return;
                }
                let Some(toplevel) = window.toplevel() else {
                    return;
                };
                if on {
                    // Full geometry of the output showing the window (not
                    // the usable area): fullscreen covers reserved bar
                    // space (though Top-layer surfaces still render above;
                    // documented limitation).
                    let window_center = self
                        .space
                        .element_geometry(&window)
                        .map(|g| Point::from((g.loc.x + g.size.w / 2, g.loc.y + g.size.h / 2)));
                    let geo = self
                        .space
                        .outputs()
                        .find(|o| match (window_center, self.space.output_geometry(o)) {
                            (Some(c), Some(g)) => g.contains(c),
                            _ => false,
                        })
                        .or_else(|| self.space.outputs().next())
                        .and_then(|o| self.space.output_geometry(o))
                        .unwrap_or_else(|| Rectangle::new((0, 0).into(), (1280, 720).into()));
                    toplevel.with_pending_state(|state| {
                        state.states.set(XdgState::Fullscreen);
                        state.size = Some(geo.size);
                    });
                    toplevel.send_pending_configure();
                    self.space.map_element(window.clone(), geo.loc, false);
                    self.space.raise_element(&window, true);
                } else {
                    // Scheme re-syncs the frame geometry right after.
                    toplevel.with_pending_state(|state| {
                        state.states.unset(XdgState::Fullscreen);
                    });
                    toplevel.send_pending_configure();
                }
            }
            WmCommand::Kill { id } => {
                let Some(window) = self.window_by_id(id) else {
                    tracing::warn!(id, "wm-kill-window: unknown window id");
                    return;
                };
                if let Some(toplevel) = window.toplevel() {
                    use smithay::reexports::wayland_server::Resource;
                    if let Ok(client) = self.display_handle.get_client(toplevel.wl_surface().id()) {
                        tracing::info!(id, "wm-kill-window: dropping client connection");
                        self.display_handle
                            .backend_handle()
                            .kill_client(client.id(), DisconnectReason::ConnectionClosed);
                    }
                } else if let Some(x11) = window.x11_surface() {
                    // Every X11 app shares the one Xwayland client;
                    // dropping that connection would take down all of
                    // them. Polite close is the best per-window kill.
                    tracing::info!(id, "wm-kill-window: X11 window, closing politely");
                    let _ = x11.close();
                }
            }
            WmCommand::WarpPointer { x, y } => {
                self.warp_pointer((x as f64, y as f64).into());
            }
            WmCommand::WarpPointerRel { dx, dy } => {
                let pos = self.pointer_location + Point::from((dx as f64, dy as f64));
                self.warp_pointer(pos);
            }
            WmCommand::SendString { text } => self.send_string(&text),
            WmCommand::SendKey { mods, keysym } => self.send_key(mods, &keysym),
            WmCommand::SetKeyRepeat { on } => {
                self.key_repeat_enabled = on;
                if !on {
                    self.cancel_key_repeat();
                }
            }
            WmCommand::Click { button } => {
                // 1=left 2=middle 3=right, as StumpWM ratclick counts them.
                const CODES: [u32; 3] = [0x110, 0x112, 0x111]; // BTN_LEFT/MIDDLE/RIGHT
                let code = CODES[(button.clamp(1, 3) - 1) as usize];
                if let Some(pointer) = self.seat.get_pointer() {
                    let time = self.start_time.elapsed().as_millis() as u32;
                    for state in [
                        smithay::backend::input::ButtonState::Pressed,
                        smithay::backend::input::ButtonState::Released,
                    ] {
                        let serial = SERIAL_COUNTER.next_serial();
                        pointer.button(
                            self,
                            &smithay::input::pointer::ButtonEvent {
                                button: code,
                                state,
                                serial,
                                time,
                            },
                        );
                    }
                    pointer.frame(self);
                }
            }
            WmCommand::ReapplyInputConfig => self.reapply_input_config(),
            WmCommand::Spawn { cmd } => guile::spawn_on_main_thread(&cmd),
            WmCommand::Paste => self.request_paste(),
            WmCommand::SetClipboard { text } => {
                smithay::wayland::selection::data_device::set_data_device_selection(
                    &self.display_handle,
                    &self.seat,
                    vec![
                        "text/plain;charset=utf-8".to_string(),
                        "text/plain".to_string(),
                        "UTF8_STRING".to_string(),
                    ],
                    crate::handlers::SelectionOwner::Text(text),
                );
            }
        }
    }

    /// Warps the pointer to a global logical position (clamped to the
    /// outputs) and emits the matching motion event.
    pub(crate) fn warp_pointer(&mut self, pos: Point<f64, Logical>) {
        let pos = self.clamp_to_outputs(pos);
        self.pointer_location = pos;
        let under = self.surface_under(pos);
        if let Some(pointer) = self.seat.get_pointer() {
            let serial = SERIAL_COUNTER.next_serial();
            let time = self.start_time.elapsed().as_millis() as u32;
            pointer.motion(
                self,
                under,
                &smithay::input::pointer::MotionEvent {
                    location: pos,
                    serial,
                    time,
                },
            );
            pointer.frame(self);
        }
    }

    /// Drops the active compositor-side key-repeat timer, if any.
    pub fn cancel_key_repeat(&mut self) {
        if let Some((_, token)) = self.key_repeat.take() {
            self.handle.remove(token);
        }
    }

    /// Synthesizes one key press/release pair, wrapped in the requested
    /// modifiers (Scheme bitmask: shift=1 ctrl=4 alt=8 super=64), into the
    /// focused window (send-raw-key / meta / remapped keys).
    fn send_key(&mut self, mods: u32, keysym_name: &str) {
        use smithay::backend::input::KeyState;
        use smithay::input::keyboard::xkb;

        let target = xkb::keysym_from_name(keysym_name, xkb::KEYSYM_NO_FLAGS);
        if target.raw() == xkb::keysyms::KEY_NoSymbol {
            tracing::warn!(keysym_name, "wm-send-key: unknown keysym name");
            return;
        }
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };

        // One keymap scan: the target keysym's keycode (preferring the
        // unshifted level) plus the keycodes of the wrapping modifiers.
        let (found, shift, ctrl, alt, superk) = keyboard.with_xkb_state(self, |ctx| {
            let guard = ctx.xkb().lock().unwrap();
            // Safety: the refs don't outlive the lock guard.
            let keymap = unsafe { guard.keymap() }.clone();
            let layout = guard.active_layout().0;
            let mut found: Option<(xkb::Keycode, bool)> = None;
            let (mut shift, mut ctrl, mut alt, mut superk) = (None, None, None, None);
            for raw in keymap.min_keycode().raw()..=keymap.max_keycode().raw() {
                let kc = xkb::Keycode::new(raw);
                for level in 0..2u32 {
                    for sym in keymap.key_get_syms_by_level(kc, layout, level) {
                        if level == 0 {
                            match sym.raw() {
                                xkb::keysyms::KEY_Shift_L => shift = shift.or(Some(kc)),
                                xkb::keysyms::KEY_Control_L => ctrl = ctrl.or(Some(kc)),
                                xkb::keysyms::KEY_Alt_L => alt = alt.or(Some(kc)),
                                xkb::keysyms::KEY_Super_L => superk = superk.or(Some(kc)),
                                _ => {}
                            }
                        }
                        if *sym == target && found.is_none_or(|(_, shifted)| shifted && level == 0)
                        {
                            found = Some((kc, level == 1));
                        }
                    }
                }
            }
            (found, shift, ctrl, alt, superk)
        });

        let Some((kc, shifted)) = found else {
            tracing::warn!(keysym_name, "wm-send-key: keysym not in the active layout");
            return;
        };
        let mut mod_keys: Vec<xkb::Keycode> = Vec::new();
        for (bit, key) in [(1u32, shift), (4, ctrl), (8, alt), (64, superk)] {
            if mods & bit != 0 {
                let Some(kc) = key else {
                    tracing::warn!(mods, "wm-send-key: modifier key not found in layout");
                    return;
                };
                mod_keys.push(kc);
            }
        }
        // A shift-level keysym ("A", "colon" on some layouts) needs Shift
        // held even when the caller didn't ask for it.
        if shifted && mods & 1 == 0 {
            let Some(s) = shift else {
                tracing::warn!(keysym_name, "wm-send-key: no Shift key found");
                return;
            };
            mod_keys.push(s);
        }

        let press = |state_self: &mut Self, code: xkb::Keycode, state: KeyState| {
            let serial = SERIAL_COUNTER.next_serial();
            let time = state_self.start_time.elapsed().as_millis() as u32;
            keyboard.input::<(), _>(state_self, code, state, serial, time, |_, _, _| {
                smithay::input::keyboard::FilterResult::Forward
            });
        };
        for &m in &mod_keys {
            press(self, m, KeyState::Pressed);
        }
        press(self, kc, KeyState::Pressed);
        press(self, kc, KeyState::Released);
        for &m in mod_keys.iter().rev() {
            press(self, m, KeyState::Released);
        }
    }

    /// Types TEXT into the focused window (StumpWM window-send-string) by
    /// synthesizing key press/release pairs. Characters are looked up in
    /// the active layout's first two shift levels; anything deeper
    /// (AltGr etc.) is skipped with a log.
    fn send_string(&mut self, text: &str) {
        use smithay::backend::input::KeyState;
        use smithay::input::keyboard::xkb;

        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };

        // One keymap scan builds char -> (keycode, needs-shift) plus the
        // keycode of Shift_L itself.
        let (table, shift_key) = keyboard.with_xkb_state(self, |ctx| {
            let guard = ctx.xkb().lock().unwrap();
            // Safety: the refs don't outlive the lock guard.
            let keymap = unsafe { guard.keymap() }.clone();
            let layout = guard.active_layout().0;
            let mut table: std::collections::HashMap<char, (xkb::Keycode, bool)> =
                std::collections::HashMap::new();
            let mut shift_key = None;
            for raw in keymap.min_keycode().raw()..=keymap.max_keycode().raw() {
                let kc = xkb::Keycode::new(raw);
                for level in 0..2u32 {
                    for sym in keymap.key_get_syms_by_level(kc, layout, level) {
                        if level == 0 && sym.raw() == xkb::keysyms::KEY_Shift_L {
                            shift_key = Some(kc);
                        }
                        let cp = xkb::keysym_to_utf32(*sym);
                        if cp != 0
                            && let Some(ch) = char::from_u32(cp)
                        {
                            // Prefer the unshifted level when both exist.
                            let entry = (kc, level == 1);
                            table.entry(ch).or_insert(entry);
                            if level == 0 {
                                table.insert(ch, entry);
                            }
                        }
                    }
                }
            }
            (table, shift_key)
        });

        for ch in text.chars() {
            let Some(&(kc, shifted)) = table.get(&ch) else {
                tracing::debug!(?ch, "wm-send-string: no key for char in the active layout");
                continue;
            };
            let press = |state_self: &mut Self, code: xkb::Keycode, state: KeyState| {
                let serial = SERIAL_COUNTER.next_serial();
                let time = state_self.start_time.elapsed().as_millis() as u32;
                keyboard.input::<(), _>(state_self, code, state, serial, time, |_, _, _| {
                    smithay::input::keyboard::FilterResult::Forward
                });
            };
            match (shifted, shift_key) {
                (true, Some(shift)) => {
                    press(self, shift, KeyState::Pressed);
                    press(self, kc, KeyState::Pressed);
                    press(self, kc, KeyState::Released);
                    press(self, shift, KeyState::Released);
                }
                (true, None) => {
                    tracing::debug!(?ch, "wm-send-string: no Shift key found; skipping");
                }
                (false, _) => {
                    press(self, kc, KeyState::Pressed);
                    press(self, kc, KeyState::Released);
                }
            }
        }
    }

    /// Reads the current clipboard selection and delivers it to Scheme via
    /// `(handle-paste! text)`. A client-owned selection is piped through a
    /// calloop source (never blocking the loop); a compositor-owned one
    /// short-circuits to its stored text.
    fn request_paste(&mut self) {
        use smithay::reexports::rustix;
        use smithay::wayland::selection::data_device::{
            SelectionRequestError, current_data_device_selection_userdata,
            request_data_device_client_selection,
        };

        let (read_fd, write_fd) = match rustix::pipe::pipe() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(%e, "wm-request-paste: pipe failed");
                return;
            }
        };
        match request_data_device_client_selection(
            &self.seat,
            "text/plain;charset=utf-8".to_string(),
            write_fd,
        ) {
            Ok(()) => self.deliver_pipe_to_scheme(read_fd),
            Err(SelectionRequestError::ServerSideSelection) => {
                // We registered the selection ourselves: either literal
                // text (wm-set-clipboard) or a mirror of an X11 copy.
                let owner = current_data_device_selection_userdata(&self.seat).map(|o| o.clone());
                match owner {
                    Some(crate::handlers::SelectionOwner::Text(text)) => guile::on_paste(&text),
                    Some(crate::handlers::SelectionOwner::X11) => {
                        // The first pipe's write end was consumed by the
                        // failed request; use a fresh one for Xwayland.
                        let forwarded = rustix::pipe::pipe().ok().and_then(|(r, w)| {
                            let ok = self.xwm.as_mut().is_some_and(|xwm| {
                                xwm.send_selection(
                                    smithay::wayland::selection::SelectionTarget::Clipboard,
                                    "text/plain;charset=utf-8".to_string(),
                                    w,
                                )
                                .is_ok()
                            });
                            ok.then_some(r)
                        });
                        match forwarded {
                            Some(r) => self.deliver_pipe_to_scheme(r),
                            None => guile::on_paste(""),
                        }
                    }
                    None => guile::on_paste(""),
                }
            }
            Err(e) => {
                tracing::info!(%e, "wm-request-paste: no usable selection");
                guile::on_paste("");
            }
        }
    }

    /// Streams READ_FD through a calloop source and delivers the collected
    /// text to Scheme via `(handle-paste! text)` once the writer closes.
    fn deliver_pipe_to_scheme(&mut self, read_fd: std::os::fd::OwnedFd) {
        use smithay::reexports::rustix;
        let mut acc: Vec<u8> = Vec::new();
        let source = Generic::new(read_fd, Interest::READ, Mode::Level);
        let _ = self.handle.insert_source(source, move |_, fd, _| {
            let mut buf = [0u8; 4096];
            match rustix::io::read(&**fd, &mut buf) {
                Ok(n) if n > 0 => {
                    // Cap pastes at 64 KiB; the prompt is one line.
                    if acc.len() < 64 * 1024 {
                        acc.extend_from_slice(&buf[..n]);
                    }
                    Ok(PostAction::Continue)
                }
                _ => {
                    let text = String::from_utf8_lossy(&acc).into_owned();
                    guile::on_paste(&text);
                    Ok(PostAction::Remove)
                }
            }
        });
    }

    fn window_by_id(&self, id: u64) -> Option<Window> {
        self.windows
            .iter()
            .find(|(wid, _)| *wid == id)
            .map(|(_, w)| w.clone())
    }

    /// The registered id of WINDOW, if any (reverse of `window_by_id`).
    pub fn id_for_window(&self, window: &Window) -> Option<u64> {
        self.windows
            .iter()
            .find(|(_, w)| w == window)
            .map(|(id, _)| *id)
    }

    /// Registers a newly-mapped toplevel window and returns its assigned id.
    pub fn register_window(&mut self, window: Window) -> u64 {
        let id = self.next_window_id;
        self.next_window_id += 1;
        // Any title/app-id already known at map time (empty for a Wayland
        // toplevel that configures first, the class/title for an X11 window).
        let (title, app_id) = self.window_title_app_id(&window);
        self.windows.push((id, window));
        self.foreign_toplevel_created(id, title, app_id);
        id
    }

    /// Best-effort current title/app-id for a window, from whichever shell
    /// backs it. Used to seed the foreign-toplevel handle at map time.
    fn window_title_app_id(&self, window: &Window) -> (String, String) {
        if let Some(toplevel) = window.toplevel() {
            use smithay::wayland::compositor::with_states;
            use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;
            return with_states(toplevel.wl_surface(), |states| {
                states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .and_then(|d| d.lock().ok())
                    .map(|d| {
                        (
                            d.title.clone().unwrap_or_default(),
                            d.app_id.clone().unwrap_or_default(),
                        )
                    })
                    .unwrap_or_default()
            });
        }
        if let Some(x11) = window.x11_surface() {
            return (x11.title(), x11.class());
        }
        (String::new(), String::new())
    }

    /// Removes a window (by Wayland surface identity) from the registry,
    /// returning its id if it was registered. Called on unmap/destroy.
    pub fn unregister_window(&mut self, window: &Window) -> Option<u64> {
        if self.focused_window.as_ref() == Some(window) {
            self.focused_window = None;
        }
        let pos = self.windows.iter().position(|(_, w)| w == window)?;
        let id = self.windows.remove(pos).0;
        self.floating_ids.remove(&id);
        self.reported_titles.remove(&id);
        self.foreign_toplevel_closed(id);
        Some(id)
    }

    /// Reports a toplevel's title/app-id to Scheme when either changed
    /// since the last report (see `reported_titles`). Called from the
    /// commit handler; X11 windows are skipped (their class arrives with
    /// the map request and doesn't change).
    pub fn report_title_if_changed(&mut self, window: &Window) {
        use smithay::wayland::compositor::with_states;
        use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;
        let Some(toplevel) = window.toplevel() else {
            return;
        };
        let Some(id) = self.id_for_window(window) else {
            return;
        };
        let current = with_states(toplevel.wl_surface(), |states| {
            states.data_map.get::<XdgToplevelSurfaceData>().map(|d| {
                let d = d.lock().unwrap();
                (
                    d.title.clone().unwrap_or_default(),
                    d.app_id.clone().unwrap_or_default(),
                )
            })
        });
        let Some(current) = current else {
            return;
        };
        if self.reported_titles.get(&id) != Some(&current) {
            guile::on_window_title(id, &current.0, &current.1);
            self.foreign_toplevel_title(id, &current.0, &current.1);
            self.reported_titles.insert(id, current);
        }
    }

    fn init_wayland_listener(display: Display<Self>, event_loop: &mut EventLoop<Self>) -> OsString {
        let listening_socket = ListeningSocketSource::new_auto().unwrap();
        let socket_name = listening_socket.socket_name().to_os_string();
        let loop_handle = event_loop.handle();

        loop_handle
            .insert_source(listening_socket, move |client_stream, _, state| {
                if let Err(error) = state
                    .display_handle
                    .insert_client(client_stream, Arc::new(ClientState::default()))
                {
                    tracing::warn!(%error, "failed to register Wayland client");
                }
            })
            .expect("Failed to init the wayland event source.");

        loop_handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, state| {
                    // Safety: we don't drop the display
                    unsafe {
                        if let Err(error) = display.get_mut().dispatch_clients(state) {
                            tracing::warn!(%error, "failed to dispatch Wayland clients");
                        }
                    }
                    Ok(PostAction::Continue)
                },
            )
            .unwrap();

        socket_name
    }

    /// Spawns the embedded Xwayland server (anvil's `start_xwayland`,
    /// trimmed): once it reports Ready, an X11 window manager connection
    /// is attached (see `XwmHandler` in handlers/xwayland.rs) and
    /// DISPLAY is exported for children. Failure to start is logged and
    /// the compositor runs on without X11 support.
    pub fn start_xwayland(&mut self) {
        use smithay::xwayland::{X11Wm, XWayland, XWaylandEvent};

        if std::env::var_os("MINDE_NO_XWAYLAND").is_some() {
            guile::set_xwayland_status("disabled", None);
            tracing::info!("MINDE_NO_XWAYLAND set; skipping Xwayland");
            guile::publish_status();
            return;
        }

        guile::set_xwayland_status("starting", None);
        guile::publish_status();

        let spawn_result = XWayland::spawn(
            &self.display_handle,
            None,
            std::iter::empty::<(String, String)>(),
            std::iter::empty::<String>(),
            true,
            std::process::Stdio::null(),
            std::process::Stdio::null(),
            |_| (),
        );
        let (xwayland, client) = match spawn_result {
            Ok(x) => x,
            Err(err) => {
                guile::set_xwayland_status("failed", None);
                tracing::warn!(%err, "failed to start Xwayland; X11 apps unavailable");
                guile::publish_status();
                return;
            }
        };

        let display_handle = self.display_handle.clone();
        let ret = self
            .handle
            .insert_source(xwayland, move |event, _, state| match event {
                XWaylandEvent::Ready {
                    x11_socket,
                    display_number,
                } => {
                    match X11Wm::start_wm(
                        state.handle.clone(),
                        &display_handle,
                        x11_socket,
                        client.clone(),
                    ) {
                        Ok(mut wm) => {
                            tracing::info!(display_number, "xwayland ready");
                            // Default X11 root cursor: without this, hovering an
                            // X11 window that sets no cursor of its own shows a
                            // hollow box (anvil sets one too). Reuse the embedded
                            // fallback arrow (64x64 RGBA, hotspot at the tip).
                            if let Err(err) = wm.set_cursor(
                                crate::render::FALLBACK_CURSOR_RGBA,
                                smithay::utils::Size::from((64u16, 64u16)),
                                smithay::utils::Point::from((0u16, 0u16)),
                            ) {
                                tracing::warn!(%err, "failed to set the Xwayland default cursor");
                            }
                            state.xwm = Some(wm);
                            state.xdisplay = Some(display_number);
                            guile::set_xwayland_status("ready", Some(display_number));
                            // Children get DISPLAY via wm-spawn (X11_DISPLAY).
                            // NEVER set it process-wide: in nested (winit)
                            // mode the compositor is itself an X client, and
                            // mesa/EGL lazily open X connections from
                            // $DISPLAY -- pointing that at our own Xwayland
                            // deadlocks eglSwapBuffers against ourselves
                            // (same class as the WAYLAND_DISPLAY/winit
                            // startup deadlock).
                            let _ = guile::X11_DISPLAY.set(format!(":{display_number}"));
                            guile::publish_status();
                        }
                        Err(err) => {
                            guile::set_xwayland_status("failed", None);
                            tracing::warn!(%err, "failed to attach the X11 window manager");
                            guile::publish_status();
                        }
                    }
                }
                XWaylandEvent::Error => {
                    guile::set_xwayland_status("failed", None);
                    tracing::warn!("Xwayland crashed on startup; X11 apps unavailable");
                    guile::publish_status();
                }
            });
        if let Err(err) = ret {
            guile::set_xwayland_status("failed", None);
            tracing::warn!(%err, "failed to insert the Xwayland event source");
            guile::publish_status();
        }
    }

    /// The stable id of OUTPUT, assigning the next free one on first use.
    pub fn output_id(&mut self, output: &smithay::output::Output) -> u64 {
        if let Some(id) = output.user_data().get::<OutputId>() {
            return id.0;
        }
        let id = self.next_output_id;
        self.next_output_id += 1;
        output.user_data().insert_if_missing(|| OutputId(id));
        id
    }

    /// Recomputes every output's usable area (geometry minus layer-shell
    /// exclusive zones) and tells Scheme when anything changed, so the
    /// frame trees shrink around docked panels (eww bars etc.). Call
    /// after any layer surface maps, unmaps, or commits, and on output
    /// init/resize/hotplug.
    pub fn update_usable_area(&mut self) {
        let outputs: Vec<_> = self.space.outputs().cloned().collect();
        let mut heads = Vec::new();
        for output in outputs {
            let Some(geo) = self.space.output_geometry(&output) else {
                continue;
            };
            // The non-exclusive zone is in output-local coordinates;
            // heads are reported in global ones.
            let zone = {
                let mut map = smithay::desktop::layer_map_for_output(&output);
                map.arrange();
                map.non_exclusive_zone()
            };
            heads.push(guile::HeadInfo {
                id: self.output_id(&output),
                x: geo.loc.x + zone.loc.x,
                y: geo.loc.y + zone.loc.y,
                w: zone.size.w.max(0) as u32,
                h: zone.size.h.max(0) as u32,
                name: output.name(),
            });
        }
        if heads.is_empty() || heads == self.reported_heads {
            return;
        }
        self.reported_heads = heads.clone();
        guile::on_heads_changed(heads);
        // Window/output association may have shifted with the geometry.
        self.refresh_foreign_toplevel_outputs();
        // Re-advertise the layout to wlr-output-management clients (kanshi,
        // wlr-randr) so an external resize/hotplug reconciles back to them.
        self.output_management_refresh();
    }

    /// Preferred fractional scale for a surface: the scale of the output it
    /// is mapped onto, falling back to the first output (or 1.0 with none).
    /// Used to seed a freshly-created `wp_fractional_scale` object.
    pub fn output_scale_for_surface(&self, surface: &WlSurface) -> f64 {
        for window in self.space.elements() {
            let matches = window.wl_surface().map(|s| &*s == surface).unwrap_or(false);
            if matches && let Some(output) = self.space.outputs_for_element(window).first() {
                return output.current_scale().fractional_scale();
            }
        }
        self.space
            .outputs()
            .next()
            .map(|o| o.current_scale().fractional_scale())
            .unwrap_or(1.0)
    }

    /// Pushes each mapped surface's preferred fractional scale to match the
    /// output it sits on. Call after any output-scale change (wlr-randr
    /// `--scale`, hotplug) so `wp_fractional_scale` clients repaint at the
    /// new density. A no-op for clients that never bound the protocol.
    pub fn update_fractional_scales(&mut self) {
        let default_scale = self
            .space
            .outputs()
            .next()
            .map(|o| o.current_scale().fractional_scale())
            .unwrap_or(1.0);
        let mut targets: Vec<(WlSurface, f64)> = Vec::new();
        for window in self.space.elements() {
            let scale = self
                .space
                .outputs_for_element(window)
                .first()
                .map(|o| o.current_scale().fractional_scale())
                .unwrap_or(default_scale);
            if let Some(surface) = window.wl_surface() {
                targets.push((surface.into_owned(), scale));
            }
        }
        for output in self.space.outputs() {
            let scale = output.current_scale().fractional_scale();
            for layer in smithay::desktop::layer_map_for_output(output).layers() {
                targets.push((layer.wl_surface().clone(), scale));
            }
        }
        for (surface, scale) in targets {
            send_surface_preferred_scale(&surface, scale);
        }
    }

    pub fn surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        use smithay::wayland::shell::wlr_layer::Layer;

        // The output under the pointer owns the layer surfaces there;
        // fall back to the first output (e.g. pointer parked on a dead
        // zone between differently-sized heads).
        let output = self
            .space
            .outputs()
            .find(|o| {
                self.space
                    .output_geometry(o)
                    .map(|g| g.to_f64().contains(pos))
                    .unwrap_or(false)
            })
            .or_else(|| self.space.outputs().next())?;
        let output_geo = self.space.output_geometry(output)?;
        let layers = smithay::desktop::layer_map_for_output(output);
        let local = pos - output_geo.loc.to_f64();

        let layer_surface_under = |layer_types: &[Layer]| {
            layer_types.iter().find_map(|lt| {
                layers.layer_under(*lt, local).and_then(|layer| {
                    let layer_loc = layers.layer_geometry(layer)?.loc;
                    layer
                        .surface_under(local - layer_loc.to_f64(), WindowSurfaceType::ALL)
                        .map(|(s, p)| (s, (p + layer_loc + output_geo.loc).to_f64()))
                })
            })
        };

        layer_surface_under(&[Layer::Overlay, Layer::Top])
            .or_else(|| {
                self.space
                    .element_under(pos)
                    .and_then(|(window, location)| {
                        window
                            .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                            .map(|(s, p)| (s, (p + location).to_f64()))
                    })
            })
            .or_else(|| layer_surface_under(&[Layer::Bottom, Layer::Background]))
    }
}

/// Sets the preferred fractional scale on a surface and its whole subsurface
/// tree. Smithay sends the `preferred_scale` event only when the value
/// actually changes, so this is cheap to call redundantly.
fn send_surface_preferred_scale(surface: &WlSurface, scale: f64) {
    use smithay::wayland::compositor::{TraversalAction, with_surface_tree_downward};
    use smithay::wayland::fractional_scale::with_fractional_scale;
    with_surface_tree_downward(
        surface,
        (),
        |_, _, _| TraversalAction::DoChildren(()),
        |_, states, _| {
            with_fractional_scale(states, |fractional| {
                fractional.set_preferred_scale(scale);
            });
        },
        |_, _, _| true,
    );
}

/// Stable per-output id handed to Scheme, stored in the `Output`'s user
/// data (survives as long as the output does; a re-plugged monitor gets
/// a fresh id).
struct OutputId(u64);

/// Data associated with a wayland client that connects to us.
/// One instance of this type per client.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle(x: i32, y: i32, width: i32, height: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (width, height).into())
    }

    #[test]
    fn output_clamp_handles_gaps_vertical_layouts_and_negative_origins() {
        let outputs = [rectangle(-800, 0, 800, 600), rectangle(200, 700, 1200, 900)];

        assert_eq!(
            clamp_point_to_rectangles((-400.0, 300.0).into(), outputs),
            (-400.0, 300.0).into()
        );
        assert_eq!(
            clamp_point_to_rectangles((100.0, 300.0).into(), outputs),
            (0.0, 300.0).into()
        );
        assert_eq!(
            clamp_point_to_rectangles((500.0, 650.0).into(), outputs),
            (500.0, 700.0).into()
        );
        assert_eq!(
            clamp_point_to_rectangles((1800.0, 1800.0).into(), outputs),
            (1400.0, 1600.0).into()
        );
    }

    #[test]
    fn output_clamp_without_outputs_preserves_the_point() {
        assert_eq!(
            clamp_point_to_rectangles((12.5, -7.0).into(), []),
            (12.5, -7.0).into()
        );
    }
}
