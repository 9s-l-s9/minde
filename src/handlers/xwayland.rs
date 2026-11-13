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
        xwm::{Reorder, ResizeEdge, WmWindowProperty, XwmId},
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
        _wl_surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
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
        if !self.locked && is_focused {
            self.restore_keyboard_focus();
        }
        // The X11/Wayland association can arrive after the buffer commit.
        // Refresh the inspection mirror even if the client never repaints.
        if let Some((id, window)) = self.window_for_x11(&surface) {
            window.on_commit();
            self.update_window_fractional_scale(&window);
            self.publish_window_geometry(id);
        }
        self.schedule_redraw();
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

    /// Retire a failed X server without disturbing native clients.
    pub(crate) fn fail_xwayland(&mut self) {
        use smithay::reexports::wayland_server::backend::DisconnectReason;
        use smithay::wayland::selection::{
            data_device::{clear_data_device_selection, current_data_device_selection_userdata},
            primary_selection::{clear_primary_selection, current_primary_selection_userdata},
        };

        self.xdisplay = None;
        guile::set_xwayland_status("failed", None);
        // Include unmanaged menus and any registered window missing from
        // Space. Keep the XWM alive until focus and grabs have been cleared:
        // dropping it first destroys the surface associations we need here.
        let mut windows: Vec<_> = self
            .space
            .elements()
            .filter(|window| window.x11_surface().is_some())
            .cloned()
            .collect();
        for window in self
            .windows
            .values()
            .filter(|window| window.x11_surface().is_some())
        {
            if !windows.contains(window) {
                windows.push(window.clone());
            }
        }
        for window in windows {
            self.unmap_window(&window);
        }
        // Prevent queued Wayland commits from consulting a retired XWM.
        if let Some(client) = self.xwayland_client.take() {
            self.display_handle
                .backend_handle()
                .kill_client(client.id(), DisconnectReason::ConnectionClosed);
        }
        self.xwm = None;

        let clipboard = current_data_device_selection_userdata(&self.seat)
            .is_some_and(|owner| matches!(*owner, super::SelectionOwner::X11));
        let primary = current_primary_selection_userdata(&self.seat)
            .is_some_and(|owner| matches!(*owner, super::SelectionOwner::X11));
        if clipboard {
            clear_data_device_selection(&self.display_handle, &self.seat);
        }
        if primary {
            clear_primary_selection(&self.display_handle, &self.seat);
        }
        self.schedule_redraw();
        guile::publish_status();
    }
}

