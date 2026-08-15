// SPDX-License-Identifier: MIT

//! Standalone DRM/udev/libinput backend: owns a VT via libseat, scans the
//! primary GPU for the first usable connector, and renders via GBM/EGL.
//! Adapted from Smithay's `anvil` example (`anvil/src/udev.rs`); see
//! README for the exact upstream revision this mirrors.
//!
//! Trimmed vs. anvil, per this compositor's needs:
//! - multi-output: every connected connector becomes a head
//!   ([`HeadEntry`]), enabled by default and laid out left-to-right in
//!   connection order (anvil's policy); hotplug adds and removes heads at
//!   runtime via `DrmScanner`, wlr-output-management (`enable_head` /
//!   `disable_head`) switches CRTCs on and off, and Scheme is told through
//!   `update_usable_area` -> `handle-heads-change!`.
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
//!
//! Repaint scheduling is vblank- and damage-driven (see [`RedrawState`]):
//! every scene change marks the affected outputs dirty through
//! [`MindeState::schedule_redraw`]; an output renders once from a calloop
//! idle callback (coalescing a burst of events into one frame), then waits
//! for the frame's vblank and renders again right away if it was dirtied in
//! the meantime. At idle nothing runs at all -- no timer, no element-list
//! rebuild. Frame callbacks are sent from every render pass, and every
//! client commit dirties the outputs, so a client waiting on a callback is
//! never starved. A short watchdog covers a vblank that never arrives.

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
            DrmEventTime, DrmNode, NodeType, VrrSupport,
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
        drm::control::{self, Device as ControlDevice, ModeFlags, ModeTypeFlags, connector, crtc},
        input::{Device as LibinputDevice, Libinput},
        rustix::fs::OFlags,
        wayland_server::backend::GlobalId,
    },
    utils::{DeviceFd, Logical, Monotonic, Point},
    wayland::{
        dmabuf::{DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        drm_syncobj::{DrmSyncobjHandler, DrmSyncobjState, supports_syncobj_eventfd},
        presentation::{PresentationState, Refresh},
    },
};
use smithay_drm_extras::drm_scanner::{DrmScanEvent, DrmScanner};
use tracing::{debug, error, info, warn};

use crate::edid::{EdidInfo, parse_edid};
use crate::handlers::output_management::HeadChange;
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

/// Where an output's repaint loop stands. Transitions live in
/// `schedule_redraw` (dirtying), `render_now` (rendering), `frame_finish`
/// (vblank) and the two timer callbacks; see the module docs.
enum RedrawState {
    /// Nothing in flight; the next dirtying event schedules a render.
    Idle,
    /// A render is queued as a calloop idle callback (coalesces the rest of
    /// the current dispatch batch: a burst of commits or 1000 Hz motion).
    Scheduled,
    /// A frame was queued to KMS; rendering again waits for its vblank. The
    /// watchdog timer fires if the vblank never comes (VT switch mid-flip,
    /// driver hiccup) so the loop cannot get stuck here.
    WaitingForVblank { watchdog: RegistrationToken },
    /// The last render produced no damage or failed, so no vblank is
    /// expected; rate-limit the next render to the refresh interval.
    WaitingForTimer { token: RegistrationToken },
}

/// The render-side half of an enabled head: the DRM output driving the
/// CRTC plus per-output render caches. `None` in [`HeadEntry::surface`]
/// means the CRTC is off (head disabled); dropping it releases the CRTC
/// (see `Drop for DrmOutput` in Smithay's `backend/drm/output.rs`).
struct OutputSurface {
    drm_output: DrmOutput<
        GbmAllocator<DrmDeviceFd>,
        GbmFramebufferExporter<DrmDeviceFd>,
        FrameUserData,
        DrmDeviceFd,
    >,
    border_buffers: BorderBuffers,
    /// Something on this output changed since the last render.
    dirty: bool,
    redraw: RedrawState,
}

/// The output's refresh interval (from its current mode), or 60 Hz when the
/// mode is unknown. Drives the rate-limit and watchdog timers.
fn refresh_interval(output: &Output) -> Duration {
    output
        .current_mode()
        .map(|mode| mode.refresh)
        .filter(|refresh| *refresh > 0)
        .map(|refresh| Duration::from_secs_f64(1_000f64 / refresh as f64))
        .unwrap_or(Duration::from_millis(16))
}

impl OutputSurface {
    /// Cancels any pending repaint timer and returns the loop to `Idle`
    /// (keeping `dirty` as is). Precedes every render and a session pause.
    fn park_redraw(
        &mut self,
        handle: &smithay::reexports::calloop::LoopHandle<'static, MindeState>,
    ) {
        match std::mem::replace(&mut self.redraw, RedrawState::Idle) {
            RedrawState::WaitingForVblank { watchdog: token }
            | RedrawState::WaitingForTimer { token } => handle.remove(token),
            RedrawState::Idle | RedrawState::Scheduled => {}
        }
    }
}

/// DPMS state of an enabled head (`wlr-output-power-management`). `Off`
/// means the CRTC is switched off (no [`OutputSurface`]) while the head
/// stays enabled: mapped, `wl_output` global present, windows in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PowerState {
    On,
    Off,
}

