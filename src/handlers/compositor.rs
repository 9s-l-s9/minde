// SPDX-License-Identifier: MIT

use crate::{MindeState, grabs::resize_grab, state::ClientState};
use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    reexports::{
        calloop::Interest,
        wayland_server::{
            Client, Resource,
            protocol::{wl_buffer, wl_surface::WlSurface},
        },
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            BufferAssignment, CompositorClientState, CompositorHandler, CompositorState,
            SurfaceAttributes, add_blocker, add_pre_commit_hook, get_parent, is_sync_subsurface,
            with_states,
        },
        dmabuf::get_dmabuf,
        drm_syncobj::DrmSyncobjCachedState,
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

    fn new_surface(&mut self, surface: &WlSurface) {
        add_pre_commit_hook::<Self, _>(surface, |state, _dh, surface| {
            state.block_until_buffer_ready(surface);
        });
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        let committed_window = if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            let window = crate::state::window_for_surface(&self.space, &root);
            if let Some(window) = &window {
                window.on_commit();
                // Title/app-id arrive (and change) via ordinary commits
                // after the map-time report, which is usually empty.
                self.report_title_if_changed(window);
            }
            window
        } else {
            None
        };

        xdg_shell::handle_commit(&mut self.popups, &self.space, surface);
        resize_grab::handle_commit(&mut self.space, surface);
        // Resizing can also move the origin during commit. Publish after that
        // adjustment and after Window::on_commit refreshed the committed size.
        // `committed_window` owns a clone, not a borrow into compositor state
        // held across report_title_if_changed's possible Scheme callback.
        if let Some(window) = committed_window
            && let Some(id) = self.id_for_window(&window)
        {
            self.publish_window_geometry(id);
        }
        self.handle_layer_commit(surface);
        // Every commit may carry new content or a frame-callback request;
        // both need a render pass (see `udev` repaint scheduling).
        self.schedule_redraw();
    }
}

impl MindeState {
    /// Holds a commit that attaches a dmabuf until the client's GPU work on
    /// it has finished, so the buffer is never sampled or scanned out half
    /// drawn.  With `linux-drm-syncobj-v1` the wait is on the explicit
    /// acquire point: Smithay's renderer and DRM compositor assume it has
    /// already signalled when the transaction applies (see
    /// `smithay::wayland::drm_syncobj`), so without this blocker an
    /// explicit-sync client (Vulkan WSI, Xwayland) can be shown before it
    /// finished rendering.  Implicit-sync clients wait on the dmabuf's
    /// read fence instead, which keeps a slow client from stalling the
    /// compositor's own GPU work.
    fn block_until_buffer_ready(&mut self, surface: &WlSurface) {
        let mut acquire_point = None;
        let dmabuf = with_states(surface, |states| {
            acquire_point.clone_from(
                &states
                    .cached_state
                    .get::<DrmSyncobjCachedState>()
                    .pending()
                    .acquire_point,
            );
            states
                .cached_state
                .get::<SurfaceAttributes>()
                .pending()
                .buffer
                .as_ref()
                .and_then(|assignment| match assignment {
                    BufferAssignment::NewBuffer(buffer) => get_dmabuf(buffer).cloned().ok(),
                    _ => None,
                })
        });
        let Some(dmabuf) = dmabuf else {
            return;
        };
        let Some(client) = surface.client() else {
            return;
        };

        if let Some(acquire_point) = acquire_point
            && let Ok((blocker, source)) = acquire_point.generate_blocker()
        {
            let client = client.clone();
            if self
                .handle
                .insert_source(source, move |_, _, state| {
                    let dh = state.display_handle.clone();
                    state
                        .client_compositor_state(&client)
                        .blocker_cleared(state, &dh);
                    Ok(())
                })
                .is_ok()
            {
                add_blocker(surface, blocker);
                return;
            }
        }

        if let Ok((blocker, source)) = dmabuf.generate_blocker(Interest::READ)
            && self
                .handle
                .insert_source(source, move |_, _, state| {
                    let dh = state.display_handle.clone();
                    state
                        .client_compositor_state(&client)
                        .blocker_cleared(state, &dh);
                    Ok(())
                })
                .is_ok()
        {
            add_blocker(surface, blocker);
        }
    }

    /// Layer-surface commit handling: arrange the output's layer map,
    /// send the initial configure if this was the first commit (required
    /// by the protocol before the client may attach a buffer), track
    /// exclusive-zone changes, and give exclusive-keyboard layers
    /// (fuzzel, swaylock) the keyboard.
    fn handle_layer_commit(&mut self, surface: &WlSurface) {
        use smithay::desktop::{WindowSurfaceType, layer_map_for_output};
        use smithay::wayland::shell::wlr_layer::{
            KeyboardInteractivity, Layer, LayerSurfaceCachedState, LayerSurfaceData,
        };

        // Plain toplevel/subsurface commits (the overwhelming majority) never
        // carry `LayerSurfaceData`; check that first so they skip the
        // per-output `layer_map_for_output` borrows below entirely instead of
        // scanning every output's layer map only to find nothing.
        let has_layer_role = with_states(surface, |states| {
            states.data_map.get::<LayerSurfaceData>().is_some()
        });
        if !has_layer_role {
            return;
        }

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
