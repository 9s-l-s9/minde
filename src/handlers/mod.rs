// SPDX-License-Identifier: MIT

mod compositor;
mod dispatch;
pub mod foreign_toplevel;
mod fractional_scale;
pub mod gamma_control;
mod idle;
pub mod input_method;
mod keyboard_focus;
pub mod keyboard_shortcuts_inhibit;
mod layer_shell;
pub mod output_management;
pub mod output_power;
pub mod pointer_constraints;
pub mod screencopy;
pub(crate) mod session_lock;
pub(crate) mod tablet;
pub mod tearing_control;
pub mod virtual_pointer;
mod window;
pub mod wlr_screencopy;
mod xdg_decoration;
mod xdg_shell;
mod xwayland;

use crate::MindeState;
use keyboard_focus::KeyboardFocusTarget;

//
// Wl Seat
//

use smithay::input::dnd::{DnDGrab, DndGrabHandler, DndTarget, GrabType, Source};
use smithay::input::pointer::Focus;
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Serial};
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::selection::data_device::{
    DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler, set_data_device_focus,
};

impl MindeState {
    /// Keep selections, text input and shortcut inhibition in sync when focus
    /// is cleared: the pinned Smithay only calls `focus_changed` for Some.
    pub(crate) fn set_keyboard_focus(&mut self, focus: Option<WlSurface>, serial: Serial) {
        if focus.as_ref() != self.keyboard_focus().as_ref() {
            self.restore_unicode_keymap();
        }
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let focus = focus.map(|surface| {
            let x11 = self
                .windows
                .values()
                .filter_map(|window| window.x11_surface())
                .find(|window| window.wl_surface().as_ref() == Some(&surface))
                .cloned();
            KeyboardFocusTarget { surface, x11 }
        });
        keyboard.set_focus(self, focus, serial);
        // A grab may retain or redirect the requested focus. Reconcile the
        // resulting focus, rather than assuming the request was applied.
        if keyboard.current_focus().is_none() {
            self.focus_changed(&self.seat.clone(), None);
        }
    }
}

impl SeatHandler for MindeState {
    type KeyboardFocus = KeyboardFocusTarget;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<MindeState> {
        &mut self.seat_state
    }

    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        image: smithay::input::pointer::CursorImageStatus,
    ) {
        self.cursor_state.set_status(image);
        self.schedule_redraw_at(&[self.pointer_location]);
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&KeyboardFocusTarget>) {
        self.restore_unicode_keymap();
        let focused = focused.map(KeyboardFocusTarget::surface);
        let dh = &self.display_handle;
        let client = focused.and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, client.clone());
        // The primary selection follows keyboard focus too, so the newly
        // focused client can read the middle-click selection.
        smithay::wayland::selection::primary_selection::set_primary_focus(dh, seat, client);
        // Text-input focus must follow keyboard focus so a running IME
        // (fcitx5/ibus) services the right surface. `set_keyboard_focus`
        // also invokes this callback when Smithay clears focus.
        self.set_text_input_focus(focused.cloned());
        // A keyboard-shortcuts inhibitor is live only while its surface has
        // keyboard focus; reconcile on every focus change (see
        // handlers::keyboard_shortcuts_inhibit).
        self.update_keyboard_shortcuts_inhibitors(focused);
    }
}

// A tablet-aware client can set its own tool cursor (via `zwp_tablet_tool_v2`'s
// set_cursor, and `wp_cursor_shape_v1` can also target tablet tools). Route the
// requested image into the shared `CursorState` the pointer uses, so the stylus
// cursor renders through the same path (themed/shape/client-surface). A client
// that sets no tool cursor falls through to the default themed pointer, which
// already follows the stylus via `self.pointer_location`.
impl smithay::wayland::tablet_manager::TabletSeatHandler for MindeState {
    fn tablet_tool_image(
        &mut self,
        _tool: &smithay::backend::input::TabletToolDescriptor,
        image: smithay::input::pointer::CursorImageStatus,
    ) {
        if self.locked {
            return;
        }
        self.cursor_state.set_status(image);
        self.schedule_redraw_at(&[self.pointer_location]);
    }
}

//
// Primary selection & data control (clipboard managers)
//

use smithay::wayland::selection::primary_selection::{
    PrimarySelectionHandler, PrimarySelectionState,
};

impl PrimarySelectionHandler for MindeState {
    fn primary_selection_state(&mut self) -> &mut PrimarySelectionState {
        &mut self.primary_selection_state
    }
}

impl smithay::wayland::selection::wlr_data_control::DataControlHandler for MindeState {
    fn data_control_state(
        &mut self,
    ) -> &mut smithay::wayland::selection::wlr_data_control::DataControlState {
        &mut self.data_control_state
    }
}

