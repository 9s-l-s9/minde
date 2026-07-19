// SPDX-License-Identifier: MIT

//! Standalone DRM/udev/libinput backend: owns a VT via libseat, scans the
//! primary GPU for the first usable connector, and renders via GBM/EGL.
//! Adapted from Smithay's `anvil` example (`anvil/src/udev.rs`); see
//! README for the exact upstream revision this mirrors.
//!
//! Trimmed vs. anvil, per this compositor's needs:
//! - multi-output: every connected connector becomes an output, laid out
//!   left-to-right in connection order (anvil's policy); hotplug adds and
//!   removes outputs at runtime via `DrmScanner`, and Scheme is told
//!   through `update_usable_area` -> `handle-heads-change!`.
//! - primary GPU only: no `all_gpus`/multi-GPU render-node handling beyond
//!   what's needed to keep the `GpuManager`/`MultiRenderer` plumbing
//!   working (anvil's structure is kept here since fighting the API to
//!   remove it entirely isn't worth it).
//! - no DRM leasing, no fps/debug overlays, no profiling.
//!
//! udev-only ecosystem protocols wired here (they need real hardware the
//! nested winit backend cannot provide):
//! - `wp-presentation-time`: the DRM output user-data carries an
//!   [`OutputPresentationFeedback`] per queued frame; on vblank the real
//!   monotonic timestamp, sequence and refresh from [`DrmEventMetadata`] are
//!   sent to the clients that requested feedback for surfaces scanned out on
//!   that output.
//! - `linux-drm-syncobj-v1` (explicit sync): created only when the primary
//!   GPU supports syncobj timeline eventfds; Smithay's renderer/DrmCompositor
//!   then imports the acquire fence as a KMS in-fence and signals the release
//!   point when the buffer is dropped.
//!
//! `wp-tearing-control` is advertised on both backends from `state.rs` but is
//! advisory only (the DRM compositor has no async page-flip path here).

use std::{collections::HashMap, path::Path, time::Duration};

use smithay::backend::renderer::element::default_primary_scanout_output_compare;
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;
use smithay::{
    backend::{
        SwapBuffersError,
        allocator::{
            Fourcc,
            format::FormatSet,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        },
        drm::{
            CreateDrmNodeError, DrmDevice, DrmDeviceFd, DrmError, DrmEvent, DrmEventMetadata,
            DrmEventTime, DrmNode, NodeType,
            compositor::FrameFlags,
            exporter::gbm::GbmFramebufferExporter,
            output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
        },
        egl::{EGLContext, EGLDevice, EGLDisplay, context::ContextPriority},
        input::InputEvent,
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            ImportDma,
            gles::{Capability, GlesRenderer},
            multigpu::{GpuManager, MultiRenderer, gbm::GbmGlesBackend},
        },
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{UdevBackend, UdevEvent, primary_gpu},
    },
    desktop::utils::{
        OutputPresentationFeedback, surface_presentation_feedback_flags_from_states,
        surface_primary_scanout_output, update_surface_primary_scanout_output,
    },
    output::{Mode as WlMode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{
            EventLoop, RegistrationToken,
            timer::{TimeoutAction, Timer},
        },
        drm::control::{ModeTypeFlags, connector, crtc},
        input::{
            AccelProfile, ClickMethod, Device as LibinputDevice, DeviceCapability,
            DeviceConfigError, Libinput,
        },
        rustix::fs::OFlags,
        wayland_server::{DisplayHandle, backend::GlobalId},
    },
    utils::{DeviceFd, Monotonic, Transform},
    wayland::{
        dmabuf::{DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        drm_syncobj::{DrmSyncobjHandler, DrmSyncobjState, supports_syncobj_eventfd},
        presentation::{PresentationState, Refresh},
    },
};
use smithay_drm_extras::drm_scanner::{DrmScanEvent, DrmScanner};
use tracing::{error, info, warn};

use crate::render::{BorderBuffers, MindeRenderElements};
use crate::{MindeState, guile};

/// The renderer type produced by `GpuManager::single_renderer`, matching
/// anvil's `UdevRenderer` alias. We only ever address the primary node
/// (see module docs), but keep the multi-GPU-shaped type since that's what
/// `DrmOutputManager`/`DrmOutput` at this smithay revision expect.
type UdevRenderer<'a> = MultiRenderer<
    'a,
    'a,
    GbmGlesBackend<GlesRenderer, DrmDeviceFd>,
    GbmGlesBackend<GlesRenderer, DrmDeviceFd>,
>;

/// Per-frame user-data attached to each queued DRM frame: the presentation
/// feedback owed to clients once the frame is actually scanned out. Returned
/// by `frame_submitted` on the matching vblank (see `frame_finish`). `None`
/// for frames that owe no feedback (e.g. the locked blank frame).
type FrameUserData = Option<OutputPresentationFeedback>;

type GbmDrmOutputManager = DrmOutputManager<
    GbmAllocator<DrmDeviceFd>,
    GbmFramebufferExporter<DrmDeviceFd>,
    FrameUserData,
    DrmDeviceFd,
>;

const SUPPORTED_FORMATS: &[Fourcc] = &[
    Fourcc::Abgr2101010,
    Fourcc::Argb2101010,
    Fourcc::Abgr8888,
    Fourcc::Argb8888,
];

/// One output: a connected connector driving a CRTC. Lives in its
/// device's `surfaces` map.
struct OutputSurface {
    output: Output,
    global: Option<GlobalId>,
    dh: DisplayHandle,
    drm_output: DrmOutput<
        GbmAllocator<DrmDeviceFd>,
        GbmFramebufferExporter<DrmDeviceFd>,
        FrameUserData,
        DrmDeviceFd,
    >,
    border_buffers: BorderBuffers,
}

impl Drop for OutputSurface {
    fn drop(&mut self) {
        self.output.leave_all();
        if let Some(global) = self.global.take() {
            self.dh.remove_global::<MindeState>(global);
        }
    }
}

struct DeviceData {
    drm_output_manager: GbmDrmOutputManager,
    registration_token: RegistrationToken,
    drm_scanner: DrmScanner,
    surfaces: HashMap<crtc::Handle, OutputSurface>,
}