impl XwmHandler for MindeState {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().unwrap()
    }

    fn disconnected(&mut self, xwm: XwmId) {
        if self.xwm.as_ref().is_some_and(|wm| wm.id() == xwm) {
            tracing::warn!("Xwayland connection lost; removing X11 windows");
            self.fail_xwayland();
        }
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn property_notify(&mut self, _xwm: XwmId, window: X11Surface, property: WmWindowProperty) {
        if matches!(property, WmWindowProperty::Title | WmWindowProperty::Class)
            && let Some((_, element)) = self.window_for_x11(&window)
        {
            self.report_title_if_changed(&element);
        }
    }

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Err(err) = window.set_mapped(true) {
            tracing::warn!(%err, "failed to map X11 window");
            return;
        }
        let title = window.title();
        let class = window.class();
        tracing::debug!(?title, ?class, geo = ?window.geometry(), "x11 map request");
        // Clients may set _NET_WM_STATE_FULLSCREEN before MapRequest.
        let fullscreen = window.is_fullscreen();
        let element = Window::new_x11_window(window);
        self.space.map_element(element.clone(), (0, 0), false);
        let id = self.register_window(element);
        guile::on_window_map(id, &title, &class);
        if fullscreen {
            guile::on_foreign_fullscreen(id, true);
        }
        tracing::info!(
            component = "window",
            protocol = "xwayland",
            window_id = id,
            "managed toplevel mapped"
        );
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        // Menus, tooltips, dnd feedback: render where the client asked,
        // unmanaged (not registered with Scheme).
        let location = window.last_configure().loc;
        let element = Window::new_x11_window(window);
        self.space.map_element(element.clone(), location, true);
        self.update_window_fractional_scale(&element);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.schedule_redraw();
        if let Some((_, element)) = self.window_for_x11(&window) {
            self.unmap_window(&element);
        } else {
            // Unmanaged override-redirect window. (Bind before unmapping:
            // the scrutinee's space borrow lives through an if-let body.)
            let unmanaged = self
                .space
                .elements()
                .find(|e| matches!(e.x11_surface(), Some(x) if x == &window))
                .cloned();
            if let Some(element) = unmanaged {
                self.unmap_window(&element);
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
        // The placement cache only reconfigures changed frame geometry.
        // Accepting a tiled client's self-resize would leave its real size
        // smaller than the cached frame indefinitely (e.g. Chromium).
        if let Some((id, _)) = self.window_for_x11(&window)
            && (!self.floating_ids.contains(&id) || window.is_fullscreen())
        {
            let _ = window.configure(window.geometry());
            return;
        }
        // Unmanaged/pre-map and floating clients may choose their size.
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
            self.space.map_element(element.clone(), geometry.loc, false);
            self.update_window_fractional_scale(&element);
            self.schedule_redraw();
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
        use smithay::wayland::selection::SelectionTarget;
        // Both clipboard and primary selection are wired (the latter added
        // with the primary-selection protocol state).
        matches!(
            selection,
            SelectionTarget::Clipboard | SelectionTarget::Primary
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
        use smithay::wayland::selection::primary_selection::{
            current_primary_selection_userdata, request_primary_client_selection,
        };
        // The current owner userdata and the "ask the Wayland owner"
        // request differ per target; everything else is identical.
        let owner = match selection {
            SelectionTarget::Clipboard => {
                current_data_device_selection_userdata(&self.seat).map(|o| o.clone())
            }
            SelectionTarget::Primary => {
                current_primary_selection_userdata(&self.seat).map(|o| o.clone())
            }
        };
        // Check ownership before requesting: a compositor-registered
        // selection would make the request fail with the fd already
        // consumed, dropping the paste.
        match owner {
            Some(super::SelectionOwner::Text(text)) => {
                self.transfers.write(fd, text);
            }
            Some(super::SelectionOwner::X11) => {
                // An X11 mirror asking X11 back would loop; drop it.
            }
            None => {
                // A Wayland client owns the selection: ask it. The two
                // targets return distinct error types, so handle inline.
                match selection {
                    SelectionTarget::Clipboard => {
                        if let Err(err) =
                            request_data_device_client_selection(&self.seat, mime_type, fd)
                        {
                            tracing::warn!(
                                ?err,
                                "failed to hand the Wayland clipboard to an X11 paste"
                            );
                        }
                    }
                    SelectionTarget::Primary => {
                        if let Err(err) =
                            request_primary_client_selection(&self.seat, mime_type, fd)
                        {
                            tracing::warn!(
                                ?err,
                                "failed to hand the Wayland primary selection to an X11 paste"
                            );
                        }
                    }
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
        self.cancel_selection_transfers(selection);
        match selection {
            SelectionTarget::Clipboard => {
                smithay::wayland::selection::data_device::set_data_device_selection(
                    &self.display_handle,
                    &self.seat,
                    mime_types,
                    super::SelectionOwner::X11,
                );
            }
            SelectionTarget::Primary => {
                smithay::wayland::selection::primary_selection::set_primary_selection(
                    &self.display_handle,
                    &self.seat,
                    mime_types,
                    super::SelectionOwner::X11,
                );
            }
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
        use smithay::wayland::selection::primary_selection::{
            clear_primary_selection, current_primary_selection_userdata,
        };
        self.cancel_selection_transfers(selection);
        match selection {
            SelectionTarget::Clipboard => {
                if current_data_device_selection_userdata(&self.seat).is_some() {
                    clear_data_device_selection(&self.display_handle, &self.seat);
                }
            }
            SelectionTarget::Primary => {
                if current_primary_selection_userdata(&self.seat).is_some() {
                    clear_primary_selection(&self.display_handle, &self.seat);
                }
            }
        }
    }

    // Keep tiling policy for maximize/move/resize. Fullscreen requests go
    // through the same Scheme model as native Wayland clients and taskbars.
    fn maximize_request(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn unmaximize_request(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some((id, _)) = self.window_for_x11(&window) {
            guile::on_foreign_fullscreen(id, true);
        }
    }
    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some((id, _)) = self.window_for_x11(&window) {
            guile::on_foreign_fullscreen(id, false);
        }
    }
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
