// SPDX-License-Identifier: MIT

//! `zwp_pointer_constraints_v1` (pointer lock / confinement) and
//! `zwp_relative_pointer_manager_v1` glue.
//!
//! The protocol state and dispatch live in Smithay's
//! `wayland::pointer_constraints` and `wayland::relative_pointer` modules;
//! the globals are registered in [`MindeState::new`] and dispatched
//! through the crate-wide `delegate_dispatch2!` blanket. This file supplies
//! the one handler trait Smithay requires plus the input-path integration:
//! activating a constraint when the pointer enters its surface, keeping a
//! locked pointer parked (while still forwarding relative motion so games
//! read deltas), clamping a confined pointer to its region, and warping to
//! the client's cursor-position hint on unlock.
//!
//! No Scheme policy surface: pointer constraints are per-surface client
//! requests activated by pointer focus, not compositor policy, so there is
//! nothing to route through `(minde ...)`.

use smithay::input::pointer::PointerHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::compositor::{RectangleKind, RegionAttributes};
use smithay::wayland::pointer_constraints::{
    PointerConstraint, PointerConstraintsHandler, with_pointer_constraint,
};

use crate::state::MindeState;

impl PointerConstraintsHandler for MindeState {
    fn new_constraint(&mut self, surface: &WlSurface, _pointer: &PointerHandle<Self>) {
        // Per the protocol the constraint takes effect once the surface has
        // pointer focus; if the pointer is already over it, activate now.
        self.activate_pointer_constraint_if_entered(&Some((
            surface.clone(),
            match self.surface_under(self.pointer_location) {
                Some((s, origin)) if &s == surface => origin,
                _ => return,
            },
        )));
    }

    fn remove_constraint(&mut self, surface: &WlSurface, _pointer: &PointerHandle<Self>) {
        // Honor the locked position hint: on unlock, move the cursor to
        // wherever the client was rendering it (defaulting to leaving it
        // parked if no hint was committed for this surface).
        if let Some((hint_surface, hint)) = self.pointer_lock_hint.take()
            && &hint_surface == surface
            && let Some((s, origin)) = self.surface_under(self.pointer_location)
            && &s == surface
        {
            self.warp_pointer(origin + hint);
        }
    }

    fn cursor_position_hint(
        &mut self,
        surface: &WlSurface,
        _pointer: &PointerHandle<Self>,
        location: Point<f64, Logical>,
    ) {
        // Surface-local hint; resolved to global coordinates on unlock.
        self.pointer_lock_hint = Some((surface.clone(), location));
    }
}

impl MindeState {
    /// Apply any active pointer constraint on the surface currently holding
    /// pointer focus to a proposed new pointer location. Returns the
    /// (possibly clamped) location and whether the pointer is *locked* in
    /// place -- in which case the caller must suppress the absolute motion
    /// but still forward relative motion.
    pub fn constrain_pointer(&self, proposed: Point<f64, Logical>) -> (Point<f64, Logical>, bool) {
        let Some(pointer) = self.seat.get_pointer() else {
            return (proposed, false);
        };
        // The constraint follows pointer focus: it applies to the surface the
        // pointer is currently over, evaluated before the move.
        let Some((surface, origin)) = self.surface_under(self.pointer_location) else {
            return (proposed, false);
        };
        let current = self.pointer_location;
        let mut out = (proposed, false);
        with_pointer_constraint(&surface, &pointer, |constraint| {
            let Some(constraint) = constraint else {
                return;
            };
            if !constraint.is_active() {
                return;
            }
            match &*constraint {
                PointerConstraint::Locked(_) => out = (current, true),
                PointerConstraint::Confined(confined) => {
                    if let Some(region) = confined.region() {
                        let local = proposed - origin;
                        if !region_contains(region, local) {
                            out = (clamp_local_to_region(local, region) + origin, false);
                        }
                    }
                }
            }
        });
        out
    }

    /// Activate the constraint on the surface the pointer just entered, if it
    /// is present, inactive, and the pointer is inside its region. Called
    /// after every real pointer move.
    pub fn activate_pointer_constraint_if_entered(
        &mut self,
        under: &Option<(WlSurface, Point<f64, Logical>)>,
    ) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let Some((surface, origin)) = under else {
            return;
        };
        let loc = self.pointer_location;
        with_pointer_constraint(surface, &pointer, |constraint| {
            if let Some(constraint) = constraint
                && !constraint.is_active()
            {
                let local = loc - *origin;
                let inside = constraint
                    .region()
                    .is_none_or(|region| region_contains(region, local));
                if inside {
                    constraint.activate();
                }
            }
        });
    }
}

fn region_contains(region: &RegionAttributes, local: Point<f64, Logical>) -> bool {
    region.contains((local.x.round() as i32, local.y.round() as i32))
}

/// Clamp a surface-local point into the bounding box of a region's additive
/// rectangles. A confinement region can be an arbitrary add/subtract shape;
/// clamping to the additive bounding box keeps the pointer on the surface
/// without solving nearest-point-in-region, which is enough to keep games
/// and lock clients from escaping.
fn clamp_local_to_region(
    local: Point<f64, Logical>,
    region: &RegionAttributes,
) -> Point<f64, Logical> {
    let mut bbox: Option<Rectangle<i32, Logical>> = None;
    for (kind, rect) in &region.rects {
        if matches!(kind, RectangleKind::Add) {
            bbox = Some(match bbox {
                Some(b) => b.merge(*rect),
                None => *rect,
            });
        }
    }
    let Some(b) = bbox else {
        return local;
    };
    let x = local.x.clamp(b.loc.x as f64, (b.loc.x + b.size.w) as f64);
    let y = local.y.clamp(b.loc.y as f64, (b.loc.y + b.size.h) as f64);
    (x, y).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_rect(x: i32, y: i32, w: i32, h: i32) -> RegionAttributes {
        RegionAttributes {
            rects: vec![(
                RectangleKind::Add,
                Rectangle::new((x, y).into(), (w, h).into()),
            )],
        }
    }

    #[test]
    fn confinement_clamps_into_the_region_bbox() {
        let region = add_rect(10, 20, 100, 50);
        // Left/top escape clamps to the region origin.
        assert_eq!(
            clamp_local_to_region((-5.0, 5.0).into(), &region),
            (10.0, 20.0).into()
        );
        // Right/bottom escape clamps to the far edge.
        assert_eq!(
            clamp_local_to_region((500.0, 500.0).into(), &region),
            (110.0, 70.0).into()
        );
        // Inside is unchanged.
        assert_eq!(
            clamp_local_to_region((50.0, 40.0).into(), &region),
            (50.0, 40.0).into()
        );
    }

    #[test]
    fn region_membership_rounds_to_the_nearest_pixel() {
        let region = add_rect(0, 0, 10, 10);
        assert!(region_contains(&region, (9.4, 0.0).into()));
        assert!(!region_contains(&region, (10.6, 0.0).into()));
    }
}
