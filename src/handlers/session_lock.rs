// SPDX-License-Identifier: MIT

//! Session lock ownership, exclusive input and presentation acknowledgements.
//! `locked` guards rendering/input immediately. The protocol acknowledgement
//! and Scheme suspend hook wait until all active outputs have shown a locked
//! frame. An abandoned lock remains closed until another locker takes over.

use smithay::{
    output::Output,
    reexports::wayland_server::{
        Client, DataInit, DisplayHandle, Resource,
        backend::ClientId,
        protocol::{wl_output::WlOutput, wl_surface::WlSurface},
    },
    utils::SERIAL_COUNTER,
    wayland::{
        Dispatch2,
        seat::WaylandFocus,
        session_lock::{
            LockSurface, LockSurfaceConfigure, SessionLockHandler,
            SessionLockManagerState, SessionLocker, SessionLockState,
        },
    },
};
pub(super) use smithay::reexports::wayland_protocols::ext::session_lock::v1::server::
    ext_session_lock_v1::{ExtSessionLockV1, Request as LockRequest};
use smithay::reexports::wayland_protocols::ext::session_lock::v1::server::{
    ext_session_lock_surface_v1::ExtSessionLockSurfaceV1, ext_session_lock_v1::Error,
};

use crate::MindeState;

#[derive(Default)]
pub(crate) struct LockState {
    owner: Option<ExtSessionLockV1>,
    owner_client: Option<ClientId>,
    pending: Option<SessionLocker>,
    confirmed: bool,
    presentation: LockPresentation,
}

/// A generation tags the scene that was actually queued, rather than the
/// compositor's state when a delayed vblank happens to arrive.
#[derive(Default)]
struct LockPresentation {
    generation: u64,
    presented: Vec<Output>,
}

impl LockPresentation {
    fn reset(&mut self) {
        self.generation += 1;
        self.presented.clear();
    }

    fn presented(&mut self, output: &Output, generation: u64) {
        if generation == self.generation && !self.presented.contains(output) {
            self.presented.push(output.clone());
        }
    }

    fn ready(&self, outputs: &[Output]) -> bool {
        outputs.iter().all(|output| self.presented.contains(output))
    }
}

// Clients may pipeline get_lock_surface before receiving finished. Initialize
// those objects but give them no surface role, focus, or rendering authority.
struct RejectedLockSurface;
impl Dispatch2<ExtSessionLockSurfaceV1, MindeState> for RejectedLockSurface {
    fn request(
        &self,
        _state: &mut MindeState,
        _client: &Client,
        _resource: &ExtSessionLockSurfaceV1,
        _request: <ExtSessionLockSurfaceV1 as Resource>::Request,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, MindeState>,
    ) {
    }
}

impl SessionLockHandler for MindeState {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        // Dropping the confirmation sends finished. Ownership belongs to the
        // lock object, not merely its client (one client can create two locks).
        if self
            .session_lock
            .owner
            .as_ref()
            .is_some_and(Resource::is_alive)
        {
            return;
        }
        let client = confirmation
            .ext_session_lock()
            .client()
            .map(|client| client.id());
        // Smithay retains output bindings after an unconfirmed lock is
        // destroyed. Require a new connection to recover from abandonment;
        // accepting another object here would later fail on stale bindings.
        if client.is_some() && client == self.session_lock.owner_client {
            return;
        }
        self.session_lock.owner_client = client;
        self.session_lock.owner = Some(confirmation.ext_session_lock().clone());
        self.session_lock.pending = Some(confirmation);
        self.session_lock.presentation.reset();
        self.lock_surfaces.clear();
        self.locked = true;
        crate::guile::set_session_locked(true);

        self.cancel_key_repeat();
        self.enforce_lock_focus();
        self.set_text_input_focus(None);
        self.update_keyboard_shortcuts_inhibitors(None);
        let serial = SERIAL_COUNTER.next_serial();
        let time = self.start_time.elapsed().as_millis() as u32;
        if let Some(pointer) = self.seat.get_pointer() {
            pointer.unset_grab(self, serial, time);
            let location = self.pointer_location;
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
        if let Some(touch) = self.seat.get_touch() {
            touch.cancel(self);
            touch.unset_grab(self);
        }
        // Repaint through the normal backend scheduler. A desktop flip may
        // still be pending; its completion must not acknowledge this lock.
        self.schedule_lock_redraw();
    }

    fn unlock(&mut self) {
        // Only the owning, confirmed protocol object reaches this callback:
        // dispatch_lock_request checks it before Smithay's dispatch.
        crate::guile::set_session_lock_confirmed(false);
        self.locked = false;
        self.session_lock.owner = None;
        self.session_lock.owner_client = None;
        self.session_lock.pending = None;
        self.session_lock.confirmed = false;
        self.lock_surfaces.clear();
        self.schedule_redraw();
        crate::guile::set_session_locked(false);
        tracing::info!("session unlocked (ext-session-lock)");
        let serial = SERIAL_COUNTER.next_serial();
        let focus = self
            .focused_window
            .as_ref()
            .and_then(|w| w.wl_surface().map(|s| s.into_owned()));
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.unset_grab(self);
            keyboard.set_focus(self, focus.clone(), serial);
        }
        self.set_text_input_focus(focus);
        crate::guile::on_session_unlock();
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let Some(output) = Output::from_resource(&output) else {
            return;
        };
        self.configure_lock_surface(&surface, &output);
        self.lock_surfaces.push((output, surface));
        self.enforce_lock_focus();
        self.schedule_redraw();
    }

    fn ack_configure(&mut self, _surface: WlSurface, _configure: LockSurfaceConfigure) {}
}