/// State private to the udev backend, held inside `MindeState` for the
/// duration of the process (created once by `init_udev`).
pub struct UdevBackendData {
    session: LibSeatSession,
    primary_gpu: DrmNode,
    gpus: GpuManager<GbmGlesBackend<GlesRenderer, DrmDeviceFd>>,
    devices: HashMap<DrmNode, DeviceData>,
    /// Whether `handle-startup!` has fired (once, on the first output).
    started: bool,
    dmabuf_state: Option<(DmabufState, DmabufGlobal)>,
    /// `wp_presentation` global (udev-only): kept alive so the global stays
    /// advertised. Feedback is collected per frame and delivered on vblank;
    /// the state itself only owns the global's lifetime.
    #[allow(dead_code)]
    presentation_state: PresentationState,
    /// `linux-drm-syncobj-v1` explicit-sync state, created only when the
    /// primary GPU exposes syncobj timeline eventfds (`DRM_CAP_SYNCOBJ_TIMELINE`
    /// via [`supports_syncobj_eventfd`]). `None` on hardware without support,
    /// in which case the global is simply never advertised. See
    /// [`DrmSyncobjHandler`] below.
    syncobj_state: Option<DrmSyncobjState>,
    /// libinput devices currently present on the seat. Retained so
    /// `wm-configure-input!` can re-apply rules to devices already plugged
    /// in (rules are otherwise applied on `InputEvent::DeviceAdded`).
    /// libinput `Device` is refcounted and cheap to clone; it is `!Send`,
    /// which is fine since it never leaves the compositor thread.
    input_devices: Vec<LibinputDevice>,
}

/// The POSIX clock id (`CLOCK_MONOTONIC`) the DRM backend reports presentation
/// timestamps against. The kernel delivers vblank timestamps on this clock
/// (`has_monotonic_timestamps`), and it is advertised to clients through the
/// `wp_presentation` global so they can correlate the numbers. This is a
/// Linux-only DRM backend, so the value is fixed.
const CLOCK_MONOTONIC: u32 = 1;

/// Explicit-sync (`linux-drm-syncobj-v1`): Smithay's renderer picks the
/// acquire/release sync points straight out of the surface's committed state,
/// so all this handler has to expose is the state delegate. It is only `Some`
/// when the primary GPU supports syncobj timelines (see `syncobj_state`).
impl DrmSyncobjHandler for MindeState {
    fn drm_syncobj_state(&mut self) -> Option<&mut DrmSyncobjState> {
        self.udev_data.as_mut()?.syncobj_state.as_mut()
    }
}

impl DmabufHandler for MindeState {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self
            .udev_data
            .as_mut()
            .unwrap()
            .dmabuf_state
            .as_mut()
            .unwrap()
            .0
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: smithay::backend::allocator::dmabuf::Dmabuf,
        notifier: ImportNotifier,
    ) {
        let udev = self.udev_data.as_mut().unwrap();
        if udev
            .gpus
            .single_renderer(&udev.primary_gpu)
            .and_then(|mut renderer| renderer.import_dmabuf(&dmabuf, None))
            .is_ok()
        {
            dmabuf.set_node(udev.primary_gpu);
            let _ = notifier.successful::<MindeState>();
        } else {
            notifier.failed();
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum DeviceAddError {
    #[error("failed to open drm device via libseat: {0}")]
    DeviceOpen(smithay::backend::session::libseat::Error),
    #[error("failed to initialize drm device: {0}")]
    DrmDevice(DrmError),
    #[error("failed to initialize gbm device: {0}")]
    GbmDevice(std::io::Error),
    #[error("failed to access drm node: {0}")]
    DrmNode(CreateDrmNodeError),
}

/// Entry point: takes over a VT via libseat, discovers DRM/libinput
/// devices, and wires everything into `event_loop`. Mirrors
/// `crate::winit::init_winit`'s contract (calls `guile::on_output_geometry`
/// once the output size is known, then `guile::on_startup`).
pub fn init_udev(
    event_loop: &mut EventLoop<MindeState>,
    state: &mut MindeState,
) -> Result<(), Box<dyn std::error::Error>> {
    let (session, notifier) = LibSeatSession::new()?;
    let seat_name = session.seat();

    let primary_gpu = primary_gpu(&seat_name)
        .ok()
        .flatten()
        .and_then(|path| {
            DrmNode::from_path(path)
                .ok()?
                .node_with_type(NodeType::Render)?
                .ok()
        })
        .ok_or("no primary GPU found")?;
    info!(%primary_gpu, "using primary gpu");

    let gpus = GpuManager::new(GbmGlesBackend::with_factory(|display| {
        let context = EGLContext::new_with_priority(display, ContextPriority::High)?;
        let capabilities = unsafe { GlesRenderer::supported_capabilities(&context)? }
            .into_iter()
            .filter(|c| *c != Capability::Instancing)
            .collect::<Vec<_>>();
        Ok(unsafe { GlesRenderer::with_capabilities(context, capabilities)? })
    }))?;

    // wp-presentation-time global (udev-only: real vblank timestamps). The
    // winit backend has no true presentation clock, so it deliberately never
    // advertises this global -- see the capability matrix.
    let presentation_state =
        PresentationState::new::<MindeState>(&state.display_handle, CLOCK_MONOTONIC);

    state.session = Some(session.clone());
    state.udev_data = Some(UdevBackendData {
        session: session.clone(),
        primary_gpu,
        gpus,
        devices: HashMap::new(),
        started: false,
        dmabuf_state: None,
        presentation_state,
        // Filled in by `device_added` once the primary GPU is opened and its
        // syncobj-timeline capability is probed.
        syncobj_state: None,
        input_devices: Vec::new(),
    });

    let udev_backend = UdevBackend::new(&seat_name)?;

    let mut libinput_context =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput_context.udev_assign_seat(&seat_name).unwrap();
    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());

    event_loop
        .handle()
        .insert_source(libinput_backend, |event, _, state| {
            // Track device arrival/removal for `wm-input-devices` and apply
            // stored `wm-configure-input!` rules. `event`'s device is the
            // libinput `Device` here (LibinputInputBackend), so its config
            // methods are reachable -- they are not on the generic
            // `process_input_event` path.
            match &event {
                InputEvent::DeviceAdded { device } => {
                    state.libinput_device_added(device.clone());
                }
                InputEvent::DeviceRemoved { device } => {
                    state.libinput_device_removed(device);
                }
                _ => {}
            }
            state.process_input_event(event);
        })?;

    event_loop
        .handle()
        .insert_source(notifier, move |event, &mut (), state| match event {
            SessionEvent::PauseSession => {
                libinput_context.suspend();
                info!("pausing session");
                if let Some(udev) = state.udev_data.as_mut() {
                    for device in udev.devices.values_mut() {
                        device.drm_output_manager.pause();
                    }
                }
            }
            SessionEvent::ActivateSession => {
                info!("resuming session");
                if libinput_context.resume().is_err() {
                    error!("failed to resume libinput context");
                }
                let mut to_repaint = Vec::new();
                if let Some(udev) = state.udev_data.as_mut() {
                    for (node, device) in udev.devices.iter_mut() {
                        let _ = device.drm_output_manager.lock().activate(false);
                        for crtc in device.surfaces.keys() {
                            to_repaint.push((*node, *crtc));
                        }
                    }
                }
                for (node, crtc) in to_repaint {
                    state.handle_repaint_now(node, crtc);
                }
            }
        })?;

    // Bring up every device udev already knows about; each connected
    // connector becomes an output.
    for (device_id, path) in udev_backend.device_list() {
        if let Ok(node) = DrmNode::from_dev_id(device_id)
            && let Err(err) = device_added(state, node, path)
        {
            warn!(?err, ?node, "skipping drm device");
        }
    }

    if let Some(udev) = state.udev_data.as_ref()
        && !udev.started
    {
        warn!("no connected DRM output found at startup; waiting for hotplug");
    }

    setup_dmabuf_global(state);

    // Gamma control is udev-only: it drives real CRTCs via the legacy DRM
    // SETGAMMA ioctl, so winit deliberately never advertises this global.
    crate::handlers::gamma_control::init_gamma_control_manager(&state.display_handle);

    event_loop
        .handle()
        .insert_source(udev_backend, |event, _, state| match event {
            UdevEvent::Added { device_id, path } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id)
                    && let Err(err) = device_added(state, node, &path)
                {
                    warn!(?err, ?node, "failed to add hotplugged drm device");
                }
            }
            UdevEvent::Changed { device_id } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    device_changed(state, node);
                }
            }
            UdevEvent::Removed { device_id } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    device_removed(state, node);
                }
            }
        })?;

    Ok(())
}

