// SPDX-License-Identifier: MIT

use crate::{MindeState, grabs::resize_grab, state::ClientState};
use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    desktop::{Window, WindowSurfaceType, find_popup_root_surface, layer_map_for_output},
    input::pointer::CursorImageStatus,
    output::{Output, WeakOutput},
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
            SurfaceAttributes, TraversalAction, add_blocker, add_pre_commit_hook, get_parent,
            get_role, is_sync_subsurface, with_states, with_surface_tree_downward,
        },
        dmabuf::get_dmabuf,
        drm_syncobj::DrmSyncobjCachedState,
        shell::wlr_layer::LayerSurfaceCachedState,
        shm::{ShmHandler, ShmState},
    },
};
use std::sync::Mutex;

use super::xdg_shell;

#[derive(Default)]
struct CommitOutputs(Mutex<Vec<WeakOutput>>);

// Configure acknowledgements do not change layer placement. Compare only
// the layout fields, not the serial or keyboard-interactivity bookkeeping.
fn same_layer_layout(a: &LayerSurfaceCachedState, b: &LayerSurfaceCachedState) -> bool {
    a.size == b.size
        && a.anchor == b.anchor
        && a.exclusive_zone == b.exclusive_zone
        && a.exclusive_edge == b.exclusive_edge
        && a.layer == b.layer
        && (a.margin.top, a.margin.right, a.margin.bottom, a.margin.left)
            == (b.margin.top, b.margin.right, b.margin.bottom, b.margin.left)
}

#[derive(Default)]
struct LayerLayout(Mutex<Option<LayerSurfaceCachedState>>);

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

    fn new_subsurface(&mut self, surface: &WlSurface, _parent: &WlSurface) {
        self.update_surface_fractional_scale(surface);
        self.enter_layer_surface_output(surface);
    }

    fn destroyed(&mut self, surface: &WlSurface) {
        self.idle_surface_destroyed(surface);
        // A disappearing popup/subsurface may send no final commit. Include
        // its owner's current footprint and the last committed footprint.
        let outputs = self.surface_outputs(surface, None);
        self.redraw_surface_outputs(surface, outputs);
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        // A synchronized child is applied by its parent's commit. It must
        // not wake a renderer while its pending state is still invisible.
        if is_sync_subsurface(surface) {
            return;
        }
        let committed_window = {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            let window = crate::state::window_for_surface(&root);
            let window = if &root == surface {
                self.handle_toplevel_commit(surface, window)
            } else {
                window
            };
            if let Some(window) = &window {
                window.on_commit();
                // Xdg metadata changes with commits; X11 metadata changes
                // through XWM property notifications, independently of paint.
                if window.toplevel().is_some() {
                    self.report_title_if_changed(window);
                }
            }
            window
        };

        xdg_shell::handle_popup_commit(&mut self.popups, surface);
        resize_grab::handle_commit(self, surface, committed_window.as_ref());
        // Resizing can also move the origin during commit. Publish after that
        // adjustment and after Window::on_commit refreshed the committed size.
        // `committed_window` owns a clone, not a borrow into compositor state
        // held across report_title_if_changed's possible Scheme callback.
        if let Some(window) = &committed_window {
            self.update_window_fractional_scale(window);
            self.publish_geometry_for_window(window);
        }
        self.handle_layer_commit(surface);
        let owner = committed_window.or_else(|| self.surface_window(surface));
        let outputs = self.surface_outputs(surface, owner.as_ref());
        if outputs.is_empty()
            && let Some(window) = &owner
        {
            self.schedule_parked_window_frames(window);
        }
        self.redraw_surface_outputs(surface, outputs);
    }
}

impl MindeState {
    // LayerMap::arrange used to send these enters as a side effect of every
    // parent repaint. New child roles must now receive them independently.
    pub(super) fn enter_layer_surface_output(&self, surface: &WlSurface) {
        let root = self.surface_root(surface);
        if !with_states(&root, |s| {
            s.data_map
                .get::<smithay::wayland::shell::wlr_layer::LayerSurfaceData>()
                .is_some()
        }) {
            return;
        }
        for output in self.surface_outputs(&root, None) {
            with_surface_tree_downward(
                surface,
                (),
                |_, _, _| TraversalAction::DoChildren(()),
                |s, _, _| output.enter(s),
                |_, _, _| true,
            );
        }
    }

