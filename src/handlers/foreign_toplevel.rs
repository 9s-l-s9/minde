// SPDX-License-Identifier: MIT

//! `wlr-foreign-toplevel-management-unstable-v1`: lets external taskbars,
//! docks and window switchers (waybar, the Eww status consumers, wofi-based
//! switchers) enumerate the compositor's toplevel windows and request
//! activate/close/fullscreen/minimize on them.
//!
//! Smithay's vendored revision ships only `ext-foreign-toplevel-list-v1`
//! (enumeration, no control), so -- like `gamma_control` and
//! `wlr_screencopy` -- the manager is hand-written here on top of the raw
//! `wayland-protocols-wlr` bindings. The blanket `delegate_dispatch2!`
//! (handlers/mod.rs) covers only Smithay's own dispatch2 user-data, so the
//! `GlobalDispatch`/`Dispatch` impls live here in full.
//!
//! State ownership: the compositor already tracks the window registry
//! (`MindeState::windows`), focus (`focused_window`) and per-window
//! title/app-id. This module mirrors that into one protocol handle per
//! bound manager and keeps it synchronized from the same lifecycle points
//! that drive the Scheme model (`register_window`, `unregister_window`,
//! `report_title_if_changed`, the focus/fullscreen `WmCommand`s and
//! `update_usable_area`). Requests that carry policy (activate, fullscreen,
//! minimize) are routed back through Scheme so the frame-tree/group model
//! stays authoritative; close is a direct shell close.

use std::collections::HashMap;

use smithay::output::Output;
use smithay::reexports::{
    wayland_protocols_wlr::foreign_toplevel::v1::server::{
        zwlr_foreign_toplevel_handle_v1::{
            self, State as HandleState, ZwlrForeignToplevelHandleV1,
        },
        zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
    },
    wayland_server::{
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        backend::{ClientId, GlobalId},
    },
};

use crate::MindeState;

/// User data on a `ZwlrForeignToplevelHandleV1`: the compositor window id it
/// represents. `activate`/`close`/`set_fullscreen`/`set_minimized` are routed
/// by this id.
pub struct ForeignToplevelHandleUserData {
    id: u64,
}

/// One tracked toplevel: its last-reported properties plus the live handle
/// resources (one per bound manager).
struct ToplevelEntry {
    title: String,
    app_id: String,
    activated: bool,
    fullscreen: bool,
    /// Compositor outputs the window currently overlaps (for
    /// output_enter/output_leave). Stored as compositor `Output`s and
    /// resolved to each client's `wl_output` when sending.
    outputs: Vec<Output>,
    handles: Vec<ZwlrForeignToplevelHandleV1>,
}

/// `wlr-foreign-toplevel-management` manager state, held in
/// `MindeState::foreign_toplevel`. Registered unconditionally (both
/// backends) in `MindeState::new`.
pub struct ForeignToplevelManagerState {
    global: GlobalId,
    managers: Vec<ZwlrForeignToplevelManagerV1>,
    toplevels: HashMap<u64, ToplevelEntry>,
}

impl ForeignToplevelManagerState {
    /// [`GlobalId`] getter (parity with the Smithay-provided states).
    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }
}

/// Creates the manager global (version 3) and returns the manager state.
pub fn init_foreign_toplevel_manager(dh: &DisplayHandle) -> ForeignToplevelManagerState {
    let global = dh.create_global::<MindeState, ZwlrForeignToplevelManagerV1, ()>(3, ());
    ForeignToplevelManagerState {
        global,
        managers: Vec::new(),
        toplevels: HashMap::new(),
    }
}

/// Encodes a set of `HandleState` values as the protocol's `array` payload
/// (native-endian u32 each), as wlroots does.
fn encode_states(states: &[HandleState]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(states.len() * 4);
    for s in states {
        bytes.extend_from_slice(&(*s as u32).to_ne_bytes());
    }
    bytes
}

impl MindeState {
    /// Sends the current title/app-id/state/outputs plus a `done` to a single
    /// freshly-created handle.
    fn foreign_send_initial(&self, handle: &ZwlrForeignToplevelHandleV1, entry: &ToplevelEntry) {
        handle.title(entry.title.clone());
        handle.app_id(entry.app_id.clone());
        if let Some(client) = handle.client() {
            for output in &entry.outputs {
                for wl_output in output.client_outputs(&client) {
                    handle.output_enter(&wl_output);
                }
            }
        }
        handle.state(encode_states(&entry.state_vec()));
        handle.done();
    }

