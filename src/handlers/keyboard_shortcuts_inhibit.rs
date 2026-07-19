// SPDX-License-Identifier: MIT

//! `zwp_keyboard_shortcuts_inhibit_manager_v1` glue.
//!
//! The protocol state and dispatch live in Smithay's
//! `wayland::keyboard_shortcuts_inhibit` module; the global is registered in
//! [`MindeState::new`] and dispatched through the crate-wide
//! `delegate_dispatch2!` blanket. This file supplies the one handler trait
//! Smithay requires plus the focus integration that makes an inhibitor take
//! effect exactly while its surface holds keyboard focus.
//!
//! # What an active inhibitor does
//!
//! While an inhibitor is *active*, `process_input_event` (see `src/input.rs`)
//! consults `Seat::keyboard_shortcuts_inhibited()` right after the session-lock
//! gate and, if inhibited, returns `FilterResult::Forward` for every key --
//! bypassing the prefix-key grab and every Scheme shortcut so the client (a
//! remote-desktop or VM viewer) receives the keys the compositor would
//! otherwise reserve. Because the check runs *after* the locked gate, an
//! inhibitor can never take effect on the session-lock surface.
//!
//! # Policy: auto-grant, focus-scoped
//!
//! Some compositors prompt the user before honoring an inhibitor. We
//! auto-grant (documented, no Scheme surface -- the same "the tools are the
//! user's own session" stance as virtual-keyboard and the IME), but honor the
//! protocol's hard rule that the inhibitor is only active while its surface
//! has keyboard focus. An inhibitor must never survive focus loss: that would
//! let a background client keep swallowing the compositor's shortcuts, which
//! is a security problem. So:
//!
//! - a new inhibitor is activated immediately only if its surface already has
//!   keyboard focus, otherwise it stays inactive until focus arrives;
//! - every keyboard-focus change ([`update_keyboard_shortcuts_inhibitors`],
//!   called from `SeatHandler::focus_changed` and every focus-clear path)
//!   deactivates the previously active inhibitor and activates the new
//!   focused surface's inhibitor, if any.

use smithay::input::keyboard::KeyboardHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::keyboard_shortcuts_inhibit::{
    KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState, KeyboardShortcutsInhibitor,
    KeyboardShortcutsInhibitorSeat,
};

use crate::state::MindeState;

impl KeyboardShortcutsInhibitHandler for MindeState {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.keyboard_shortcuts_inhibit_state
    }

    fn new_inhibitor(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        // Auto-grant, but only actually activate if the inhibiting surface
        // already holds keyboard focus. Otherwise it waits, inactive, for
        // `update_keyboard_shortcuts_inhibitors` to activate it on focus.
        let focused = self.keyboard_focus();
        if focused.as_ref() == Some(inhibitor.wl_surface()) {
            inhibitor.activate();
            self.active_shortcuts_inhibitor = Some(inhibitor);
        }
    }

    fn inhibitor_destroyed(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        // Smithay already flipped the resource inactive; just drop our
        // tracking reference if this was the active one.
        if self.active_shortcuts_inhibitor.as_ref() == Some(&inhibitor) {
            self.active_shortcuts_inhibitor = None;
        }
    }
}

impl MindeState {
    /// Current keyboard-focus surface, if any.
    fn keyboard_focus(&self) -> Option<WlSurface> {
        let keyboard: KeyboardHandle<Self> = self.seat.get_keyboard()?;
        keyboard.current_focus()
    }

    /// Reconcile the active keyboard-shortcuts inhibitor with a new keyboard
    /// focus. Deactivates the previously active inhibitor when focus leaves
    /// its surface and activates the newly focused surface's inhibitor, if it
    /// has one. Called from `SeatHandler::focus_changed` and every explicit
    /// focus-clear path (which does not invoke `focus_changed`).
    ///
    /// This is the security-relevant enforcement of the protocol rule that an
    /// inhibitor is live only while its surface is focused.
    pub fn update_keyboard_shortcuts_inhibitors(&mut self, focus: Option<&WlSurface>) {
        // Drop the currently active inhibitor if focus moved off its surface.
        if let Some(active) = self.active_shortcuts_inhibitor.take() {
            if focus == Some(active.wl_surface()) {
                // Same surface still focused -- keep it active.
                self.active_shortcuts_inhibitor = Some(active);
                return;
            }
            active.inactivate();
        }
        // Activate the newly focused surface's inhibitor, if any.
        if let Some(surface) = focus
            && let Some(inhibitor) = self.seat.keyboard_shortcuts_inhibitor_for_surface(surface)
        {
            inhibitor.activate();
            self.active_shortcuts_inhibitor = Some(inhibitor);
        }
    }
}