impl smithay::wayland::selection::ext_data_control::DataControlHandler for MindeState {
    fn data_control_state(
        &mut self,
    ) -> &mut smithay::wayland::selection::ext_data_control::DataControlState {
        &mut self.ext_data_control_state
    }
}

//
// Wl Data Device
//

/// Who backs a compositor-registered selection: literal text
/// (`wm-set-clipboard`) or an X11 client whose copy was mirrored onto
/// the Wayland side (`XwmHandler::new_selection`).
#[derive(Debug, Clone)]
pub enum SelectionOwner {
    Text(crate::transfer::Payload),
    X11,
}

impl SelectionHandler for MindeState {
    type SelectionUserData = SelectionOwner;

    /// Mirror new Wayland-side selections into the X11 world so X apps
    /// can paste them (anvil does the same).
    fn new_selection(
        &mut self,
        ty: smithay::wayland::selection::SelectionTarget,
        source: Option<smithay::wayland::selection::SelectionSource>,
        _seat: Seat<Self>,
    ) {
        self.cancel_selection_transfers(ty);
        if let Some(xwm) = self.xwm.as_mut()
            && let Err(err) = xwm.new_selection(ty, source.map(|source| source.mime_types()))
        {
            tracing::warn!(?err, ?ty, "failed to mirror selection into Xwayland");
        }
    }

    fn send_selection(
        &mut self,
        ty: smithay::wayland::selection::SelectionTarget,
        mime_type: String,
        fd: std::os::fd::OwnedFd,
        _seat: Seat<Self>,
        user_data: &Self::SelectionUserData,
    ) {
        match user_data {
            SelectionOwner::Text(text) => {
                self.transfers.write(fd, text.clone());
            }
            SelectionOwner::X11 => {
                // A Wayland client pastes something an X11 app copied:
                // ask Xwayland to write the data (X11 -> Wayland).
                if let Some(xwm) = self.xwm.as_mut()
                    && let Err(err) = xwm.send_selection(ty, mime_type, fd)
                {
                    tracing::warn!(?err, "failed to forward X11 selection to Wayland");
                }
            }
        }
    }
}

impl DataDeviceHandler for MindeState {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

impl DndGrabHandler for MindeState {
    fn dropped(
        &mut self,
        target: Option<DndTarget<'_, Self>>,
        validated: bool,
        _seat: Seat<Self>,
        _location: Point<f64, Logical>,
    ) {
        if let Some(source) = self.active_automation_dnd.take() {
            let status = if target.is_none() {
                crate::automation_dnd::AutomationStatus::NoTarget
            } else if validated {
                crate::automation_dnd::AutomationStatus::Accepted
            } else {
                crate::automation_dnd::AutomationStatus::Rejected
            };
            source.report_result(status);
        }
    }

    fn cancelled(&mut self, _seat: Seat<Self>, _location: Point<f64, Logical>) {
        if let Some(source) = self.active_automation_dnd.take() {
            source.report_result(crate::automation_dnd::AutomationStatus::Cancelled);
        }
    }
}
impl WaylandDndGrabHandler for MindeState {
    fn dnd_requested<S: Source>(
        &mut self,
        source: S,
        _icon: Option<WlSurface>,
        seat: Seat<Self>,
        serial: Serial,
        type_: GrabType,
    ) {
        if self.locked {
            source.cancel();
            return;
        }
        match type_ {
            GrabType::Pointer => {
                let Some(ptr) = seat.get_pointer() else {
                    tracing::warn!("pointer DnD requested on seat without pointer");
                    source.cancel();
                    return;
                };
                let Some(start_data) = ptr.grab_start_data() else {
                    tracing::warn!("pointer DnD requested without an active grab");
                    source.cancel();
                    return;
                };

                // create a dnd grab to start the operation
                let grab = DnDGrab::new_pointer(&self.display_handle, start_data, source, seat);
                ptr.set_grab(self, grab, serial, Focus::Keep);
            }
            GrabType::Touch => {
                let Some(touch) = seat.get_touch() else {
                    tracing::warn!("touch DnD requested on seat without touch");
                    source.cancel();
                    return;
                };
                let Some(start_data) = touch.grab_start_data() else {
                    tracing::warn!("touch DnD requested without an active grab");
                    source.cancel();
                    return;
                };

                // create a dnd grab to start the operation
                let grab = DnDGrab::new_touch(&self.display_handle, start_data, source, seat);
                touch.set_grab(self, grab, serial);
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
    /// StumpWM-style urgency via `(handle-urgent-window! id)`.
    fn request_activation(
        &mut self,
        token: XdgActivationToken,
        _token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        self.xdg_activation_state.remove_token(&token);
        let id = crate::state::window_for_surface(&surface).and_then(|w| self.id_for_window(&w));
        if let Some(id) = id {
            crate::guile::on_urgent(id);
        }
    }
}

//
// Wl Output & Xdg Output
//

impl OutputHandler for MindeState {}
