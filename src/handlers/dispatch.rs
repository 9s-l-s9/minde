// SPDX-License-Identifier: MIT
//! Smithay dispatch delegation with a session-lock ownership check.
//!
//! The pinned Smithay `SessionLockState` calls `unlock()` even after posting
//! `InvalidUnlock`. Intercept lock requests here so rejected or stale objects
//! cannot reach that callback. Keep this equivalent to `delegate_dispatch2!`
//! for all other protocols; no changes to the untracked vendor tree are needed.

use std::any::Any;

use smithay::{
    reexports::wayland_server::{
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        backend::ClientId, protocol::wl_surface,
    },
    utils::SERIAL_COUNTER,
    wayland::{Dispatch2, GlobalDispatch2},
};

use super::session_lock::{ExtSessionLockV1, LockRequest};
use crate::MindeState;

impl<I, UserData> Dispatch<I, UserData> for MindeState
where
    I: Resource + 'static,
    I::Request: 'static,
    UserData: Dispatch2<I, Self>,
{
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &I,
        request: I::Request,
        data: &UserData,
        dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        // Explicit surface destruction leaves the client's keyboard alive.
        // Send leave while the surface still exists; after destruction its ID
        // is no longer valid as a keyboard event argument.
        if state.locked
            && let Some(surface) = (resource as &dyn Any).downcast_ref::<wl_surface::WlSurface>()
            && matches!(
                (&request as &dyn Any).downcast_ref::<wl_surface::Request>(),
                Some(wl_surface::Request::Destroy)
            )
        {
            state
                .lock_surfaces
                .retain(|(_, lock)| lock.wl_surface() != surface);
            if let Some(keyboard) = state.seat.get_keyboard()
                && keyboard.current_focus().as_ref() == Some(surface)
            {
                keyboard.unset_grab(state);
                keyboard.set_focus(state, None, SERIAL_COUNTER.next_serial());
            }
            state.schedule_redraw();
        }
        if let Some(lock) = (resource as &dyn Any).downcast_ref::<ExtSessionLockV1>() {
            let request: Box<dyn Any> = Box::new(request);
            let request = *request
                .downcast::<LockRequest>()
                .expect("lock request type");
            state.dispatch_lock_request(client, lock, request, dhandle, data_init);
        } else {
            data.request(state, client, resource, request, dhandle, data_init);
        }
        // An input-method client can install a keyboard grab in a protocol
        // request, even without text-input focus. Desktop grabs must never
        // survive to the next key delivery while locked (including virtual keys).
        if state.locked {
            state.enforce_lock_focus();
        }
    }

    fn destroyed(state: &mut Self, client: ClientId, resource: &I, data: &UserData) {
        data.destroyed(state, client, resource);
        // A disconnect destroys resources one at a time. Sending keyboard
        // leave here can reference a surface already removed from the client.
        // Reconcile focus after dispatch has finished tearing the client down.
    }
}

impl<I, UserData> GlobalDispatch<I, UserData> for MindeState
where
    I: Resource + 'static,
    UserData: GlobalDispatch2<I, Self>,
{
    fn bind(
        state: &mut Self,
        dhandle: &DisplayHandle,
        client: &Client,
        resource: New<I>,
        data: &UserData,
        data_init: &mut DataInit<'_, Self>,
    ) {
        data.bind(state, dhandle, client, resource, data_init);
    }

    fn can_view(client: Client, data: &UserData) -> bool {
        data.can_view(&client)
    }
}