/// The libinput capabilities a device exposes, as the names reported to
/// Scheme through `(wm-input-devices)`.
fn device_capabilities(device: &LibinputDevice) -> Vec<String> {
    [
        (DeviceCapability::Keyboard, "keyboard"),
        (DeviceCapability::Pointer, "pointer"),
        (DeviceCapability::Touch, "touch"),
        (DeviceCapability::TabletTool, "tablet-tool"),
        (DeviceCapability::TabletPad, "tablet-pad"),
        (DeviceCapability::Gesture, "gesture"),
        (DeviceCapability::Switch, "switch"),
    ]
    .into_iter()
    .filter(|(cap, _)| device.has_capability(*cap))
    .map(|(_, name)| name.to_string())
    .collect()
}

/// Logs the outcome of one libinput config setter. Devices that do not
/// support a setting return an error status; that is expected and only
/// logged, never fatal (the roadmap's "ignored gracefully").
fn log_config(device: &str, setting: &str, result: Result<(), DeviceConfigError>) {
    match result {
        Ok(()) => info!(device, setting, "applied libinput setting"),
        Err(err) => info!(
            device,
            setting,
            ?err,
            "libinput setting not applied (unsupported by device)"
        ),
    }
}

/// Applies every rule matching `device` (in registration order; later rules
/// win). A rule with an empty match string applies to all devices.
fn apply_input_rules(device: &mut LibinputDevice, rules: &[guile::InputRule]) {
    let name = device.name().to_string();
    for rule in rules {
        if rule.match_name.is_empty() || name.contains(&rule.match_name) {
            apply_one_rule(device, &name, rule);
        }
    }
}

fn apply_one_rule(device: &mut LibinputDevice, name: &str, rule: &guile::InputRule) {
    if let Some(on) = rule.tap {
        log_config(name, "tap-to-click", device.config_tap_set_enabled(on));
    }
    if let Some(on) = rule.natural_scroll {
        log_config(
            name,
            "natural-scroll",
            device.config_scroll_set_natural_scroll_enabled(on),
        );
    }
    if let Some(profile) = &rule.accel_profile {
        match profile.as_str() {
            "flat" => log_config(
                name,
                "accel-profile",
                device.config_accel_set_profile(AccelProfile::Flat),
            ),
            "adaptive" => log_config(
                name,
                "accel-profile",
                device.config_accel_set_profile(AccelProfile::Adaptive),
            ),
            other => warn!(other, "wm-configure-input!: unknown accel-profile"),
        }
    }
    if let Some(method) = &rule.click_method {
        match method.as_str() {
            "button-areas" => log_config(
                name,
                "click-method",
                device.config_click_set_method(ClickMethod::ButtonAreas),
            ),
            "clickfinger" => log_config(
                name,
                "click-method",
                device.config_click_set_method(ClickMethod::Clickfinger),
            ),
            other => warn!(other, "wm-configure-input!: unknown click-method"),
        }
    }
}

impl MindeState {
    /// A libinput device arrived (udev backend). Register it for
    /// `(wm-input-devices)`, apply any stored `wm-configure-input!` rules,
    /// retain it for later re-configuration, and fire the optional
    /// `(handle-input-device-added!)` Scheme hook.
    pub(crate) fn libinput_device_added(&mut self, mut device: LibinputDevice) {
        let name = device.name().to_string();
        let capabilities = device_capabilities(&device);
        info!(device = %name, ?capabilities, "input device added");
        apply_input_rules(&mut device, &guile::input_rules());
        guile::register_input_device(name.clone(), capabilities.clone());
        if let Some(udev) = self.udev_data.as_mut() {
            udev.input_devices.push(device);
        }
        guile::on_input_device_added(&name, &capabilities);
    }

