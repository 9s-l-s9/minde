// SPDX-License-Identifier: MIT

//! `wlr-output-management-unstable-v1`: lets `wlr-randr`, `kanshi` and
//! `wdisplays` query the output layout (heads, modes, position, scale,
//! transform) and request changes to it.
//!
//! Smithay's vendored revision ships no server module for this protocol, so
//! -- like `gamma_control` and `foreign_toplevel` -- it is hand-written here
//! on the raw `wayland-protocols-wlr` bindings. The blanket
//! `delegate_dispatch2!` (handlers/mod.rs) covers only Smithay's own
//! dispatch2 user-data, so all `GlobalDispatch`/`Dispatch` impls live here.
//!
//! ## Reconciliation with the Scheme head model
//!
//! The compositor's outputs are the single source of truth. Queries read
//! straight from each [`Output`]. An accepted configuration is applied with
//! [`Output::change_current_state`] plus `Space::map_output`, after which
//! [`MindeState::update_usable_area`] re-derives the usable-rect head list
//! and hands it to Scheme (`handle-heads-change!`) exactly as a hotplug or
//! resize would -- so the frame trees reflow through the existing
//! `(wm-outputs)`/heads path with no separate code. External changes we did
//! not originate (a winit resize, a DRM hotplug) reconcile the other way:
//! [`MindeState::output_management_refresh`] re-advertises the current
//! state to every bound manager.
//!
//! ## Policy
//!
//! Whether an external tool may reconfigure outputs is policy, so an `apply`
//! is gated on `guile::output_config_allowed()` (the optional Scheme
//! predicate `output-configuration-allowed?`, default accept). Mode changes
//! are honoured only where the backend can realize them: under winit the
//! output size is fixed by the host window, so a differing mode fails the
//! configuration rather than lying about success.

use std::sync::{Arc, Mutex};

use smithay::output::{Mode, Output, Scale};
use smithay::reexports::{
    wayland_protocols_wlr::output_management::v1::server::{
        zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
        zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
        zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
        zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
        zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
    },
    wayland_server::{
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        backend::{ClientId, GlobalId},
        protocol::wl_output::Transform as WlTransform,
    },
};
use smithay::utils::{Point, Transform};

use crate::MindeState;

/// User data on a `ZwlrOutputHeadV1`: the compositor output it describes.
pub struct HeadData {
    output: Output,
}

/// User data on a `ZwlrOutputModeV1`: the mode it describes, so a later
/// `set_mode` referencing this resource can be resolved back to a [`Mode`].
pub struct ModeData {
    mode: Mode,
}

/// A single head's pending changes inside a configuration.
#[derive(Clone)]
struct PendingHead {
    output: Output,
    enabled: bool,
    mode: Option<Mode>,
    custom_mode: Option<(i32, i32, i32)>,
    position: Option<(i32, i32)>,
    scale: Option<f64>,
    transform: Option<WlTransform>,
}

/// Shared state of a `ZwlrOutputConfigurationV1` (mutated from its own and
/// its child config-head requests).
#[derive(Default)]
struct ConfigInner {
    serial: u32,
    heads: Vec<PendingHead>,
    /// Set once apply/test has been sent (further requests are errors).
    finished: bool,
}

/// User data on a `ZwlrOutputConfigurationV1`.
pub struct ConfigData {
    inner: Arc<Mutex<ConfigInner>>,
}

/// User data on a `ZwlrOutputConfigurationHeadV1`.
pub struct ConfigHeadData {
    inner: Arc<Mutex<ConfigInner>>,
    output: Output,
}

/// Per-head resources created for one bound manager.
struct HeadState {
    output: Output,
    resource: ZwlrOutputHeadV1,
    modes: Vec<(Mode, ZwlrOutputModeV1)>,
}

/// One bound `zwlr_output_manager_v1` and the head/mode resources created
/// for it.
struct ManagerState {
    manager: ZwlrOutputManagerV1,
    heads: Vec<HeadState>,
}

/// `wlr-output-management` global state, held in
/// `MindeState::output_management`.
pub struct OutputManagementState {
    global: GlobalId,
    managers: Vec<ManagerState>,
    serial: u32,
}

impl OutputManagementState {
    /// [`GlobalId`] getter (parity with the Smithay-provided states).
    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }
}

/// Creates the manager global (version 4) and returns the state.
pub fn init_output_management(dh: &DisplayHandle) -> OutputManagementState {
    let global = dh.create_global::<MindeState, ZwlrOutputManagerV1, ()>(4, ());
    OutputManagementState {
        global,
        managers: Vec::new(),
        serial: 0,
    }
}

