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

use smithay::backend::renderer::utils::with_renderer_surface_state;
use smithay::input::pointer::PointerHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::compositor::{
    RectangleKind, RegionAttributes, SurfaceAttributes, with_states,
};
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
    /// but still forward relative motion. `under` is the surface (and its
    /// global origin) under the pointer *before* the move -- the constraint
    /// follows pointer focus; the caller already has it for relative motion.
    pub fn constrain_pointer(
        &self,
        under: Option<&(WlSurface, Point<f64, Logical>)>,
        proposed: Point<f64, Logical>,
    ) -> (Point<f64, Logical>, bool) {
        let Some(pointer) = self.seat.get_pointer() else {
            return (proposed, false);
        };
        let Some((surface, origin)) = under else {
            return (proposed, false);
        };
        let origin = *origin;
        let current = self.pointer_location;
        let mut out = (proposed, false);
        // Smithay holds the surface-state lock inside with_pointer_constraint;
        // collect input geometry first, never re-enter with_states there.
        let input_region = surface_input_region(surface);
        with_pointer_constraint(surface, &pointer, |constraint| {
            let Some(constraint) = constraint else {
                return;
            };
            if !constraint.is_active() {
                return;
            }
            let region = constraint_region(&input_region, constraint.region());
            let local = current - origin;
            // A committed input/constraint region can exclude the parked
            // pointer. Release it instead of retaining an impossible grab.
            if !region.iter().any(|r| r.to_f64().contains(local)) {
                constraint.deactivate();
                return;
            }
            match &*constraint {
                PointerConstraint::Locked(_) => out = (current, true),
                PointerConstraint::Confined(_) => {
                    out = (
                        confine_motion(local, proposed - origin, &region) + origin,
                        false,
                    );
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
        // Smithay holds the surface-state lock inside with_pointer_constraint;
        // collect input geometry first, never re-enter with_states there.
        let input_region = surface_input_region(surface);
        with_pointer_constraint(surface, &pointer, |constraint| {
            if let Some(constraint) = constraint
                && !constraint.is_active()
            {
                let local = loc - *origin;
                let inside = constraint_region(&input_region, constraint.region())
                    .iter()
                    .any(|r| r.to_f64().contains(local));
                if inside {
                    constraint.activate();
                }
            }
        });
    }
}

/// The effective region is the intersection of the surface bounds, its
/// committed input region, and the optional constraint region. A null region
/// means the whole input region, not "unconfined".
fn surface_input_region(surface: &WlSurface) -> Vec<Rectangle<i32, Logical>> {
    let Some(size) = with_renderer_surface_state(surface, |data| data.surface_size()).flatten()
    else {
        return Vec::new();
    };
    let input = with_states(surface, |states| {
        states
            .cached_state
            .get::<SurfaceAttributes>()
            .current()
            .input_region
            .clone()
    });
    region_rectangles(Rectangle::from_size(size), input.as_ref())
}

fn constraint_region(
    input: &[Rectangle<i32, Logical>],
    constraint: Option<&RegionAttributes>,
) -> Vec<Rectangle<i32, Logical>> {
    input
        .iter()
        .flat_map(|r| region_rectangles(*r, constraint))
        .collect()
}

/// Preserve ordered add/subtract operations; a bounding box would fill holes
/// and join disjoint islands, allowing the pointer through forbidden space.
fn region_rectangles(
    bounds: Rectangle<i32, Logical>,
    region: Option<&RegionAttributes>,
) -> Vec<Rectangle<i32, Logical>> {
    let Some(region) = region else {
        return vec![bounds];
    };
    let mut rectangles = Vec::new();
    for (kind, rect) in &region.rects {
        let Some(rect) = bounds.intersection(*rect) else {
            continue;
        };
        match kind {
            RectangleKind::Add => rectangles.push(rect),
            RectangleKind::Subtract => {
                rectangles = Rectangle::subtract_rects_many(rectangles, [rect]);
            }
        }
    }
    rectangles
}

/// Clip the motion segment at the first exit from the union. In particular,
/// a large delta cannot jump across a hole into another valid rectangle.
fn confine_motion(
    current: Point<f64, Logical>,
    proposed: Point<f64, Logical>,
    region: &[Rectangle<i32, Logical>],
) -> Point<f64, Logical> {
    let clipped = clip_motion(current, proposed, region);
    // Retain tangential movement when pushing against an edge, as games
    // need to reach corners for edge scrolling.
    let horizontal = clip_motion(clipped, (proposed.x, clipped.y).into(), region);
    clip_motion(horizontal, (horizontal.x, proposed.y).into(), region)
}

fn clip_motion(
    current: Point<f64, Logical>,
    proposed: Point<f64, Logical>,
    region: &[Rectangle<i32, Logical>],
) -> Point<f64, Logical> {
    let delta = proposed - current;
    if delta == Point::from((0.0, 0.0)) {
        return current;
    }
    let mut intervals = Vec::new();
    for rect in region {
        let mut enter: f64 = 0.0;
        let mut leave: f64 = 1.0;
        for (p, d, lo, hi) in [
            (
                current.x,
                delta.x,
                rect.loc.x as f64,
                (rect.loc.x + rect.size.w) as f64,
            ),
            (
                current.y,
                delta.y,
                rect.loc.y as f64,
                (rect.loc.y + rect.size.h) as f64,
            ),
        ] {
            if d == 0.0 {
                if p < lo || p >= hi {
                    leave = -1.0;
                }
            } else {
                let a = (lo - p) / d;
                let b = (hi - p) / d;
                enter = enter.max(a.min(b));
                leave = leave.min(a.max(b));
            }
        }
        if enter <= leave {
            intervals.push((enter, leave));
        }
    }
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut end: f64 = 0.0;
    for (enter, leave) in intervals {
        if enter > end {
            break;
        }
        end = end.max(leave);
    }
    let at = current + Point::from((delta.x * end, delta.y * end));
    if end == 1.0 && region.iter().any(|r| r.to_f64().contains(at)) {
        return at;
    }
    // wl_pointer coordinates have 1/256 pixel precision. Stay a full protocol
    // unit inside exclusive right/bottom edges, even after wire conversion.
    let inset = (1.0 / 256.0) / delta.x.abs().max(delta.y.abs());
    current
        + Point::from((
            delta.x * (end - inset).max(0.0),
            delta.y * (end - inset).max(0.0),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Rectangle<i32, Logical> {
        Rectangle::new((0, 0).into(), (200, 200).into())
    }

    #[test]
    fn null_region_confines_to_surface_edges() {
        let region = region_rectangles(bounds(), None);
        let result = confine_motion((50.0, 50.0).into(), (800.0, 50.0).into(), &region);
        assert!(result.x > 199.0 && result.x < 200.0);
        assert_eq!(result.y, 50.0);
        assert!(bounds().to_f64().contains(result));
    }

    #[test]
    fn movement_slides_along_an_edge() {
        let result = confine_motion(
            (199.99609375, 50.0).into(),
            (220.0, 90.0).into(),
            &[bounds()],
        );
        assert!(result.x < 200.0);
        assert_eq!(result.y, 90.0);
    }

    #[test]
    fn exclusive_edge_is_inside_at_wire_precision() {
        let result = confine_motion((50.0, 50.0).into(), (200.0, 200.0).into(), &[bounds()]);
        assert!(result.x <= 200.0 - 1.0 / 256.0);
        assert!(result.y <= 200.0 - 1.0 / 256.0);
    }

    #[test]
    fn region_holes_and_disjoint_islands_cannot_be_crossed() {
        let region = RegionAttributes {
            rects: vec![
                (RectangleKind::Add, bounds()),
                (
                    RectangleKind::Subtract,
                    Rectangle::new((80, 0).into(), (40, 200).into()),
                ),
            ],
        };
        let rectangles = region_rectangles(bounds(), Some(&region));
        // Both the hole and a valid point on its far side must stop at x=80.
        for x in [100.0, 150.0, 800.0] {
            let result = confine_motion((50.0, 50.0).into(), (x, 50.0).into(), &rectangles);
            assert!(result.x > 79.0 && result.x < 80.0);
        }
    }

    #[test]
    fn adjacent_rectangles_do_not_create_artificial_barriers() {
        let rects = [
            Rectangle::new((0, 0).into(), (100, 200).into()),
            Rectangle::new((100, 0).into(), (100, 200).into()),
        ];
        assert_eq!(
            confine_motion((50.0, 50.0).into(), (150.0, 50.0).into(), &rects),
            (150.0, 50.0).into()
        );
    }

    #[test]
    fn regions_are_clipped_to_surface_and_add_after_subtract_is_preserved() {
        let region = RegionAttributes {
            rects: vec![
                (
                    RectangleKind::Add,
                    Rectangle::new((-100, -100).into(), (500, 500).into()),
                ),
                (RectangleKind::Subtract, bounds()),
                (
                    RectangleKind::Add,
                    Rectangle::new((10, 10).into(), (20, 20).into()),
                ),
            ],
        };
        assert_eq!(
            region_rectangles(bounds(), Some(&region)),
            vec![Rectangle::new((10, 10).into(), (20, 20).into())]
        );
        assert!(region_rectangles(bounds(), Some(&RegionAttributes::default())).is_empty());
    }
}