    /// A newly-registered window: create a handle on every bound manager and
    /// announce it. Called from `register_window`.
    pub fn foreign_toplevel_created(&mut self, id: u64, title: String, app_id: String) {
        let outputs = self.foreign_outputs_for(id);
        let entry = ToplevelEntry {
            title,
            app_id,
            activated: false,
            fullscreen: false,
            outputs,
            handles: Vec::new(),
        };
        self.foreign_toplevel.toplevels.insert(id, entry);
        // Borrow-splitting: pull the managers out, then re-enter per manager.
        let managers = self.foreign_toplevel.managers.clone();
        let dh = self.display_handle.clone();
        for manager in managers {
            if let Some(handle) = create_handle(&dh, &manager, id) {
                manager.toplevel(&handle);
                if let Some(entry) = self.foreign_toplevel.toplevels.get_mut(&id) {
                    entry.handles.push(handle.clone());
                }
                if let Some(entry) = self.foreign_toplevel.toplevels.get(&id) {
                    self.foreign_send_initial(&handle, entry);
                }
            }
        }
    }

    /// A window's title/app-id changed. Called from map and
    /// `report_title_if_changed`.
    pub fn foreign_toplevel_title(&mut self, id: u64, title: &str, app_id: &str) {
        let Some(entry) = self.foreign_toplevel.toplevels.get_mut(&id) else {
            return;
        };
        let mut changed = false;
        if entry.title != title {
            entry.title = title.to_string();
            for h in &entry.handles {
                h.title(title.to_string());
            }
            changed = true;
        }
        if entry.app_id != app_id {
            entry.app_id = app_id.to_string();
            for h in &entry.handles {
                h.app_id(app_id.to_string());
            }
            changed = true;
        }
        if changed {
            for h in &entry.handles {
                h.done();
            }
        }
    }

    /// A window was unmapped/destroyed: close every handle and drop the entry.
    /// Called from `unregister_window`.
    pub fn foreign_toplevel_closed(&mut self, id: u64) {
        if let Some(entry) = self.foreign_toplevel.toplevels.remove(&id) {
            for h in &entry.handles {
                h.closed();
            }
        }
    }

    /// Focus moved: set the `activated` state on the newly focused window and
    /// clear it on all others. `focused` is `None` when focus is cleared.
    pub fn foreign_toplevel_focus(&mut self, focused: Option<u64>) {
        let ids: Vec<u64> = self.foreign_toplevel.toplevels.keys().copied().collect();
        for id in ids {
            let want = Some(id) == focused;
            let entry = self.foreign_toplevel.toplevels.get_mut(&id).unwrap();
            if entry.activated != want {
                entry.activated = want;
                let states = encode_states(&entry.state_vec());
                for h in &entry.handles {
                    h.state(states.clone());
                    h.done();
                }
            }
        }
    }

    /// A window's fullscreen state changed (from `WmCommand::Fullscreen`).
    pub fn foreign_toplevel_fullscreen(&mut self, id: u64, on: bool) {
        let Some(entry) = self.foreign_toplevel.toplevels.get_mut(&id) else {
            return;
        };
        if entry.fullscreen != on {
            entry.fullscreen = on;
            let states = encode_states(&entry.state_vec());
            for h in &entry.handles {
                h.state(states.clone());
                h.done();
            }
        }
    }

    /// Recomputes the output association for every tracked toplevel and emits
    /// output_enter/output_leave (plus `done`) for the changes. Cheap enough
    /// to call whenever geometry changes (`update_usable_area`, window
    /// placement).
    pub fn refresh_foreign_toplevel_outputs(&mut self) {
        let ids: Vec<u64> = self.foreign_toplevel.toplevels.keys().copied().collect();
        for id in ids {
            let new_outputs = self.foreign_outputs_for(id);
            let Some(entry) = self.foreign_toplevel.toplevels.get(&id) else {
                continue;
            };
            let old = entry.outputs.clone();
            let entered: Vec<Output> = new_outputs
                .iter()
                .filter(|o| !old.contains(o))
                .cloned()
                .collect();
            let left: Vec<Output> = old
                .iter()
                .filter(|o| !new_outputs.contains(o))
                .cloned()
                .collect();
            if entered.is_empty() && left.is_empty() {
                continue;
            }
            let entry = self.foreign_toplevel.toplevels.get_mut(&id).unwrap();
            entry.outputs = new_outputs;
            for h in &entry.handles {
                let Some(client) = h.client() else { continue };
                for output in &entered {
                    for wl_output in output.client_outputs(&client) {
                        h.output_enter(&wl_output);
                    }
                }
                for output in &left {
                    for wl_output in output.client_outputs(&client) {
                        h.output_leave(&wl_output);
                    }
                }
                h.done();
            }
        }
    }

