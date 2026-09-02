// SPDX-License-Identifier: MIT

use crate::{MindeState, grabs::resize_grab, state::ClientState};
use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    reexports::wayland_server::{
        Client,
        protocol::{wl_buffer, wl_surface::WlSurface},
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            CompositorClientState, CompositorHandler, CompositorState, get_parent,
            is_sync_subsurface,
        },
        shm::{ShmHandler, ShmState},
    },
};

use super::xdg_shell;

impl CompositorHandler for MindeState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        // The Xwayland connection is created by smithay itself and
        // carries XWaylandClientData instead of our ClientState.
        if let Some(state) = client.get_data::<smithay::xwayland::XWaylandClientData>() {
            return &state.compositor_state;
        }
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = crate::state::window_for_surface(&self.space, &root) {
                window.on_commit();
                // Title/app-id arrive (and change) via ordinary commits
                // after the map-time report, which is usually empty.
                self.report_title_if_changed(&window);
            }
        };

        xdg_shell::handle_commit(&mut self.popups, &self.space, surface);
        resize_grab::handle_commit(&mut self.space, surface);
        self.handle_layer_commit(surface);
        // Every commit may carry new content or a frame-callback request;
        // both need a render pass (see `udev` repaint scheduling).
        self.schedule_redraw();
    }
}

impl MindeState {
    /// Layer-surface commit handling: arrange the output's layer map,
    /// send the initial configure if this was the first commit (required
    /// by the protocol before the client may attach a buffer), track
    /// exclusive-zone changes, and give exclusive-keyboard layers
    /// (fuzzel, swaylock) the keyboard.
    fn handle_layer_commit(&mut self, surface: &WlSurface) {
        use smithay::desktop::{WindowSurfaceType, layer_map_for_output};
        use smithay::wayland::compositor::with_states;
        use smithay::wayland::shell::wlr_layer::{
            KeyboardInteractivity, Layer, LayerSurfaceCachedState, LayerSurfaceData,
        };

        let Some(output) = self
            .space
            .outputs()
            .find(|o| {
                layer_map_for_output(o)
                    .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                    .is_some()
            })
            .cloned()
        else {
            return;
        };

        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<LayerSurfaceData>()
                .map(|d| d.lock().unwrap().initial_configure_sent)
                .unwrap_or(true)
        });

        // Arrange (once: `update_usable_area` does it for every output)
        // before the initial configure so the configure carries the size the
        // anchors/margins produce.
        self.update_usable_area();
        if !initial_configure_sent
            && let Some(layer) = layer_map_for_output(&output)
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
        {
            layer.layer_surface().send_configure();
        }

        // Exclusive keyboard interactivity on a top/overlay layer takes
        // the keyboard (this is how fuzzel/swaylock type).
        let wants_keyboard = with_states(surface, |states| {
            let mut guard = states.cached_state.get::<LayerSurfaceCachedState>();
            let state = guard.current();
            state.keyboard_interactivity == KeyboardInteractivity::Exclusive
                && matches!(state.layer, Layer::Top | Layer::Overlay)
        });
        // While the session is locked, keyboard focus belongs to the lock
        // surface only -- never let a layer client (a regular client) grab it.
        if wants_keyboard
            && !self.locked
            && let Some(keyboard) = self.seat.get_keyboard()
            && keyboard.current_focus().as_ref() != Some(surface)
        {
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            keyboard.set_focus(self, Some(surface.clone()), serial);
        }
    }
}

impl BufferHandler for MindeState {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for MindeState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}
