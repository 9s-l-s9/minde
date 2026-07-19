// SPDX-License-Identifier: MIT

//! `ext-session-lock-v1` handler.
//!
//! Security contract: while the session is locked, the render pass shows
//! ONLY each output's lock surface (or solid black before it commits, or if
//! the lock client dies), and no input reaches a regular client or the
//! Scheme keybinding layer. The rendering half lives in the two backends
//! (`src/winit.rs`, `src/udev.rs`); the input half in `src/input.rs`. This
//! module owns the protocol handler and the lock-surface bookkeeping.

use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::session_lock::{
    LockSurface, LockSurfaceConfigure, SessionLockHandler, SessionLockManagerState, SessionLocker,
};

use crate::MindeState;

impl SessionLockHandler for MindeState {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_state
    }

    /// A client asked to lock the session. Enter the locked state, take
    /// input away from everything else, force a blank frame onto every
    /// output, then confirm the lock.
    fn lock(&mut self, confirmation: SessionLocker) {
        // Taking over an abandoned lock: a previous lock client disconnected
        // without unlocking, so the session stayed blank (see `unlock` and
        // the render paths). Drop the stale (now-dead) lock surfaces so the
        // new client can register fresh ones. Smithay's manager does not
        // block the retake: the new client binds its own wl_output objects,
        // which are distinct protocol objects from the dead client's, so the
        // manager's per-output "already locked" guard does not trip.
        self.lock_surfaces.clear();

        // Enter the locked state *before* confirming (and before rendering)
        // so that no desktop frame can ever be produced once we are locked.
        let was_locked = self.locked;
        self.locked = true;
        crate::guile::set_session_locked(true);

        // Input hygiene: cancel any armed compositor-side key-repeat and take
        // keyboard + pointer focus away from regular clients. The lock
        // surface gets keyboard focus in `new_surface` once it arrives.
        self.cancel_key_repeat();
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, Option::<WlSurface>::None, serial);
        }
        // Drop text-input focus too: no IME activity while the session is locked.
        self.set_text_input_focus(None);
        if let Some(pointer) = self.seat.get_pointer() {
            let location = self.pointer_location;
            let time = self.start_time.elapsed().as_millis() as u32;
            pointer.motion(
                self,
                None,
                &smithay::input::pointer::MotionEvent {
                    location,
                    serial,
                    time,
                },
            );
            pointer.frame(self);
        }

        // Force a blank frame onto every output before telling the client the
        // session is locked: the spec wants a cleared frame presented first,
        // so the last desktop frame is not still on screen when we confirm.
        // (Both backends also keep repainting on their own; this just closes
        // the gap synchronously. No-op under winit -- its redraw loop
        // repaints continuously.)
        self.render_all_outputs_now();

        // Only fire the Scheme transition hook on a real unlocked->locked
        // edge, not on a takeover of an already-locked session.
        if !was_locked {
            tracing::info!("session locked (ext-session-lock)");
            crate::guile::on_session_lock();
        }

        confirmation.lock();
    }

    /// The lock client unlocked the session. Leave the locked state and
    /// restore focus to whatever the Scheme layer considered focused.
    fn unlock(&mut self) {
        if !self.locked {
            return;
        }
        self.locked = false;
        self.lock_surfaces.clear();
        crate::guile::set_session_locked(false);
        tracing::info!("session unlocked (ext-session-lock)");

        // Restore keyboard focus by re-running the same focus path as
        // `WmCommand::Focus` for the currently-focused window; clear it if
        // nothing was focused. The Scheme side sees the unlock hook and can
        // resync too if it wants.
        let serial = SERIAL_COUNTER.next_serial();
        let focus = self
            .focused_window
            .as_ref()
            .and_then(|w| w.wl_surface().map(|s| s.into_owned()));
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, focus, serial);
        }

        crate::guile::on_session_unlock();
    }

    /// A lock client created a lock surface for one output. Size it to that
    /// output, give it keyboard focus (so password entry lands there), and
    /// track it for the render pass.
    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let Some(output) = Output::from_resource(&output) else {
            tracing::warn!("session-lock surface for an unknown output; ignoring");
            return;
        };
        self.configure_lock_surface(&surface, &output);

        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, Some(surface.wl_surface().clone()), serial);
        }

        // Replace any prior surface for this output (client reconnect).
        self.lock_surfaces.retain(|(o, _)| o != &output);
        self.lock_surfaces.push((output, surface));
    }

    fn ack_configure(&mut self, _surface: WlSurface, _configure: LockSurfaceConfigure) {}
}

impl MindeState {
    /// Configures a lock surface to its output's current logical size and
    /// sends the configure. Called on surface creation and whenever the
    /// output changes size (see the backends' resize paths).
    pub fn configure_lock_surface(&self, surface: &LockSurface, output: &Output) {
        let size = self
            .space
            .output_geometry(output)
            .map(|geo| geo.size)
            .or_else(|| {
                output.current_mode().map(|mode| {
                    let scale = output.current_scale().integer_scale().max(1);
                    (mode.size.w / scale, mode.size.h / scale).into()
                })
            })
            .unwrap_or_else(|| (0, 0).into());
        surface.with_pending_state(|state| {
            state.size = Some((size.w.max(0) as u32, size.h.max(0) as u32).into());
        });
        surface.send_configure();
    }

    /// Re-configures every tracked lock surface to its output's current size.
    /// Called by the backends after an output resize so the lock surface
    /// always covers the whole output.
    pub fn reconfigure_lock_surfaces(&self) {
        for (output, surface) in &self.lock_surfaces {
            if surface.alive() {
                self.configure_lock_surface(surface, output);
            }
        }
    }

    /// The alive, committed lock surface for OUTPUT, or `None` -- which the
    /// render paths treat as "draw solid black", never desktop content.
    pub fn lock_surface_for(&self, output: &Output) -> Option<&LockSurface> {
        self.lock_surfaces
            .iter()
            .find(|(o, _)| o == output)
            .map(|(_, surface)| surface)
            .filter(|surface| surface.alive())
    }
}
