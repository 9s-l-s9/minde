// SPDX-License-Identifier: MIT
//! Physical absolute-device output associations and coordinate mapping.
use std::collections::HashMap;

use smithay::{
    desktop::{Space, Window},
    utils::{Logical, Point},
};

use crate::{guile::InputRule, state::clamp_point_to_rectangles};

struct Mapping {
    configured: Option<String>,
    associated: Option<String>,
}

#[derive(Default)]
pub(crate) struct AbsoluteInputMappings(HashMap<String, Mapping>);

impl AbsoluteInputMappings {
    /// Cache metadata and matching rules when a libinput device arrives or
    /// configuration changes, rather than querying libinput on every motion.
    pub(crate) fn configure(
        &mut self,
        id: String,
        name: &str,
        associated: Option<String>,
        rules: &[InputRule],
    ) {
        let configured = rules
            .iter()
            .filter(|rule| name.contains(&rule.match_name))
            .filter_map(|rule| rule.map_to_output.as_ref())
            .next_back()
            .filter(|name| !name.is_empty())
            .cloned();
        self.0.insert(
            id,
            Mapping {
                configured,
                associated,
            },
        );
    }

    pub(crate) fn remove(&mut self, id: &str) {
        self.0.remove(id);
    }

    pub(crate) fn position(
        &self,
        space: &Space<Window>,
        id: &str,
        normalized: Point<f64, Logical>,
    ) -> Option<Point<f64, Logical>> {
        if !normalized.x.is_finite() || !normalized.y.is_finite() {
            return None;
        }
        let mapping = self.0.get(id);
        let output = if let Some(mapping) = mapping {
            let named = |name: &String| {
                space.outputs().find(|output| {
                    output.name() == *name && space.output_geometry(output).is_some()
                })
            };
            mapping
                .configured
                .as_ref()
                .and_then(named)
                .or_else(|| mapping.associated.as_ref().and_then(named))
                .or_else(|| {
                    space
                        .outputs()
                        .filter(|output| {
                            internal_panel(&output.name())
                                && space.output_geometry(output).is_some()
                        })
                        .min_by_key(|output| output.name())
                })
                .or_else(|| {
                    space
                        .outputs()
                        .filter(|output| space.output_geometry(output).is_some())
                        .min_by_key(|output| output.name())
                })?
        } else {
            // Winit supplies window-relative coordinates. Its output's
            // Flipped180 is a framebuffer convention, not a physical panel
            // orientation, and must not flip host pointer/touch input.
            space
                .outputs()
                .find(|output| space.output_geometry(output).is_some())?
        };
        let geometry = space.output_geometry(output)?;
        if mapping.is_none() {
            // Keep unbounded host samples for relative deltas under a lock.
            // pointer_absolute_motion clamps the visible cursor separately.
            return Some(
                geometry.loc.to_f64()
                    + Point::from((
                        normalized.x * geometry.size.w as f64,
                        normalized.y * geometry.size.h as f64,
                    )),
            );
        }
        let transform = output.current_transform();
        let normalized: Point<f64, Logical> = transform.transform_point_in(
            (normalized.x.clamp(0.0, 1.0), normalized.y.clamp(0.0, 1.0)).into(),
            &(1.0, 1.0).into(),
        );
        let position = geometry.loc.to_f64()
            + Point::from((
                normalized.x * geometry.size.w as f64,
                normalized.y * geometry.size.h as f64,
            ));
        Some(clamp_point_to_rectangles(position, [geometry]))
    }
}

fn internal_panel(name: &str) -> bool {
    ["eDP-", "LVDS-", "DSI-"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

#[cfg(test)]
mod tests;