/// One connected connector and its CRTC, whether or not it is currently
/// enabled. Lives in its device's `heads` map, keyed by CRTC. The
/// [`Output`] (and its stable id in user data) survives disable/enable so
/// Scheme frames restore per head; only the `wl_output` global and the
/// render surface come and go with the enabled state.
struct HeadEntry {
    output: Output,
    connector: connector::Handle,
    /// Advertised (non-interlaced) connector modes paired with the DRM
    /// mode they came from, so a mode requested via wlr-output-management
    /// can be translated back to a modeset.
    modes: Vec<(WlMode, control::Mode)>,
    /// The `wl_output` global; present only while the head is enabled
    /// (like wlroots, so layer clients don't stall on a dead output).
    global: Option<GlobalId>,
    /// `Some` while enabled and powered on; `None` while disabled or
    /// powered off (DPMS).
    surface: Option<OutputSurface>,
    /// DPMS state; only meaningful while enabled (reset to `On` on
    /// disable so a re-enabled head always lights up).
    power: PowerState,
    /// What the connector reports about variable refresh rate, cached
    /// when the head is enabled (`NotSupported` while disabled or unknown).
    vrr_support: VrrSupport,
    /// Desired adaptive-sync state; re-applied whenever the CRTC is
    /// (re)initialised so it survives disable/enable cycles.
    vrr: bool,
}

impl HeadEntry {
    /// Enabled: has a `wl_output` global (and a surface unless powered
    /// off).
    fn enabled(&self) -> bool {
        self.global.is_some()
    }
}

/// Translates a mode requested through wlr-output-management into the
/// connector mode to set. An advertised mode (`custom == false`) must match
/// exactly; a custom mode matches a connector mode of the same size whose
/// refresh is within +-1000 mHz (a `wlr-randr --custom-mode 1920x1080@60`
/// is meant to hit the connector's 59.94 Hz mode). Real custom modelines
/// are not supported: they would need `drmModeAttachMode` and the kernel
/// tends to reject them anyway. Generic over the connector-mode type only
/// so it can be unit-tested without a DRM device (`control::Mode` cannot
/// be constructed outside the drm crate).
fn resolve_connector_mode<M: Copy>(
    modes: &[(WlMode, M)],
    wanted: WlMode,
    custom: bool,
) -> Option<M> {
    if let Some((_, drm)) = modes.iter().find(|(wl, _)| *wl == wanted) {
        return Some(*drm);
    }
    if !custom {
        return None;
    }
    modes
        .iter()
        .filter(|(wl, _)| wl.size == wanted.size)
        .map(|(wl, drm)| ((wl.refresh - wanted.refresh).abs(), *drm))
        .filter(|(diff, _)| *diff <= 1000)
        .min_by_key(|(diff, _)| *diff)
        .map(|(_, drm)| drm)
}

