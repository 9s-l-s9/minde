// SPDX-License-Identifier: MIT

//! Xwayland integration: the xwayland-shell protocol handler plus the
//! X11 window manager (`XwmHandler`). X11 toplevels are wrapped in the
//! same `desktop::Window` type as Wayland ones and registered with the
//! Scheme layer through the usual `handle-window-map!`/`unmap` events, so
//! frames, numbers, placement rules and hooks treat them identically.
//! Adapted from anvil's `shell/x11.rs`, minus decorations/grabs: the
//! frame tree owns all geometry, so client-initiated move/resize/
//! maximize requests are refused like in the xdg handlers.

use smithay::{
    desktop::Window,
    utils::{Logical, Rectangle},
    wayland::xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    xwayland::{
        X11Surface, X11Wm, XwmHandler,
        xwm::{Reorder, ResizeEdge, XwmId},
    },
};

use crate::{MindeState, guile};

impl XWaylandShellHandler for MindeState {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }

    fn surface_associated(
        &mut self,
        _xwm: XwmId,
        wl_surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        surface: X11Surface,
    ) {
        // Scheme may have focused this window at map time, before its
        // wl_surface existed (the association lags the map). Apply the
        // deferred keyboard focus now, or the client never receives
        // wl_keyboard.enter -> no X FocusIn -> apps like emacs render
        // their unfocused state (hollow cursor).
        let is_focused = self
            .focused_window
            .as_ref()
            .and_then(|w| w.x11_surface())
            .map(|x| x == &surface)
            .unwrap_or(false);
        if is_focused && let Some(keyboard) = self.seat.get_keyboard() {
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            keyboard.set_focus(self, Some(wl_surface), serial);
        }
    }
}

impl MindeState {
    /// The registered (managed) window wrapping this X11 surface, if any.
    fn window_for_x11(&self, surface: &X11Surface) -> Option<(u64, Window)> {
        self.windows
            .iter()
            .find(|(_, w)| matches!(w.x11_surface(), Some(x) if x == surface))
            .map(|(id, w)| (*id, w.clone()))
    }
}