    /// A libinput device was removed (udev backend): drop it from the
    /// `(wm-input-devices)` registry and the retained set.
    pub(crate) fn libinput_device_removed(&mut self, device: &LibinputDevice) {
        let name = device.name().to_string();
        info!(device = %name, "input device removed");
        guile::unregister_input_device(&name);
        if let Some(udev) = self.udev_data.as_mut() {
            udev.input_devices.retain(|d| d != device);
        }
    }

    /// Re-applies the stored input rules to every device already present.
    /// Driven by `WmCommand::ReapplyInputConfig` when `wm-configure-input!`
    /// is called at runtime. No-op under winit (no `udev_data`).
    pub(crate) fn reapply_input_config(&mut self) {
        let rules = guile::input_rules();
        let Some(udev) = self.udev_data.as_mut() else {
            return;
        };
        for device in &mut udev.input_devices {
            apply_input_rules(device, &rules);
        }
    }
}

fn setup_dmabuf_global(state: &mut MindeState) {
    let Some(udev) = state.udev_data.as_mut() else {
        return;
    };
    let Ok(renderer) = udev.gpus.single_renderer(&udev.primary_gpu) else {
        return;
    };
    let dmabuf_formats = renderer.dmabuf_formats();
    let Some(default_feedback) =
        DmabufFeedbackBuilder::new(udev.primary_gpu.dev_id(), dmabuf_formats)
            .build()
            .ok()
    else {
        return;
    };
    let mut dmabuf_state = DmabufState::new();
    let global = dmabuf_state.create_global_with_default_feedback::<MindeState>(
        &state.display_handle,
        &default_feedback,
    );
    udev.dmabuf_state = Some((dmabuf_state, global));
}

fn device_added(
    state: &mut MindeState,
    node: DrmNode,
    path: &Path,
) -> Result<(), DeviceAddError> {
    let handle = state.handle.clone();
    let dh = state.display_handle.clone();
    let udev = state.udev_data.as_mut().unwrap();

    let fd = udev
        .session
        .open(
            path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        )
        .map_err(DeviceAddError::DeviceOpen)?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));

    let (drm, notifier) = DrmDevice::new(fd.clone(), true).map_err(DeviceAddError::DrmDevice)?;

    // linux-drm-syncobj-v1 (explicit sync): create the global once, from the
    // first DRM device that supports syncobj timeline eventfds
    // (DRM_CAP_SYNCOBJ_TIMELINE, probed by `supports_syncobj_eventfd`). We only
    // ever render on the primary GPU, so its device fd is the import device
    // Smithay uses to import client acquire/release fences. Hardware without
    // support just never gets the global -- no faked explicit-sync claim.
    if udev.syncobj_state.is_none() {
        if supports_syncobj_eventfd(&fd) {
            info!("drm device supports syncobj timelines; enabling linux-drm-syncobj-v1");
            udev.syncobj_state = Some(DrmSyncobjState::new::<MindeState>(&dh, fd.clone()));
        } else {
            info!("drm device lacks syncobj timeline support; linux-drm-syncobj-v1 not advertised");
        }
    }

    let gbm = GbmDevice::new(fd).map_err(DeviceAddError::GbmDevice)?;

    let registration_token = handle
        .insert_source(
            notifier,
            move |event, metadata, state: &mut MindeState| match event {
                DrmEvent::VBlank(crtc) => state.frame_finish(node, crtc, metadata),
                DrmEvent::Error(err) => error!(%err, "drm error"),
            },
        )
        .unwrap();

    // Try to add this GPU's render node to the shared GpuManager (needed
    // even though we only ever render with the primary node, since
    // `single_renderer` looks the node up there).
    let egl_display = unsafe { EGLDisplay::new(gbm.clone()) };
    if let Ok(display) = egl_display
        && let Ok(egl_device) = EGLDevice::device_for_display(&display)
    {
        let render_node = egl_device
            .try_get_render_node()
            .ok()
            .flatten()
            .unwrap_or(node);
        let _ = udev.gpus.as_mut().add_node(render_node, gbm.clone());
    }

    let allocator = GbmAllocator::new(
        gbm.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    let framebuffer_exporter = GbmFramebufferExporter::new(gbm.clone(), None.into());

    let mut renderer = udev
        .gpus
        .single_renderer(&udev.primary_gpu)
        .map_err(|_| DeviceAddError::DrmNode(CreateDrmNodeError::NotDrmNode))?;
    let render_formats = renderer
        .as_mut()
        .egl_context()
        .dmabuf_render_formats()
        .iter()
        .copied()
        .collect::<FormatSet>();

    let drm_output_manager = DrmOutputManager::new(
        drm,
        allocator,
        framebuffer_exporter,
        Some(gbm),
        SUPPORTED_FORMATS.iter().copied(),
        render_formats,
    );

    udev.devices.insert(
        node,
        DeviceData {
            drm_output_manager,
            registration_token,
            drm_scanner: DrmScanner::new(),
            surfaces: HashMap::new(),
        },
    );

    scan_connectors(state, node);
    Ok(())
}

/// Scans `node` for connector changes and creates/tears down an output
/// per (dis)connected connector (anvil's multi-output policy).
fn scan_connectors(state: &mut MindeState, node: DrmNode) {
    let scan_result = {
        let udev = state.udev_data.as_mut().unwrap();
        let Some(device) = udev.devices.get_mut(&node) else {
            return;
        };
        match device
            .drm_scanner
            .scan_connectors(device.drm_output_manager.device())
        {
            Ok(scan) => scan,
            Err(err) => {
                warn!(?err, "failed to scan connectors");
                return;
            }
        }
    };

    for event in scan_result {
        match event {
            DrmScanEvent::Connected {
                connector,
                crtc: Some(crtc),
            } => {
                if !connector_connected(state, node, &connector, crtc) {
                    warn!(?node, ?crtc, "failed to set up connected connector");
                }
            }
            DrmScanEvent::Disconnected {
                connector,
                crtc: Some(crtc),
            } => {
                connector_disconnected(state, node, &connector, crtc);
            }
            _ => {}
        }
    }
}

