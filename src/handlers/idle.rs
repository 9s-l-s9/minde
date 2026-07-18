// SPDX-License-Identifier: MIT

//! `ext-idle-notify-v1` and `zwp_idle_inhibit_manager_v1` glue.
//!
//! The protocol state and dispatch live in Smithay's `wayland::idle_notify`
//! and `wayland::idle_inhibit` modules; the globals are registered in
//! [`MindeState::new`] and dispatched through the crate-wide
//! `delegate_dispatch2!` blanket. This file supplies the two handler traits
//! Smithay requires.
//!
//! Activity wiring: every input event resets the per-seat idle timers. That
//! notification is issued from `MindeState::process_input_event`
//! (`src/input.rs`) via [`MindeState::notify_idle_activity`], covering
//! keyboard, pointer motion/button/axis, and touch uniformly at the single
//! dispatch point.
//!
//! Inhibit wiring: a client (fullscreen video, a call) creates a
//! `zwp_idle_inhibitor_v1` on a surface to keep the screen awake. We track
//! the set of surfaces holding a live inhibitor and, while that set is
//! non-empty, mark the idle notifier inhibited so no `idled` event fires.
//!
//! Visibility: we deliberately do NOT track per-surface visibility. An
//! inhibitor takes effect as soon as it exists and stops when the client
//! destroys it (or the surface/client is gone -- Smithay drops the inhibitor
//! resource, calling `uninhibit`). In practice inhibitor clients create the
//! inhibitor only while their inhibiting content is actually presented and
//! destroy it otherwise (mpv, browsers, Zoom all do), so "any live
//! inhibitor" tracks "a visible inhibiting surface" closely enough without
//! the complexity of wiring inhibit state to map/unmap and focus changes.
//! This is the simple, honest option sanctioned by the roadmap item.
//!
//! No Scheme policy surface: idle policy (auto-lock, DPMS, dimming) is owned
//! externally by a swayidle-style daemon that speaks `ext-idle-notify-v1`
//! directly -- the same "run a program, don't reinvent one" stance
//! `(minde session)` already takes for the locker itself. There is no
//! compositor-side idle decision to route through Scheme.

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::IsAlive;
use smithay::wayland::idle_inhibit::IdleInhibitHandler;
use smithay::wayland::idle_notify::{IdleNotifierHandler, IdleNotifierState};

use crate::state::MindeState;

impl IdleNotifierHandler for MindeState {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        &mut self.idle_notifier_state
    }
}

impl IdleInhibitHandler for MindeState {
    fn inhibit(&mut self, surface: WlSurface) {
        self.idle_inhibitors.insert(surface);
        self.refresh_idle_inhibit();
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.idle_inhibitors.remove(&surface);
        self.refresh_idle_inhibit();
    }
}

impl MindeState {
    /// Reset the per-seat idle timers on user activity. Called for every
    /// input event from `process_input_event`.
    pub fn notify_idle_activity(&mut self) {
        // Opportunistically drop inhibitors whose client died without a clean
        // Destroy (the vendored inhibitor does not run uninhibit on the
        // destructor path), so a crashed inhibiting client cannot pin the
        // screen awake indefinitely. Only pays the cost while inhibitors
        // exist.
        if !self.idle_inhibitors.is_empty() {
            self.refresh_idle_inhibit();
        }
        self.idle_notifier_state.notify_activity(&self.seat);
    }

    /// Recompute the notifier's inhibited flag from the set of surfaces that
    /// currently hold a live idle inhibitor, pruning surfaces whose resource
    /// is no longer alive. `set_is_inhibited` is a no-op when the value is
    /// unchanged, so calling this on every inhibitor add/remove is cheap.
    fn refresh_idle_inhibit(&mut self) {
        self.idle_inhibitors.retain(|surface| surface.alive());
        let inhibited = !self.idle_inhibitors.is_empty();
        self.idle_notifier_state.set_is_inhibited(inhibited);
    }
}
