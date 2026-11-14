// SPDX-License-Identifier: MIT

//! Preserve X11's input model when delivering keyboard focus through Wayland.

use std::borrow::Cow;

use smithay::{
    backend::input::KeyState,
    desktop::PopupKind,
    input::{
        Seat,
        keyboard::{KeyboardTarget, KeysymHandle, ModifiersState},
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{IsAlive, Serial},
    wayland::seat::WaylandFocus,
    xwayland::X11Surface,
};

use crate::MindeState;

/// Focus is applied after surface association, so every target has a wl_surface.
/// Keep the X11 window too: its KeyboardTarget implements SetInputFocus,
/// WM_TAKE_FOCUS and deferred focus release, which a bare wl_surface bypasses.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyboardFocusTarget {
    pub(super) surface: WlSurface,
    pub(super) x11: Option<X11Surface>,
}

impl KeyboardFocusTarget {
    pub(crate) fn surface(&self) -> &WlSurface {
        &self.surface
    }

    fn target(&self) -> &dyn KeyboardTarget<MindeState> {
        match &self.x11 {
            Some(window) => window,
            None => &self.surface,
        }
    }
}

impl IsAlive for KeyboardFocusTarget {
    fn alive(&self) -> bool {
        self.surface.alive() && self.x11.as_ref().is_none_or(X11Surface::alive)
    }
}

impl WaylandFocus for KeyboardFocusTarget {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        Some(Cow::Borrowed(&self.surface))
    }
}

impl From<PopupKind> for KeyboardFocusTarget {
    fn from(popup: PopupKind) -> Self {
        popup.wl_surface().clone().into()
    }
}

impl From<WlSurface> for KeyboardFocusTarget {
    fn from(surface: WlSurface) -> Self {
        Self { surface, x11: None }
    }
}

impl From<KeyboardFocusTarget> for WlSurface {
    fn from(target: KeyboardFocusTarget) -> Self {
        target.surface
    }
}

impl KeyboardTarget<MindeState> for KeyboardFocusTarget {
    fn enter(
        &self,
        seat: &Seat<MindeState>,
        data: &mut MindeState,
        keys: Vec<KeysymHandle<'_>>,
        serial: Serial,
    ) {
        self.target().enter(seat, data, keys, serial);
    }

    fn leave(&self, seat: &Seat<MindeState>, data: &mut MindeState, serial: Serial) {
        self.target().leave(seat, data, serial);
    }

    fn key(
        &self,
        seat: &Seat<MindeState>,
        data: &mut MindeState,
        key: KeysymHandle<'_>,
        state: KeyState,
        serial: Serial,
        time: u32,
    ) {
        if key.raw_code().raw() == crate::unicode_input::KEYCODE
            && let Some(stroke) = data.unicode_stroke.as_ref()
        {
            if state == KeyState::Pressed {
                stroke.send(time);
            }
            return;
        }
        if let Some((projected, actual)) = data.synthetic_key_modifiers
            && projected != actual
        {
            // Remapping C-F9 to Down must deliver Down while Control is
            // physically held. Project only the client's event stream: XKB
            // groups, latched/locked modifiers and press ownership stay intact.
            self.target().modifiers(seat, data, projected, serial);
            self.target().key(seat, data, key, state, serial, time);
            // Xwayland drains queued key events when modifiers arrive.
            self.target().modifiers(seat, data, projected, serial);
            self.target().modifiers(seat, data, actual, serial);
        } else {
            self.target().key(seat, data, key, state, serial, time);
        }
    }

    fn modifiers(
        &self,
        seat: &Seat<MindeState>,
        data: &mut MindeState,
        modifiers: ModifiersState,
        serial: Serial,
    ) {
        self.target().modifiers(seat, data, modifiers, serial);
    }
}
