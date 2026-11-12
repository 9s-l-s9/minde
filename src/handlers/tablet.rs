// SPDX-License-Identifier: MIT
//! Cache tablet awareness at binding/lifecycle boundaries, not per sample.
use std::{any::Any, collections::HashMap};

use smithay::reexports::{
    wayland_protocols::wp::tablet::zv2::server::zwp_tablet_manager_v2::Request,
    wayland_server::{
        DisplayHandle, Resource,
        backend::{ClientId, ObjectId},
        protocol::wl_surface::WlSurface,
    },
};

use crate::MindeState;

#[derive(Default)]
pub(crate) struct TabletClients {
    aware: HashMap<ClientId, CachedAwareness>,
    #[cfg(test)]
    scans: usize,
}

struct CachedAwareness {
    aware: bool,
    surface: ObjectId,
}

impl TabletClients {
    pub(crate) fn aware(&mut self, handle: &DisplayHandle, surface: &WlSurface) -> bool {
        let Ok(client) = handle.get_client(surface.id()) else {
            return false;
        };
        if let Some(aware) = self.aware.get(&client.id()) {
            return aware.aware;
        }
        #[cfg(test)]
        {
            self.scans += 1;
        }
        let mut aware = false;
        // interface() uses object metadata and does not re-enter the backend
        // lock held during enumeration. Cache negative results as well.
        if handle
            .backend_handle()
            .with_all_objects_for(client.id(), |object| {
                aware |= object.interface().name == "zwp_tablet_tool_v2";
            })
            .is_ok()
        {
            self.aware.insert(
                client.id(),
                CachedAwareness {
                    aware,
                    surface: surface.id(),
                },
            );
        }
        aware
    }

    pub(crate) fn request(&mut self, client: &ClientId, request: &dyn Any) {
        // GetTabletSeat creates instances of tools that already exist. Merely
        // binding the manager does not make a client tablet-aware.
        if matches!(
            request.downcast_ref::<Request>(),
            Some(Request::GetTabletSeat { .. })
        ) {
            self.aware.remove(client);
        }
    }

    pub(crate) fn destroyed<I: Resource>(&mut self, client: &ClientId, resource: &I) {
        if matches!(I::interface().name, "zwp_tablet_tool_v2" | "wl_surface") {
            let removed = self.aware.remove(client);
            if removed.is_none() && I::interface().name == "wl_surface" {
                // libwayland may already have discarded the client identity
                // during disconnect. The surface that established the cache
                // still has a stable object ID in its destructor. This also
                // covers Xwayland's foreign ClientData without retaining a
                // surface or scanning protocol objects in the input path.
                let surface = resource.id();
                self.aware.retain(|_, entry| entry.surface != surface);
            }
        }
    }

    pub(crate) fn tool_added(&mut self) {
        // Smithay creates instances for existing tablet seats when a
        // previously unknown tool first enters proximity.
        self.aware.clear();
    }
}

impl MindeState {
    pub(crate) fn client_is_tablet_aware(&mut self, surface: &WlSurface) -> bool {
        self.tablet_clients.aware(&self.display_handle, surface)
    }
}

#[cfg(test)]
mod tests;
