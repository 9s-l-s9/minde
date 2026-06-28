//! Xwayland integration: the xwayland-shell protocol handler plus the
//! X11 window manager (`XwmHandler`). X11 toplevels are wrapped in the
//! same `desktop::Window` type as Wayland ones and registered with the
//! Scheme layer through the usual `wm-on-window-map`/`unmap` events, so
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

    // The frame tree owns geometry: refuse client-initiated state
    // changes, mirroring the xdg-shell handlers.
    fn maximize_request(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn unmaximize_request(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn fullscreen_request(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn unfullscreen_request(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn resize_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32, _edges: ResizeEdge) {}
    fn move_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32) {}
}
