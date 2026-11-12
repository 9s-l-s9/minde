// SPDX-License-Identifier: MIT

//! wlr-layer-shell: panels (eww), launchers (fuzzel), wallpaper (swaybg),
//! lockers (swaylock). Layer surfaces live in each output's `LayerMap`,
//! not in the window `Space`; exclusive zones shrink the usable area the
//! Scheme frame tree tiles into (see `MindeState::update_usable_area`).
//! Adapted from Smithay's `anvil` example (see README for the revision).

use smithay::{
    backend::renderer::utils::with_renderer_surface_state,
    desktop::{LayerSurface, PopupManager, layer_map_for_output},
    output::Output,
    reexports::wayland_server::protocol::{wl_output, wl_surface::WlSurface},
    wayland::seat::WaylandFocus,
    wayland::shell::wlr_layer::{
        KeyboardInteractivity, Layer, LayerSurface as WlrLayerSurface, WlrLayerShellHandler,
        WlrLayerShellState,
    },
    wayland::shell::xdg::PopupSurface,
};

use crate::MindeState;

impl WlrLayerShellHandler for MindeState {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        wl_output: Option<wl_output::WlOutput>,
        layer: Layer,
        namespace: String,
    ) {
        let output = match wl_output.as_ref() {
            Some(resource) => Output::from_resource(resource).filter(|o| self.output_enabled(o)),
            None => self.space.outputs().next().cloned(),
        };
        let Some(output) = output else {
            tracing::warn!(namespace, "layer surface has no enabled output; closing");
            surface.send_close();
            return;
        };
        let mapped_layer = LayerSurface::new(surface, namespace.clone());
        if let Err(err) = layer_map_for_output(&output).map_layer(&mapped_layer) {
            tracing::warn!(?err, "failed to map layer surface");
            mapped_layer.layer_surface().send_close();
            return;
        }
        self.update_surface_fractional_scale(mapped_layer.wl_surface());
        tracing::info!(
            component = "layer-shell",
            namespace,
            ?layer,
            "layer surface mapped"
        );
        self.update_usable_area();
        self.schedule_redraw();
    }

    fn new_popup(&mut self, _parent: WlrLayerSurface, popup: PopupSurface) {
        // xdg_surface.get_popup had no parent. Layer-shell assigns it here,
        // before the popup's initial commit and configure handshake.
        self.unconstrain_popup(&popup);
        self.update_surface_fractional_scale(popup.wl_surface());
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let layer = self.space.outputs().find_map(|o| {
            let mut map = layer_map_for_output(o);
            let layer = map
                .layers()
                .find(|&layer| layer.layer_surface() == &surface)
                .cloned();
            if let Some(layer) = &layer {
                map.unmap_layer(layer);
            }
            layer
        });
        self.update_usable_area();
        self.schedule_redraw();

        if let Some(layer) = layer {
            self.release_layer_focus(&layer);
        } else if self.keyboard_focus().as_ref() == Some(surface.wl_surface()) {
            // The output may already have left Space before this destructor.
            self.restore_keyboard_focus();
        }
    }
}

impl MindeState {
    /// Return to the eligible layer owner before the selected desktop window.
    pub(crate) fn restore_keyboard_focus(&mut self) {
        if self.locked {
            self.enforce_lock_focus();
            return;
        }
        let focus = self.exclusive_layer_focus().or_else(|| {
            self.focused_window
                .as_ref()
                .and_then(|w| w.wl_surface().map(|s| s.into_owned()))
        });
        self.set_keyboard_focus(focus, smithay::utils::SERIAL_COUNTER.next_serial());
    }

    fn release_layer_focus(&mut self, layer: &LayerSurface) {
        let mut surfaces = Vec::new();
        layer.with_surfaces(|surface, _| surfaces.push(surface.clone()));
        let focused = self
            .keyboard_focus()
            .as_ref()
            .is_some_and(|s| surfaces.contains(s));
        let keyboard = self.seat.get_keyboard();
        let grabbed = keyboard
            .as_ref()
            .and_then(|k| k.grab_start_data())
            .and_then(|grab| grab.focus)
            .is_some_and(|focus| surfaces.contains(focus.surface()));
        let root = layer.wl_surface();
        for (popup, _) in PopupManager::popups_for_surface(root) {
            let _ = PopupManager::dismiss_popup(root, &popup);
        }
        if focused || grabbed {
            if let Some(keyboard) = keyboard {
                keyboard.unset_grab(self);
            }
            self.restore_keyboard_focus();
        }
    }

    /// Close layers only after an output is finally removed or disabled.
    /// Configuration rollback must retain their roles and focus ownership.
    pub(crate) fn close_output_layers(&mut self, output: &Output) {
        let layers = {
            let mut map = layer_map_for_output(output);
            let layers: Vec<_> = map.layers().cloned().collect();
            for layer in &layers {
                map.unmap_layer(layer);
            }
            layers
        };
        if layers.is_empty() {
            return;
        }
        for layer in layers {
            layer.layer_surface().send_close();
            self.release_layer_focus(&layer);
        }
        self.schedule_redraw();
    }

    /// Mapped exclusive Overlay surfaces take priority over Top surfaces.
    /// Within one layer keep its current owner, otherwise choose the most
    /// recently mapped candidate on the first enabled output with one.
    pub(crate) fn exclusive_layer_focus(&self) -> Option<WlSurface> {
        let current = self.keyboard_focus();
        for kind in [Layer::Overlay, Layer::Top] {
            let mut candidate = None;
            for output in self.space.outputs() {
                let map = layer_map_for_output(output);
                for layer in map.layers_on(kind).rev() {
                    let surface = layer.wl_surface();
                    if layer.cached_state().keyboard_interactivity
                        != KeyboardInteractivity::Exclusive
                        || !with_renderer_surface_state(surface, |state| state.buffer().is_some())
                            .unwrap_or(false)
                    {
                        continue;
                    }
                    if current.as_ref() == Some(surface) {
                        return Some(surface.clone());
                    }
                    candidate.get_or_insert_with(|| surface.clone());
                }
            }
            if candidate.is_some() {
                return candidate;
            }
        }
        None
    }
}

// Dispatch is covered by the blanket `smithay::delegate_dispatch2!` in
// handlers/mod.rs (same as xdg-decoration at this smithay revision).
