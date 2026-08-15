// SPDX-License-Identifier: MIT

//! `wlr-output-management-unstable-v1`: lets `wlr-randr`, `kanshi`,
//! `shikane` and `wdisplays` query the output layout (heads, modes,
//! position, scale, transform, adaptive sync) and request changes to it.
//!
//! Smithay's vendored revision ships no server module for this protocol, so
//! -- like `gamma_control` and `foreign_toplevel` -- it is hand-written here
//! on the raw `wayland-protocols-wlr` bindings. The blanket
//! `delegate_dispatch2!` (handlers/mod.rs) covers only Smithay's own
//! dispatch2 user-data, so all `GlobalDispatch`/`Dispatch` impls live here.
//!
//! ## Heads versus enabled outputs
//!
//! `Space::outputs()` keeps meaning "enabled and rendering". Every head the
//! compositor knows about -- including disabled ones -- is registered in
//! [`OutputManagementState::heads_all`] through
//! [`MindeState::output_management_add_output`] /
//! [`MindeState::output_management_remove_output`] (called by the backends
//! when a connector/window appears or goes away). A head is *enabled* iff
//! the space contains its output, so a disabled head stays advertised (with
//! `enabled = 0` and its mode list) instead of being `finished()`.
//!
//! ## Reconciliation with the Scheme head model
//!
//! The compositor's outputs are the single source of truth. Queries read
//! straight from each [`Output`]. An accepted configuration is applied by
//! [`MindeState::apply_output_configuration`] (shared with the Scheme
//! primitive of a later stage): validation first with no side effects, then
//! per-head application with snapshots that are reverted in reverse order
//! if a backend refuses. On success the protocol call site clears
//! `reported_heads` and runs [`MindeState::update_usable_area`], which
//! re-derives the usable-rect head list, hands it to Scheme
//! (`handle-heads-change!`) exactly as a hotplug or resize would, and
//! re-advertises to bound managers. External changes we did not originate
//! (a winit resize, a DRM hotplug) reconcile the other way through
//! [`MindeState::output_management_refresh`].
//!
//! ## Policy
//!
//! Whether an external tool may reconfigure outputs is policy, so both
//! `test` and `apply` are gated on `guile::output_config_allowed()` (the
//! optional Scheme predicate `output-configuration-allowed?`, default
//! accept). Backend capabilities are honoured honestly: under winit the
//! output size is fixed by the host window, so a differing mode fails the
//! configuration rather than lying about success; adaptive sync is only
//! accepted where the backend reports support.

use std::sync::{Arc, Mutex};

use smithay::output::{Mode, Output, Scale};
use smithay::reexports::{
    wayland_protocols_wlr::output_management::v1::server::{
        zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
        zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
        zwlr_output_head_v1::{self, AdaptiveSyncState, ZwlrOutputHeadV1},
        zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
        zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
    },
    wayland_server::{
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        backend::{ClientId, GlobalId},
        protocol::wl_output::Transform as WlTransform,
    },
};
use smithay::utils::{Logical, Point, Transform};

use crate::MindeState;

/// Environment variable that, when set to `1` at startup, lets a
/// configuration disable the last enabled head. Test-only: the nested winit
/// e2e has a single output and needs to exercise disable -> enable; a real
/// session must never end up with zero enabled heads.
pub const ALLOW_NO_HEADS_ENV: &str = "MINDE_OUTPUT_MGMT_ALLOW_NO_HEADS";

/// User data on a `ZwlrOutputHeadV1`: the compositor output it describes.
pub struct HeadData {
    output: Output,
}

/// User data on a `ZwlrOutputModeV1`: the mode it describes, so a later
/// `set_mode` referencing this resource can be resolved back to a [`Mode`].
pub struct ModeData {
    mode: Mode,
}

/// A single head's requested changes inside a configuration. Built by the
/// protocol (`enable_head`/`disable_head` + config-head requests) and, in a
/// later stage, by the Scheme `configure-output!` primitive.
#[derive(Clone, Debug)]
pub(crate) struct HeadChange {
    pub output: Output,
    pub enabled: bool,
    /// An advertised mode (from a `zwlr_output_mode_v1`).
    pub mode: Option<Mode>,
    /// A custom `(width, height, refresh_mHz)` mode.
    pub custom_mode: Option<(i32, i32, i32)>,
    pub position: Option<(i32, i32)>,
    pub scale: Option<f64>,
    pub transform: Option<WlTransform>,
    /// Requested adaptive-sync (VRR) state.
    pub adaptive_sync: Option<bool>,
}