fn connector_connected(
    state: &mut MindeState,
    node: DrmNode,
    info: &connector::Info,
    crtc: crtc::Handle,
) -> bool {
    let udev = state.udev_data.as_mut().unwrap();
    let Some(device) = udev.devices.get_mut(&node) else {
        return false;
    };

    let output_name = format!("{}-{}", info.interface().as_str(), info.interface_id());
    info!(?crtc, %output_name, "setting up connector as an output");

    let mode_id = info
        .modes()
        .iter()
        .position(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
        .unwrap_or(0);
    let Some(&drm_mode) = info.modes().get(mode_id) else {
        warn!(%output_name, "connector reports no modes");
        return false;
    };
    let wl_mode = WlMode::from(drm_mode);

    let (phys_w, phys_h) = info.size().unwrap_or((0, 0));
    let output = Output::new(
        output_name.clone(),
        PhysicalProperties {
            size: (phys_w as i32, phys_h as i32).into(),
            subpixel: Subpixel::Unknown,
            make: "Unknown".into(),
            model: "Unknown".into(),
            serial_number: "Unknown".into(),
        },
    );
    let global = output.create_global::<MindeState>(&state.display_handle);
    // Lay outputs out left-to-right in connection order (anvil's policy).
    let position_x = state.space.outputs().fold(0, |acc, o| {
        acc + state
            .space
            .output_geometry(o)
            .map(|g| g.size.w)
            .unwrap_or(0)
    });
    output.set_preferred(wl_mode);
    output.change_current_state(
        Some(wl_mode),
        Some(Transform::Normal),
        None,
        Some((position_x, 0).into()),
    );
    state.space.map_output(&output, (position_x, 0));

    let mut renderer = match udev.gpus.single_renderer(&udev.primary_gpu) {
        Ok(r) => r,
        Err(err) => {
            warn!(%err, "failed to get renderer for output init");
            state.space.unmap_output(&output);
            return false;
        }
    };

    let drm_output = match device
        .drm_output_manager
        .lock()
        .initialize_output::<_, MindeRenderElements<UdevRenderer<'_>>>(
            crtc,
            drm_mode,
            &[info.handle()],
            &output,
            None,
            &mut renderer,
            &DrmOutputRenderElements::default(),
        ) {
        Ok(drm_output) => drm_output,
        Err(err) => {
            warn!(%err, "failed to initialize drm output");
            state.space.unmap_output(&output);
            return false;
        }
    };

    device.surfaces.insert(
        crtc,
        OutputSurface {
            output,
            global: Some(global),
            dh: state.display_handle.clone(),
            drm_output,
            border_buffers: BorderBuffers::default(),
        },
    );

    let first_output = !udev.started;
    udev.started = true;

    state.update_usable_area();
    if first_output {
        guile::on_startup();
    }

    state.render_now(node, crtc);
    true
}

fn connector_disconnected(
    state: &mut MindeState,
    node: DrmNode,
    info: &connector::Info,
    crtc: crtc::Handle,
) {
    let output_name = format!("{}-{}", info.interface().as_str(), info.interface_id());
    info!(?crtc, %output_name, "connector disconnected; removing its output");
    let Some(udev) = state.udev_data.as_mut() else {
        return;
    };
    let Some(device) = udev.devices.get_mut(&node) else {
        return;
    };
    if let Some(surface) = device.surfaces.remove(&crtc) {
        // Void any gamma control on this output before its CRTC goes away
        // (no restore possible once the surface is gone).
        state.gamma_output_removed(&surface.output);
        // OutputSurface::drop removes the global; unmap first.
        state.space.unmap_output(&surface.output);
        drop(surface);
        state.space.refresh();
        state.update_usable_area();
    }
}

fn device_changed(state: &mut MindeState, node: DrmNode) {
    let has_device = state
        .udev_data
        .as_ref()
        .map(|u| u.devices.contains_key(&node))
        .unwrap_or(false);
    if !has_device {
        return;
    }
    scan_connectors(state, node);
}

fn device_removed(state: &mut MindeState, node: DrmNode) {
    let handle = state.handle.clone();
    // Tear down every output this device was driving.
    let connectors: Vec<(connector::Info, crtc::Handle)> = state
        .udev_data
        .as_mut()
        .and_then(|u| u.devices.get_mut(&node))
        .map(|d| {
            d.drm_scanner
                .crtcs()
                .map(|(info, crtc)| (info.clone(), crtc))
                .collect()
        })
        .unwrap_or_default();
    for (info, crtc) in connectors {
        connector_disconnected(state, node, &info, crtc);
    }
    let Some(udev) = state.udev_data.as_mut() else {
        return;
    };
    if let Some(device) = udev.devices.remove(&node) {
        handle.remove(device.registration_token);
    }
}

