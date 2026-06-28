use smithay::wayland::seat::WaylandFocus;
use std::{ffi::OsString, sync::Arc};

use smithay::{
    desktop::{PopupManager, Space, Window, WindowSurfaceType},
    input::{Seat, SeatState},
    reexports::{
        calloop::{
            EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction, channel::Event as ChannelEvent,
            generic::Generic,
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
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::xdg::XdgShellState,
        shm::ShmState,
        socket::ListeningSocketSource,
    },
};

use crate::guile::{self, WmCommand};

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
    pub xdg_decoration_state: smithay::wayland::shell::xdg::decoration::XdgDecorationState,
    pub xdg_activation_state: smithay::wayland::xdg_activation::XdgActivationState,
    pub xwayland_shell_state: smithay::wayland::xwayland_shell::XWaylandShellState,
    /// The X11 window manager connection, once Xwayland is up.
    pub xwm: Option<smithay::xwayland::X11Wm>,
    /// The X display number (":N") Xwayland serves, once ready.
    pub xdisplay: Option<u32>,
    pub layer_shell_state: smithay::wayland::shell::wlr_layer::WlrLayerShellState,
    pub popups: PopupManager,

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
    /// Last head list (usable rects) sent to Scheme, to avoid
    /// re-announcing unchanged geometry on every commit.
    pub reported_heads: Vec<guile::HeadInfo>,
    /// Monotonic source of stable per-output ids (stored in each
    /// `Output`'s user data as `OutputId`).
    pub next_output_id: u64,
    /// Focus border color; Scheme flips it while the prefix key is armed
    /// (StumpWM's pointer-box equivalent).
    pub border_color: [f32; 4],

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
        let xdg_decoration_state =
            smithay::wayland::shell::xdg::decoration::XdgDecorationState::new::<Self>(&dh);
        let xdg_activation_state =
            smithay::wayland::xdg_activation::XdgActivationState::new::<Self>(&dh);
        let xwayland_shell_state =
            smithay::wayland::xwayland_shell::XWaylandShellState::new::<Self>(&dh);
        let layer_shell_state =
            smithay::wayland::shell::wlr_layer::WlrLayerShellState::new::<Self>(&dh);

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
            xdg_decoration_state,
            xdg_activation_state,
            xwayland_shell_state,
            xwm: None,
            xdisplay: None,
            layer_shell_state,
            popups,
            seat,

            windows: Vec::new(),
            next_window_id: 0,
            floating_ids: std::collections::HashSet::new(),
            focused_window: None,
            focus_rect: None,
            message: None,
            message_generation: 0,
            reported_heads: Vec::new(),
            next_output_id: 0,
            border_color: crate::render::BORDER_COLOR,

            pointer_location: (0.0, 0.0).into(),
            cursor_state: crate::render::CursorState::default(),
            session: None,
            udev_data: None,
        }
    }

    /// Clamps `pos` to the union of all mapped outputs' geometry, mirroring
    /// anvil's `clamp_coords`. Used by the udev backend's relative pointer
    /// motion (winit only ever gets absolute motion, already in range).
    pub fn clamp_to_outputs(&self, pos: Point<f64, Logical>) -> Point<f64, Logical> {
        if self.space.outputs().next().is_none() {
            return pos;
        }

        let (pos_x, pos_y) = pos.into();
        let max_x = self
            .space
            .outputs()
            .fold(0, |acc, o| acc + self.space.output_geometry(o).unwrap().size.w);
        let clamped_x = pos_x.clamp(0.0, max_x as f64);
        let max_y = self
            .space
            .outputs()
            .find(|o| {
                let geo = self.space.output_geometry(o).unwrap();
                geo.contains((clamped_x as i32, 0))
            })
            .map(|o| self.space.output_geometry(o).unwrap().size.h);

        if let Some(max_y) = max_y {
            (clamped_x, pos_y.clamp(0.0, max_y as f64)).into()
        } else {
            (clamped_x, pos_y).into()
        }
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
            }
            WmCommand::ClearFocus => {
                let serial = SERIAL_COUNTER.next_serial();
                if let Some(keyboard) = self.seat.get_keyboard() {
                    keyboard.set_focus(self, Option::<WlSurface>::None, serial);
                }
                for (_, w) in &self.windows {
                    w.set_activated(false);
                    if let Some(t) = w.toplevel() {
                        t.send_pending_configure();
                    }
                }
                self.focused_window = None;
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
                self.message = Some(crate::render::render_message(&text, generation, max_w, max_h));
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
                        .find(|o| {
                            match (window_center, self.space.output_geometry(o)) {
                                (Some(c), Some(g)) => g.contains(c),
                                _ => false,
                            }
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
                let pos = self.clamp_to_outputs((x as f64, y as f64).into());
                self.pointer_location = pos;
                let under = self.surface_under(pos);
                if let Some(pointer) = self.seat.get_pointer() {
                    let serial = SERIAL_COUNTER.next_serial();
                    let time = self.start_time.elapsed().as_millis() as u32;
                    pointer.motion(
                        self,
                        under,
                        &smithay::input::pointer::MotionEvent { location: pos, serial, time },
                    );
                    pointer.frame(self);
                }
            }
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
                    text,
                );
            }
        }
    }

    /// Reads the current clipboard selection and delivers it to Scheme via
    /// `(wm-on-paste text)`. A client-owned selection is piped through a
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
            Ok(()) => {
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
            Err(SelectionRequestError::ServerSideSelection) => {
                // We own the clipboard; its text is the selection user data.
                let text =
                    current_data_device_selection_userdata(&self.seat).map(|t| t.clone());
                if let Some(text) = text {
                    guile::on_paste(&text);
                }
            }
            Err(e) => {
                tracing::info!(%e, "wm-request-paste: no usable selection");
                guile::on_paste("");
            }
        }
    }

    fn window_by_id(&self, id: u64) -> Option<Window> {
        self.windows.iter().find(|(wid, _)| *wid == id).map(|(_, w)| w.clone())
    }

    /// The registered id of WINDOW, if any (reverse of `window_by_id`).
    pub fn id_for_window(&self, window: &Window) -> Option<u64> {
        self.windows.iter().find(|(_, w)| w == window).map(|(id, _)| *id)
    }

    /// Registers a newly-mapped toplevel window and returns its assigned id.
    pub fn register_window(&mut self, window: Window) -> u64 {
        let id = self.next_window_id;
        self.next_window_id += 1;
        self.windows.push((id, window));
        id
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
        Some(id)
    }

    fn init_wayland_listener(display: Display<Self>, event_loop: &mut EventLoop<Self>) -> OsString {
        let listening_socket = ListeningSocketSource::new_auto().unwrap();
        let socket_name = listening_socket.socket_name().to_os_string();
        let loop_handle = event_loop.handle();

        loop_handle
            .insert_source(listening_socket, move |client_stream, _, state| {
                state
                    .display_handle
                    .insert_client(client_stream, Arc::new(ClientState::default()))
                    .unwrap();
            })
            .expect("Failed to init the wayland event source.");

        loop_handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, state| {
                    // Safety: we don't drop the display
                    unsafe {
                        display.get_mut().dispatch_clients(state).unwrap();
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
            tracing::info!("MINDE_NO_XWAYLAND set; skipping Xwayland");
            return;
        }

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
                tracing::warn!(%err, "failed to start Xwayland; X11 apps unavailable");
                return;
            }
        };

        let display_handle = self.display_handle.clone();
        let ret = self.handle.insert_source(xwayland, move |event, _, state| match event {
            XWaylandEvent::Ready { x11_socket, display_number } => {
                match X11Wm::start_wm(state.handle.clone(), &display_handle, x11_socket, client.clone()) {
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
                        // Children get DISPLAY via wm-spawn (X11_DISPLAY).
                        // NEVER set it process-wide: in nested (winit)
                        // mode the compositor is itself an X client, and
                        // mesa/EGL lazily open X connections from
                        // $DISPLAY -- pointing that at our own Xwayland
                        // deadlocks eglSwapBuffers against ourselves
                        // (same class as the WAYLAND_DISPLAY/winit
                        // startup deadlock).
                        let _ = guile::X11_DISPLAY.set(format!(":{display_number}"));
                    }
                    Err(err) => {
                        tracing::warn!(%err, "failed to attach the X11 window manager");
                    }
                }
            }
            XWaylandEvent::Error => {
                tracing::warn!("Xwayland crashed on startup; X11 apps unavailable");
            }
        });
        if let Err(err) = ret {
            tracing::warn!(%err, "failed to insert the Xwayland event source");
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
    }

    pub fn surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
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
        let output_geo = self.space.output_geometry(output).unwrap();
        let layers = smithay::desktop::layer_map_for_output(output);
        let local = pos - output_geo.loc.to_f64();

        let layer_surface_under = |layer_types: &[Layer]| {
            layer_types.iter().find_map(|lt| {
                layers.layer_under(*lt, local).and_then(|layer| {
                    let layer_loc = layers.layer_geometry(layer).unwrap().loc;
                    layer
                        .surface_under(local - layer_loc.to_f64(), WindowSurfaceType::ALL)
                        .map(|(s, p)| (s, (p + layer_loc + output_geo.loc).to_f64()))
                })
            })
        };

        layer_surface_under(&[Layer::Overlay, Layer::Top])
            .or_else(|| {
                self.space.element_under(pos).and_then(|(window, location)| {
                    window
                        .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                        .map(|(s, p)| (s, (p + location).to_f64()))
                })
            })
            .or_else(|| layer_surface_under(&[Layer::Bottom, Layer::Background]))
    }
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