impl HeadChange {
    /// A change that only enables/disables `output` and leaves everything
    /// else untouched.
    pub(crate) fn new(output: Output, enabled: bool) -> Self {
        HeadChange {
            output,
            enabled,
            mode: None,
            custom_mode: None,
            position: None,
            scale: None,
            transform: None,
            adaptive_sync: None,
        }
    }

    /// The mode this change asks for, resolving a custom mode to a [`Mode`].
    pub(crate) fn requested_mode(&self) -> Option<Mode> {
        self.mode.or_else(|| {
            self.custom_mode.map(|(w, h, r)| Mode {
                size: (w, h).into(),
                refresh: r,
            })
        })
    }
}

/// Why a configuration was rejected (or could not be applied).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OutputConfigError(pub String);

impl OutputConfigError {
    fn new(msg: impl Into<String>) -> Self {
        OutputConfigError(msg.into())
    }
}

impl std::fmt::Display for OutputConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What validation needs to know about one existing head. Decoupled from
/// the compositor so [`validate`] is unit-testable.
#[derive(Clone, Debug)]
pub(crate) struct HeadFacts {
    pub output: Output,
    /// Currently enabled (mapped in the space).
    pub enabled: bool,
    /// Backend-reported adaptive-sync state; `None` when unsupported.
    pub adaptive_sync: Option<bool>,
}

/// A validated [`HeadChange`]: `mode` is the mode to set (already resolved
/// for an enable-without-mode), `was_enabled` the pre-apply state.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedChange {
    pub change: HeadChange,
    /// The mode to apply, if any (`None` = keep the current one).
    pub mode: Option<Mode>,
    pub was_enabled: bool,
}

/// Pure validation of `changes` against `heads` (no side effects). Returns
/// the per-head resolved changes in request order.
///
/// Rules: every changed head must be known and appear once; an advertised
/// mode must be in the head's mode list; enabling a disabled head without
/// a mode picks its preferred (else current) mode; adaptive sync may only
/// be enabled where the backend supports it; the configuration must leave
/// at least one head enabled unless `allow_no_heads`. A no-op configuration
/// is fine.
pub(crate) fn validate(
    heads: &[HeadFacts],
    changes: &[HeadChange],
    allow_no_heads: bool,
) -> Result<Vec<ResolvedChange>, OutputConfigError> {
    let mut resolved: Vec<ResolvedChange> = Vec::with_capacity(changes.len());
    for change in changes {
        let Some(facts) = heads.iter().find(|h| h.output == change.output) else {
            return Err(OutputConfigError::new(
                "configuration references an unknown head",
            ));
        };
        if resolved.iter().any(|r| r.change.output == change.output) {
            return Err(OutputConfigError::new("head configured twice"));
        }
        let mut mode = None;
        if change.enabled {
            if let Some(m) = change.mode {
                if !change.output.modes().contains(&m) {
                    return Err(OutputConfigError::new(
                        "requested mode is not advertised for this head",
                    ));
                }
                mode = Some(m);
            } else if let Some(m) = change.requested_mode() {
                if m.size.w <= 0 || m.size.h <= 0 {
                    return Err(OutputConfigError::new("custom mode has non-positive size"));
                }
                mode = Some(m);
            } else if !facts.enabled {
                // EnableHead without a mode: preferred, else current.
                mode = change
                    .output
                    .preferred_mode()
                    .or(change.output.current_mode());
                if mode.is_none() {
                    return Err(OutputConfigError::new("head has no mode to enable with"));
                }
            }
            if let Some(s) = change.scale
                && !(s > 0.0)
            {
                return Err(OutputConfigError::new("scale must be positive"));
            }
            if change.adaptive_sync == Some(true) && facts.adaptive_sync.is_none() {
                return Err(OutputConfigError::new(
                    "adaptive sync is not supported on this head",
                ));
            }
        }
        resolved.push(ResolvedChange {
            change: change.clone(),
            mode,
            was_enabled: facts.enabled,
        });
    }

    let enabled_after = heads
        .iter()
        .filter(|h| {
            resolved
                .iter()
                .find(|r| r.change.output == h.output)
                .map(|r| r.change.enabled)
                .unwrap_or(h.enabled)
        })
        .count();
    if enabled_after == 0 && !allow_no_heads && !heads.is_empty() {
        return Err(OutputConfigError::new(
            "configuration would leave no enabled head",
        ));
    }
    Ok(resolved)
}

