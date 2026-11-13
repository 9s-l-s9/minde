// SPDX-License-Identifier: MIT
//! Window eligibility for a DRM output's frame-callback pass.

use smithay::{
    desktop::{Space, space::SpaceElement},
    output::Output,
    utils::{Logical, Rectangle},
};

/// Keep powered-off outputs in the layout for visibility checks, but select
/// one rendering output to service windows parked outside every output.
pub(crate) struct FrameCallbacks<'a, E: SpaceElement> {
    space: &'a Space<E>,
    output_geometry: Option<Rectangle<i32, Logical>>,
    serves_parked: bool,
}

impl<'a, E: SpaceElement + PartialEq> FrameCallbacks<'a, E> {
    pub(crate) fn new(
        space: &'a Space<E>,
        output: &Output,
        can_render: impl Fn(&Output) -> bool,
    ) -> Self {
        Self {
            space,
            output_geometry: space.output_geometry(output).filter(|_| can_render(output)),
            serves_parked: space.outputs().find(|o| can_render(o)) == Some(output),
        }
    }

    pub(crate) fn windows(&self) -> impl Iterator<Item = &E> {
        self.space.elements().filter(|window| {
            let Some(output_geometry) = self.output_geometry else {
                return false;
            };
            let geo = self.space.element_geometry(window);
            let on_this = geo.is_some_and(|g| g.overlaps(output_geometry));
            // Visible windows already qualify. Only the selected rendering
            // output needs the more expensive parked-window check.
            on_this
                || (self.serves_parked
                    && geo
                        .map(|g| {
                            !self
                                .space
                                .outputs()
                                .filter_map(|o| self.space.output_geometry(o))
                                .any(|og| g.overlaps(og))
                        })
                        .unwrap_or(true))
        })
    }
}