impl MindeState {
    fn frame_finish(
        &mut self,
        node: DrmNode,
        crtc: crtc::Handle,
        metadata: &mut Option<DrmEventMetadata>,
    ) {
        // Submit the flipped frame and reclaim the presentation feedback that
        // was attached when the frame was queued. Scoped so the udev/device/
        // surface borrows are released before we touch `self.start_time` and
        // deliver the feedback.
        let (output, submitted) = {
            let Some(udev) = self.udev_data.as_mut() else {
                return;
            };
            let Some(device) = udev.devices.get_mut(&node) else {
                return;
            };
            let Some(surface) = device.surfaces.get_mut(&crtc) else {
                return; // output vanished (unplug); don't reschedule repaints
            };
            (surface.output.clone(), surface.drm_output.frame_submitted())
        };

        // wp-presentation-time: mark every surface scanned out on this output
        // as presented, with the real vblank timestamp/sequence when the kernel
        // provided monotonic timestamps (otherwise fall back to our own clock
        // and only claim Vsync).
        match submitted {
            Ok(Some(Some(mut feedback))) => {
                let (clock, flags) = match metadata.as_ref().map(|m| m.time) {
                    Some(DrmEventTime::Monotonic(tp)) => (
                        tp,
                        wp_presentation_feedback::Kind::Vsync
                            | wp_presentation_feedback::Kind::HwClock
                            | wp_presentation_feedback::Kind::HwCompletion,
                    ),
                    _ => (
                        self.start_time.elapsed(),
                        wp_presentation_feedback::Kind::Vsync,
                    ),
                };
                let seq = metadata.as_ref().map(|m| m.sequence as u64).unwrap_or(0);
                let refresh = output
                    .current_mode()
                    .map(|mode| {
                        Refresh::fixed(Duration::from_secs_f64(1_000f64 / mode.refresh as f64))
                    })
                    .unwrap_or(Refresh::Unknown);
                feedback.presented::<_, Monotonic>(clock, refresh, seq, flags);
            }
            Ok(_) => {}
            Err(err) => warn!(%err, "drm frame_submitted failed"),
        }

        // Schedule the next repaint roughly one frame out.
        self.handle
            .insert_source(
                Timer::from_duration(Duration::from_millis(16)),
                move |_, _, state| {
                    state.render_now(node, crtc);
                    TimeoutAction::Drop
                },
            )
            .ok();
    }

    /// Dmabuf capture constraints for `ext-image-copy-capture-v1`: the
    /// primary render node plus its supported format/modifier pairs. Offered
    /// so a future zero-copy screen-cast path can allocate GPU buffers; shm
    /// capture (grim) needs none of this. `None` if no renderer is available.
    pub(crate) fn dmabuf_capture_constraints(
        &mut self,
    ) -> Option<smithay::wayland::image_copy_capture::DmabufConstraints> {
        let udev = self.udev_data.as_mut()?;
        let node = udev.primary_gpu;
        let renderer = udev.gpus.single_renderer(&node).ok()?;
        let mut grouped: HashMap<Fourcc, Vec<smithay::backend::allocator::Modifier>> =
            HashMap::new();
        for format in renderer.dmabuf_formats().iter() {
            grouped
                .entry(format.code)
                .or_default()
                .push(format.modifier);
        }
        Some(smithay::wayland::image_copy_capture::DmabufConstraints {
            node,
            formats: grouped.into_iter().collect(),
        })
    }

    fn handle_repaint_now(&mut self, node: DrmNode, crtc: crtc::Handle) {
        self.render_now(node, crtc);
    }

    /// Resolves an output to the DRM device fd, CRTC, and gamma ramp length
    /// needed to drive its gamma ramps. `None` if the output isn't a udev
    /// surface or its CRTC reports no gamma. Used by the gamma-control
    /// handler (udev-only; winit never advertises the global).
    pub fn gamma_info_for_output(
        &self,
        output: &Output,
    ) -> Option<(DrmDeviceFd, crtc::Handle, u32)> {
        use smithay::reexports::drm::control::Device as ControlDevice;
        let udev = self.udev_data.as_ref()?;
        for device in udev.devices.values() {
            for surface in device.surfaces.values() {
                if &surface.output != output {
                    continue;
                }
                let (fd, crtc) = surface.drm_output.with_compositor(|compositor| {
                    let drm_surface = compositor.surface();
                    (drm_surface.device_fd().clone(), drm_surface.crtc())
                });
                let size = fd.get_crtc(crtc).ok()?.gamma_length();
                return Some((fd, crtc, size));
            }
        }
        None
    }

    /// Forces an immediate repaint of every udev output. Used by the
    /// session-lock handler so a blank frame reaches every screen before the
    /// lock is confirmed. No-op under winit (its redraw loop repaints
    /// continuously; the lock flag makes those frames blank on its own).
    pub(crate) fn render_all_outputs_now(&mut self) {
        let Some(udev) = self.udev_data.as_ref() else {
            return;
        };
        let targets: Vec<(DrmNode, crtc::Handle)> = udev
            .devices
            .iter()
            .flat_map(|(node, device)| device.surfaces.keys().map(move |crtc| (*node, *crtc)))
            .collect();
        for (node, crtc) in targets {
            self.render_now(node, crtc);
        }
    }

    fn render_now(&mut self, node: DrmNode, crtc: crtc::Handle) {
        // If no frame was queued (no damage, or a render error), no VBlank
        // will arrive to drive `frame_finish`, so the repaint chain would
        // die -- keep it alive with a timer instead.
        let queued = match self.render_surface(node, crtc) {
            Ok(queued) => queued,
            Err(err) => {
                warn!(%err, "error rendering udev output");
                false
            }
        };
        if !queued {
            self.handle
                .insert_source(
                    Timer::from_duration(Duration::from_millis(16)),
                    move |_, _, state| {
                        state.render_now(node, crtc);
                        TimeoutAction::Drop
                    },
                )
                .ok();
        }
    }

