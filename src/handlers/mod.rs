mod compositor;
mod layer_shell;
mod xdg_decoration;
mod xdg_shell;
mod xwayland;

use crate::MindeState;

//
// Wl Seat
//

use smithay::input::dnd::{DnDGrab, DndGrabHandler, GrabType, Source};
use smithay::input::pointer::Focus;
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::Serial;
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::selection::data_device::{
    DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler, set_data_device_focus,
};

impl SeatHandler for MindeState {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<MindeState> {
        &mut self.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: smithay::input::pointer::CursorImageStatus) {
        self.cursor_state.set_status(image);
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let dh = &self.display_handle;
        let client = focused.and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, client);
    }
}

//
// Wl Data Device
//

impl SelectionHandler for MindeState {
    /// Compositor-owned selections carry their text as user data
    /// (`wm-set-clipboard`); client-owned selections never see this.
    type SelectionUserData = String;

    /// Mirror new Wayland-side selections into the X11 world so X apps
    /// can paste them (anvil does the same).
    fn new_selection(
        &mut self,
        ty: smithay::wayland::selection::SelectionTarget,
        source: Option<smithay::wayland::selection::SelectionSource>,
        _seat: Seat<Self>,
    ) {
        if let Some(xwm) = self.xwm.as_mut()
            && let Err(err) = xwm.new_selection(ty, source.map(|source| source.mime_types()))
        {
            tracing::warn!(?err, ?ty, "failed to mirror selection into Xwayland");
        }
    }

    fn send_selection(
        &mut self,
        _ty: smithay::wayland::selection::SelectionTarget,
        _mime_type: String,
        fd: std::os::fd::OwnedFd,
        _seat: Seat<Self>,
        user_data: &Self::SelectionUserData,
    ) {
        // Write on a detached thread so a slow/stalled reader can't block
        // the event loop.
        let text = user_data.clone();
        std::thread::spawn(move || {
            use std::io::Write;
            let mut f = std::fs::File::from(fd);
            let _ = f.write_all(text.as_bytes());
        });
    }
}

impl DataDeviceHandler for MindeState {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

impl DndGrabHandler for MindeState {}
impl WaylandDndGrabHandler for MindeState {
    fn dnd_requested<S: Source>(
        &mut self,
        source: S,
        _icon: Option<WlSurface>,
        seat: Seat<Self>,
        serial: Serial,
        type_: GrabType,
    ) {
        match type_ {
            GrabType::Pointer => {
                let ptr = seat.get_pointer().unwrap();
                let start_data = ptr.grab_start_data().unwrap();

                // create a dnd grab to start the operation
                let grab = DnDGrab::new_pointer(&self.display_handle, start_data, source, seat);
                ptr.set_grab(self, grab, serial, Focus::Keep);
            }
            GrabType::Touch => {
                // smallvil lacks touch handling
                source.cancel();
            }
        }
    }
}

//
// Xdg Activation (urgency)
//

use smithay::wayland::xdg_activation::{
    XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
};

impl XdgActivationHandler for MindeState {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.xdg_activation_state
    }

    /// A client asked for a surface to be activated. We never grant focus
    /// from here (focus stays Scheme-driven); instead this is surfaced as
    /// StumpWM-style urgency via `(wm-on-urgent id)`.
    fn request_activation(
        &mut self,
        token: XdgActivationToken,
        _token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        self.xdg_activation_state.remove_token(&token);
        let id = self
            .windows
            .iter()
            .find(|(_, w)| {
                w.toplevel().map(|t| t.wl_surface() == &surface).unwrap_or(false)
            })
            .map(|(id, _)| *id);
        if let Some(id) = id {
            crate::guile::on_urgent(id);
        }
    }
}

//
// Wl Output & Xdg Output
//

impl OutputHandler for MindeState {}

smithay::delegate_dispatch2!(MindeState);