/// wl_output transform -> smithay transform (the reverse conversion Smithay
/// only provides one way).
fn wl_to_transform(t: WlTransform) -> Transform {
    match t {
        WlTransform::Normal => Transform::Normal,
        WlTransform::_90 => Transform::_90,
        WlTransform::_180 => Transform::_180,
        WlTransform::_270 => Transform::_270,
        WlTransform::Flipped => Transform::Flipped,
        WlTransform::Flipped90 => Transform::Flipped90,
        WlTransform::Flipped180 => Transform::Flipped180,
        WlTransform::Flipped270 => Transform::Flipped270,
        _ => Transform::Normal,
    }
}

impl MindeState {
    /// The compositor outputs, in a stable order (space order).
    fn output_management_outputs(&self) -> Vec<Output> {
        self.space.outputs().cloned().collect()
    }

    /// Creates head + mode resources for `output` on `manager` and sends all
    /// its properties. Returns the new [`HeadState`], or `None` if resource
    /// creation failed (dead client).
    fn advertise_head(
        &self,
        dh: &DisplayHandle,
        manager: &ZwlrOutputManagerV1,
        output: &Output,
    ) -> Option<HeadState> {
        let client = manager.client()?;
        let version = manager.version();
        let head = client
            .create_resource::<ZwlrOutputHeadV1, _, MindeState>(
                dh,
                version,
                HeadData {
                    output: output.clone(),
                },
            )
            .ok()?;
        manager.head(&head);

        head.name(output.name());
        head.description(output.description());
        let phys = output.physical_properties();
        if phys.size.w != 0 || phys.size.h != 0 {
            head.physical_size(phys.size.w, phys.size.h);
        }

        let mut mode_resources = Vec::new();
        let current = output.current_mode();
        let preferred = output.preferred_mode();
        for mode in output.modes() {
            let Ok(mode_res) = client.create_resource::<ZwlrOutputModeV1, _, MindeState>(
                dh,
                version,
                ModeData { mode },
            ) else {
                continue;
            };
            head.mode(&mode_res);
            mode_res.size(mode.size.w, mode.size.h);
            if mode.refresh != 0 {
                mode_res.refresh(mode.refresh);
            }
            if Some(mode) == preferred {
                mode_res.preferred();
            }
            mode_resources.push((mode, mode_res));
        }

        // Always enabled in minde (no output is disabled).
        head.enabled(1);
        if let Some(mode) = current
            && let Some((_, res)) = mode_resources.iter().find(|(m, _)| *m == mode)
        {
            head.current_mode(res);
        }
        let loc = output.current_location();
        head.position(loc.x, loc.y);
        head.transform(output.current_transform().into());
        head.scale(output.current_scale().fractional_scale());
        if version >= 2 {
            head.make(phys.make.clone());
            head.model(phys.model.clone());
            head.serial_number(phys.serial_number.clone());
        }

        Some(HeadState {
            output: output.clone(),
            resource: head,
            modes: mode_resources,
        })
    }

    /// Re-advertises the current output layout to every bound manager,
    /// reconciling added/removed outputs and changed head properties, then
    /// bumps the serial and sends `done`. Call after applying a
    /// configuration and after any external output change (resize, hotplug).
    pub fn output_management_refresh(&mut self) {
        let outputs = self.output_management_outputs();
        let dh = self.display_handle.clone();
        self.output_management.serial = self.output_management.serial.wrapping_add(1);
        let serial = self.output_management.serial;

        // Take the managers out to avoid borrowing self while calling
        // advertise_head (which borrows &self).
        let mut managers = std::mem::take(&mut self.output_management.managers);
        for mgr in &mut managers {
            // Remove heads whose output is gone.
            mgr.heads.retain(|h| {
                if outputs.contains(&h.output) {
                    true
                } else {
                    for (_, m) in &h.modes {
                        m.finished();
                    }
                    h.resource.finished();
                    false
                }
            });
            // Add heads for new outputs.
            for output in &outputs {
                if !mgr.heads.iter().any(|h| &h.output == output) {
                    if let Some(head) = self.advertise_head(&dh, &mgr.manager, output) {
                        mgr.heads.push(head);
                    }
                    continue;
                }
                // Existing head: re-send the mutable properties.
                let head = mgr.heads.iter().find(|h| &h.output == output).unwrap();
                if let Some(mode) = output.current_mode()
                    && let Some((_, res)) = head.modes.iter().find(|(m, _)| *m == mode)
                {
                    head.resource.current_mode(res);
                }
                let loc = output.current_location();
                head.resource.position(loc.x, loc.y);
                head.resource.transform(output.current_transform().into());
                head.resource
                    .scale(output.current_scale().fractional_scale());
            }
            mgr.manager.done(serial);
        }
        self.output_management.managers = managers;
    }

