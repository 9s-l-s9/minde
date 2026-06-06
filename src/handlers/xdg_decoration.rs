//! xdg-decoration: always request server-side decorations so tiled clients
//! (foot, alacritty, ...) don't draw their own titlebars. We don't actually
//! render any decoration -- in a frame-tiled WM there is nothing to draw.

use smithay::{
    reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
    wayland::shell::xdg::{ToplevelSurface, decoration::XdgDecorationHandler},
};

use crate::MindeState;

impl XdgDecorationHandler for MindeState {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ServerSide);
        });
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _requested: Mode) {
        // Ignore the client's preference: tiling means no CSD.
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ServerSide);
        });
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ServerSide);
        });
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }
}