struct DeviceData {
    drm_output_manager: GbmDrmOutputManager,
    registration_token: RegistrationToken,
    drm_scanner: DrmScanner,
    heads: HashMap<crtc::Handle, HeadEntry>,
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
    /// Set while the libseat session is paused (VT switched away): the DRM
    /// device is inactive, so renders are deferred (outputs only accumulate
    /// `dirty`) until `ActivateSession` repaints everything.
    paused: bool,
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
    pub(crate) input_devices: Vec<LibinputDevice>,
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
        paused: false,
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
                let handle = state.handle.clone();
                if let Some(udev) = state.udev_data.as_mut() {
                    udev.paused = true;
                    for device in udev.devices.values_mut() {
                        device.drm_output_manager.pause();
                        // Park every repaint loop: no timer may fire into an
                        // inactive device, and a vblank for a flip that was
                        // in flight may never arrive.
                        for head in device.heads.values_mut() {
                            if let Some(surface) = head.surface.as_mut() {
                                surface.park_redraw(&handle);
                            }
                        }
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
                    udev.paused = false;
                    for (node, device) in udev.devices.iter_mut() {
                        let _ = device.drm_output_manager.lock().activate(false);
                        for (crtc, head) in device.heads.iter() {
                            if head.enabled() {
                                to_repaint.push((*node, *crtc));
                            }
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

fn device_added(state: &mut MindeState, node: DrmNode, path: &Path) -> Result<(), DeviceAddError> {
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
            heads: HashMap::new(),
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

/// The single `control::Mode` -> `WlMode` conversion used by this backend
/// (Smithay's `From` impl, wrapped so every site agrees on refresh rounding).
fn wl_mode_from_drm(mode: control::Mode) -> WlMode {
    WlMode::from(mode)
}

/// Read and parse the connector's `EDID` blob property, if any.
fn read_edid(device: &impl ControlDevice, connector: connector::Handle) -> Option<EdidInfo> {
    let props = device.get_properties(connector).ok()?;
    let (info, value) = props
        .into_iter()
        .filter_map(|(handle, value)| Some((device.get_property(handle).ok()?, value)))
        .find(|(info, _)| info.name().to_str() == Ok("EDID"))?;
    let blob = info.value_type().convert_value(value).as_blob()?;
    let data = device.get_property_blob(blob).ok()?;
    parse_edid(&data)
}

/// A connector was connected: register it as a head and bring it up at
/// its preferred mode, to the right of the outputs already enabled
/// (anvil's left-to-right policy). Heads default to enabled; a daemon such
/// as kanshi/shikane may disable it again through wlr-output-management.
fn connector_connected(
    state: &mut MindeState,
    node: DrmNode,
    info: &connector::Info,
    crtc: crtc::Handle,
) -> bool {
    let Some(preferred) = create_head(state, node, info, crtc) else {
        return false;
    };
    let position = right_edge_of_outputs(state);
    match enable_head(state, node, crtc, preferred, position) {
        Ok(()) => true,
        Err(err) => {
            warn!(%err, "failed to enable hotplugged head");
            false
        }
    }
}

/// The x coordinate just right of every enabled output: where a newly
/// enabled head goes when nobody asked for a position.
pub(crate) fn right_edge_of_outputs(state: &MindeState) -> Point<i32, Logical> {
    let x = state
        .space
        .outputs()
        .filter_map(|o| state.space.output_geometry(o))
        .map(|g| g.loc.x + g.size.w)
        .max()
        .unwrap_or(0);
    (x, 0).into()
}

/// Creates the [`Output`] for a freshly connected connector -- identity
/// from EDID, the full mode list, preferred mode -- registers it as a
/// (still disabled) head with wlr-output-management and stores the
/// [`HeadEntry`]. Returns the preferred DRM mode, `None` if the connector
/// is unusable (no modes).
fn create_head(
    state: &mut MindeState,
    node: DrmNode,
    info: &connector::Info,
    crtc: crtc::Handle,
) -> Option<control::Mode> {
    let udev = state.udev_data.as_mut().unwrap();
    let device = udev.devices.get_mut(&node)?;

    let output_name = format!("{}-{}", info.interface().as_str(), info.interface_id());
    info!(?crtc, %output_name, "setting up connector as a head");

    let mode_id = info
        .modes()
        .iter()
        .position(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
        .unwrap_or(0);
    let Some(&drm_mode) = info.modes().get(mode_id) else {
        warn!(%output_name, "connector reports no modes");
        return None;
    };
    let wl_mode = wl_mode_from_drm(drm_mode);

    // Every non-interlaced connector mode, deduplicated on the advertised
    // (size, refresh) pair; the first DRM mode wins for a given WlMode.
    let mut modes: Vec<(WlMode, control::Mode)> = Vec::new();
    for &m in info.modes() {
        if m.flags().contains(ModeFlags::INTERLACE) {
            continue;
        }
        let wl = wl_mode_from_drm(m);
        if modes.iter().all(|(existing, _)| *existing != wl) {
            modes.push((wl, m));
        }
    }

    let edid = read_edid(device.drm_output_manager.device(), info.handle());
    let (make, model, serial) = match &edid {
        Some(e) => (e.make.clone(), e.model.clone(), e.serial.clone()),
        None => ("Unknown".into(), "Unknown".into(), "Unknown".into()),
    };
    info!(
        %output_name, %make, %model, %serial,
        modes = modes.len(), preferred = ?wl_mode,
        "output identity and mode list"
    );

    let (phys_w, phys_h) = info.size().unwrap_or((0, 0));
    let output = Output::new(
        output_name.clone(),
        PhysicalProperties {
            size: (phys_w as i32, phys_h as i32).into(),
            subpixel: Subpixel::from(info.subpixel()),
            make,
            model,
            serial_number: serial,
        },
    );
    if let Some(edid) = edid {
        // Smithay's description string is fixed at construction; the EDID
        // identity lives in user data so protocol handlers can build the
        // wlroots-style "Make Model Serial" description (crate::edid).
        output.user_data().insert_if_missing(|| edid);
    }
    for (wl, _) in &modes {
        output.add_mode(*wl);
    }
    output.set_preferred(wl_mode);

    // A previous entry on this CRTC (should not happen: DrmScanner reports
    // a disconnect first) is torn down so its CRTC is free.
    if device.heads.contains_key(&crtc) {
        warn!(?crtc, "replacing an existing head on this crtc");
        disable_head(state, node, crtc);
        remove_head(state, node, crtc);
    }
    let udev = state.udev_data.as_mut().unwrap();
    let device = udev.devices.get_mut(&node)?;
    device.heads.insert(
        crtc,
        HeadEntry {
            output: output.clone(),
            connector: info.handle(),
            modes,
            global: None,
            surface: None,
            power: PowerState::On,
            vrr_support: VrrSupport::NotSupported,
            vrr: false,
        },
    );
    // Assign the stable head id now so it survives disable/enable cycles.
    state.output_id(&output);
    state.output_management_add_output(&output);
    Some(drm_mode)
}

/// Enables a known head: creates its `wl_output` global, sets `mode` and
/// `position`, maps it into the space and starts driving the CRTC. A
/// no-op for an already enabled head. Used on hotplug and by the
/// wlr-output-management apply path (`backend_realize_head`).
///
/// This does the output-state and space bookkeeping itself (rather than
/// leaving it to `commit_head`) because `initialize_output` reads the
/// output's current mode and the render loop needs the output mapped;
/// `commit_head` re-applying the same values afterwards is idempotent.
pub(crate) fn enable_head(
    state: &mut MindeState,
    node: DrmNode,
    crtc: crtc::Handle,
    mode: control::Mode,
    position: Point<i32, Logical>,
) -> Result<(), String> {
    let dh = state.display_handle.clone();
    let udev = state.udev_data.as_mut().ok_or("no udev backend")?;
    let device = udev.devices.get_mut(&node).ok_or("unknown drm device")?;
    let entry = device.heads.get_mut(&crtc).ok_or("unknown head")?;
    if entry.enabled() {
        return Ok(());
    }
    let output = entry.output.clone();
    let wl_mode = wl_mode_from_drm(mode);
    info!(?crtc, output = %output.name(), ?wl_mode, ?position, "enabling head");

    if entry.global.is_none() {
        entry.global = Some(output.create_global::<MindeState>(&dh));
    }
    entry.power = PowerState::On;
    output.change_current_state(Some(wl_mode), None, None, Some(position));
    state.space.map_output(&output, position);

    if let Err(err) = init_surface(udev, node, crtc, mode) {
        // Back to fully disabled: no global (that is what `enabled()`
        // means), not mapped.
        if let Some(global) = udev
            .devices
            .get_mut(&node)
            .and_then(|d| d.heads.get_mut(&crtc))
            .and_then(|e| e.global.take())
        {
            dh.remove_global::<MindeState>(global);
        }
        state.space.unmap_output(&output);
        return Err(err);
    }

    let first_output = !udev.started;
    udev.started = true;

    state.space.refresh();
    state.update_usable_area();
    state.update_fractional_scales();
    state.pointer_location = state.clamp_to_outputs(state.pointer_location);
    if first_output {
        guile::on_startup();
    }

    state.render_now(node, crtc);
    Ok(())
}

/// Starts driving the CRTC of the head on `(node, crtc)` at `mode`:
/// creates the [`OutputSurface`] in the idle redraw state. The output must
/// already carry its mode and be mapped in the space (the render loop
/// reads both). Shared by [`enable_head`] and the DPMS power-on path.
fn init_surface(
    udev: &mut UdevBackendData,
    node: DrmNode,
    crtc: crtc::Handle,
    mode: control::Mode,
) -> Result<(), String> {
    let UdevBackendData {
        gpus,
        devices,
        primary_gpu,
        ..
    } = udev;
    let device = devices.get_mut(&node).ok_or("unknown drm device")?;
    let entry = device.heads.get_mut(&crtc).ok_or("unknown head")?;
    let mut renderer = gpus
        .single_renderer(primary_gpu)
        .map_err(|err| format!("failed to get renderer for output init: {err}"))?;
    let drm_output = device
        .drm_output_manager
        .lock()
        .initialize_output::<_, MindeRenderElements<UdevRenderer<'_>>>(
            crtc,
            mode,
            &[entry.connector],
            &entry.output,
            None,
            &mut renderer,
            &DrmOutputRenderElements::default(),
        )
        .map_err(|err| format!("failed to initialize drm output: {err}"))?;
    drop(renderer);
    // Cache what the connector says about VRR and re-apply the desired
    // state so a disable/enable (or power) cycle keeps adaptive sync.
    entry.vrr_support = drm_output
        .with_compositor(|c| c.vrr_supported(entry.connector))
        .unwrap_or_else(|err| {
            warn!(%err, "failed to query vrr support");
            VrrSupport::NotSupported
        });
    if entry.vrr {
        if entry.vrr_support == VrrSupport::NotSupported {
            warn!(output = %entry.output.name(), "adaptive sync no longer supported; dropping it");
            entry.vrr = false;
        } else if let Err(err) = drm_output.with_compositor(|c| c.use_vrr(true)) {
            warn!(%err, output = %entry.output.name(), "failed to re-enable adaptive sync");
            entry.vrr = false;
        }
    }
    entry.surface = Some(OutputSurface {
        drm_output,
        border_buffers: BorderBuffers::default(),
        dirty: true,
        redraw: RedrawState::Idle,
    });
    Ok(())
}

/// Disables an enabled head: the CRTC is switched off (by dropping the
/// [`OutputSurface`]), the output leaves the space and every surface, and
/// its `wl_output` global is removed like wlroots does. The [`HeadEntry`]
/// (and the Output with its stable id) stays, so the head remains
/// advertised to wlr-output-management clients and can be re-enabled. A
/// no-op for a head that is already disabled.
pub(crate) fn disable_head(state: &mut MindeState, node: DrmNode, crtc: crtc::Handle) {
    let dh = state.display_handle.clone();
    let handle = state.handle.clone();
    let Some(udev) = state.udev_data.as_mut() else {
        return;
    };
    let Some(device) = udev.devices.get_mut(&node) else {
        return;
    };
    let Some(entry) = device.heads.get_mut(&crtc) else {
        return;
    };
    if !entry.enabled() {
        return;
    }
    // `None` when the head was powered off (DPMS) before being disabled.
    let mut surface = entry.surface.take();
    // Cancel any pending watchdog/rate-limit timer for this surface so a
    // stale token cannot fire into a disabled head.
    if let Some(surface) = surface.as_mut() {
        surface.park_redraw(&handle);
    }
    entry.power = PowerState::On;

    let output = entry.output.clone();
    let global = entry.global.take();
    info!(?crtc, output = %output.name(), "disabling head");

    // Void any gamma control on this output before its CRTC goes away
    // (no restore possible once the surface is gone).
    state.gamma_output_removed(&output);
    state.space.unmap_output(&output);
    output.leave_all();
    // Releases the CRTC (Drop for DrmOutput).
    drop(surface);
    if let Some(global) = global {
        dh.remove_global::<MindeState>(global);
    }

    // The remaining outputs may have fallen back to implicit modifiers to
    // satisfy the bandwidth of the one just removed; try to undo that.
    let udev = state.udev_data.as_mut().unwrap();
    let primary_gpu = udev.primary_gpu;
    let UdevBackendData { gpus, devices, .. } = udev;
    if let Some(device) = devices.get_mut(&node) {
        match gpus.single_renderer(&primary_gpu) {
            Ok(mut renderer) => {
                if let Err(err) = device
                    .drm_output_manager
                    .lock()
                    .try_to_restore_modifiers::<_, MindeRenderElements<UdevRenderer<'_>>>(
                        &mut renderer,
                        &DrmOutputRenderElements::default(),
                    )
                {
                    warn!(%err, "failed to restore modifiers after disabling head");
                }
            }
            Err(err) => warn!(%err, "no renderer to restore modifiers with"),
        }
    }

    state.space.refresh();
    state.update_usable_area();
    state.pointer_location = state.clamp_to_outputs(state.pointer_location);
}

/// Forgets a (disabled) head entirely and tells wlr-output-management
/// clients it is gone.
fn remove_head(state: &mut MindeState, node: DrmNode, crtc: crtc::Handle) {
    let Some(entry) = state
        .udev_data
        .as_mut()
        .and_then(|u| u.devices.get_mut(&node))
        .and_then(|d| d.heads.remove(&crtc))
    else {
        return;
    };
    debug_assert!(!entry.enabled(), "remove_head on an enabled head");
    state.output_power_output_removed(&entry.output);
    state.output_management_remove_output(&entry.output);
}

fn connector_disconnected(
    state: &mut MindeState,
    node: DrmNode,
    info: &connector::Info,
    crtc: crtc::Handle,
) {
    let output_name = format!("{}-{}", info.interface().as_str(), info.interface_id());
    info!(?crtc, %output_name, "connector disconnected; removing its head");
    disable_head(state, node, crtc);
    remove_head(state, node, crtc);
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
            let Some(entry) = device.heads.get_mut(&crtc) else {
                return; // output vanished (unplug); don't reschedule repaints
            };
            let Some(surface) = entry.surface.as_mut() else {
                return; // head disabled meanwhile; nothing to account for
            };
            (entry.output.clone(), surface.drm_output.frame_submitted())
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

        // The flip completed: repaint right away if anything changed while
        // it was in flight, otherwise go idle until the next dirtying event.
        let handle = self.handle.clone();
        let Some(surface) = self.output_surface_mut(node, crtc) else {
            return;
        };
        if let RedrawState::WaitingForVblank { watchdog } =
            std::mem::replace(&mut surface.redraw, RedrawState::Idle)
        {
            handle.remove(watchdog);
        } else {
            // A vblank for a frame we did not account for (forced repaint
            // while another flip was pending, or a flip from before a VT
            // switch); leave whatever the loop is doing alone.
            return;
        }
        if surface.dirty {
            self.render_now(node, crtc);
        }
    }

    fn output_surface_mut(
        &mut self,
        node: DrmNode,
        crtc: crtc::Handle,
    ) -> Option<&mut OutputSurface> {
        self.udev_data
            .as_mut()?
            .devices
            .get_mut(&node)?
            .heads
            .get_mut(&crtc)?
            .surface
            .as_mut()
    }

    /// Marks every udev output dirty and schedules a render for each one that
    /// is idle. The entry point for every scene change: surface commits,
    /// window map/unmap, layer changes, focus/placement, messages and
    /// overlays, output configuration, queued captures. A no-op under winit
    /// (its redraw loop is continuous).
    pub fn schedule_redraw(&mut self) {
        self.schedule_redraw_where(|_| true);
    }

    /// Like [`schedule_redraw`](Self::schedule_redraw), but only for the
    /// outputs whose geometry contains one of `points` -- the pointer's old
    /// and new location on motion, or its location on a cursor image change.
    pub fn schedule_redraw_at(&mut self, points: &[Point<f64, Logical>]) {
        self.schedule_redraw_where(|geo| points.iter().any(|p| geo.to_f64().contains(*p)));
    }

    fn schedule_redraw_where(
        &mut self,
        mut wanted: impl FnMut(smithay::utils::Rectangle<i32, Logical>) -> bool,
    ) {
        let Some(udev) = self.udev_data.as_mut() else {
            return;
        };
        let paused = udev.paused;
        let mut to_schedule = Vec::new();
        for (node, device) in udev.devices.iter_mut() {
            for (crtc, entry) in device.heads.iter_mut() {
                let Some(surface) = entry.surface.as_mut() else {
                    continue;
                };
                let Some(geo) = self.space.output_geometry(&entry.output) else {
                    continue;
                };
                if !wanted(geo) {
                    continue;
                }
                surface.dirty = true;
                if !paused && matches!(surface.redraw, RedrawState::Idle) {
                    surface.redraw = RedrawState::Scheduled;
                    to_schedule.push((*node, *crtc));
                }
            }
        }
        for (node, crtc) in to_schedule {
            self.handle.insert_idle(move |state| {
                let scheduled = state
                    .output_surface_mut(node, crtc)
                    .map(|surface| matches!(surface.redraw, RedrawState::Scheduled))
                    .unwrap_or(false);
                if scheduled {
                    state.render_now(node, crtc);
                }
            });
        }
    }

    /// Watchdog: a queued frame's vblank never arrived. Treat it as done so
    /// the loop cannot stall; the next render re-queues whatever is pending.
    fn vblank_watchdog_fired(&mut self, node: DrmNode, crtc: crtc::Handle) {
        let Some(surface) = self.output_surface_mut(node, crtc) else {
            return;
        };
        if !matches!(surface.redraw, RedrawState::WaitingForVblank { .. }) {
            return;
        }
        debug!(
            ?node,
            ?crtc,
            "no vblank within the watchdog interval; repainting"
        );
        surface.redraw = RedrawState::Idle;
        surface.dirty = true;
        self.render_now(node, crtc);
    }

    /// Rate-limit timer after a render that queued nothing: render again if
    /// something changed in the meantime, else go idle.
    fn redraw_timer_fired(&mut self, node: DrmNode, crtc: crtc::Handle) {
        let Some(surface) = self.output_surface_mut(node, crtc) else {
            return;
        };
        if !matches!(surface.redraw, RedrawState::WaitingForTimer { .. }) {
            return;
        }
        surface.redraw = RedrawState::Idle;
        if surface.dirty {
            self.render_now(node, crtc);
        }
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

    /// Forced repaint (session resume): whatever the loop was doing is
    /// stale, so restart it from a fresh render.
    fn handle_repaint_now(&mut self, node: DrmNode, crtc: crtc::Handle) {
        if let Some(surface) = self.output_surface_mut(node, crtc) {
            surface.dirty = true;
        }
        self.render_now(node, crtc);
    }

    /// Locates the udev head whose [`Output`] is `output`.
    fn udev_head_for_output(&self, output: &Output) -> Option<(DrmNode, crtc::Handle)> {
        let udev = self.udev_data.as_ref()?;
        for (node, device) in udev.devices.iter() {
            for (crtc, entry) in device.heads.iter() {
                if &entry.output == output {
                    return Some((*node, *crtc));
                }
            }
        }
        None
    }

    /// The udev backend's DPMS switch. Off: the CRTC is released by
    /// dropping the head's [`OutputSurface`] (ending its repaint chain --
    /// `render_now`/`frame_finish` return without rescheduling when there
    /// is no surface) while the head stays enabled: mapped, `wl_output`
    /// global kept, windows untouched. On: the surface is re-created at
    /// the output's current mode and repainted. Refused
    /// for a disabled head.
    pub(crate) fn udev_set_output_power(
        &mut self,
        output: &Output,
        on: bool,
    ) -> Result<(), String> {
        let (node, crtc) = self
            .udev_head_for_output(output)
            .ok_or("head is not driven by the udev backend")?;
        let handle = self.handle.clone();
        let udev = self.udev_data.as_mut().ok_or("no udev backend")?;
        let entry = udev
            .devices
            .get_mut(&node)
            .and_then(|d| d.heads.get_mut(&crtc))
            .ok_or("unknown head")?;
        if !entry.enabled() {
            return Err("output is disabled".into());
        }
        if on == (entry.power == PowerState::On) {
            return Ok(());
        }
        if !on {
            info!(?crtc, output = %output.name(), "powering head off");
            entry.power = PowerState::Off;
            // Cancel any pending watchdog/rate-limit timer, then release
            // the CRTC (Drop for DrmOutput); with no surface the damage
            // loop simply stays idle for this head.
            if let Some(mut surface) = entry.surface.take() {
                surface.park_redraw(&handle);
                drop(surface);
            }
            return Ok(());
        }
        let wl_mode = output.current_mode().ok_or("output has no current mode")?;
        let mode = entry
            .modes
            .iter()
            .find(|(wl, _)| *wl == wl_mode)
            .map(|(_, drm)| *drm)
            .ok_or("current mode is not a connector mode")?;
        info!(?crtc, output = %output.name(), "powering head on");
        init_surface(udev, node, crtc, mode)?;
        if let Some(entry) = udev
            .devices
            .get_mut(&node)
            .and_then(|d| d.heads.get_mut(&crtc))
        {
            entry.power = PowerState::On;
        }
        self.render_now(node, crtc);
        Ok(())
    }

    /// Whether the udev head for `output` is powered on. `None` if the
    /// output is not a udev head.
    pub(crate) fn udev_output_power(&self, output: &Output) -> Option<bool> {
        let (node, crtc) = self.udev_head_for_output(output)?;
        let entry = self
            .udev_data
            .as_ref()?
            .devices
            .get(&node)?
            .heads
            .get(&crtc)?;
        Some(entry.power == PowerState::On)
    }

    /// The udev backend's `backend_realize_head`: enables or disables the
    /// head, sets its mode and its adaptive-sync state as requested. A
    /// disabled head is enabled at `change.mode` (already resolved by
    /// validation to an advertised mode, translated back to the connector
    /// mode it came from) at `change.position` or, absent that, right of
    /// every enabled output. Position, scale and transform are left to
    /// `commit_head`; only the pieces that need the DRM device happen here.
    pub(crate) fn udev_realize_head(
        &mut self,
        output: &Output,
        change: &HeadChange,
    ) -> Result<(), String> {
        let (node, crtc) = self
            .udev_head_for_output(output)
            .ok_or("head is not driven by the udev backend")?;
        let enabled = self.output_enabled(output);
        if !change.enabled {
            if enabled {
                disable_head(self, node, crtc);
            }
            return Ok(());
        }
        if enabled {
            if let Some(mode) = change.requested_mode()
                && Some(mode) != output.current_mode()
            {
                self.udev_set_mode(output, mode, change.custom_mode.is_some())?;
            }
        } else {
            let wl_mode = change
                .requested_mode()
                .or_else(|| output.preferred_mode())
                .or_else(|| output.current_mode())
                .ok_or("head has no mode to enable with")?;
            let drm_mode = self
                .udev_data
                .as_ref()
                .and_then(|u| u.devices.get(&node))
                .and_then(|d| d.heads.get(&crtc))
                .and_then(|entry| {
                    resolve_connector_mode(&entry.modes, wl_mode, change.custom_mode.is_some())
                })
                .ok_or("requested mode is not a connector mode")?;
            let position: Point<i32, Logical> = change
                .position
                .map(Into::into)
                .unwrap_or_else(|| right_edge_of_outputs(self));
            enable_head(self, node, crtc, drm_mode, position)?;
        }
        if let Some(on) = change.adaptive_sync {
            self.udev_set_adaptive_sync(node, crtc, on)?;
        }
        Ok(())
    }

    /// Switches the scanout mode of an enabled head to `wl_mode`, which is
    /// translated to a connector mode with [`resolve_connector_mode`]
    /// (`custom` selects the tolerant match for custom modes). The DRM
    /// compositor keeps sizing its frames from the [`Output`], so the
    /// output's current mode is updated in the same step, and a frame is
    /// rendered right away so the modeset commits. A head without a live
    /// surface (disabled, or powered off by output-power-management) only
    /// gets the mode recorded on the Output; the next `enable_head` /
    /// power-on initialises the CRTC from that.
    ///
    /// Known limitation: the mode is set with
    /// `DrmOutputRenderElements::default()`, so if the new mode exceeds the
    /// available bandwidth Smithay's fallback path re-submits the *other*
    /// heads with empty element lists to free planes -- those may show a
    /// black frame for one refresh. Collecting all heads' elements without
    /// submitting them is a follow-up.
    pub(crate) fn udev_set_mode(
        &mut self,
        output: &Output,
        wl_mode: WlMode,
        custom: bool,
    ) -> Result<(), String> {
        let (node, crtc) = self
            .udev_head_for_output(output)
            .ok_or("head is not driven by the udev backend")?;
        let udev = self.udev_data.as_mut().ok_or("no udev backend")?;
        let primary_gpu = udev.primary_gpu;
        // Split borrows (see `render_surface`): surface in `devices`,
        // renderer in `gpus`.
        let UdevBackendData { gpus, devices, .. } = udev;
        let entry = devices
            .get_mut(&node)
            .and_then(|d| d.heads.get_mut(&crtc))
            .ok_or("unknown head")?;
        let drm_mode = resolve_connector_mode(&entry.modes, wl_mode, custom)
            .ok_or("no matching connector mode; custom modelines are not supported")?;
        let Some(surface) = entry.surface.as_mut() else {
            output.change_current_state(Some(wl_mode), None, None, None);
            return Ok(());
        };
        if output.current_mode() == Some(wl_mode) {
            return Ok(());
        }
        info!(?crtc, output = %output.name(), ?wl_mode, "setting mode");
        let mut renderer = gpus
            .single_renderer(&primary_gpu)
            .map_err(|err| format!("failed to get renderer for modeset: {err}"))?;
        surface
            .drm_output
            .use_mode::<_, MindeRenderElements<UdevRenderer<'_>>>(
                drm_mode,
                &mut renderer,
                &DrmOutputRenderElements::default(),
            )
            .map_err(|err| format!("modeset failed: {err}"))?;
        drop(renderer);
        output.change_current_state(Some(wl_mode), None, None, None);
        self.render_now(node, crtc);
        Ok(())
    }

    /// Enables or disables adaptive sync (VRR) on an enabled head and
    /// remembers the choice in its [`HeadEntry`]. Fails where the
    /// connector does not support VRR; on a `RequiresModeset` connector
    /// the next frame carries the modeset.
    fn udev_set_adaptive_sync(
        &mut self,
        node: DrmNode,
        crtc: crtc::Handle,
        on: bool,
    ) -> Result<(), String> {
        let entry = self
            .udev_data
            .as_mut()
            .and_then(|u| u.devices.get_mut(&node))
            .and_then(|d| d.heads.get_mut(&crtc))
            .ok_or("unknown head")?;
        let Some(surface) = entry.surface.as_ref() else {
            // No CRTC to talk to: remember the wish for the next enable.
            entry.vrr = on;
            return Ok(());
        };
        if entry.vrr_support == VrrSupport::NotSupported {
            return if on {
                Err("adaptive sync not supported on this head".into())
            } else {
                entry.vrr = false;
                Ok(())
            };
        }
        surface
            .drm_output
            .with_compositor(|c| c.use_vrr(on))
            .map_err(|err| format!("failed to set adaptive sync: {err}"))?;
        entry.vrr = on;
        info!(?crtc, output = %entry.output.name(), on, "adaptive sync");
        self.render_now(node, crtc);
        Ok(())
    }

    /// The udev backend's answer to `output_adaptive_sync`: `Some(on)` on
    /// a connector that supports VRR (with or without a modeset) -- the
    /// live state while the head is enabled, the remembered wish while it
    /// is disabled (support is only learnt on the first enable, so a
    /// never-enabled head reports `None`) -- and `None` where the
    /// connector cannot do it.
    pub(crate) fn udev_output_adaptive_sync(&self, output: &Output) -> Option<bool> {
        let (node, crtc) = self.udev_head_for_output(output)?;
        let entry = self
            .udev_data
            .as_ref()?
            .devices
            .get(&node)?
            .heads
            .get(&crtc)?;
        if entry.vrr_support == VrrSupport::NotSupported {
            return None;
        }
        match entry.surface.as_ref() {
            Some(surface) => Some(surface.drm_output.with_compositor(|c| c.vrr_enabled())),
            None => Some(entry.vrr),
        }
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
            for entry in device.heads.values() {
                if &entry.output != output {
                    continue;
                }
                let Some(surface) = entry.surface.as_ref() else {
                    return None; // disabled: no CRTC to drive
                };
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
    /// If a flip is still pending the frame is queued behind it by the DRM
    /// compositor; its vblank is then absorbed by `frame_finish`.
    pub(crate) fn render_all_outputs_now(&mut self) {
        let Some(udev) = self.udev_data.as_ref() else {
            return;
        };
        if udev.paused {
            return;
        }
        let targets: Vec<(DrmNode, crtc::Handle)> = udev
            .devices
            .iter()
            .flat_map(|(node, device)| {
                device
                    .heads
                    .iter()
                    .filter(|(_, head)| head.enabled())
                    .map(move |(crtc, _)| (*node, *crtc))
            })
            .collect();
        for (node, crtc) in targets {
            self.render_now(node, crtc);
        }
    }

    /// Renders one output now and arms the follow-up: the vblank (plus a
    /// watchdog) when a frame was queued, a refresh-interval timer when
    /// nothing was (no damage, or an error -- the error case keeps `dirty`
    /// so the timer retries, matching the old fixed-interval behaviour).
    fn render_now(&mut self, node: DrmNode, crtc: crtc::Handle) {
        let handle = self.handle.clone();
        // A disabled or vanished head has no surface to render to; the
        // damage loop simply ends here.
        let Some(output) = self
            .udev_data
            .as_ref()
            .and_then(|u| u.devices.get(&node))
            .and_then(|d| d.heads.get(&crtc))
            .filter(|entry| entry.enabled())
            .map(|entry| entry.output.clone())
        else {
            return;
        };
        let Some(surface) = self.output_surface_mut(node, crtc) else {
            return;
        };
        surface.park_redraw(&handle);
        surface.dirty = false;
        let interval = refresh_interval(&output);

        let started = std::time::Instant::now();
        let rendered = self.render_surface(node, crtc);
        crate::timing::record(crate::timing::Probe::Render, started);
        let queued = match rendered {
            Ok(queued) => queued,
            Err(err) => {
                match err {
                    // Expected while the session is paused or the device is
                    // mid-hotplug; the resume path repaints.
                    SwapBuffersError::TemporaryFailure(_) => {
                        debug!(%err, "temporary failure rendering udev output")
                    }
                    _ => warn!(%err, "error rendering udev output"),
                }
                if let Some(surface) = self.output_surface_mut(node, crtc) {
                    surface.dirty = true;
                }
                false
            }
        };

        let Some(surface) = self.output_surface_mut(node, crtc) else {
            return;
        };
        let redraw = if queued {
            // Vblanks normally arrive within one interval; give the driver
            // generous slack before assuming the flip was lost.
            let watchdog_after = (interval * 4).max(Duration::from_millis(100));
            handle
                .insert_source(Timer::from_duration(watchdog_after), move |_, _, state| {
                    state.vblank_watchdog_fired(node, crtc);
                    TimeoutAction::Drop
                })
                .ok()
                .map(|watchdog| RedrawState::WaitingForVblank { watchdog })
        } else {
            handle
                .insert_source(Timer::from_duration(interval), move |_, _, state| {
                    state.redraw_timer_fired(node, crtc);
                    TimeoutAction::Drop
                })
                .ok()
                .map(|token| RedrawState::WaitingForTimer { token })
        };
        // A failed timer insert leaves the loop idle; the next dirtying
        // event restarts it.
        surface.redraw = redraw.unwrap_or(RedrawState::Idle);
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
        let Some(entry) = devices.get_mut(&node).and_then(|d| d.heads.get_mut(&crtc)) else {
            return Ok(false);
        };
        let output = entry.output.clone();
        let Some(output_surface) = entry.surface.as_mut() else {
            return Ok(false);
        };

        let mut renderer = gpus
            .single_renderer(&primary_gpu)
            .map_err(|_| SwapBuffersError::AlreadySwapped)?;
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

        // Messages are shown on the output holding the current frame only
        // (StumpWM shows them on the current head).
        let message_here = self
            .focus_rect
            .map(|r| output_geo.contains(r.loc))
            .unwrap_or(true);
        let focus = self.focus_rect.or_else(|| {
            self.focused_window
                .as_ref()
                .and_then(|w| self.space.element_geometry(w))
        });
        let all_elements = crate::handlers::screencopy::output_scene_elements(
            &mut renderer,
            &output,
            output_geo,
            (0, 0).into(),
            scale,
            &self.space,
            Some((&mut self.cursor_state, self.pointer_location)),
            self.message.as_ref().filter(|_| message_here),
            &self.overlays,
            focus,
            self.border_color,
            &mut output_surface.border_buffers,
        );

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
        // `output_scene_elements` released its layer-map guard; open one here
        // for the feedback and frame-callback walks (a second guard on the
        // same output panics -- see the frame-callback NOTE below).
        let layer_map = smithay::desktop::layer_map_for_output(&output);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn wl(w: i32, h: i32, refresh: i32) -> WlMode {
        WlMode {
            size: (w, h).into(),
            refresh,
        }
    }

    fn modes() -> Vec<(WlMode, &'static str)> {
        vec![
            (wl(1920, 1080, 60_000), "1080p60"),
            (wl(1920, 1080, 59_940), "1080p59.94"),
            (wl(1920, 1080, 144_000), "1080p144"),
            (wl(2560, 1440, 60_000), "1440p60"),
        ]
    }

    #[test]
    fn advertised_mode_matches_exactly_or_not_at_all() {
        let m = modes();
        assert_eq!(
            resolve_connector_mode(&m, wl(1920, 1080, 59_940), false),
            Some("1080p59.94")
        );
        assert_eq!(
            resolve_connector_mode(&m, wl(1920, 1080, 59_950), false),
            None
        );
        assert_eq!(
            resolve_connector_mode(&m, wl(1280, 720, 60_000), false),
            None
        );
    }

    #[test]
    fn custom_mode_matches_size_and_refresh_within_a_hertz() {
        let m = modes();
        // Exact hit wins over the tolerant search.
        assert_eq!(
            resolve_connector_mode(&m, wl(1920, 1080, 60_000), true),
            Some("1080p60")
        );
        // Nearest refresh within +-1000 mHz.
        assert_eq!(
            resolve_connector_mode(&m, wl(1920, 1080, 59_500), true),
            Some("1080p59.94")
        );
        assert_eq!(
            resolve_connector_mode(&m, wl(1920, 1080, 143_100), true),
            Some("1080p144")
        );
        // Same size, refresh too far off: no modeline synthesis.
        assert_eq!(
            resolve_connector_mode(&m, wl(1920, 1080, 75_000), true),
            None
        );
        // Different size: refused even with tolerance.
        assert_eq!(
            resolve_connector_mode(&m, wl(1921, 1080, 60_000), true),
            None
        );
        assert_eq!(
            resolve_connector_mode(&[], wl(1920, 1080, 60_000), true),
            None::<&str>
        );
    }
}