    /// Renders one frame; returns whether a frame was queued to the DRM
    /// surface (i.e. whether a VBlank event is expected).
    fn render_surface(
        &mut self,
        node: DrmNode,
        crtc: crtc::Handle,
    ) -> Result<bool, SwapBuffersError> {
        let udev = self
            .udev_data
            .as_mut()
            .ok_or(SwapBuffersError::AlreadySwapped)?;
        let primary_gpu = udev.primary_gpu;

        // Split borrows: the surface lives in `devices`, the renderer in
        // `gpus` -- disjoint fields of the same UdevBackendData.
        let UdevBackendData { gpus, devices, .. } = udev;
        let Some(output_surface) = devices
            .get_mut(&node)
            .and_then(|d| d.surfaces.get_mut(&crtc))
        else {
            return Ok(false);
        };

        let mut renderer = gpus
            .single_renderer(&primary_gpu)
            .map_err(|_| SwapBuffersError::AlreadySwapped)?;

        let output = output_surface.output.clone();
        let Some(output_geo) = self.space.output_geometry(&output) else {
            tracing::warn!("render requested for disconnected output");
            return Ok(false);
        };
        let scale = smithay::utils::Scale::from(output.current_scale().fractional_scale());

        // Locked: render ONLY this output's lock surface (or solid black if
        // it has not committed / the client died). Never the desktop -- this
        // is the ext-session-lock guarantee. `lock_surfaces` is borrowed as a
        // field here (not via `lock_surface_for`, whose `&self` would clash
        // with the outstanding `udev_data` borrow above).
        if self.locked {
            let lock_surface = self
                .lock_surfaces
                .iter()
                .find(|(o, _)| o == &output)
                .map(|(_, surface)| surface)
                .filter(|surface| surface.alive());
            let mut elements: Vec<MindeRenderElements<UdevRenderer<'_>>> = Vec::new();
            if let Some(lock) = lock_surface {
                elements =
                    smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
                        &mut renderer,
                        lock.wl_surface(),
                        (0, 0),
                        scale,
                        1.0,
                        smithay::backend::renderer::element::Kind::Unspecified,
                    );
            }
            let render_result = output_surface
                .drm_output
                .render_frame(
                    &mut renderer,
                    &elements,
                    smithay::backend::renderer::Color32F::new(0.0, 0.0, 0.0, 1.0),
                    FrameFlags::DEFAULT,
                )
                .map_err(|err| match err {
                    smithay::backend::drm::compositor::RenderFrameError::PrepareFrame(err) => {
                        SwapBuffersError::from(err)
                    }
                    smithay::backend::drm::compositor::RenderFrameError::RenderFrame(
                        smithay::backend::renderer::damage::Error::Rendering(err),
                    ) => SwapBuffersError::from(err),
                    _ => SwapBuffersError::AlreadySwapped,
                })?;
            let queued = !render_result.is_empty;
            if queued {
                output_surface
                    .drm_output
                    .queue_frame(None)
                    .map_err(Into::<SwapBuffersError>::into)?;
            }
            // Frame callback so the lock client keeps drawing.
            if let Some((_, lock)) = self.lock_surfaces.iter().find(|(o, _)| o == &output) {
                smithay::desktop::utils::send_frames_surface_tree(
                    lock.wl_surface(),
                    &output,
                    self.start_time.elapsed(),
                    Some(Duration::ZERO),
                    |_, _| Some(output.clone()),
                );
            }
            self.popups.cleanup();
            let _ = self.display_handle.flush_clients();
            return Ok(queued);
        }

        let mut custom: Vec<MindeRenderElements<UdevRenderer<'_>>> = Vec::new();

        // Cursor, at the current pointer location.
        if output_geo.to_f64().contains(self.pointer_location) {
            let hotspot = self.cursor_state.hotspot();
            let cursor_pos = self.pointer_location - output_geo.loc.to_f64();
            let cursor_phys = (cursor_pos - hotspot.to_f64())
                .to_physical(scale)
                .to_i32_round();
            custom.extend(
                self.cursor_state
                    .render_elements(&mut renderer, cursor_phys, scale),
            );
        }

        // Message overlay, centered (below the cursor, above windows) --
        // on the output holding the current frame only (StumpWM shows
        // messages on the current head).
        let message_here = self
            .focus_rect
            .map(|r| output_geo.contains(r.loc))
            .unwrap_or(true);
        if let Some(msg) = self.message.as_ref().filter(|_| message_here)
            && let Some(elem) = crate::render::message_element(
                &mut renderer,
                msg,
                (output_geo.size.w, output_geo.size.h),
                scale,
            )
        {
            custom.push(elem);
        }

        // Positioned overlays (fselect/expose frame labels): global
        // coords, shifted into this output's framebuffer like everything
        // else; only drawn when they land on this output.
        for (loc, msg) in self
            .overlays
            .iter()
            .filter(|(l, _)| output_geo.contains(*l))
        {
            if let Some(elem) =
                crate::render::overlay_element(&mut renderer, msg, *loc - output_geo.loc, scale)
            {
                custom.push(elem);
            }
        }