    fn surface_root(&self, surface: &WlSurface) -> WlSurface {
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        if let Some(popup) = self.popups.find_popup(&root)
            && let Ok(parent) = find_popup_root_surface(&popup)
        {
            root = parent;
        }
        root
    }

    fn surface_window(&self, surface: &WlSurface) -> Option<Window> {
        crate::state::window_for_surface(&self.surface_root(surface))
    }

    fn surface_outputs(&self, surface: &WlSurface, window: Option<&Window>) -> Vec<Output> {
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        let cursor =
            matches!(self.cursor_state.status(), CursorImageStatus::Surface(s) if s == &root);
        if self.locked && !cursor {
            return self
                .lock_surfaces
                .iter()
                .filter(|(_, lock)| lock.wl_surface() == &root)
                .map(|(output, _)| output.clone())
                .collect();
        }
        if cursor
            || get_role(&root) == Some(smithay::wayland::selection::data_device::DND_ICON_ROLE)
        {
            return self
                .space
                .outputs()
                .filter(|o| {
                    self.space
                        .output_geometry(o)
                        .is_some_and(|r| r.to_f64().contains(self.pointer_location))
                })
                .cloned()
                .collect();
        }
        if let Some(popup) = self.popups.find_popup(&root)
            && let Ok(parent) = find_popup_root_surface(&popup)
        {
            root = parent;
        }
        let owner = window
            .cloned()
            .or_else(|| crate::state::window_for_surface(&root));
        if let Some(window) = owner {
            let bounds = self.space.element_location(&window).map(|location| {
                let mut bounds = window.bbox_with_popups();
                bounds.loc += location - window.geometry().loc;
                bounds
            });
            return super::foreign_toplevel::outputs_for_geometry(&self.space, bounds);
        }
        if !with_states(&root, |s| {
            s.data_map
                .get::<smithay::wayland::shell::wlr_layer::LayerSurfaceData>()
                .is_some()
        }) {
            return Vec::new();
        }
        self.space
            .outputs()
            .filter(|o| {
                layer_map_for_output(o)
                    .layer_for_surface(&root, WindowSurfaceType::TOPLEVEL)
                    .is_some()
            })
            .cloned()
            .collect()
    }

    fn redraw_surface_outputs(&mut self, surface: &WlSurface, current: Vec<Output>) {
        let affected = with_states(surface, |states| {
            states.data_map.insert_if_missing(CommitOutputs::default);
            let mut previous = states
                .data_map
                .get::<CommitOutputs>()
                .unwrap()
                .0
                .lock()
                .unwrap();
            let mut affected = current.clone();
            for output in previous.iter().filter_map(WeakOutput::upgrade) {
                if !affected.contains(&output) {
                    affected.push(output);
                }
            }
            *previous = current.iter().map(Output::downgrade).collect();
            affected
        });
        self.schedule_redraw_outputs(&affected);
    }

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
        use smithay::wayland::shell::wlr_layer::LayerSurfaceData;

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

        let layout_changed = with_states(surface, |states| {
            let current = *states
                .cached_state
                .get::<LayerSurfaceCachedState>()
                .current();
            states.data_map.insert_if_missing(LayerLayout::default);
            let mut previous = states
                .data_map
                .get::<LayerLayout>()
                .unwrap()
                .0
                .lock()
                .unwrap();
            let changed = previous
                .as_ref()
                .is_none_or(|old| !same_layer_layout(old, &current));
            *previous = Some(current);
            changed
        });
        // Content-only commits and configure acknowledgements do not change
        // usable areas. The initial configure still needs arranged dimensions.
        if !initial_configure_sent || layout_changed {
            self.update_usable_area();
        }
        if !initial_configure_sent
            && let Some(layer) = layer_map_for_output(&output)
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
        {
            layer.layer_surface().send_configure();
        }

        // Exclusive keyboard interactivity on a top/overlay layer takes
        // the keyboard (this is how fuzzel/swaylock type).
        // While the session is locked, keyboard focus belongs to the lock
        // surface only -- never let a layer client (a regular client) grab it.
        if !self.locked
            && let Some(focus) = self.exclusive_layer_focus()
            && self.keyboard_focus().as_ref() != Some(&focus)
        {
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            self.set_keyboard_focus(Some(focus), serial);
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