impl XwmHandler for MindeState {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().unwrap()
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Err(err) = window.set_mapped(true) {
            tracing::warn!(%err, "failed to map X11 window");
            return;
        }
        let title = window.title();
        let class = window.class();
        tracing::debug!(?title, ?class, geo = ?window.geometry(), "x11 map request");
        let element = Window::new_x11_window(window);
        self.space.map_element(element.clone(), (0, 0), false);
        let id = self.register_window(element);
        guile::on_window_map(id, &title, &class);
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        // Menus, tooltips, dnd feedback: render where the client asked,
        // unmanaged (not registered with Scheme).
        let location = window.geometry().loc;
        let element = Window::new_x11_window(window);
        self.space.map_element(element, location, true);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some((id, element)) = self.window_for_x11(&window) {
            self.space.unmap_elem(&element);
            self.unregister_window(&element);
            guile::on_window_unmap(id);
        } else {
            // Unmanaged override-redirect window. (Bind before unmapping:
            // the scrutinee's space borrow lives through an if-let body.)
            let unmanaged = self
                .space
                .elements()
                .find(|e| matches!(e.x11_surface(), Some(x) if x == &window))
                .cloned();
            if let Some(element) = unmanaged {
                self.space.unmap_elem(&element);
            }
        }
        if !window.is_override_redirect() {
            let _ = window.set_mapped(false);
        }
    }

    fn destroyed_window(&mut self, xwm: XwmId, window: X11Surface) {
        // Some clients destroy without a preceding unmap.
        self.unmapped_window(xwm, window);
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        _x: Option<i32>,
        _y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        // Grant the size (needed pre-map so clients can draw); position
        // stays ours -- the next sync re-places managed windows anyway.
        let mut geo = window.geometry();
        if let Some(w) = w {
            geo.size.w = w as i32;
        }
        if let Some(h) = h {
            geo.size.h = h as i32;
        }
        tracing::debug!(?geo, "x11 configure request granted (size only)");
        let _ = window.configure(geo);
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
        // Track override-redirect windows moving themselves (menus).
        if !window.is_override_redirect() {
            return;
        }
        let element = self
            .space
            .elements()
            .find(|e| matches!(e.x11_surface(), Some(x) if x == &window))
            .cloned();
        if let Some(element) = element {
            self.space.map_element(element, geometry.loc, false);
        }
    }

    // Clipboard, X11 side. Wayland->X11 is handled by
    // SelectionHandler::new_selection mirroring ownership via
    // xwm.new_selection; these four complete the loop:
    // - an X11 app PASTES: send_selection asks the real Wayland owner
    //   (or writes our own wm-set-clipboard text),
    // - an X11 app COPIES: new_selection registers a compositor-side
    //   selection tagged SelectionOwner::X11, which
    //   SelectionHandler::send_selection routes back through Xwayland
    //   when a Wayland client pastes.
    fn allow_selection_access(
        &mut self,
        _xwm: XwmId,
        selection: smithay::wayland::selection::SelectionTarget,
    ) -> bool {
        // No primary-selection protocol state; clipboard only.
        matches!(
            selection,
            smithay::wayland::selection::SelectionTarget::Clipboard
        )
    }

    fn send_selection(
        &mut self,
        _xwm: XwmId,
        selection: smithay::wayland::selection::SelectionTarget,
        mime_type: String,
        fd: std::os::fd::OwnedFd,
    ) {
        use smithay::wayland::selection::SelectionTarget;
        use smithay::wayland::selection::data_device::{
            current_data_device_selection_userdata, request_data_device_client_selection,
        };
        if !matches!(selection, SelectionTarget::Clipboard) {
            return;
        }
        // Check ownership before requesting: a compositor-registered
        // selection would make the request fail with the fd already
        // consumed, dropping the paste.
        match current_data_device_selection_userdata(&self.seat).map(|o| o.clone()) {
            Some(super::SelectionOwner::Text(text)) => {
                // wm-set-clipboard text: write it ourselves.
                std::thread::spawn(move || {
                    use std::io::Write;
                    let mut f = std::fs::File::from(fd);
                    let _ = f.write_all(text.as_bytes());
                });
            }
            Some(super::SelectionOwner::X11) => {
                // An X11 mirror asking X11 back would loop; drop it.
            }
            None => {
                // A Wayland client owns the selection: ask it.
                if let Err(err) = request_data_device_client_selection(&self.seat, mime_type, fd) {
                    tracing::warn!(?err, "failed to hand the Wayland selection to an X11 paste");
                }
            }
        }
    }

    fn new_selection(
        &mut self,
        _xwm: XwmId,
        selection: smithay::wayland::selection::SelectionTarget,
        mime_types: Vec<String>,
    ) {
        use smithay::wayland::selection::SelectionTarget;
        if matches!(selection, SelectionTarget::Clipboard) {
            smithay::wayland::selection::data_device::set_data_device_selection(
                &self.display_handle,
                &self.seat,
                mime_types,
                super::SelectionOwner::X11,
            );
        }
    }

    fn cleared_selection(
        &mut self,
        _xwm: XwmId,
        selection: smithay::wayland::selection::SelectionTarget,
    ) {
        use smithay::wayland::selection::SelectionTarget;
        use smithay::wayland::selection::data_device::{
            clear_data_device_selection, current_data_device_selection_userdata,
        };
        if matches!(selection, SelectionTarget::Clipboard)
            && current_data_device_selection_userdata(&self.seat).is_some()
        {
            clear_data_device_selection(&self.display_handle, &self.seat);
        }
    }

    // The frame tree owns geometry: refuse client-initiated state
    // changes, mirroring the xdg-shell handlers.
    fn maximize_request(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn unmaximize_request(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn fullscreen_request(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn unfullscreen_request(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn resize_request(
        &mut self,
        _xwm: XwmId,
        _window: X11Surface,
        _button: u32,
        _edges: ResizeEdge,
    ) {
    }
    fn move_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32) {}
}
