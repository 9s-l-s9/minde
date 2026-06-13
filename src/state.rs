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
    pub layer_shell_state: smithay::wayland::shell::wlr_layer::WlrLayerShellState,
    pub popups: PopupManager,

    pub seat: Seat<Self>,

    /// Registry of mapped toplevels by their stable id, assigned in
    /// `handlers::xdg_shell::new_toplevel`. Scheme addresses windows only by
    /// this id (see `wm-place-window` &c. in `guile::mod`).
    pub windows: Vec<(u64, Window)>,
    pub next_window_id: u64,
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
    /// Last usable area (output minus layer-shell exclusive zones) sent to
    /// Scheme, to avoid re-announcing an unchanged rect on every commit.
    pub usable_area: Option<Rectangle<i32, Logical>>,

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
            layer_shell_state,
            popups,
            seat,

            windows: Vec::new(),
            next_window_id: 0,
            focused_window: None,
            focus_rect: None,
            message: None,
            message_generation: 0,
            usable_area: None,

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
                }
                self.space.map_element(window, (x, y), false);
            }
            WmCommand::Focus { id } => {
                let Some(window) = self.window_by_id(id) else {
                    tracing::warn!(id, "wm-focus-window: unknown window id");
                    return;
                };
                let Some(toplevel) = window.toplevel() else {
                    return;
                };
                let serial = SERIAL_COUNTER.next_serial();
                let surface = toplevel.wl_surface().clone();
                if let Some(keyboard) = self.seat.get_keyboard() {
                    keyboard.set_focus(self, Some(surface), serial);
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
            WmCommand::Close { id } => {
                let Some(window) = self.window_by_id(id) else {
                    tracing::warn!(id, "wm-close-window: unknown window id");
                    return;
                };
                if let Some(toplevel) = window.toplevel() {
                    toplevel.send_close();
                }
            }
        }
    }

    fn window_by_id(&self, id: u64) -> Option<Window> {
        self.windows.iter().find(|(wid, _)| *wid == id).map(|(_, w)| w.clone())
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
        Some(self.windows.remove(pos).0)
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

    /// Recomputes the usable area (output geometry minus layer-shell
    /// exclusive zones) and tells Scheme when it changed, so the frame
    /// tree shrinks around docked panels (eww bars etc.). Call after any
    /// layer surface maps, unmaps, or commits, and on output init/resize.
    pub fn update_usable_area(&mut self) {
        let Some(output) = self.space.outputs().next().cloned() else {
            return;
        };
        let zone = {
            let mut map = smithay::desktop::layer_map_for_output(&output);
            map.arrange();
            map.non_exclusive_zone()
        };
        if self.usable_area != Some(zone) {
            self.usable_area = Some(zone);
            guile::on_output_geometry(
                zone.loc.x,
                zone.loc.y,
                zone.size.w.max(0) as u32,
                zone.size.h.max(0) as u32,
            );
        }
    }

    pub fn surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        use smithay::wayland::shell::wlr_layer::Layer;

        let output = self.space.outputs().next()?;
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
