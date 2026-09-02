// SPDX-License-Identifier: MIT

use smithay::{
    desktop::{
        PopupKeyboardGrab, PopupKind, PopupManager, PopupPointerGrab, PopupUngrabStrategy, Space,
        Window, find_popup_root_surface, get_popup_toplevel_coords,
    },
    input::{
        Seat,
        pointer::{Focus, GrabStartData as PointerGrabStartData},
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            Resource,
            protocol::{wl_seat, wl_surface::WlSurface},
        },
    },
    utils::{Rectangle, Serial},
    wayland::{
        compositor::with_states,
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            XdgToplevelSurfaceData,
        },
    },
};

use crate::{
    MindeState,
    grabs::{MoveSurfaceGrab, ResizeSurfaceGrab},
    guile,
};

impl XdgShellHandler for MindeState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let window = Window::new_wayland_window(surface);
        let Some(toplevel) = window.toplevel() else {
            tracing::warn!("new xdg toplevel had no toplevel surface");
            return;
        };
        let wl_surface = toplevel.wl_surface().clone();
        self.space.map_element(window.clone(), (0, 0), false);

        let id = self.register_window(window);
        self.schedule_redraw();

        let (title, app_id) = with_states(&wl_surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .and_then(|data| data.lock().ok())
                .map(|data| {
                    (
                        data.title.clone().unwrap_or_default(),
                        data.app_id.clone().unwrap_or_default(),
                    )
                })
                .unwrap_or_default()
        });

        guile::on_window_map(id, &title, &app_id);
        tracing::info!(
            component = "window",
            protocol = "wayland",
            window_id = id,
            "managed toplevel mapped"
        );
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        // A dying client sends no further commits, so the render pass must
        // be asked for explicitly or its last frame lingers on screen.
        self.schedule_redraw();
        let window = crate::state::window_for_surface(&self.space, surface.wl_surface());

        if let Some(window) = window {
            self.space.unmap_elem(&window);
            if let Some(id) = self.unregister_window(&window) {
                guile::on_window_unmap(id);
                tracing::info!(
                    component = "window",
                    protocol = "wayland",
                    window_id = id,
                    "managed toplevel unmapped"
                );
            }
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        let _ = self.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            let geometry = positioner.get_geometry();
            state.geometry = geometry;
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let Some(seat) = Seat::from_resource(&seat) else {
            tracing::warn!("move request used an unknown seat");
            return;
        };

        let wl_surface = surface.wl_surface();

        if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
            let Some(pointer) = seat.get_pointer() else {
                return;
            };

            let Some(window) = crate::state::window_for_surface(&self.space, wl_surface) else {
                return;
            };
            let Some(initial_window_location) = self.space.element_location(&window) else {
                return;
            };

            let grab = MoveSurfaceGrab {
                start_data,
                window,
                initial_window_location,
            };

            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        let Some(seat) = Seat::from_resource(&seat) else {
            tracing::warn!("resize request used an unknown seat");
            return;
        };

        let wl_surface = surface.wl_surface();

        if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
            let Some(pointer) = seat.get_pointer() else {
                return;
            };

            let Some(window) = crate::state::window_for_surface(&self.space, wl_surface) else {
                return;
            };
            let Some(initial_window_location) = self.space.element_location(&window) else {
                return;
            };
            let initial_window_size = window.geometry().size;

            surface.with_pending_state(|state| {
                state.states.set(xdg_toplevel::State::Resizing);
            });

            surface.send_pending_configure();

            let grab = ResizeSurfaceGrab::start(
                start_data,
                window,
                edges.into(),
                Rectangle::new(initial_window_location, initial_window_size),
            );

            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let Some(seat) = Seat::<MindeState>::from_resource(&seat) else {
            tracing::warn!("popup grab used an unknown seat");
            return;
        };
        let kind = PopupKind::Xdg(surface.clone());
        let Ok(root) = find_popup_root_surface(&kind) else {
            return;
        };
        match self.popups.grab_popup(root, kind, &seat, serial) {
            Ok(mut grab) => {
                if let Some(keyboard) = seat.get_keyboard() {
                    if keyboard.is_grabbed()
                        && !(keyboard.has_grab(serial)
                            || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
                    {
                        grab.ungrab(PopupUngrabStrategy::All);
                        return;
                    }
                    keyboard.set_focus(self, grab.current_grab(), serial);
                    keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
                }
                if let Some(pointer) = seat.get_pointer() {
                    if pointer.is_grabbed()
                        && !(pointer.has_grab(serial)
                            || pointer
                                .has_grab(grab.previous_serial().unwrap_or_else(|| grab.serial())))
                    {
                        grab.ungrab(PopupUngrabStrategy::All);
                        return;
                    }
                    pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
                }
            }
            Err(error) => {
                // A denied grab must dismiss the popup (xdg-shell spec);
                // leaving it hanging strands Firefox-family clients in
                // menu-open state, eating all keyboard input.
                tracing::debug!(%error, "popup grab denied, dismissing popup");
                surface.send_popup_done();
            }
        }
    }

    // The frame tree owns all geometry, so (un)maximize requests are
    // never granted. The protocol still demands a configure in reply
    // either way -- Firefox-family clients (zen restoring a maximized
    // session calls set_maximized right at startup) otherwise stop
    // obeying our sizes entirely. The pending state already carries the
    // frame geometry from wm-place-window, so replying re-asserts it.
    fn maximize_request(&mut self, surface: ToplevelSurface) {
        if surface.is_initial_configure_sent() {
            surface.send_configure();
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        if surface.is_initial_configure_sent() {
            surface.send_configure();
        }
    }

    // Client (un)fullscreen requests route through the same Scheme path
    // as taskbar requests (handle-foreign-fullscreen!), so the single-
    // fullscreen model stays authoritative and the frame layout re-syncs
    // on exit. A surface not yet registered (request before map) just
    // gets the configure reply the protocol demands.
    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        _output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
    ) {
        match self.id_for_toplevel(&surface) {
            Some(id) => guile::on_foreign_fullscreen(id, true),
            None if surface.is_initial_configure_sent() => {
                surface.send_configure();
            }
            None => {}
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        match self.id_for_toplevel(&surface) {
            Some(id) => guile::on_foreign_fullscreen(id, false),
            None if surface.is_initial_configure_sent() => {
                surface.send_configure();
            }
            None => {}
        }
    }
}

fn check_grab(
    seat: &Seat<MindeState>,
    surface: &WlSurface,
    serial: Serial,
) -> Option<PointerGrabStartData<MindeState>> {
    let pointer = seat.get_pointer()?;

    // Check that this surface has a click grab.
    if !pointer.has_grab(serial) {
        return None;
    }

    let start_data = pointer.grab_start_data()?;

    let (focus, _) = start_data.focus.as_ref()?;
    // If the focus was for a different surface, ignore the request.
    if !focus.id().same_client_as(&surface.id()) {
        return None;
    }

    Some(start_data)
}

/// Should be called on `WlSurface::commit`
pub fn handle_commit(popups: &mut PopupManager, space: &Space<Window>, surface: &WlSurface) {
    // Handle toplevel commits.
    if let Some(window) = crate::state::window_for_surface(space, surface) {
        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .and_then(|data| data.lock().ok())
                .map(|data| data.initial_configure_sent)
                .unwrap_or(false)
        });

        if !initial_configure_sent && let Some(toplevel) = window.toplevel() {
            toplevel.send_configure();
        }
    }

    // Handle popup commits.
    popups.commit(surface);
    if let Some(popup) = popups.find_popup(surface) {
        match popup {
            PopupKind::Xdg(ref xdg) => {
                if !xdg.is_initial_configure_sent() {
                    // NOTE: This should never fail as the initial configure is always
                    // allowed.
                    if let Err(error) = xdg.send_configure() {
                        tracing::warn!(%error, "failed to send initial popup configure");
                    }
                }
            }
            PopupKind::InputMethod(ref _input_method) => {}
        }
    }
}

impl MindeState {
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else {
            return;
        };
        let Some(window) = crate::state::window_for_surface(&self.space, &root) else {
            return;
        };

        let Some(output) = self.space.outputs().next() else {
            return;
        };
        let Some(output_geo) = self.space.output_geometry(output) else {
            return;
        };
        let Some(window_geo) = self.space.element_geometry(&window) else {
            return;
        };

        // The target geometry for the positioner should be relative to its parent's geometry, so
        // we will compute that here.
        let mut target = output_geo;
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        target.loc -= window_geo.loc;

        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}
