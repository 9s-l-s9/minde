// SPDX-License-Identifier: MIT
//! Preferred scale follows a window's committed geometry and its whole family
//! of popup/subsurface children. Newly assigned roles refresh their own tree.
use smithay::{
    desktop::{Space, Window, WindowSurfaceType, find_popup_root_surface, layer_map_for_output},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Rectangle},
    wayland::{
        compositor::{
            SurfaceData, TraversalAction, get_parent, with_states, with_surface_tree_downward,
        },
        fractional_scale::{FractionalScaleHandler, with_fractional_scale},
        seat::WaylandFocus,
    },
};

use crate::{
    MindeState,
    state::{largest_output_overlap, window_for_surface},
};

#[cfg(test)]
mod tests;

fn scale_for_geometry(space: &Space<Window>, geometry: Option<Rectangle<i32, Logical>>) -> f64 {
    geometry
        .and_then(|rect| {
            largest_output_overlap(
                rect,
                space
                    .outputs()
                    .filter_map(|o| Some((o, space.output_geometry(o)?))),
            )
        })
        .or_else(|| space.outputs().next())
        .map_or(1.0, |o| o.current_scale().fractional_scale())
}

fn set_scale(states: &SurfaceData, scale: f64) {
    with_fractional_scale(states, |fractional| fractional.set_preferred_scale(scale));
}

impl FractionalScaleHandler for MindeState {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        let scale = self.output_scale_for_surface(&surface);
        with_states(&surface, |states| set_scale(states, scale));
    }
}

impl MindeState {
    pub(crate) fn output_scale_for_surface(&self, surface: &WlSurface) -> f64 {
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        if let Some(popup) = self.popups.find_popup(&root)
            && let Ok(parent) = find_popup_root_surface(&popup)
        {
            root = parent;
        }
        if let Some(window) = window_for_surface(&root) {
            return scale_for_geometry(&self.space, self.space.element_geometry(&window));
        }
        for output in self.space.outputs() {
            if layer_map_for_output(output)
                .layer_for_surface(&root, WindowSurfaceType::TOPLEVEL)
                .is_some()
            {
                return output.current_scale().fractional_scale();
            }
        }
        scale_for_geometry(&self.space, None)
    }

    /// Called after a popup, layer or subsurface acquires its parent/output.
    /// It may have bound fractional-scale earlier, before that role existed.
    pub(crate) fn update_surface_fractional_scale(&self, surface: &WlSurface) {
        let scale = self.output_scale_for_surface(surface);
        with_surface_tree_downward(
            surface,
            (),
            |_, _, _| TraversalAction::DoChildren(()),
            |_, states, _| set_scale(states, scale),
            |_, _, _| true,
        );
    }

    /// Geometry lookup is immediate: Space's cached output memberships may
    /// still describe the previous placement until its next refresh.
    pub(crate) fn update_window_fractional_scale(&self, window: &Window) {
        let Some(surface) = window.wl_surface() else {
            return;
        };
        let scale = scale_for_geometry(&self.space, self.space.element_geometry(window));
        let unchanged = with_states(&surface, |states| {
            with_fractional_scale(states, |fractional| {
                fractional.preferred_scale() == Some(scale)
            })
        });
        if !unchanged {
            // Includes nested popups and each surface's subsurface tree.
            window.with_surfaces(|_, states| set_scale(states, scale));
        }
    }

    /// Output reconfiguration can change every owner's scale; placement and
    /// buffer commits refresh only the affected window.
    pub fn update_fractional_scales(&self) {
        for window in self.space.elements() {
            self.update_window_fractional_scale(window);
        }
        for output in self.space.outputs() {
            let scale = output.current_scale().fractional_scale();
            for layer in layer_map_for_output(output).layers() {
                layer.with_surfaces(|_, states| set_scale(states, scale));
            }
        }
    }
}