/// Shared state of a `ZwlrOutputConfigurationV1` (mutated from its own and
/// its child config-head requests).
#[derive(Default)]
struct ConfigInner {
    serial: u32,
    heads: Vec<HeadChange>,
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

/// The identity properties last advertised for a head, so a refresh can
/// re-send only what changed (and rebuild the mode objects on a mode-list
/// change).
#[derive(Clone, PartialEq)]
struct HeadSnapshot {
    name: String,
    description: String,
    size: smithay::utils::Size<i32, smithay::utils::Raw>,
    make: String,
    model: String,
    serial_number: String,
    modes: Vec<Mode>,
    preferred: Option<Mode>,
}

impl HeadSnapshot {
    fn of(output: &Output) -> Self {
        let phys = output.physical_properties();
        HeadSnapshot {
            name: output.name(),
            description: head_description(output),
            size: phys.size,
            make: phys.make,
            model: phys.model,
            serial_number: phys.serial_number,
            modes: output.modes(),
            preferred: output.preferred_mode(),
        }
    }
}

/// Per-head resources created for one bound manager.
struct HeadState {
    output: Output,
    resource: ZwlrOutputHeadV1,
    modes: Vec<(Mode, ZwlrOutputModeV1)>,
    snapshot: HeadSnapshot,
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
    /// Every known head, enabled or not, in registration order.
    heads_all: Vec<Output>,
    /// [`ALLOW_NO_HEADS_ENV`] was set at startup.
    allow_no_heads: bool,
}

impl OutputManagementState {
    /// [`GlobalId`] getter (parity with the Smithay-provided states).
    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    /// Every known head (enabled or not), in registration order.
    pub fn heads_all(&self) -> &[Output] {
        &self.heads_all
    }
}

/// Creates the manager global (version 4) and returns the state.
pub fn init_output_management(dh: &DisplayHandle) -> OutputManagementState {
    let global = dh.create_global::<MindeState, ZwlrOutputManagerV1, ()>(4, ());
    let allow_no_heads = std::env::var(ALLOW_NO_HEADS_ENV).as_deref() == Ok("1");
    if allow_no_heads {
        tracing::warn!(
            "{ALLOW_NO_HEADS_ENV}=1: output configurations may disable every head (test only)"
        );
    }
    OutputManagementState {
        global,
        managers: Vec::new(),
        serial: 0,
        heads_all: Vec::new(),
        allow_no_heads,
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

/// The description advertised for a head. (A later stage swaps the body
/// for the EDID-derived "Make Model Serial" string.)
fn head_description(output: &Output) -> String {
    // wlroots format "Make Model Serial" from EDID (udev), else the
    // output name; this is the string kanshi/shikane profiles match.
    crate::edid::output_description(output)
}

/// Protocol scale (f64) -> Smithay scale, integer where exact.
fn to_scale(s: f64) -> Scale {
    if s.fract() == 0.0 {
        Scale::Integer(s as i32)
    } else {
        Scale::Fractional(s)
    }
}

/// Pre-apply state of one head, restored on failure.
struct HeadBackup {
    output: Output,
    mode: Option<Mode>,
    location: Point<i32, Logical>,
    scale: Scale,
    transform: Transform,
    enabled: bool,
    adaptive_sync: Option<bool>,
}

impl MindeState {
    /// Registers `output` as a known head (call once the backend has created
    /// it, whether or not it is mapped) and re-advertises the layout.
    pub fn output_management_add_output(&mut self, output: &Output) {
        if !self.output_management.heads_all.contains(output) {
            self.output_management.heads_all.push(output.clone());
        }
        self.output_management_refresh();
    }

    /// Forgets `output` (connector gone / window closed) and re-advertises,
    /// which `finished()`s its head on every bound manager.
    pub fn output_management_remove_output(&mut self, output: &Output) {
        self.output_management.heads_all.retain(|o| o != output);
        self.output_management_refresh();
    }

    /// Every known head, in registration order.
    fn output_management_outputs(&self) -> Vec<Output> {
        self.output_management.heads_all.clone()
    }

    /// Whether `output` is enabled: mapped in the space.
    pub fn output_enabled(&self, output: &Output) -> bool {
        self.space.outputs().any(|o| o == output)
    }

    /// The backend's adaptive-sync (VRR) state for `output`: `Some(on)`
    /// where supported, `None` where the backend cannot do it (winit; udev
    /// until VRR lands).
    pub fn output_adaptive_sync(&self, output: &Output) -> Option<bool> {
        let _ = output;
        None
    }

    /// Backend-specific part of applying one head change: mode set,
    /// enable/disable and adaptive sync where the backend can realize
    /// them. `change.mode` is already resolved (an enable-without-mode
    /// carries the preferred one; `custom_mode` is folded in). The generic
    /// caller has validated the change and runs [`Self::commit_head`]
    /// afterwards, which sets the output state and (un)maps the output in
    /// the space; `commit_head` is idempotent, so a backend whose
    /// enable/disable needs the output mapped and mode-bearing up front
    /// (udev: the CRTC is initialised from the output's current mode) may
    /// do that bookkeeping itself. `Err` reverts the whole configuration.
    fn backend_realize_head(&mut self, output: &Output, change: &HeadChange) -> Result<(), String> {
        if self.udev_data.is_none() {
            return crate::winit::realize_head(self, output, change);
        }
        self.udev_realize_head(output, change)
    }

    /// Facts about every known head, for [`validate`].
    fn head_facts(&self) -> Vec<HeadFacts> {
        self.output_management
            .heads_all
            .iter()
            .map(|o| HeadFacts {
                output: o.clone(),
                enabled: self.output_enabled(o),
                adaptive_sync: self.output_adaptive_sync(o),
            })
            .collect()
    }

    /// Validates `changes` and, unless `test_only`, applies them head by
    /// head, reverting every head already changed if a later one fails.
    /// Does *not* reflow the Scheme model or re-advertise: the caller runs
    /// the post-success sequence (`reported_heads.clear()`,
    /// `update_usable_area`, `update_fractional_scales`,
    /// `reconfigure_lock_surfaces`, `guile::on_output_configured`).
    pub(crate) fn apply_output_configuration(
        &mut self,
        changes: &[HeadChange],
        test_only: bool,
    ) -> Result<(), OutputConfigError> {
        let facts = self.head_facts();
        let resolved = validate(&facts, changes, self.output_management.allow_no_heads)?;
        if test_only {
            return Ok(());
        }

        let mut backups: Vec<HeadBackup> = Vec::with_capacity(resolved.len());
        let mut failure: Option<OutputConfigError> = None;
        for r in &resolved {
            let output = &r.change.output;
            backups.push(HeadBackup {
                output: output.clone(),
                mode: output.current_mode(),
                location: output.current_location(),
                scale: output.current_scale(),
                transform: output.current_transform(),
                enabled: r.was_enabled,
                adaptive_sync: self.output_adaptive_sync(output),
            });
            let mut change = r.change.clone();
            change.mode = r.mode;
            change.custom_mode = None;
            if let Err(msg) = self.backend_realize_head(output, &change) {
                failure = Some(OutputConfigError(msg));
                break;
            }
            self.commit_head(&change);
        }

        if let Some(err) = failure {
            // Reverse revert: last changed head first. The failed head's
            // backup is in the list too; restoring it is harmless.
            for b in backups.iter().rev() {
                let mut revert = HeadChange::new(b.output.clone(), b.enabled);
                revert.mode = b.mode;
                revert.position = Some((b.location.x, b.location.y));
                revert.adaptive_sync = b.adaptive_sync;
                if let Err(msg) = self.backend_realize_head(&b.output, &revert) {
                    tracing::warn!(%msg, output = %b.output.name(), "output config revert failed");
                }
                b.output.change_current_state(
                    b.mode,
                    Some(b.transform),
                    Some(b.scale),
                    Some(b.location),
                );
                if b.enabled {
                    self.space.map_output(&b.output, b.location);
                } else {
                    self.space.unmap_output(&b.output);
                }
            }
            return Err(err);
        }
        Ok(())
    }

    /// Generic part of applying one (already backend-realized) change:
    /// output state and space membership. Idempotent: re-applying values
    /// the backend hook already set is harmless, and unmapping an output
    /// the hook already unmapped is a no-op.
    fn commit_head(&mut self, change: &HeadChange) {
        let output = &change.output;
        if !change.enabled {
            self.space.unmap_output(output);
            return;
        }
        let new_transform = change.transform.map(wl_to_transform);
        let new_scale = change.scale.map(to_scale);
        let new_location: Option<Point<i32, Logical>> = change.position.map(|(x, y)| (x, y).into());
        output.change_current_state(change.mode, new_transform, new_scale, new_location);
        let mapped = self.output_enabled(output);
        if !mapped || change.position.is_some() {
            let loc = new_location.unwrap_or_else(|| output.current_location());
            self.space.map_output(output, loc);
        }
    }

    /// Creates the mode resources for `output` on `head` (sends `mode`,
    /// `size`, `refresh`, `preferred`), returning `(mode, resource)` pairs.
    fn advertise_modes(
        &self,
        dh: &DisplayHandle,
        client: &Client,
        head: &ZwlrOutputHeadV1,
        output: &Output,
    ) -> Vec<(Mode, ZwlrOutputModeV1)> {
        let version = head.version();
        let preferred = output.preferred_mode();
        let mut mode_resources = Vec::new();
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
        mode_resources
    }

    /// Sends the mutable per-head state: `enabled`, then (only when
    /// enabled) `current_mode`/`position`/`transform`/`scale`, and
    /// `adaptive_sync` on v4.
    fn send_head_state(&self, head: &HeadState) {
        let output = &head.output;
        let enabled = self.output_enabled(output);
        head.resource.enabled(enabled as i32);
        if enabled {
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
        if head.resource.version() >= 4 {
            let vrr = match self.output_adaptive_sync(output) {
                Some(true) => AdaptiveSyncState::Enabled,
                _ => AdaptiveSyncState::Disabled,
            };
            head.resource.adaptive_sync(vrr);
        }
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

        let snapshot = HeadSnapshot::of(output);
        head.name(snapshot.name.clone());
        head.description(snapshot.description.clone());
        if snapshot.size.w != 0 || snapshot.size.h != 0 {
            head.physical_size(snapshot.size.w, snapshot.size.h);
        }
        let modes = self.advertise_modes(dh, &client, &head, output);
        if version >= 2 {
            head.make(snapshot.make.clone());
            head.model(snapshot.model.clone());
            head.serial_number(snapshot.serial_number.clone());
        }
        let state = HeadState {
            output: output.clone(),
            resource: head,
            modes,
            snapshot,
        };
        self.send_head_state(&state);
        Some(state)
    }

    /// Re-sends whatever identity properties of an existing head changed
    /// since it was last advertised (name, description, physical size,
    /// make/model/serial, mode list), then the mutable state.
    fn refresh_head(&self, dh: &DisplayHandle, client: &Client, head: &mut HeadState) {
        let now = HeadSnapshot::of(&head.output);
        let old = &head.snapshot;
        let res = &head.resource;
        if now.name != old.name {
            res.name(now.name.clone());
        }
        if now.description != old.description {
            res.description(now.description.clone());
        }
        if now.size != old.size && (now.size.w != 0 || now.size.h != 0) {
            res.physical_size(now.size.w, now.size.h);
        }
        if res.version() >= 2 {
            if now.make != old.make {
                res.make(now.make.clone());
            }
            if now.model != old.model {
                res.model(now.model.clone());
            }
            if now.serial_number != old.serial_number {
                res.serial_number(now.serial_number.clone());
            }
        }
        if now.modes != old.modes || now.preferred != old.preferred {
            for (_, m) in head.modes.drain(..) {
                m.finished();
            }
            head.modes = self.advertise_modes(dh, client, &head.resource, &head.output);
        }
        head.snapshot = now;
        self.send_head_state(head);
    }

    /// Re-advertises the current output layout to every bound manager,
    /// reconciling added/removed heads and changed head properties, then
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
            // Remove heads whose output is gone (not merely disabled).
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
            let client = mgr.manager.client();
            for output in &outputs {
                match mgr.heads.iter_mut().find(|h| &h.output == output) {
                    None => {
                        if let Some(head) = self.advertise_head(&dh, &mgr.manager, output) {
                            mgr.heads.push(head);
                        }
                    }
                    Some(head) => {
                        if let Some(client) = client.as_ref() {
                            self.refresh_head(&dh, client, head);
                        }
                    }
                }
            }
            mgr.manager.done(serial);
        }
        self.output_management.managers = managers;
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
                inner.heads.push(HeadChange::new(output.clone(), true));
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
                inner.heads.push(HeadChange::new(output, false));
            }
            Request::Apply | Request::Test => {
                let test_only = matches!(request, Request::Test);
                let changes = {
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
                    inner.heads.clone()
                };

                // Policy gate: a test must answer the same as an apply
                // would, so it is gated too.
                if !crate::guile::output_config_allowed() {
                    tracing::info!(
                        "wlr-output-management: configuration refused by output-configuration-allowed?"
                    );
                    config.failed();
                    return;
                }

                match state.apply_output_configuration(&changes, test_only) {
                    Ok(()) => {
                        config.succeeded();
                        if !test_only {
                            // Reflow the Scheme head model and re-advertise to
                            // output-management clients. update_usable_area
                            // does both (it calls output_management_refresh
                            // itself); clearing reported_heads defeats its
                            // unchanged-geometry short-circuit.
                            state.reported_heads.clear();
                            state.update_usable_area();
                            // A scale change must reach fractional-scale
                            // clients so they repaint at the new density.
                            state.update_fractional_scales();
                            // Lock surfaces must keep covering each output.
                            state.reconfigure_lock_surfaces();
                            state.schedule_redraw();
                            crate::guile::on_output_configured();
                        }
                    }
                    Err(reason) => {
                        tracing::info!(%reason, "wlr-output-management: rejected configuration");
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
            Request::SetAdaptiveSync { state } => match state.into_result() {
                Ok(AdaptiveSyncState::Enabled) => ph.adaptive_sync = Some(true),
                Ok(AdaptiveSyncState::Disabled) => ph.adaptive_sync = Some(false),
                _ => {
                    config_head.post_error(
                        zwlr_output_configuration_head_v1::Error::InvalidAdaptiveSyncState,
                        "invalid adaptive sync state",
                    );
                }
            },
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::output::{PhysicalProperties, Subpixel};

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

    fn mode(w: i32, h: i32) -> Mode {
        Mode {
            size: (w, h).into(),
            refresh: 60_000,
        }
    }

    fn head(name: &str, enabled: bool, vrr: Option<bool>) -> HeadFacts {
        let output = Output::new(
            name.to_string(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "t".into(),
                model: "t".into(),
                serial_number: "t".into(),
            },
        );
        output.add_mode(mode(800, 600));
        output.set_preferred(mode(1280, 800));
        output.change_current_state(Some(mode(1280, 800)), None, None, Some((0, 0).into()));
        HeadFacts {
            output,
            enabled,
            adaptive_sync: vrr,
        }
    }

    #[test]
    fn noop_configuration_is_ok() {
        let heads = [head("a", true, None)];
        let change = HeadChange::new(heads[0].output.clone(), true);
        let r = validate(&heads, &[change], false).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].mode, None);
        // Empty configuration too.
        assert!(validate(&heads, &[], false).is_ok());
    }

    #[test]
    fn last_enabled_head_cannot_be_disabled_without_override() {
        let heads = [head("a", true, None), head("b", false, None)];
        let off = HeadChange::new(heads[0].output.clone(), false);
        assert!(validate(&heads, &[off.clone()], false).is_err());
        assert!(validate(&heads, &[off.clone()], true).is_ok());
        // Disabling one while enabling the other is fine.
        let on = HeadChange::new(heads[1].output.clone(), true);
        let r = validate(&heads, &[off, on], false).unwrap();
        // Enable without a mode resolves to the preferred one.
        assert_eq!(r[1].mode, Some(mode(1280, 800)));
    }

    #[test]
    fn unknown_mode_is_rejected() {
        let heads = [head("a", true, None)];
        let mut c = HeadChange::new(heads[0].output.clone(), true);
        c.mode = Some(mode(640, 480));
        assert!(validate(&heads, &[c.clone()], false).is_err());
        c.mode = Some(mode(800, 600));
        assert!(validate(&heads, &[c], false).is_ok());
    }

    #[test]
    fn unknown_head_and_duplicate_are_rejected() {
        let heads = [head("a", true, None)];
        let other = head("b", true, None);
        let c = HeadChange::new(other.output.clone(), true);
        assert!(validate(&heads, &[c], false).is_err());
        let d = HeadChange::new(heads[0].output.clone(), true);
        assert!(validate(&heads, &[d.clone(), d], false).is_err());
    }

    #[test]
    fn adaptive_sync_requires_backend_support() {
        let heads = [head("a", true, None), head("b", true, Some(false))];
        let mut c = HeadChange::new(heads[0].output.clone(), true);
        c.adaptive_sync = Some(true);
        assert!(validate(&heads, &[c], false).is_err());
        let mut d = HeadChange::new(heads[1].output.clone(), true);
        d.adaptive_sync = Some(true);
        assert!(validate(&heads, &[d], false).is_ok());
        // Asking for "disabled" is always fine.
        let mut e = HeadChange::new(heads[0].output.clone(), true);
        e.adaptive_sync = Some(false);
        assert!(validate(&heads, &[e], false).is_ok());
    }
}
