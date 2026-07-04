use smithay::wayland::seat::WaylandFocus;
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
            CompositorClientState, CompositorHandler, CompositorState, get_parent, is_sync_subsurface,
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
            let window = self
                .space
                .elements()
                .find(|w| w.wl_surface().map(|s| *s == root).unwrap_or(false))
                .cloned();
            if let Some(window) = window {
                window.on_commit();
                // Title/app-id arrive (and change) via ordinary commits
                // after the map-time report, which is usually empty.
                self.report_title_if_changed(&window);
            }
        };

        xdg_shell::handle_commit(&mut self.popups, &self.space, surface);
        resize_grab::handle_commit(&mut self.space, surface);
        self.handle_layer_commit(surface);
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

        {
            let mut map = layer_map_for_output(&output);
            // Arrange before the initial configure so the configure
            // carries the size the anchors/margins produce.
            map.arrange();
            if !initial_configure_sent {
                if let Some(layer) = map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL) {
                    layer.layer_surface().send_configure();
                }
            }
        }
        self.update_usable_area();

        // Exclusive keyboard interactivity on a top/overlay layer takes
        // the keyboard (this is how fuzzel/swaylock type).
        let wants_keyboard = with_states(surface, |states| {
            let mut guard = states.cached_state.get::<LayerSurfaceCachedState>();
            let state = guard.current();
            state.keyboard_interactivity == KeyboardInteractivity::Exclusive
                && matches!(state.layer, Layer::Top | Layer::Overlay)
        });
        if wants_keyboard {
            if let Some(keyboard) = self.seat.get_keyboard() {
                if keyboard.current_focus().as_ref() != Some(surface) {
                    let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                    keyboard.set_focus(self, Some(surface.clone()), serial);
                }
            }
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
