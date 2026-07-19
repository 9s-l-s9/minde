// SPDX-License-Identifier: MIT

//! `text-input-v3` and `input-method-v2` integration for IME support
//! (fcitx5/ibus) and on-screen keyboards.
//!
//! Smithay ships the whole protocol machinery in `wayland::text_input`
//! (`TextInputManagerState`) and `wayland::input_method`
//! (`InputMethodManagerState`); the request/global dispatch rides the
//! crate-wide `delegate_dispatch2!` blanket in `handlers/mod.rs`. This file
//! only supplies:
//!
//!   * the two manager globals (created in [`init_input_method`], kept alive
//!     in [`MindeState`]), and
//!   * the [`InputMethodHandler`] impl that positions and renders the
//!     input-method popup (candidate window) relative to the text-input
//!     cursor rectangle.
//!
//! Focus propagation (text-input focus follows keyboard focus) lives in
//! `MindeState::set_text_input_focus`, driven from `SeatHandler::
//! focus_changed` and the explicit focus-clear paths -- see `handlers/mod.rs`.
//!
//! ## Input-method bind policy
//!
//! `zwp_input_method_v2` is a privileged protocol: exactly one input method
//! may be active on a seat at a time. Smithay's manager takes a bind filter
//! (the same shape as data-control's). We admit every client (`|_| true`),
//! first-come-first-served, matching sway/wlroots: the desktop's session
//! daemon (fcitx5/ibus) is trusted, and the protocol itself enforces the
//! single-active-instance rule -- `InputMethodHandle::add_instance` sends
//! `unavailable` to a second binder rather than letting two IMEs fight over
//! one seat. There is no Scheme surface: choosing an IME is the user's
//! session concern, not compositor policy (same stance as the locker/idle).

use smithay::{
    desktop::{PopupKind, PopupManager},
    reexports::wayland_server::{DisplayHandle, protocol::wl_surface::WlSurface},
    utils::{Logical, Rectangle},
    wayland::{
        input_method::{InputMethodHandler, InputMethodManagerState, PopupSurface},
        text_input::TextInputManagerState,
    },
};

use crate::MindeState;

/// Creates the `zwp_text_input_manager_v3` and `zwp_input_method_manager_v2`
/// globals, advertised on both backends. See the module docs for the bind
/// policy behind the `|_| true` input-method filter.
pub fn init_input_method(dh: &DisplayHandle) -> (TextInputManagerState, InputMethodManagerState) {
    let text_input = TextInputManagerState::new::<MindeState>(dh);
    let input_method = InputMethodManagerState::new::<MindeState, _>(dh, |_client| true);
    (text_input, input_method)
}

impl InputMethodHandler for MindeState {
    fn new_popup(&mut self, surface: PopupSurface) {
        // Track the candidate window in the shared PopupManager. Its parent
        // is the focused text-input surface, so it roots in that toplevel's
        // popup tree and `Window::render_elements` draws it automatically at
        // the cursor rectangle -- no extra wiring in the backend render loops.
        if let Err(err) = self.popups.track_popup(PopupKind::from(surface)) {
            tracing::warn!(%err, "failed to track input-method popup");
        }
    }

    fn dismiss_popup(&mut self, surface: PopupSurface) {
        if let Some(parent) = surface.get_parent().map(|parent| parent.surface.clone()) {
            let _ = PopupManager::dismiss_popup(&parent, &PopupKind::from(surface));
        }
    }

    fn popup_repositioned(&mut self, _surface: PopupSurface) {
        // The popup position is derived from the text-input cursor rectangle
        // on every commit; nothing extra to recompute here.
    }

    /// Geometry of the parent surface (the focused text field's toplevel),
    /// used to place the candidate window. Returns the window geometry so the
    /// popup's cursor-relative location resolves against the right origin.
    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, Logical> {
        use smithay::wayland::seat::WaylandFocus;
        self.space
            .elements()
            .find(|window| window.wl_surface().as_deref() == Some(parent))
            .map(|window| window.geometry())
            .unwrap_or_default()
    }
}

impl MindeState {
    /// Retarget text-input focus to `focus`, so a running IME (fcitx5/ibus)
    /// services the surface that just gained keyboard focus. Text-input focus
    /// must follow keyboard focus (the protocol requires it); we send `leave`
    /// to the old client, move the focus, then `enter` the new one. Both
    /// sends are no-ops unless an input method is actually bound, so this is
    /// cheap to call on every focus change.
    pub fn set_text_input_focus(&mut self, focus: Option<WlSurface>) {
        use smithay::wayland::text_input::TextInputSeat;
        let text_input = self.seat.text_input();
        text_input.leave();
        text_input.set_focus(focus);
        text_input.enter();
    }
}