    /// Applies (or, for `test`, only validates) a configuration. Returns
    /// `Ok(())` if it can be / was applied, `Err(reason)` otherwise.
    fn output_management_apply(
        &mut self,
        inner: &ConfigInner,
        test_only: bool,
    ) -> Result<(), &'static str> {
        let is_winit = self.udev_data.is_none();

        // Validate first so a failed test/apply changes nothing.
        for ph in &inner.heads {
            if !ph.enabled {
                // minde always keeps its outputs enabled; refuse to
                // disable rather than silently ignore.
                return Err("disabling outputs is not supported");
            }
            // Resolve the requested mode, if any.
            let requested_mode = ph.mode.or_else(|| {
                ph.custom_mode.map(|(w, h, r)| Mode {
                    size: (w, h).into(),
                    refresh: r,
                })
            });
            if let Some(mode) = requested_mode {
                let current = ph.output.current_mode();
                if Some(mode) != current {
                    if is_winit {
                        // The host window fixes the size under winit.
                        return Err("mode changes are not supported on the winit backend");
                    }
                    if ph.mode.is_some() && !ph.output.modes().contains(&mode) {
                        return Err("requested mode is not advertised for this head");
                    }
                }
            }
        }

        if test_only {
            return Ok(());
        }

        // Apply. update_usable_area at the end reflows the Scheme model.
        for ph in &inner.heads {
            let requested_mode = ph.mode.or_else(|| {
                ph.custom_mode.map(|(w, h, r)| Mode {
                    size: (w, h).into(),
                    refresh: r,
                })
            });
            let new_scale = ph.scale.map(|s| {
                if s.fract() == 0.0 {
                    Scale::Integer(s as i32)
                } else {
                    Scale::Fractional(s)
                }
            });
            let new_transform = ph.transform.map(wl_to_transform);
            let new_location: Option<Point<i32, smithay::utils::Logical>> =
                ph.position.map(|(x, y)| (x, y).into());
            ph.output
                .change_current_state(requested_mode, new_transform, new_scale, new_location);
            if let Some((x, y)) = ph.position {
                self.space.map_output(&ph.output, (x, y));
            }
        }
        Ok(())
    }
}

// zwlr_output_manager_v1

impl GlobalDispatch<ZwlrOutputManagerV1, ()> for MindeState {
    fn bind(
        state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrOutputManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let manager = data_init.init(resource, ());
        let dh = state.display_handle.clone();
        let outputs = state.output_management_outputs();
        let mut heads = Vec::new();
        for output in &outputs {
            if let Some(head) = state.advertise_head(&dh, &manager, output) {
                heads.push(head);
            }
        }
        state.output_management.serial = state.output_management.serial.wrapping_add(1);
        let serial = state.output_management.serial;
        manager.done(serial);
        state
            .output_management
            .managers
            .push(ManagerState { manager, heads });
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for MindeState {
    fn request(
        state: &mut Self,
        _client: &Client,
        manager: &ZwlrOutputManagerV1,
        request: zwlr_output_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_output_manager_v1::Request;
        match request {
            Request::CreateConfiguration { id, serial } => {
                let inner = Arc::new(Mutex::new(ConfigInner {
                    serial,
                    ..Default::default()
                }));
                data_init.init(id, ConfigData { inner });
            }
            Request::Stop => {
                manager.finished();
                state
                    .output_management
                    .managers
                    .retain(|m| &m.manager != manager);
            }
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, manager: &ZwlrOutputManagerV1, _data: &()) {
        state
            .output_management
            .managers
            .retain(|m| &m.manager != manager);
    }
}

// zwlr_output_head_v1 / zwlr_output_mode_v1 (read-only; only Release)

impl Dispatch<ZwlrOutputHeadV1, HeadData> for MindeState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _head: &ZwlrOutputHeadV1,
        request: zwlr_output_head_v1::Request,
        _data: &HeadData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        // Only `release` (a destructor); nothing to do.
        let _ = request;
    }
}

impl Dispatch<ZwlrOutputModeV1, ModeData> for MindeState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _mode: &ZwlrOutputModeV1,
        request: zwlr_output_mode_v1::Request,
        _data: &ModeData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        let _ = request;
    }
}

// zwlr_output_configuration_v1

