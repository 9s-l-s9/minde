// SPDX-License-Identifier: MIT
//! Resource identities supplement Smithay's surface/seat constraint lookup.
use std::{any::Any, collections::HashMap};

use smithay::{
    input::pointer::PointerHandle,
    reexports::{
        wayland_protocols::wp::pointer_constraints::zv1::server::zwp_pointer_constraints_v1::Request,
        wayland_server::{
            Client, DisplayHandle, backend::ObjectId, protocol::wl_surface::WlSurface,
        },
    },
    utils::{Logical, Point},
    wayland::pointer_constraints::{PointerConstraint, with_pointer_constraint},
};

use super::{constraint_region, surface_input_region};
use crate::MindeState;

pub(super) struct ConstraintRecord {
    surface: WlSurface,
    pointer: Option<PointerHandle<MindeState>>,
    current: bool,
    locked: bool,
    pub(super) origin: Option<Point<f64, Logical>>,
    pub(super) hint: Option<Point<f64, Logical>>,
}

#[derive(Default)]
pub(crate) struct ConstraintRecords(HashMap<ObjectId, ConstraintRecord>);

impl ConstraintRecords {
    pub(super) fn current(
        &self,
        surface: &WlSurface,
        pointer: &PointerHandle<MindeState>,
    ) -> Option<&ConstraintRecord> {
        self.0.values().find(|record| {
            record.current && &record.surface == surface && record.pointer.as_ref() == Some(pointer)
        })
    }

    pub(super) fn current_mut(
        &mut self,
        surface: &WlSurface,
        pointer: &PointerHandle<MindeState>,
    ) -> Option<&mut ConstraintRecord> {
        self.0.values_mut().find(|record| {
            record.current && &record.surface == surface && record.pointer.as_ref() == Some(pointer)
        })
    }
}

impl MindeState {
    pub(crate) fn track_pointer_constraint_request(
        &mut self,
        client: &Client,
        request: &dyn Any,
        handle: &DisplayHandle,
    ) {
        let Some(request) = request.downcast_ref::<Request>() else {
            return;
        };
        let (surface, pointer, locked, interface) = match request {
            Request::LockPointer {
                surface, pointer, ..
            } => (surface, pointer, true, "zwp_locked_pointer_v1"),
            Request::ConfinePointer {
                surface, pointer, ..
            } => (surface, pointer, false, "zwp_confined_pointer_v1"),
            _ => return,
        };
        // The backend allocates the request's new object before dispatch. New<I>
        // has no ID accessor, so find the sole unregistered child through the
        // public object iterator. Keep retired records until resource destruction
        // so this remains unambiguous even after a oneshot lock deactivates.
        let mut objects = Vec::new();
        let _ = handle
            .backend_handle()
            .with_all_objects_for(client.id(), |id| objects.push(id));
        let objects: Vec<_> = objects
            .into_iter()
            .filter(|id| {
                id.interface().name == interface
                    && !self.pointer_constraint_records.0.contains_key(id)
            })
            .collect();
        let [id] = objects.as_slice() else {
            tracing::warn!(
                interface,
                "could not identify new pointer constraint resource"
            );
            return;
        };
        let pointer = PointerHandle::from_resource(pointer);
        let current = pointer.as_ref().is_some_and(|pointer| {
            with_pointer_constraint(surface, pointer, |constraint| constraint.is_none())
        });
        if current {
            for record in self.pointer_constraint_records.0.values_mut() {
                if &record.surface == surface && record.pointer == pointer {
                    record.current = false;
                    record.hint = None;
                }
            }
        }
        self.pointer_constraint_records.0.insert(
            id.clone(),
            ConstraintRecord {
                surface: surface.clone(),
                pointer,
                current,
                locked,
                origin: None,
                hint: None,
            },
        );
    }

    pub(crate) fn pointer_constraint_request_allowed(&self, id: &ObjectId) -> bool {
        self.pointer_constraint_records
            .0
            .get(id)
            .is_none_or(|record| record.current)
    }

    /// Snapshot the active lock before Smithay removes its entry. A retired or
    /// rejected object's destructor must not remove a newer surface/seat entry.
    pub(crate) fn remove_pointer_constraint_record(
        &mut self,
        id: &ObjectId,
    ) -> Option<(bool, Option<Point<f64, Logical>>)> {
        let record = self.pointer_constraint_records.0.remove(id)?;
        let hint = (|| {
            if !record.current || !record.locked {
                return None;
            }
            let pointer = record.pointer.as_ref()?;
            if pointer.current_focus().as_ref() != Some(&record.surface) {
                return None;
            }
            let hint = record.hint?;
            let origin = self
                .surface_under(self.pointer_location)
                .filter(|(surface, _)| surface == &record.surface)
                .map(|(_, origin)| origin)
                .or(record.origin)?;
            let input = surface_input_region(&record.surface);
            if !input.iter().any(|rect| rect.to_f64().contains(hint)) {
                return None;
            }
            let active = with_pointer_constraint(&record.surface, pointer, |constraint| {
                constraint.is_some_and(|constraint| {
                    constraint.is_active()
                        && matches!(&*constraint, PointerConstraint::Locked(_))
                        && constraint_region(&input, constraint.region())
                            .iter()
                            .any(|rect| rect.to_f64().contains(self.pointer_location - origin))
                })
            });
            active.then_some(origin + hint)
        })();
        Some((record.current, hint))
    }
}