impl MindeState {
    pub(super) fn dispatch_lock_request(
        &mut self,
        client: &Client,
        lock: &ExtSessionLockV1,
        request: LockRequest,
        handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        let owns_lock = self.locked && self.session_lock.owner.as_ref() == Some(lock);
        match request {
            LockRequest::UnlockAndDestroy if !owns_lock || self.session_lock.pending.is_some() => {
                // Smithay at our pinned revision posts this error but falls
                // through to unlock(). Never delegate an invalid unlock.
                lock.post_error(
                    Error::InvalidUnlock,
                    "lock is not the confirmed session owner",
                );
            }
            LockRequest::GetLockSurface { id, .. } if !owns_lock => {
                data_init.init(id, RejectedLockSurface);
            }
            LockRequest::GetLockSurface { ref output, .. }
                if Output::from_resource(output).is_some_and(|output| {
                    self.lock_surfaces
                        .iter()
                        .any(|(existing, _)| existing == &output)
                }) =>
            {
                // Two wl_output bindings can represent the same physical head.
                lock.post_error(Error::DuplicateOutput, "output already has a lock surface");
            }
            request => {
                let data = lock.data::<SessionLockState>().expect("Smithay lock state");
                data.request(self, client, lock, request, handle, data_init);
            }
        }
    }

    /// Drop desktop/IME grabs and preserve focus only on this owner's lock
    /// surfaces. Called at lock entry and after protocol requests, since an
    /// inactive IME can otherwise reinstall a grab while the session is locked.
    pub(crate) fn enforce_lock_focus(&mut self) {
        if !self.locked {
            return;
        }
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        keyboard.unset_grab(self);
        let current = keyboard.current_focus();
        let valid = |surface: &WlSurface| {
            self.session_lock
                .owner
                .as_ref()
                .is_some_and(|owner| owner.is_alive() && owner.client() == surface.client())
                && self
                    .lock_surfaces
                    .iter()
                    .any(|(_, lock)| lock.alive() && lock.wl_surface() == surface)
        };
        let focus = current.filter(&valid).or_else(|| {
            self.lock_surfaces
                .iter()
                .map(|(_, lock)| lock.wl_surface())
                .find(|surface| valid(surface))
                .cloned()
        });
        if keyboard.current_focus() != focus {
            keyboard.set_focus(self, focus, SERIAL_COUNTER.next_serial());
        }
    }

    pub(crate) fn lock_generation(&self) -> u64 {
        self.session_lock.presentation.generation
    }

    pub(crate) fn lock_frame_presented(&mut self, output: &Output, generation: u64) {
        if self.session_lock.pending.is_some() {
            self.session_lock.presentation.presented(output, generation);
        }
    }

    pub(crate) fn invalidate_lock_presentation(&mut self) {
        if self.session_lock.pending.is_some() {
            self.session_lock.presentation.reset();
            self.schedule_lock_redraw();
        }
    }

    /// Runs after event dispatch, when output hotplug/power changes and frame
    /// completions have settled. Removed/off DRM outputs no longer expose any
    /// content; newly active outputs must have their own presentation receipt.
    pub(crate) fn maybe_confirm_lock(&mut self) {
        if self.session_lock.pending.is_none() {
            return;
        }
        if !self
            .session_lock
            .owner
            .as_ref()
            .is_some_and(Resource::is_alive)
        {
            self.session_lock.pending = None;
            return; // stay locked after a client dies, even before confirmation
        }
        let Some(outputs) = self.lock_confirmation_outputs() else {
            return;
        };
        if !self.session_lock.presentation.ready(&outputs) {
            return;
        }
        self.session_lock.pending.take().unwrap().lock();
        if !self.session_lock.confirmed {
            self.session_lock.confirmed = true;
            crate::guile::set_session_lock_confirmed(true);
            tracing::info!("session locked (ext-session-lock)");
            crate::guile::on_session_lock();
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::output::{PhysicalProperties, Subpixel};

    fn output(name: &str) -> Output {
        Output::new(
            name.into(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "test".into(),
                model: "test".into(),
                serial_number: name.into(),
            },
        )
    }

    #[test]
    fn confirmation_waits_for_every_output_and_ignores_old_frames() {
        let outputs = [output("left"), output("right")];
        let mut progress = LockPresentation::default();
        progress.reset();
        assert!(!progress.ready(&outputs));
        progress.presented(&outputs[0], progress.generation - 1);
        progress.presented(&outputs[1], progress.generation);
        assert!(!progress.ready(&outputs));
        progress.presented(&outputs[0], progress.generation);
        assert!(progress.ready(&outputs));
    }

    #[test]
    fn takeover_and_output_reconfiguration_require_new_receipts() {
        let outputs = [output("screen")];
        let mut progress = LockPresentation::default();
        progress.reset();
        let old_generation = progress.generation;
        progress.presented(&outputs[0], old_generation);
        assert!(progress.ready(&outputs));
        progress.reset();
        progress.presented(&outputs[0], old_generation);
        assert!(!progress.ready(&outputs));
        progress.presented(&outputs[0], progress.generation);
        assert!(progress.ready(&outputs));
    }

    #[test]
    fn hotplug_requires_new_output_and_removal_does_not_wait_for_dead_head() {
        let outputs = [output("original"), output("hotplugged")];
        let mut progress = LockPresentation::default();
        progress.reset();
        progress.presented(&outputs[0], progress.generation);
        assert!(progress.ready(&outputs[..1]));
        assert!(!progress.ready(&outputs));
        assert!(progress.ready(&outputs[..1]));
        assert!(progress.ready(&[]));
    }
}