impl Dispatch<ZwlrOutputConfigurationV1, ConfigData> for MindeState {
    fn request(
        state: &mut Self,
        _client: &Client,
        config: &ZwlrOutputConfigurationV1,
        request: zwlr_output_configuration_v1::Request,
        data: &ConfigData,
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_output_configuration_v1::Request;
        match request {
            Request::EnableHead { id, head } => {
                let Some(head_data) = head.data::<HeadData>() else {
                    return;
                };
                let output = head_data.output.clone();
                let mut inner = data.inner.lock().unwrap();
                if inner.heads.iter().any(|h| h.output == output) {
                    config.post_error(
                        zwlr_output_configuration_v1::Error::AlreadyConfiguredHead,
                        "head already configured",
                    );
                    return;
                }
                inner.heads.push(PendingHead {
                    output: output.clone(),
                    enabled: true,
                    mode: None,
                    custom_mode: None,
                    position: None,
                    scale: None,
                    transform: None,
                });
                data_init.init(
                    id,
                    ConfigHeadData {
                        inner: data.inner.clone(),
                        output,
                    },
                );
            }
            Request::DisableHead { head } => {
                let Some(head_data) = head.data::<HeadData>() else {
                    return;
                };
                let output = head_data.output.clone();
                let mut inner = data.inner.lock().unwrap();
                if inner.heads.iter().any(|h| h.output == output) {
                    config.post_error(
                        zwlr_output_configuration_v1::Error::AlreadyConfiguredHead,
                        "head already configured",
                    );
                    return;
                }
                inner.heads.push(PendingHead {
                    output,
                    enabled: false,
                    mode: None,
                    custom_mode: None,
                    position: None,
                    scale: None,
                    transform: None,
                });
            }
            Request::Apply | Request::Test => {
                let test_only = matches!(request, Request::Test);
                let snapshot = {
                    let mut inner = data.inner.lock().unwrap();
                    if inner.finished {
                        config.post_error(
                            zwlr_output_configuration_v1::Error::AlreadyUsed,
                            "configuration already applied or tested",
                        );
                        return;
                    }
                    inner.finished = true;
                    // Serial mismatch: the client's view is stale.
                    if inner.serial != state.output_management.serial {
                        config.cancelled();
                        return;
                    }
                    ConfigInner {
                        serial: inner.serial,
                        heads: inner.heads.clone(),
                        finished: true,
                    }
                };

                // Policy gate applies only to real applies, not tests.
                if !test_only && !crate::guile::output_config_allowed() {
                    config.failed();
                    return;
                }

                match state.output_management_apply(&snapshot, test_only) {
                    Ok(()) => {
                        config.succeeded();
                        if !test_only {
                            // Reflow the Scheme head model and re-advertise to
                            // output-management clients. update_usable_area
                            // does both (it now calls output_management_refresh
                            // itself); clearing reported_heads defeats its
                            // unchanged-geometry short-circuit.
                            state.reported_heads.clear();
                            state.update_usable_area();
                            // A scale change must reach fractional-scale
                            // clients so they repaint at the new density.
                            state.update_fractional_scales();
                            crate::guile::on_output_configured();
                        }
                    }
                    Err(reason) => {
                        tracing::info!(reason, "wlr-output-management: rejected configuration");
                        config.failed();
                    }
                }
            }
            Request::Destroy => {}
            _ => {}
        }
    }
}

// zwlr_output_configuration_head_v1

impl Dispatch<ZwlrOutputConfigurationHeadV1, ConfigHeadData> for MindeState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        config_head: &ZwlrOutputConfigurationHeadV1,
        request: zwlr_output_configuration_head_v1::Request,
        data: &ConfigHeadData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_output_configuration_head_v1::Request;
        let mut inner = data.inner.lock().unwrap();
        let Some(ph) = inner.heads.iter_mut().find(|h| h.output == data.output) else {
            return;
        };
        // Setting the same property twice is a protocol error; we take the
        // lenient path (last write wins) to keep the state machine small.
        match request {
            Request::SetMode { mode } => {
                if let Some(mode_data) = mode.data::<ModeData>() {
                    ph.mode = Some(mode_data.mode);
                }
            }
            Request::SetCustomMode {
                width,
                height,
                refresh,
            } => {
                if width <= 0 || height <= 0 {
                    config_head.post_error(
                        zwlr_output_configuration_head_v1::Error::InvalidCustomMode,
                        "custom mode dimensions must be positive",
                    );
                    return;
                }
                ph.custom_mode = Some((width, height, refresh));
            }
            Request::SetPosition { x, y } => ph.position = Some((x, y)),
            Request::SetTransform { transform } => {
                if let Ok(t) = transform.into_result() {
                    ph.transform = Some(t);
                }
            }
            Request::SetScale { scale } => {
                if scale > 0.0 {
                    ph.scale = Some(scale);
                }
            }
            Request::SetAdaptiveSync { .. } => {}
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_roundtrip_matches_wl_output() {
        for t in [
            WlTransform::Normal,
            WlTransform::_90,
            WlTransform::_180,
            WlTransform::_270,
            WlTransform::Flipped,
            WlTransform::Flipped90,
            WlTransform::Flipped180,
            WlTransform::Flipped270,
        ] {
            let smithay: Transform = wl_to_transform(t);
            let back: WlTransform = smithay.into();
            assert_eq!(back, t);
        }
    }
}