        // Layer-shell surfaces: upper (top/overlay -- fuzzel, swaylock)
        // draw above windows; lower (bottom/background -- swaybg) below.
        use smithay::wayland::shell::wlr_layer::Layer as WlrLayer;
        let layer_map = smithay::desktop::layer_map_for_output(&output);
        let (lower, upper): (Vec<&smithay::desktop::LayerSurface>, Vec<_>) = layer_map
            .layers()
            .partition(|s| matches!(s.layer(), WlrLayer::Background | WlrLayer::Bottom));
        macro_rules! layer_elements {
            ($surfaces:expr) => {
                $surfaces.iter().flat_map(|surface| {
                    let loc = layer_map
                        .layer_geometry(surface)
                        .map(|geo| geo.loc)
                        .unwrap_or_default();
                    smithay::backend::renderer::element::AsRenderElements::<
                                        UdevRenderer<'_>,
                                    >::render_elements::<MindeRenderElements<UdevRenderer<'_>>>(
                                        *surface,
                                        &mut renderer,
                                        loc.to_physical_precise_round(scale),
                                        scale,
                                        1.0,
                                    )
                })
            };
        }
        custom.extend(layer_elements!(upper));

        // Border around the selected frame (falling back to the focused
        // window before the first sync). Rects are global; this output's
        // framebuffer is output-local, so shift by the output origin and
        // only draw when the frame is (partly) on this output.
        if let Some(geo) = self.focus_rect.or_else(|| {
            self.focused_window
                .as_ref()
                .and_then(|w| self.space.element_geometry(w))
        }) && geo.overlaps(output_geo)
        {
            let mut local = geo;
            local.loc -= output_geo.loc;
            custom.extend(
                output_surface
                    .border_buffers
                    .elements(local, scale, self.border_color),
            );
        }

        // Window surfaces, front-to-back (custom elements above are drawn
        // first, i.e. on top; `space.elements()` yields back-to-front, so
        // walk it in reverse). Skip windows entirely off this output and
        // shift the rest into output-local coordinates.
        let mut all_elements: Vec<MindeRenderElements<UdevRenderer<'_>>> = custom;
        for window in self.space.elements().rev() {
            let Some(loc) = self.space.element_location(window) else {
                continue;
            };
            if !self
                .space
                .element_geometry(window)
                .map(|g| g.overlaps(output_geo))
                .unwrap_or(false)
            {
                continue;
            }
            // The space stores the element location as the window
            // GEOMETRY origin; the buffer's top-left is geometry.loc
            // further up/left (CSD shadow margins live in that fringe).
            // Rendering at `loc` directly shifts CSD clients (GTK: zen,
            // inkscape, gimp) into the frame by their shadow width --
            // smithay's own space renderer subtracts this too
            // (render_location, desktop/space/mod.rs).
            let phys_loc =
                (loc - window.geometry().loc - output_geo.loc).to_physical_precise_round(scale);
            all_elements.extend(smithay::backend::renderer::element::AsRenderElements::<
                UdevRenderer<'_>,
            >::render_elements(
                window, &mut renderer, phys_loc, scale, 1.0
            ));
        }
        // Background/bottom layers under everything.
        all_elements.extend(layer_elements!(lower));

        let render_result = output_surface
            .drm_output
            .render_frame(
                &mut renderer,
                &all_elements,
                smithay::backend::renderer::Color32F::new(0.1, 0.1, 0.1, 1.0),
                FrameFlags::DEFAULT,
            )
            .map_err(|err| match err {
                smithay::backend::drm::compositor::RenderFrameError::PrepareFrame(err) => {
                    SwapBuffersError::from(err)
                }
                smithay::backend::drm::compositor::RenderFrameError::RenderFrame(
                    smithay::backend::renderer::damage::Error::Rendering(err),
                ) => SwapBuffersError::from(err),
                _ => SwapBuffersError::AlreadySwapped,
            })?;

        // wp-presentation-time: collect the feedback owed to every surface
        // scanned out on this output. `update_surface_primary_scanout_output`
        // records which output each surface landed on (needed by
        // `surface_primary_scanout_output`), and the render report flags
        // zero-copy scanout. The feedback rides along as the queued frame's
        // user-data and is delivered on the matching vblank in `frame_finish`.
        // The `layer_map` guard from above is reused (a second guard on the
        // same output panics -- see the frame-callback NOTE below).
        let mut presentation_feedback = OutputPresentationFeedback::new(&output);
        for window in self.space.elements() {
            if self.space.outputs_for_element(window).contains(&output) {
                window.with_surfaces(|surface, states| {
                    update_surface_primary_scanout_output(
                        surface,
                        &output,
                        states,
                        None,
                        &render_result.states,
                        default_primary_scanout_output_compare,
                    );
                });
                window.take_presentation_feedback(
                    &mut presentation_feedback,
                    surface_primary_scanout_output,
                    |surface, _| {
                        surface_presentation_feedback_flags_from_states(
                            surface,
                            None,
                            &render_result.states,
                        )
                    },
                );
            }
        }
        for layer_surface in layer_map.layers() {
            layer_surface.with_surfaces(|surface, states| {
                update_surface_primary_scanout_output(
                    surface,
                    &output,
                    states,
                    None,
                    &render_result.states,
                    default_primary_scanout_output_compare,
                );
            });
            layer_surface.take_presentation_feedback(
                &mut presentation_feedback,
                surface_primary_scanout_output,
                |surface, _| {
                    surface_presentation_feedback_flags_from_states(
                        surface,
                        None,
                        &render_result.states,
                    )
                },
            );
        }

        let queued = !render_result.is_empty;
        if queued {
            output_surface
                .drm_output
                .queue_frame(Some(presentation_feedback))
                .map_err(Into::<SwapBuffersError>::into)?;
        } else {
            // No frame will be scanned out, so no vblank will arrive to deliver
            // the feedback; discard it now rather than leak the callbacks.
            presentation_feedback.discarded();
        }

        // Frame callbacks: windows on this output, plus -- from the first
        // output's pass only -- windows parked offscreen (they overlap no
        // output but must keep receiving callbacks or they stall while
        // hidden). Avoids double-firing clients once per head.
        let is_first_output = self.space.outputs().next() == Some(&output);
        self.space.elements().for_each(|window| {
            let geo = self.space.element_geometry(window);
            let on_this = geo.map(|g| g.overlaps(output_geo)).unwrap_or(false);
            let parked = geo
                .map(|g| {
                    !self
                        .space
                        .outputs()
                        .filter_map(|o| self.space.output_geometry(o))
                        .any(|og| g.overlaps(og))
                })
                .unwrap_or(true);
            if on_this || (parked && is_first_output) {
                window.send_frame(
                    &output,
                    self.start_time.elapsed(),
                    Some(Duration::ZERO),
                    |_, _| Some(output.clone()),
                );
            }
        });
        // Layer surfaces need frame callbacks too, or clients like fuzzel
        // draw once and then wait forever before rendering typed input.
        // NOTE: reuse the `layer_map` guard from above -- calling
        // layer_map_for_output again here double-borrows the output's
        // RefCell and panics on the first frame (froze the whole session
        // on the TTY once; the winit path is immune because
        // space::render_output scopes its own borrow).
        for layer in layer_map.layers() {
            layer.send_frame(
                &output,
                self.start_time.elapsed(),
                Some(Duration::ZERO),
                |_, _| Some(output.clone()),
            );
        }
        self.space.refresh();
        self.popups.cleanup();

        // Satisfy any queued screen-capture frames for this output. Drop the
        // layer_map guard first: `output_scene_elements` re-opens it, and two
        // live guards on the same output's RefCell panic (see the frame-
        // callback NOTE above).
        drop(layer_map);
        if !self.pending_captures.is_empty() {
            let time = self.start_time.elapsed();
            let focus = self.focus_rect.or_else(|| {
                self.focused_window
                    .as_ref()
                    .and_then(|w| self.space.element_geometry(w))
            });
            crate::handlers::screencopy::satisfy_output_captures(
                &mut renderer,
                &output,
                output_geo,
                scale,
                time,
                &mut self.pending_captures,
                &self.space,
                &mut self.cursor_state,
                self.pointer_location,
                self.message.as_ref(),
                &self.overlays,
                focus,
                self.border_color,
            );
        }

        let _ = self.display_handle.flush_clients();

        Ok(queued)
    }
}
