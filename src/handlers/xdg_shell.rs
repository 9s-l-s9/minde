// SPDX-License-Identifier: MIT

use smithay::{
    backend::renderer::utils::with_renderer_surface_state,
    desktop::{
        PopupKeyboardGrab, PopupKind, PopupManager, PopupPointerGrab, PopupUngrabStrategy, Window,
        WindowSurfaceType, find_popup_root_surface, get_popup_toplevel_coords,
        layer_map_for_output,
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
    utils::{Logical, Point, Rectangle, Serial},
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

#[cfg(test)]
mod tests;

/// Select the output with the largest overlap with the root's window geometry,
/// then express its bounds relative to this popup's immediate parent. Ties and
/// wholly offscreen roots retain output order; an absent output has no bounds.
fn popup_constraint_rect(
    root: Rectangle<i32, Logical>,
    parent_offset: Point<i32, Logical>,
    outputs: impl IntoIterator<Item = Rectangle<i32, Logical>>,
) -> Option<Rectangle<i32, Logical>> {
    let mut target = crate::state::largest_output_overlap(
        root,
        outputs.into_iter().map(|bounds| (bounds, bounds)),
    )?;
    target.loc -= root.loc;
    target.loc -= parent_offset;
    Some(target)
}

fn latest_configure(surface: &ToplevelSurface) -> Option<Serial> {
    with_states(surface.wl_surface(), |states| {
        let attributes = states
            .data_map
            .get::<XdgToplevelSurfaceData>()?
            .lock()
            .unwrap();
        attributes
            .pending_configures()
            .last()
            .or(attributes.last_acked.as_ref())
            .map(|configure| configure.serial)
    })
}

fn request_fullscreen(surface: &ToplevelSurface, id: u64, on: bool) {
    let before = latest_configure(surface);
    guile::on_window_fullscreen(id, on);
    // xdg-shell requires a configure even when policy defers the request or
    // the requested state already holds. Do not duplicate a layout reply.
    if surface.is_initial_configure_sent() && latest_configure(surface) == before {
        surface.send_configure();
    }
}

impl XdgShellHandler for MindeState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, _surface: ToplevelSurface) {
        // The role remains in XdgShellState while unmapped. Its first empty
        // commit gets a configure; only a buffer-bearing commit maps it.
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        // A dying client sends no further commits, so the render pass must
        // be asked for explicitly or its last frame lingers on screen.
        self.schedule_redraw();
        let window = crate::state::window_for_surface(&self.space, surface.wl_surface());

        if let Some(window) = window {
            self.unmap_window(&window);
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        let _ = self.popups.track_popup(PopupKind::Xdg(surface.clone()));
        self.update_surface_fractional_scale(surface.wl_surface());
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
        if self.locked {
            return;
        }
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
        if self.locked {
            return;
        }
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
        if self.locked {
            surface.send_popup_done();
            return;
        }
        let Some(seat) = Seat::<MindeState>::from_resource(&seat) else {
            tracing::warn!("popup grab used an unknown seat");
            return;
        };
        let kind = PopupKind::Xdg(surface.clone());
        let Ok(root) = find_popup_root_surface(&kind) else {
            return;
        };
        match self.popups.grab_popup(root.into(), kind, &seat, serial) {
            Ok(mut grab) => {
                if let Some(keyboard) = seat.get_keyboard() {
                    if keyboard.is_grabbed()
                        && !(keyboard.has_grab(serial)
                            || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
                    {
                        grab.ungrab(PopupUngrabStrategy::All);
                        return;
                    }
                    self.set_keyboard_focus(grab.current_grab().map(Into::into), serial);
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

    // Scheme owns fullscreen independently of activation. Before mapping,
    // preserve the request in the configure state; the first buffer commit
    // applies it to the new managed window.
    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        _output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
    ) {
        match self.id_for_toplevel(&surface) {
            Some(id) => request_fullscreen(&surface, id, true),
            None => {
                let size = self.output_rect_containing(None).size;
                surface.with_pending_state(|state| {
                    state.states.set(xdg_toplevel::State::Fullscreen);
                    state.size = Some(size);
                });
                if surface.is_initial_configure_sent() {
                    surface.send_configure();
                }
            }
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        match self.id_for_toplevel(&surface) {
            Some(id) => request_fullscreen(&surface, id, false),
            None => {
                surface.with_pending_state(|state| {
                    state.states.unset(xdg_toplevel::State::Fullscreen);
                    state.size = None;
                });
                if surface.is_initial_configure_sent() {
                    surface.send_configure();
                }
            }
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

/// Process popup configure handshakes after `WlSurface::commit`.
pub fn handle_popup_commit(popups: &mut PopupManager, surface: &WlSurface) {
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
    /// Apply root-buffer transitions after Smithay processes the commit.
    /// Reuse the commit router's lookup; ordinary mapped commits need no
    /// additional scan of the shell's toplevel list.
    pub(crate) fn handle_toplevel_commit(
        &mut self,
        surface: &WlSurface,
        window: Option<Window>,
    ) -> Option<Window> {
        let configured = with_states(surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .map(|data| data.lock().unwrap().initial_configure_sent)
        });
        let Some(configured) = configured else {
            return window;
        };
        let has_buffer =
            with_renderer_surface_state(surface, |state| state.buffer().is_some()).unwrap_or(false);
        if has_buffer && window.is_some() {
            return window;
        }
        if !has_buffer && let Some(window) = &window {
            self.unmap_window(window);
        }
        let toplevel = self
            .xdg_shell_state
            .toplevel_surfaces()
            .iter()
            .find(|toplevel| toplevel.wl_surface() == surface)
            .cloned()?;
        if !configured {
            toplevel.send_configure();
        }
        if !has_buffer {
            return None;
        }

        let fullscreen = toplevel.with_committed_state(|state| {
            state.is_some_and(|state| state.states.contains(xdg_toplevel::State::Fullscreen))
        });
        // A fresh Window gives each mapping a new ID. Old foreign handles and
        // automation selectors must never address a remapped toplevel.
        let window = Window::new_wayland_window(toplevel);
        window.on_commit();
        self.space.map_element(window.clone(), (0, 0), false);
        let id = self.register_window(window.clone());
        let (title, app_id) = self.window_title(id).unwrap_or_default();
        guile::on_window_map(id, &title, &app_id);
        if fullscreen {
            guile::on_window_fullscreen(id, true);
        }
        tracing::info!(
            component = "window",
            protocol = "wayland",
            window_id = id,
            "managed toplevel mapped"
        );
        Some(window)
    }

    pub(crate) fn unconstrain_popup(&self, popup: &PopupSurface) {
        let kind = PopupKind::Xdg(popup.clone());
        let Ok(root) = find_popup_root_surface(&kind) else {
            return;
        };
        let parent_offset = get_popup_toplevel_coords(&kind);
        let target = if let Some(window) = crate::state::window_for_surface(&self.space, &root) {
            self.space.element_geometry(&window).and_then(|geometry| {
                popup_constraint_rect(
                    geometry,
                    parent_offset,
                    self.space
                        .outputs()
                        .filter_map(|o| self.space.output_geometry(o)),
                )
            })
        } else {
            self.space.outputs().find_map(|output| {
                let bounds = self.space.output_geometry(output)?;
                let map = layer_map_for_output(output);
                let layer = map.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL)?;
                let mut geometry = map.layer_geometry(layer)?;
                // LayerMap positions are output-local, unlike Space windows.
                geometry.loc += bounds.loc;
                popup_constraint_rect(geometry, parent_offset, [bounds])
            })
        };
        let Some(target) = target else {
            return;
        };
        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}
