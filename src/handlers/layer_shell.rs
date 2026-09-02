// SPDX-License-Identifier: MIT

//! wlr-layer-shell: panels (eww), launchers (fuzzel), wallpaper (swaybg),
//! lockers (swaylock). Layer surfaces live in each output's `LayerMap`,
//! not in the window `Space`; exclusive zones shrink the usable area the
//! Scheme frame tree tiles into (see `MindeState::update_usable_area`).
//! Adapted from Smithay's `anvil` example (see README for the revision).

use smithay::{
    desktop::{LayerSurface, layer_map_for_output},
    output::Output,
    reexports::wayland_server::protocol::wl_output,
    wayland::shell::wlr_layer::{
        Layer, LayerSurface as WlrLayerSurface, WlrLayerShellHandler, WlrLayerShellState,
    },
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
        let output = wl_output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.space.outputs().next().cloned());
        let Some(output) = output else {
            tracing::warn!(
                namespace,
                "layer surface before any output exists; ignoring"
            );
            return;
        };
        if let Err(err) =
            layer_map_for_output(&output).map_layer(&LayerSurface::new(surface, namespace.clone()))
        {
            tracing::warn!(?err, "failed to map layer surface");
        } else {
            tracing::info!(
                component = "layer-shell",
                namespace,
                ?layer,
                "layer surface mapped"
            );
        }
        self.update_usable_area();
        self.schedule_redraw();
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let mut had_keyboard_focus = false;
        if let Some((mut map, layer)) = self.space.outputs().find_map(|o| {
            let map = layer_map_for_output(o);
            let layer = map
                .layers()
                .find(|&layer| layer.layer_surface() == &surface)
                .cloned();
            layer.map(|layer| (map, layer))
        }) {
            if let Some(keyboard) = self.seat.get_keyboard() {
                had_keyboard_focus = keyboard.current_focus().as_ref() == Some(layer.wl_surface());
            }
            map.unmap_layer(&layer);
        }
        self.update_usable_area();
        self.schedule_redraw();

        // The layer (e.g. fuzzel) held the keyboard: hand focus back to
        // the frame tree's focused window.
        if had_keyboard_focus {
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            let surface = self
                .focused_window
                .as_ref()
                .and_then(|w| w.toplevel())
                .map(|t| t.wl_surface().clone());
            if let Some(keyboard) = self.seat.get_keyboard() {
                keyboard.set_focus(self, surface, serial);
            }
        }
    }
}

// Dispatch is covered by the blanket `smithay::delegate_dispatch2!` in
// handlers/mod.rs (same as xdg-decoration at this smithay revision).