    /// The compositor outputs a window currently overlaps, by intersecting its
    /// mapped geometry with each output's geometry.
    fn foreign_outputs_for(&self, id: u64) -> Vec<Output> {
        let Some(window) = self
            .windows
            .iter()
            .find(|(wid, _)| *wid == id)
            .map(|(_, w)| w.clone())
        else {
            return Vec::new();
        };
        let Some(geo) = self.space.element_geometry(&window) else {
            return Vec::new();
        };
        self.space
            .outputs()
            .filter(|o| {
                self.space
                    .output_geometry(o)
                    .map(|og| og.overlaps(geo))
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    }
}

impl ToplevelEntry {
    /// The state array for this toplevel (activated/fullscreen; maximized and
    /// minimized are never reported -- minde tiles rather than maximizing,
    /// and it has no minimized state of its own).
    fn state_vec(&self) -> Vec<HandleState> {
        let mut v = Vec::new();
        if self.activated {
            v.push(HandleState::Activated);
        }
        if self.fullscreen {
            v.push(HandleState::Fullscreen);
        }
        v
    }
}

/// Creates a handle resource on the manager's client, matching the manager's
/// negotiated version.
fn create_handle(
    dh: &DisplayHandle,
    manager: &ZwlrForeignToplevelManagerV1,
    id: u64,
) -> Option<ZwlrForeignToplevelHandleV1> {
    let client = manager.client()?;
    client
        .create_resource::<ZwlrForeignToplevelHandleV1, _, MindeState>(
            dh,
            manager.version(),
            ForeignToplevelHandleUserData { id },
        )
        .ok()
}

// zwlr_foreign_toplevel_manager_v1

impl GlobalDispatch<ZwlrForeignToplevelManagerV1, ()> for MindeState {
    fn bind(
        state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrForeignToplevelManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let manager = data_init.init(resource, ());
        // Announce every currently-mapped toplevel to the new manager.
        let ids: Vec<u64> = state.windows.iter().map(|(id, _)| *id).collect();
        let dh = state.display_handle.clone();
        for id in ids {
            if let Some(handle) = create_handle(&dh, &manager, id) {
                manager.toplevel(&handle);
                if let Some(entry) = state.foreign_toplevel.toplevels.get_mut(&id) {
                    entry.handles.push(handle.clone());
                }
                if let Some(entry) = state.foreign_toplevel.toplevels.get(&id) {
                    state.foreign_send_initial(&handle, entry);
                }
            }
        }
        state.foreign_toplevel.managers.push(manager);
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for MindeState {
    fn request(
        state: &mut Self,
        _client: &Client,
        manager: &ZwlrForeignToplevelManagerV1,
        request: zwlr_foreign_toplevel_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_foreign_toplevel_manager_v1::Request;
        match request {
            Request::Stop => {
                manager.finished();
                state.foreign_toplevel.managers.retain(|m| m != manager);
            }
            _ => unreachable!(),
        }
    }

    fn destroyed(
        state: &mut Self,
        _client: ClientId,
        manager: &ZwlrForeignToplevelManagerV1,
        _data: &(),
    ) {
        state.foreign_toplevel.managers.retain(|m| m != manager);
    }
}

// zwlr_foreign_toplevel_handle_v1

impl Dispatch<ZwlrForeignToplevelHandleV1, ForeignToplevelHandleUserData> for MindeState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _handle: &ZwlrForeignToplevelHandleV1,
        request: zwlr_foreign_toplevel_handle_v1::Request,
        data: &ForeignToplevelHandleUserData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_foreign_toplevel_handle_v1::Request;
        let id = data.id;
        match request {
            Request::Activate { .. } => crate::guile::on_foreign_activate(id),
            Request::Close => {
                state.foreign_close_window(id);
            }
            Request::SetFullscreen { .. } => crate::guile::on_foreign_fullscreen(id, true),
            Request::UnsetFullscreen => crate::guile::on_foreign_fullscreen(id, false),
            Request::SetMinimized => crate::guile::on_foreign_minimize(id, true),
            Request::UnsetMinimized => crate::guile::on_foreign_minimize(id, false),
            // minde tiles; maximize is meaningless, and set_rectangle is
            // only a minimize-animation hint we have no use for.
            Request::SetMaximized | Request::UnsetMaximized => {}
            Request::SetRectangle { .. } => {}
            Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(
        state: &mut Self,
        _client: ClientId,
        handle: &ZwlrForeignToplevelHandleV1,
        data: &ForeignToplevelHandleUserData,
    ) {
        if let Some(entry) = state.foreign_toplevel.toplevels.get_mut(&data.id) {
            entry.handles.retain(|h| h != handle);
        }
    }
}

impl MindeState {
    /// Politely close the window behind a foreign-toplevel `close` request.
    fn foreign_close_window(&mut self, id: u64) {
        let Some(window) = self
            .windows
            .iter()
            .find(|(wid, _)| *wid == id)
            .map(|(_, w)| w.clone())
        else {
            return;
        };
        if let Some(toplevel) = window.toplevel() {
            toplevel.send_close();
        } else if let Some(x11) = window.x11_surface() {
            let _ = x11.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_states_is_native_endian_u32_per_state() {
        let bytes = encode_states(&[HandleState::Activated, HandleState::Fullscreen]);
        assert_eq!(bytes.len(), 8);
        assert_eq!(
            u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            HandleState::Activated as u32
        );
        assert_eq!(
            u32::from_ne_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            HandleState::Fullscreen as u32
        );
    }

    #[test]
    fn encode_states_empty_is_empty() {
        assert!(encode_states(&[]).is_empty());
    }
}
