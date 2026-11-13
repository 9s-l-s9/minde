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
            DrmDevice, DrmDeviceFd, DrmError, DrmEvent, DrmEventMetadata, DrmEventTime, DrmNode,
            NodeType, VrrSupport,
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
        wayland_server::backend::GlobalId,
    },
    utils::{Clock, Logical, Monotonic, Point},
    wayland::{
        dmabuf::{DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        drm_syncobj::{DrmSyncobjHandler, DrmSyncobjState, supports_syncobj_eventfd},
        presentation::{PresentationState, Refresh},
    },
};
use smithay_drm_extras::drm_scanner::DrmScanner;
use tracing::{debug, error, info, warn};

use crate::edid::{EdidInfo, parse_edid};
use crate::handlers::output_management::HeadChange;
use crate::render::{BorderBuffers, MindeRenderElements};
use crate::{MindeState, guile};

mod device;
use device::{EventRegistration, SessionDevice};
mod connectors;
use connectors::ConnectorMapper;

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

/// Per-frame user-data attached to each queued DRM frame: the surface's
/// flip generation at queue time (so `frame_finish` can tell a late
/// vblank from a flip the watchdog already abandoned apart from the
/// current one) and the presentation feedback owed to clients once the
/// frame is actually scanned out. Returned by `frame_submitted` on the
/// matching vblank (see `frame_finish`). The feedback is `None` for
/// frames that owe none (e.g. the locked blank frame).
struct FrameUserData {
    generation: u64,
    feedback: Option<OutputPresentationFeedback>,
    lock_generation: Option<u64>,
}

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
    /// No vblank is expected. Wait one refresh interval after an undamaged
    /// render, or the retry delay after a failure.
    WaitingForTimer { token: RegistrationToken },
}

impl RedrawState {
    /// Consume only the watchdog belonging to the completed flip. A forced
    /// repaint may already have replaced it with a timer or idle callback.
    fn finish_vblank(&mut self) -> Option<RegistrationToken> {
        let Self::WaitingForVblank { watchdog } = self else {
            return None;
        };
        let watchdog = *watchdog;
        *self = Self::Idle;
        Some(watchdog)
    }

    /// Cancel pending timers before a forced render, pause or teardown.
    fn park<Data>(&mut self, handle: &smithay::reexports::calloop::LoopHandle<'_, Data>) {
        match std::mem::replace(self, Self::Idle) {
            Self::WaitingForVblank { watchdog: token } | Self::WaitingForTimer { token } => {
                handle.remove(token);
            }
            Self::Idle | Self::Scheduled => {}
        }
    }
}

/// Per-output recovery pacing. Keep trying without needing a new client
/// commit, but never spin at the display's refresh rate on a broken device.
#[derive(Default)]
struct RenderRetry {
    delay: Duration,
    log_after: Duration,
}

impl RenderRetry {
    fn failed(&mut self, interval: Duration, now: Duration) -> (Duration, bool) {
        self.delay = if self.delay.is_zero() {
            interval
        } else {
            self.delay.saturating_mul(2)
        }
        .clamp(Duration::from_millis(100), Duration::from_secs(1));
        let log = now >= self.log_after;
        if log {
            self.log_after = now.saturating_add(Duration::from_secs(10));
        }
        (self.delay, log)
    }

    fn succeeded(&mut self) {
        self.delay = Duration::ZERO;
        // Intermittent failures must not defeat the diagnostic rate limit.
    }
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
    retry: RenderRetry,
    /// Bumped for every queued frame and stamped into its
    /// [`FrameUserData`], so `frame_finish` can ignore the late vblank of
    /// a flip [`MindeState::vblank_watchdog_fired`] already gave up on
    /// instead of attributing it to the frame in flight.
    flip_generation: u64,
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

/// Presentation feedback must distinguish unknown, fixed and variable refresh.
/// The mode interval is the shortest interval when adaptive sync is active.
fn presentation_refresh(mode: Option<WlMode>, vrr: bool) -> Refresh {
    match mode.filter(|mode| mode.refresh > 0) {
        Some(mode) => {
            let interval = Duration::from_secs_f64(1_000.0 / mode.refresh as f64);
            if vrr {
                Refresh::variable(interval)
            } else {
                Refresh::fixed(interval)
            }
        }
        None => Refresh::Unknown,
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
    /// explicit disable so a re-enabled head always lights up).
    power: PowerState,
    /// What the connector reports about variable refresh rate, cached
    /// when the head is enabled (`NotSupported` while disabled or unknown).
    vrr_support: VrrSupport,
    /// Desired adaptive-sync state; re-applied whenever the CRTC is
    /// (re)initialised so it survives disable/enable cycles.
    vrr: bool,
    /// Reapply gamma after the first completed frame of a modeset/resume.
    /// Waiting until then avoids a driver resetting the LUT after we set it.
    gamma_reapply: bool,
    /// Retry failed automatic setup on later scans, while preserving an
    /// explicit disable across hotplug notifications and session resumes.
    wanted_enabled: bool,
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
        .map(|(wl, drm)| (wl.refresh.abs_diff(wanted.refresh), *drm))
        .filter(|(diff, _)| *diff <= 1000)
        .min_by_key(|(diff, _)| *diff)
        .map(|(_, drm)| drm)
}

struct DeviceData {
    // Drop the notifier and rendering resources before returning the
    // original descriptor to libseat (fields are dropped in this order).
    _registration: EventRegistration<MindeState>,
    drm_output_manager: GbmDrmOutputManager,
    drm_scanner: DrmScanner<ConnectorMapper>,
    waiting_connectors: std::collections::HashSet<connector::Handle>,
    heads: HashMap<crtc::Handle, HeadEntry>,
    render_node: Option<DrmNode>,
    _session_device: SessionDevice<LibSeatSession>,
}

impl DeviceData {
    /// Smithay's bandwidth fallback performs blocking commits on other
    /// heads, without vblank events. Replay those heads here, including
    /// after a partial failure; heads awaiting our own flip stay deferred.
    fn reapply_gamma_after_fallback(
        &self,
        gamma: &mut crate::handlers::gamma_control::GammaControls,
        except: Option<crtc::Handle>,
    ) {
        for (crtc, entry) in &self.heads {
            if Some(*crtc) != except
                && entry.enabled()
                && entry.power == PowerState::On
                && !entry.gamma_reapply
                && entry.surface.as_ref().is_some_and(|surface| {
                    !surface
                        .drm_output
                        .with_compositor(|c| c.surface().commit_pending())
                })
            {
                gamma.reapply(&entry.output);
            }
        }
    }
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
    /// in which case the global is simply never advertised. The tuple stores
    /// the owning card node so a secondary-device removal cannot retire it.
    /// See [`DrmSyncobjHandler`] below.
    syncobj_state: Option<(DrmNode, DrmSyncobjState)>,
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

impl UdevBackendData {
    fn can_render(&self, output: &Output) -> bool {
        !self.paused
            && self.devices.values().any(|device| {
                device
                    .heads
                    .values()
                    .any(|head| head.output == *output && head.enabled() && head.surface.is_some())
            })
    }
}

/// Explicit-sync (`linux-drm-syncobj-v1`): Smithay's renderer picks the
/// acquire/release sync points straight out of the surface's committed state,
/// so all this handler has to expose is the state delegate. It is only `Some`
/// when the primary GPU supports syncobj timelines (see `syncobj_state`).
impl DrmSyncobjHandler for MindeState {
    fn drm_syncobj_state(&mut self) -> Option<&mut DrmSyncobjState> {
        self.udev_data
            .as_mut()?
            .syncobj_state
            .as_mut()
            .map(|(_, state)| state)
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
    #[error("failed to duplicate session device descriptor: {0}")]
    DuplicateFd(std::io::Error),
    #[error("failed to obtain primary renderer: {0}")]
    Renderer(String),
    #[error("failed to register drm notifier: {0}")]
    Notifier(smithay::reexports::calloop::Error),
    #[error("render node {0} is already owned by another drm device")]
    DuplicateRenderNode(DrmNode),
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
        paused: !session.is_active(),
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
                // A key held through a VT switch or suspend never delivers
                // its release here (libinput is suspended), so an armed
                // compositor-side repeat would keep firing its Scheme
                // handler every 40 ms into the paused session -- and keep
                // going after resume. Balance forwarded physical keys and
                // discard consumed ones together with the repeat timer.
                state.reset_keyboard_input();
                state.cancel_touch_input();
                state.invalidate_lock_presentation();
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
                                surface.redraw.park(&handle);
                            }
                        }
                    }
                }
            }
            SessionEvent::ActivateSession => {
                // Resume starts a fresh Scheme idle interval even before input.
                guile::note_activity();
                info!("resuming session");
                if libinput_context.resume().is_err() {
                    error!("failed to resume libinput context");
                }
                let mut active_nodes = Vec::new();
                if let Some(udev) = state.udev_data.as_mut() {
                    udev.paused = false;
                    for (node, device) in udev.devices.iter_mut() {
                        match device.drm_output_manager.lock().activate(false) {
                            Ok(()) => active_nodes.push(*node),
                            Err(err) => warn!(?node, %err, "failed to reactivate drm device"),
                        }
                    }
                }
                state.invalidate_lock_presentation();
                // Hotplug while paused must be reconciled only after the
                // device is active. Failed setup is retried by the same scan.
                for node in &active_nodes {
                    scan_connectors(state, *node);
                }
                let mut to_repaint = Vec::new();
                if let Some(udev) = state.udev_data.as_mut() {
                    for node in active_nodes {
                        let device = udev.devices.get_mut(&node).unwrap();
                        for (crtc, head) in device.heads.iter_mut() {
                            if head.enabled() {
                                head.gamma_reapply = true;
                                to_repaint.push((node, *crtc));
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

    // Duplicate udev notifications must not replace a live device and lose
    // its heads or remove another registration for the same render node.
    if udev.devices.contains_key(&node) {
        return Ok(());
    }
    let session_device =
        SessionDevice::open(udev.session.clone(), path).map_err(DeviceAddError::DeviceOpen)?;
    let fd = DrmDeviceFd::new(
        session_device
            .duplicate()
            .map_err(DeviceAddError::DuplicateFd)?,
    );

    let (drm, notifier) = DrmDevice::new(fd.clone(), true).map_err(DeviceAddError::DrmDevice)?;
    let gbm = GbmDevice::new(fd.clone()).map_err(DeviceAddError::GbmDevice)?;

    // Try to add this GPU's render node to the shared GpuManager (needed
    // even though we only ever render with the primary node, since
    // `single_renderer` looks the node up there).
    let render_node = if let Ok(display) = unsafe { EGLDisplay::new(gbm.clone()) }
        && let Ok(egl_device) = EGLDevice::device_for_display(&display)
    {
        let render_node = egl_device
            .try_get_render_node()
            .ok()
            .flatten()
            .unwrap_or(node);
        if udev
            .devices
            .values()
            .any(|device| device.render_node == Some(render_node))
        {
            return Err(DeviceAddError::DuplicateRenderNode(render_node));
        }
        match udev.gpus.as_mut().add_node(render_node, gbm.clone()) {
            Ok(()) => Some(render_node),
            Err(err) => {
                warn!(
                    ?err,
                    ?node,
                    "could not register render node; trying primary renderer for scanout"
                );
                None
            }
        }
    } else {
        None
    };

    // Prepare rendering before registering its notifier or advertising
    // explicit sync. Undo the GPU entry on every fallible setup path.
    let setup = (|| {
        let render_formats = udev
            .gpus
            .single_renderer(&udev.primary_gpu)
            .map_err(|err| DeviceAddError::Renderer(err.to_string()))?
            .as_mut()
            .egl_context()
            .dmabuf_render_formats()
            .iter()
            .copied()
            .collect::<FormatSet>();
        let allocator = GbmAllocator::new(
            gbm.clone(),
            GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
        );
        let framebuffer_exporter = GbmFramebufferExporter::new(gbm.clone(), None.into());
        let manager = DrmOutputManager::new(
            drm,
            allocator,
            framebuffer_exporter,
            Some(gbm),
            SUPPORTED_FORMATS.iter().copied(),
            render_formats,
        );
        let token = handle
            .insert_source(
                notifier,
                move |event, metadata, state: &mut MindeState| match event {
                    DrmEvent::VBlank(crtc) => state.frame_finish(node, crtc, metadata),
                    DrmEvent::Error(err) => error!(%err, "drm error"),
                },
            )
            .map_err(|err| DeviceAddError::Notifier(err.error))?;
        Ok((manager, EventRegistration::new(handle, token)))
    })();
    let (drm_output_manager, registration) = match setup {
        Ok(resources) => resources,
        Err(err) => {
            remove_gpu_node(udev, render_node);
            return Err(err);
        }
    };

    // Card nodes key `devices`; primary_gpu is a render node. Only the
    // successfully registered primary device may own explicit-sync imports.
    if render_node == Some(udev.primary_gpu) && udev.syncobj_state.is_none() {
        if supports_syncobj_eventfd(&fd) {
            info!("primary drm device supports syncobj timelines; enabling linux-drm-syncobj-v1");
            udev.syncobj_state = Some((node, DrmSyncobjState::new::<MindeState>(&dh, fd)));
        } else {
            info!(
                "primary drm device lacks syncobj timeline support; linux-drm-syncobj-v1 not advertised"
            );
        }
    }

    udev.devices.insert(
        node,
        DeviceData {
            _registration: registration,
            drm_output_manager,
            drm_scanner: DrmScanner::new_with_mapper(ConnectorMapper::default()),
            waiting_connectors: Default::default(),
            heads: HashMap::new(),
            render_node,
            _session_device: session_device,
        },
    );

    scan_connectors(state, node);
    Ok(())
}

fn remove_gpu_node(udev: &mut UdevBackendData, node: Option<DrmNode>) {
    if let Some(node) = node {
        udev.gpus.as_mut().remove_node(&node);
        // Removing the backend entry only marks enumeration dirty. Refresh
        // now to drop cached renderers/EGL displays even if no render follows.
        if let Err(err) = udev.gpus.devices() {
            warn!(?err, "failed to refresh renderers after device removal");
        }
    }
}

fn remove_device_syncobj<N: PartialEq>(
    state: &mut Option<(N, DrmSyncobjState)>,
    node: N,
    dh: &smithay::reexports::wayland_server::DisplayHandle,
) {
    if let Some((_, syncobj)) = state.take_if(|(owner, _)| *owner == node) {
        // into_global also invalidates imported timelines and wakes waiters.
        dh.remove_global::<MindeState>(syncobj.into_global());
    }
}

/// Reconcile heads against the full current mapping. A waiting connector
/// can acquire a CRTC without another Connected event, and a known head
/// can gain usable modes after its first scan.
fn scan_connectors(state: &mut MindeState, node: DrmNode) {
    let (connected, retired) = {
        let udev = state.udev_data.as_mut().unwrap();
        if udev.paused || !udev.session.is_active() {
            return;
        }
        let Some(device) = udev.devices.get_mut(&node) else {
            return;
        };
        if !device.drm_output_manager.device().is_active() {
            return;
        }
        if let Err(err) = device
            .drm_scanner
            .scan_connectors(device.drm_output_manager.device())
        {
            warn!(?err, "failed to scan connectors");
            return;
        }
        let mut connected = Vec::new();
        let mut waiting = std::collections::HashSet::new();
        for info in device.drm_scanner.connectors().values() {
            if info.state() != connector::State::Connected {
                continue;
            }
            if let Some(crtc) = device.drm_scanner.crtc_for_connector(&info.handle()) {
                connected.push((info.clone(), crtc));
            } else {
                waiting.insert(info.handle());
                if !device.waiting_connectors.contains(&info.handle()) {
                    warn!(?node, connector = ?info.handle(), "connected output waiting for a free controller");
                }
            }
        }
        device.waiting_connectors = waiting;
        connected.sort_by_key(|(info, _)| u32::from(info.handle()));
        let heads: Vec<_> = device
            .heads
            .iter()
            .map(|(crtc, h)| (h.connector, *crtc))
            .collect();
        let mapping: Vec<_> = connected
            .iter()
            .map(|(info, crtc)| (info.handle(), *crtc))
            .collect();
        let retired = connectors::retired(&heads, &mapping);
        (connected, retired)
    };

    let was_applying = state.applying_output_config;
    let old_outputs: Vec<_> = state.space.outputs().cloned().collect();
    state.applying_output_config = true;
    let mut changed = !retired.is_empty();
    let mut relocated = HashMap::new();
    for crtc in retired {
        let entry = &state.udev_data.as_ref().unwrap().devices[&node].heads[&crtc];
        if connected
            .iter()
            .any(|(info, _)| info.handle() == entry.connector)
        {
            relocated.insert(entry.connector, HeadSettings::of(entry));
        }
        disable_head(state, node, crtc);
        remove_head(state, node, crtc);
    }
    for (info, crtc) in connected {
        changed |= refresh_connector(state, node, &info, crtc, relocated.remove(&info.handle()));
    }
    state.applying_output_config = was_applying;
    if changed && !was_applying {
        for output in old_outputs {
            if !state.output_enabled(&output) {
                state.close_output_layers(&output);
            }
        }
        state.invalidate_lock_presentation();
        state.space.refresh();
        state.update_usable_area();
        // A replacement Output can keep the same Scheme head ID and
        // geometry, so update_usable_area need not detect an association change.
        state.refresh_foreign_toplevel_outputs();
        state.update_fractional_scales();
        state.reconfigure_lock_surfaces();
        state.absolute_pointer_motion.reset();
        state.pointer_location = state.clamp_to_outputs(state.pointer_location);
        state.start_udev_if_ready();
        state.schedule_redraw();
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

struct HeadSettings {
    output: Output,
    wanted_enabled: bool,
    power: PowerState,
    vrr: bool,
}

impl HeadSettings {
    fn of(entry: &HeadEntry) -> Self {
        Self {
            output: entry.output.clone(),
            wanted_enabled: entry.wanted_enabled,
            power: entry.power,
            vrr: entry.vrr,
        }
    }
}

fn same_monitor(output: &Output, physical: &PhysicalProperties, edid: Option<&EdidInfo>) -> bool {
    let old = output.physical_properties();
    old.size == physical.size
        && old.subpixel == physical.subpixel
        && old.make == physical.make
        && old.model == physical.model
        && old.serial_number == physical.serial_number
        && output.user_data().get::<EdidInfo>() == edid
}

/// Update advertised modes without resetting a still-supported current
/// mode. Mode ordering is immaterial; keep retained modes stable so a
/// reordered kernel list cannot cause endless output-management updates.
fn set_connector_modes(output: &Output, modes: &[WlMode], preferred: Option<WlMode>) -> bool {
    let old = output.modes();
    let changed = old.len() != modes.len()
        || old.iter().any(|mode| !modes.contains(mode))
        || output.preferred_mode() != preferred;
    for mode in old {
        if !modes.contains(&mode) {
            output.delete_mode(mode);
        }
    }
    for mode in modes {
        output.add_mode(*mode);
    }
    if let Some(mode) = preferred {
        output.set_preferred(mode);
    }
    changed
}

fn automatic_mode<M: Copy>(
    output: &Output,
    modes: &[(WlMode, M)],
    wanted_enabled: bool,
    enabled: bool,
) -> Option<M> {
    if !wanted_enabled || enabled {
        return None;
    }
    output
        .current_mode()
        .or_else(|| output.preferred_mode())
        .and_then(|mode| resolve_connector_mode(modes, mode, false))
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

/// Refresh connector metadata and retry desired heads that have become
/// usable. Keep mode-less heads registered so an explicit disable can
/// cancel automatic setup before a later scan supplies modes.
fn refresh_connector(
    state: &mut MindeState,
    node: DrmNode,
    info: &connector::Info,
    crtc: crtc::Handle,
    mut restore: Option<HeadSettings>,
) -> bool {
    let preferred = info
        .modes()
        .iter()
        .filter(|m| !m.flags().contains(ModeFlags::INTERLACE))
        .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
        .copied()
        .map(wl_mode_from_drm);
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

    let preferred = preferred.or_else(|| modes.first().map(|(wl, _)| *wl));
    let device = &state.udev_data.as_ref().unwrap().devices[&node];
    let edid = read_edid(device.drm_output_manager.device(), info.handle());
    let (make, model, serial) = match &edid {
        Some(e) => (e.make.clone(), e.model.clone(), e.serial.clone()),
        None => ("Unknown".into(), "Unknown".into(), "Unknown".into()),
    };
    let (phys_w, phys_h) = info.size().unwrap_or((0, 0));
    let physical = PhysicalProperties {
        size: (phys_w as i32, phys_h as i32).into(),
        subpixel: Subpixel::from(info.subpixel()),
        make,
        model,
        serial_number: serial,
    };
    if let Some(entry) = device.heads.get(&crtc)
        && !same_monitor(&entry.output, &physical, edid.as_ref())
    {
        // Smithay's physical identity and EDID user data are immutable.
        // Replace the protocol head, retaining the connector's layout/id.
        restore = Some(HeadSettings::of(entry));
        disable_head_inner(state, node, crtc, true);
        remove_head(state, node, crtc);
    }
    let fresh = !state.udev_data.as_ref().unwrap().devices[&node]
        .heads
        .contains_key(&crtc);
    let mut changed = fresh;
    if fresh {
        let name = format!("{}-{}", info.interface().as_str(), info.interface_id());
        info!(?crtc, output = %name, modes = modes.len(), ?preferred, "registering connector head");
        let output = Output::new(name, physical);
        if let Some(edid) = edid {
            output.user_data().insert_if_missing(|| edid);
        }
        if let Some(saved) = &restore {
            MindeState::inherit_output_layout(&saved.output, &output);
            if let Some(mode) = saved.output.current_mode()
                && modes.iter().any(|(wl, _)| *wl == mode)
            {
                output.change_current_state(Some(mode), None, None, None);
            }
        } else {
            output.change_current_state(None, None, None, Some(right_edge_of_outputs(state)));
        }
        state.output_id(&output);
        state
            .udev_data
            .as_mut()
            .unwrap()
            .devices
            .get_mut(&node)
            .unwrap()
            .heads
            .insert(
                crtc,
                HeadEntry {
                    output,
                    connector: info.handle(),
                    modes: Vec::new(),
                    global: None,
                    surface: None,
                    power: restore.as_ref().map_or(PowerState::On, |s| s.power),
                    vrr_support: VrrSupport::NotSupported,
                    vrr: restore.as_ref().is_some_and(|s| s.vrr),
                    gamma_reapply: false,
                    wanted_enabled: restore.as_ref().is_none_or(|s| s.wanted_enabled),
                },
            );
    }
    let entry = &state.udev_data.as_ref().unwrap().devices[&node].heads[&crtc];
    if entry.enabled()
        && !modes
            .iter()
            .any(|(wl, _)| Some(*wl) == entry.output.current_mode())
    {
        disable_head_inner(state, node, crtc, true);
        changed = true;
    }
    let entry = state
        .udev_data
        .as_mut()
        .unwrap()
        .devices
        .get_mut(&node)
        .unwrap()
        .heads
        .get_mut(&crtc)
        .unwrap();
    let output = entry.output.clone();
    let advertised: Vec<_> = modes.iter().map(|(wl, _)| *wl).collect();
    changed |= set_connector_modes(&output, &advertised, preferred);
    entry.modes = modes;
    let mode = automatic_mode(&output, &entry.modes, entry.wanted_enabled, entry.enabled());
    let power = entry.power;
    if fresh {
        state.output_management_add_output(&output);
    }
    if let Some(mode) = mode {
        match enable_head_with_power(state, node, crtc, mode, output.current_location(), power) {
            Ok(()) => changed = true,
            Err(err) => warn!(?node, ?crtc, %err, "head setup failed; will retry on a later scan"),
        }
    }
    changed
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
    enable_head_with_power(state, node, crtc, mode, position, PowerState::On)
}

/// A metadata refresh must preserve DPMS Off without briefly lighting the
/// connector. An explicit enable still defaults to power On.
fn enable_head_with_power(
    state: &mut MindeState,
    node: DrmNode,
    crtc: crtc::Handle,
    mode: control::Mode,
    position: Point<i32, Logical>,
    power: PowerState,
) -> Result<(), String> {
    let dh = state.display_handle.clone();
    let handle = state.handle.clone();
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
    entry.power = power;
    let prev_mode = output.current_mode();
    let prev_location = output.current_location();
    output.change_current_state(Some(wl_mode), None, None, Some(position));
    state.space.map_output(&output, position);

    if power == PowerState::On
        && let Err(err) = init_surface(udev, &mut state.gamma_controls, node, crtc, mode)
    {
        // Back to fully disabled: no global (that is what `enabled()`
        // means), not mapped, and restore the previous mode/position when
        // known. Automatic setup retains its intent for a later scan.
        if let Some(global) = udev
            .devices
            .get_mut(&node)
            .and_then(|d| d.heads.get_mut(&crtc))
            .and_then(|e| e.global.take())
        {
            retire_output_global(&dh, &handle, global);
        }
        state.space.unmap_output(&output);
        state.capture_output_removed(&output);
        if prev_mode.is_some() || prev_location != position {
            output.change_current_state(prev_mode, None, None, Some(prev_location));
        }
        return Err(err);
    }

    state.space.refresh();
    if !state.applying_output_config {
        state.update_usable_area();
        state.update_fractional_scales();
        state.absolute_pointer_motion.reset();
        state.pointer_location = state.clamp_to_outputs(state.pointer_location);
        state.start_udev_if_ready();
        state.render_now(node, crtc);
    }
    Ok(())
}

/// Starts driving the CRTC of the head on `(node, crtc)` at `mode`:
/// creates the [`OutputSurface`] in the idle redraw state. The output must
/// already carry its mode and be mapped in the space (the render loop
/// reads both). Shared by [`enable_head`] and the DPMS power-on path.
fn init_surface(
    udev: &mut UdevBackendData,
    gamma: &mut crate::handlers::gamma_control::GammaControls,
    node: DrmNode,
    crtc: crtc::Handle,
    mode: control::Mode,
) -> Result<(), String> {
    let UdevBackendData {
        gpus,
        devices,
        primary_gpu,
        paused,
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
        );
    if !*paused {
        device.reapply_gamma_after_fallback(gamma, Some(crtc));
    }
    let drm_output = drm_output.map_err(|err| format!("failed to initialize drm output: {err}"))?;
    let entry = device.heads.get_mut(&crtc).ok_or("unknown head")?;
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
        retry: RenderRetry::default(),
        flip_generation: 0,
    });
    entry.gamma_reapply = true;
    Ok(())
}

/// How long a disabled head's `wl_output` global stays bindable after
/// clients were told it is gone. See [`retire_output_global`].
const OUTPUT_GLOBAL_GRACE: Duration = Duration::from_secs(10);

/// Retires an output's `wl_output` global without killing slow clients.
///
/// `remove_global` frees the global at once, and a client whose bind
/// request is already in flight -- or that only processes the registry
/// every few seconds, as gammastep does, or whose main loop is busy, as
/// eww's is during its own monitor reload -- is then disconnected with
/// `invalid global wl_output`. That is how the bar and gammastep died on
/// every monitor hotplug. So announce the removal first (`disable_global`
/// keeps late binds working) and free the global after a grace period.
fn retire_output_global(
    dh: &smithay::reexports::wayland_server::DisplayHandle,
    handle: &smithay::reexports::calloop::LoopHandle<'static, MindeState>,
    global: GlobalId,
) {
    dh.disable_global::<MindeState>(global.clone());
    let removal_dh = dh.clone();
    let removal_global = global.clone();
    if handle
        .insert_source(Timer::from_duration(OUTPUT_GLOBAL_GRACE), move |_, _, _| {
            removal_dh.remove_global::<MindeState>(removal_global.clone());
            TimeoutAction::Drop
        })
        .is_err()
    {
        warn!("no timer for the wl_output global; removing it immediately");
        dh.remove_global::<MindeState>(global);
    }
}

/// Disables an enabled head: the CRTC is switched off (by dropping the
/// [`OutputSurface`]), the output leaves the space and every surface, and
/// its `wl_output` global is removed like wlroots does. The [`HeadEntry`]
/// (and the Output with its stable id) stays, so the head remains
/// advertised to wlr-output-management clients and can be re-enabled.
/// Desired enable state is committed separately after a configuration
/// succeeds, so rollback cannot cancel pending automatic setup.
pub(crate) fn disable_head(state: &mut MindeState, node: DrmNode, crtc: crtc::Handle) {
    disable_head_inner(state, node, crtc, false);
}

/// Replacing an obsolete mode temporarily disables the head but must not
/// change its power state or send a false DPMS wake event.
fn disable_head_inner(
    state: &mut MindeState,
    node: DrmNode,
    crtc: crtc::Handle,
    preserve_power: bool,
) {
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
        surface.redraw.park(&handle);
    }
    let was_powered_off = entry.power == PowerState::Off;
    if !preserve_power {
        entry.power = PowerState::On;
    }

    let output = entry.output.clone();
    let global = entry.global.take();
    info!(?crtc, output = %output.name(), "disabling head");

    // The Output and its controls survive disable/enable. Report the reset
    // even though this head no longer participates in wake-on-input.
    if was_powered_off && !preserve_power {
        state.output_power_notify(&output, true);
    }

    // Void any gamma control on this output before its CRTC goes away
    // (no restore possible once the surface is gone).
    state.gamma_output_removed(&output);
    state.space.unmap_output(&output);
    state.capture_output_removed(&output);
    if !state.applying_output_config {
        state.close_output_layers(&output);
    }
    output.leave_all();
    // Releases the CRTC (Drop for DrmOutput).
    drop(surface);
    if let Some(global) = global {
        retire_output_global(&dh, &handle, global);
    }

    // The remaining outputs may have fallen back to implicit modifiers to
    // satisfy the bandwidth of the one just removed; try to undo that.
    let udev = state.udev_data.as_mut().unwrap();
    let primary_gpu = udev.primary_gpu;
    let paused = udev.paused;
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
                if !paused {
                    device.reapply_gamma_after_fallback(&mut state.gamma_controls, None);
                }
            }
            Err(err) => warn!(%err, "no renderer to restore modifiers with"),
        }
    }

    state.space.refresh();
    if !state.applying_output_config {
        state.update_usable_area();
        state.update_fractional_scales();
        state.absolute_pointer_motion.reset();
        state.pointer_location = state.clamp_to_outputs(state.pointer_location);
    }
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
    // Tear down every output this device was driving.
    let crtcs: Vec<crtc::Handle> = state
        .udev_data
        .as_ref()
        .and_then(|u| u.devices.get(&node))
        .map(|device| device.heads.keys().copied().collect())
        .unwrap_or_default();
    for crtc in crtcs {
        disable_head(state, node, crtc);
        remove_head(state, node, crtc);
    }
    let Some(udev) = state.udev_data.as_mut() else {
        return;
    };
    if let Some(device) = udev.devices.remove(&node) {
        remove_device_syncobj(&mut udev.syncobj_state, node, &state.display_handle);
        remove_gpu_node(udev, device.render_node);
        // Drops notifier/output-manager refs before closing the libseat lease.
        drop(device);
    }
}

impl MindeState {
    /// Hardware realization can fail and roll back. Change automatic retry
    /// intent only after the entire explicit configuration succeeds.
    pub(crate) fn commit_udev_enable_intent(&mut self, changes: &[HeadChange]) {
        if let Some(udev) = self.udev_data.as_mut() {
            for entry in udev.devices.values_mut().flat_map(|d| d.heads.values_mut()) {
                if let Some(change) = changes.iter().find(|c| c.output == entry.output) {
                    entry.wanted_enabled = change.enabled;
                }
            }
        }
    }

    /// Run startup only after the first successful layout is published,
    /// including an enable performed inside a batched output configuration.
    pub(crate) fn start_udev_if_ready(&mut self) {
        if let Some(udev) = self.udev_data.as_mut()
            && !udev.started
            && udev
                .devices
                .values()
                .any(|d| d.heads.values().any(HeadEntry::enabled))
        {
            udev.started = true;
            guile::on_startup();
        }
    }

    pub(crate) fn output_can_render(&self, output: &Output) -> bool {
        match self.udev_data.as_ref() {
            Some(udev) => udev.can_render(output),
            None => self.output_enabled(output) && !self.winit_powered_off,
        }
    }

    fn frame_finish(
        &mut self,
        node: DrmNode,
        crtc: crtc::Handle,
        metadata: &mut Option<DrmEventMetadata>,
    ) {
        // Submit the flipped frame and reclaim the presentation feedback that
        // was attached when the frame was queued. Scoped so the udev/device/
        // surface borrows are released before we deliver the feedback.
        let (output, submitted, current_generation, vrr) = {
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
            (
                entry.output.clone(),
                surface.drm_output.frame_submitted(),
                surface.flip_generation,
                surface.drm_output.with_compositor(|c| c.vrr_enabled()),
            )
        };

        // wp-presentation-time: mark every surface scanned out on this output
        // as presented, with the real vblank timestamp/sequence when the kernel
        // provided monotonic timestamps (otherwise fall back to our own clock
        // and only claim Vsync).
        let submitted_ok = submitted.is_ok();
        let mut stale_flip = false;
        match submitted {
            Ok(Some(FrameUserData {
                generation,
                feedback,
                lock_generation,
            })) => {
                stale_flip = generation != current_generation;
                if let Some(generation) = lock_generation {
                    self.lock_frame_presented(&output, generation);
                }
                if let Some(mut feedback) = feedback {
                    let (clock, flags) = match metadata.as_ref().map(|m| m.time) {
                        Some(DrmEventTime::Monotonic(tp)) => (
                            tp,
                            wp_presentation_feedback::Kind::Vsync
                                | wp_presentation_feedback::Kind::HwClock
                                | wp_presentation_feedback::Kind::HwCompletion,
                        ),
                        _ => (
                            Clock::<Monotonic>::new().now().into(),
                            wp_presentation_feedback::Kind::Vsync,
                        ),
                    };
                    let seq = metadata.as_ref().map(|m| m.sequence as u64).unwrap_or(0);
                    let refresh = presentation_refresh(output.current_mode(), vrr);
                    feedback.presented::<_, Monotonic>(clock, refresh, seq, flags);
                }
            }
            Ok(None) => {}
            Err(err) => warn!(%err, "drm frame_submitted failed"),
        }

        // The late vblank of a flip the watchdog already gave up on (it
        // then re-rendered under a newer generation): the feedback above
        // is delivered -- the frame did reach the screen -- but the
        // repaint loop is waiting on the newer flip, so leave it alone.
        if stale_flip {
            debug!(?node, ?crtc, "ignoring the vblank of an abandoned flip");
            return;
        }

        if submitted_ok {
            // An older frame can finish while a requested modeset is still
            // pending after a render failure. It must not consume the replay.
            let reapply = self
                .udev_data
                .as_mut()
                .filter(|udev| !udev.paused)
                .and_then(|udev| udev.devices.get_mut(&node))
                .and_then(|device| device.heads.get_mut(&crtc))
                .is_some_and(|entry| {
                    entry.enabled()
                        && entry.power == PowerState::On
                        && entry.surface.as_ref().is_some_and(|surface| {
                            !surface
                                .drm_output
                                .with_compositor(|c| c.surface().commit_pending())
                        })
                        && std::mem::take(&mut entry.gamma_reapply)
                });
            if reapply {
                self.gamma_controls.reapply(&output);
            }
        }

        // The flip completed: repaint right away if anything changed while
        // it was in flight, otherwise go idle until the next dirtying event.
        let handle = self.handle.clone();
        let Some(surface) = self.output_surface_mut(node, crtc) else {
            return;
        };
        if let Some(watchdog) = surface.redraw.finish_vblank() {
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
    /// overlays, output configuration, queued captures. Wakes winit through its
    /// coalescing ping source.
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
        if let Some(ping) = &self.winit_redraw_ping {
            ping.ping();
        }
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
                surface.redraw.park(&handle);
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
        if let Err(err) = init_surface(udev, &mut self.gamma_controls, node, crtc, mode) {
            // The recorded mode may have been set while powered off
            // (`udev_set_mode` cannot test it without a CRTC) and only
            // fail here. Retry at the preferred mode rather than staying
            // dark; on success the Output tracks the mode actually driven.
            let preferred = output.preferred_mode().filter(|p| *p != wl_mode);
            let Some((preferred_wl, preferred_drm)) = preferred.and_then(|p| {
                let entry = udev.devices.get(&node)?.heads.get(&crtc)?;
                entry
                    .modes
                    .iter()
                    .find(|(wl, _)| *wl == p)
                    .map(|(wl, drm)| (*wl, *drm))
            }) else {
                return Err(err);
            };
            warn!(
                %err, output = %output.name(),
                "power-on at the recorded mode failed; retrying at the preferred mode"
            );
            init_surface(udev, &mut self.gamma_controls, node, crtc, preferred_drm)?;
            output.change_current_state(Some(preferred_wl), None, None, None);
        }
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
            disable_head(self, node, crtc);
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
    /// power-on initialises the CRTC from that (and falls back to the
    /// preferred mode if the recorded one fails there -- it could not be
    /// tested against the hardware without a CRTC).
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
        let paused = udev.paused;
        // Split borrows (see `render_surface`): surface in `devices`,
        // renderer in `gpus`.
        let UdevBackendData { gpus, devices, .. } = udev;
        let device = devices.get_mut(&node).ok_or("unknown drm device")?;
        let entry = device.heads.get_mut(&crtc).ok_or("unknown head")?;
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
        let result = surface
            .drm_output
            .use_mode::<_, MindeRenderElements<UdevRenderer<'_>>>(
                drm_mode,
                &mut renderer,
                &DrmOutputRenderElements::default(),
            )
            .map_err(|err| format!("modeset failed: {err}"));
        drop(renderer);
        let except = if result.is_ok() {
            entry.gamma_reapply = true;
            Some(crtc)
        } else {
            None
        };
        if !paused {
            device.reapply_gamma_after_fallback(&mut self.gamma_controls, except);
        }
        result?;
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
        if entry.vrr == on {
            return Ok(());
        }
        surface
            .drm_output
            .with_compositor(|c| c.use_vrr(on))
            .map_err(|err| format!("failed to set adaptive sync: {err}"))?;
        entry.vrr = on;
        entry.gamma_reapply |= entry.vrr_support == VrrSupport::RequiresModeset;
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
    /// needed to drive its gamma ramps. Powered-off heads retain their CRTC;
    /// creating a control does not require a live render surface. Used by
    /// gamma control (udev-only; winit never advertises the global).
    pub fn gamma_info_for_output(
        &self,
        output: &Output,
    ) -> Option<(DrmDeviceFd, crtc::Handle, u32)> {
        use smithay::reexports::drm::control::Device as ControlDevice;
        let udev = self.udev_data.as_ref()?;
        for device in udev.devices.values() {
            for (crtc, entry) in &device.heads {
                if &entry.output != output {
                    continue;
                }
                if !entry.enabled() {
                    return None;
                }
                let fd = device.drm_output_manager.device().device_fd().clone();
                let size = fd.get_crtc(*crtc).ok()?.gamma_length();
                return Some((fd, *crtc, size));
            }
        }
        None
    }

    /// Updates while DPMS-off, paused, or awaiting a modeset completion are
    /// retained by gamma control and replayed when the head is usable again.
    pub(crate) fn gamma_output_writable(&self, output: &Output) -> bool {
        let Some(udev) = self.udev_data.as_ref().filter(|udev| !udev.paused) else {
            return false;
        };
        udev.devices.values().any(|device| {
            device.heads.values().any(|entry| {
                &entry.output == output
                    && entry.enabled()
                    && entry.power == PowerState::On
                    && entry.surface.is_some()
                    && !entry.gamma_reapply
            })
        })
    }

    /// Force damage even for a takeover of an already black screen, but
    /// respect an in-flight flip. Only frame_finish can acknowledge a lock.
    pub(crate) fn schedule_lock_redraw(&mut self) {
        if let Some(udev) = self.udev_data.as_mut() {
            for device in udev.devices.values_mut() {
                for head in device.heads.values_mut() {
                    if let Some(surface) = head.surface.as_mut() {
                        surface
                            .drm_output
                            .with_compositor(|c| c.reset_buffer_ages());
                    }
                }
            }
        }
        self.schedule_redraw();
    }

    /// Physical outputs that must acknowledge a locked frame. A paused seat
    /// cannot prove presentation; powered-off CRTCs expose no content. Winit's
    /// output is a host window, so even a disabled virtual head must swap its
    /// locked/black buffer before acknowledging.
    pub(crate) fn lock_confirmation_outputs(&self) -> Option<Vec<smithay::output::Output>> {
        if let Some(udev) = self.udev_data.as_ref() {
            if udev.paused {
                return None;
            }
            Some(
                udev.devices
                    .values()
                    .flat_map(|device| device.heads.values())
                    .filter(|head| head.enabled() && head.power == PowerState::On)
                    .map(|head| head.output.clone())
                    .collect(),
            )
        } else {
            Some(self.output_management.heads_all().to_vec())
        }
    }

    /// Renders one output now and arms the follow-up: the vblank (plus a
    /// watchdog) when a frame was queued, a refresh-interval timer when
    /// nothing changed, or a bounded retry timer after a render failure.
    /// Paused sessions keep their damage until resume forces a new render.
    fn render_now(&mut self, node: DrmNode, crtc: crtc::Handle) {
        let handle = self.handle.clone();
        // A disabled or vanished head has no surface to render to; the
        // damage loop simply ends here.
        let Some((output, paused)) = self.udev_data.as_ref().and_then(|udev| {
            let entry = udev.devices.get(&node)?.heads.get(&crtc)?;
            entry.enabled().then(|| (entry.output.clone(), udev.paused))
        }) else {
            return;
        };
        let Some(surface) = self.output_surface_mut(node, crtc) else {
            return;
        };
        surface.redraw.park(&handle);
        if paused {
            surface.dirty = true;
            return;
        }
        surface.dirty = false;
        let interval = refresh_interval(&output);

        let started = std::time::Instant::now();
        let rendered = self.render_surface(node, crtc);
        crate::timing::record(crate::timing::Probe::Render, started);
        let now = self.start_time.elapsed();
        let Some(surface) = self.output_surface_mut(node, crtc) else {
            return;
        };
        let (queued, delay) = match rendered {
            Ok(queued) => {
                surface.retry.succeeded();
                (queued, interval)
            }
            Err(err) => {
                surface.dirty = true;
                let (delay, log) = surface.retry.failed(interval, now);
                if log {
                    match err {
                        SwapBuffersError::TemporaryFailure(_) => {
                            debug!(%err, ?node, ?crtc, retry_after = ?delay,
                                "temporary failure rendering udev output")
                        }
                        _ => warn!(%err, ?node, ?crtc, retry_after = ?delay,
                            "error rendering udev output"),
                    }
                }
                (false, delay)
            }
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
                .insert_source(Timer::from_duration(delay), move |_, _, state| {
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
        let lock_generation = self.lock_generation();
        let udev = self
            .udev_data
            .as_mut()
            .ok_or(SwapBuffersError::AlreadySwapped)?;
        let primary_gpu = udev.primary_gpu;

        let Some(output) = udev
            .devices
            .get(&node)
            .and_then(|device| device.heads.get(&crtc))
            .map(|entry| entry.output.clone())
        else {
            return Ok(false);
        };
        let frame_callbacks =
            crate::frame_callbacks::FrameCallbacks::new(&self.space, &output, |output| {
                udev.can_render(output)
            });

        // Split borrows: the surface lives in `devices`, the renderer in
        // `gpus` -- disjoint fields of the same UdevBackendData.
        let UdevBackendData { gpus, devices, .. } = udev;
        let Some(entry) = devices.get_mut(&node).and_then(|d| d.heads.get_mut(&crtc)) else {
            return Ok(false);
        };
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
                output_surface.flip_generation = output_surface.flip_generation.wrapping_add(1);
                output_surface
                    .drm_output
                    .queue_frame(FrameUserData {
                        generation: output_surface.flip_generation,
                        feedback: None,
                        lock_generation: Some(lock_generation),
                    })
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
        // Use the cached membership directly: querying outputs_for_element
        // for each window repeatedly scans Space and allocates a Vec.
        for window in self.space.elements_for_output(&output) {
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
            output_surface.flip_generation = output_surface.flip_generation.wrapping_add(1);
            output_surface
                .drm_output
                .queue_frame(FrameUserData {
                    generation: output_surface.flip_generation,
                    feedback: Some(presentation_feedback),
                    lock_generation: None,
                })
                .map_err(Into::<SwapBuffersError>::into)?;
        } else {
            // No frame will be scanned out, so no vblank will arrive to deliver
            // the feedback; discard it now rather than leak the callbacks.
            presentation_feedback.discarded();
        }

        // Only the selected rendering output services parked windows.
        let now = self.start_time.elapsed();
        if frame_callbacks.send_visible(&output, now) {
            self.parked_frames.schedule(&self.handle, now);
        }
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
            let time = Clock::<Monotonic>::new().now();
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

    fn monitor() -> Output {
        Output::new(
            "DP-1".into(),
            PhysicalProperties {
                size: (600, 340).into(),
                subpixel: Subpixel::Unknown,
                make: "TST".into(),
                model: "Monitor".into(),
                serial_number: "1".into(),
            },
        )
    }

    #[test]
    fn late_modes_and_preferred_changes_preserve_valid_current_modes() {
        let output = monitor();
        let first = wl(1920, 1080, 60_000);
        let second = wl(2560, 1440, 60_000);
        assert!(!set_connector_modes(&output, &[], None));
        assert!(set_connector_modes(&output, &[first], Some(first)));
        output.change_current_state(Some(first), None, None, None);
        assert!(set_connector_modes(&output, &[first, second], Some(second)));
        assert_eq!(output.current_mode(), Some(first));
        assert_eq!(output.preferred_mode(), Some(second));
        assert!(!set_connector_modes(
            &output,
            &[second, first],
            Some(second)
        ));
        assert!(!set_connector_modes(
            &output,
            &[second, first],
            Some(second)
        ));
        assert!(set_connector_modes(&output, &[second], Some(second)));
        assert_eq!(output.current_mode(), None);
        assert_eq!(output.modes(), vec![second]);
        output.change_current_state(Some(second), None, None, None);
        assert!(set_connector_modes(&output, &[], None));
        assert_eq!(output.current_mode(), None);
        assert_eq!(output.preferred_mode(), None);
        assert!(output.modes().is_empty());
    }

    #[test]
    fn automatic_setup_retries_late_modes_but_respects_explicit_disable() {
        let output = monitor();
        let first = wl(1920, 1080, 60_000);
        let second = wl(2560, 1440, 60_000);
        let modes = [(first, 1), (second, 2)];
        assert_eq!(automatic_mode::<u8>(&output, &[], true, false), None);
        set_connector_modes(&output, &[first, second], Some(second));
        // Repeated scans after failed setup must offer the preferred mode.
        for _ in 0..3 {
            assert_eq!(automatic_mode(&output, &modes, true, false), Some(2));
        }
        output.change_current_state(Some(first), None, None, None);
        assert_eq!(automatic_mode(&output, &modes, true, false), Some(1));
        assert_eq!(automatic_mode(&output, &modes, true, true), None);
        assert_eq!(automatic_mode(&output, &modes, false, false), None);
        // Even losing the current mode cannot override an explicit disable.
        set_connector_modes(&output, &[second], Some(second));
        assert_eq!(automatic_mode(&output, &modes[1..], false, false), None);
        assert_eq!(automatic_mode(&output, &modes[1..], true, false), Some(2));
    }

    #[test]
    fn identity_changes_are_detected_even_with_unchanged_modes() {
        let output = monitor();
        let physical = output.physical_properties();
        assert!(same_monitor(&output, &physical, None));
        let edid = EdidInfo {
            make: physical.make.clone(),
            model: physical.model.clone(),
            serial: physical.serial_number.clone(),
        };
        assert!(!same_monitor(&output, &physical, Some(&edid)));
        output.user_data().insert_if_missing(|| edid.clone());
        assert!(same_monitor(&output, &physical, Some(&edid)));
        assert!(!same_monitor(&output, &physical, None));
        let mut replacement = edid.clone();
        replacement.serial = "2".into();
        assert!(!same_monitor(&output, &physical, Some(&replacement)));
        let mut replacement = physical.clone();
        replacement.size = (700, 400).into();
        assert!(!same_monitor(&output, &replacement, Some(&edid)));
        let mut replacement = physical.clone();
        replacement.subpixel = Subpixel::HorizontalRgb;
        assert!(!same_monitor(&output, &replacement, Some(&edid)));
    }

    #[test]
    fn explicit_sync_releases_only_its_owning_device() {
        use smithay::{reexports::wayland_server::Display, utils::DeviceFd};
        use std::{fs::File, os::fd::OwnedFd};

        let display = Display::<MindeState>::new().unwrap();
        // DrmNode construction probes sysfs even from a dev_t. Exercise
        // ownership with plain IDs so this test also runs without GPUs.
        let owner = 0_u8;
        let other = 1_u8;
        for _ in 0..32 {
            // No DRM ioctls are required to register this global. A harmless
            // fd lets its actual refcount/lifetime run without a real seat.
            let fd: OwnedFd = File::open("/dev/null").unwrap().into();
            let fd = DrmDeviceFd::new(DeviceFd::from(fd));
            let weak = fd.downgrade();
            let mut state = Some((
                owner,
                DrmSyncobjState::new::<MindeState>(&display.handle(), fd),
            ));
            remove_device_syncobj(&mut state, other, &display.handle());
            assert!(state.is_some());
            assert!(weak.upgrade().is_some());
            remove_device_syncobj(&mut state, owner, &display.handle());
            assert!(state.is_none());
            assert!(weak.upgrade().is_none());
            remove_device_syncobj(&mut state, owner, &display.handle());
        }
    }

    #[test]
    fn failed_renders_keep_retrying_without_refresh_rate_spinning() {
        for interval in [
            Duration::from_millis(3),
            Duration::from_millis(16),
            Duration::from_secs(2),
        ] {
            let mut retry = RenderRetry::default();
            let mut now = Duration::ZERO;
            let mut attempts = 0;
            let mut diagnostics = Vec::new();
            // Simulate a minute of persistent device failure without any
            // new client commits, at fast and slow monitor refresh rates.
            while now < Duration::from_secs(60) {
                let (delay, log) = retry.failed(interval, now);
                assert!((Duration::from_millis(100)..=Duration::from_secs(1)).contains(&delay));
                if log {
                    diagnostics.push(now);
                }
                now += delay;
                attempts += 1;
            }
            assert!(
                (60..=65).contains(&attempts),
                "{attempts} attempts for {interval:?}"
            );
            assert_eq!(diagnostics.first(), Some(&Duration::ZERO));
            assert!(diagnostics.len() <= 6);
            assert!(
                diagnostics
                    .windows(2)
                    .all(|pair| pair[1] - pair[0] >= Duration::from_secs(10))
            );
        }
    }

    #[test]
    fn successful_render_resets_retry_delay_but_not_diagnostic_throttle() {
        let interval = Duration::from_millis(16);
        let mut retry = RenderRetry::default();
        let (first_delay, log) = retry.failed(interval, Duration::ZERO);
        assert!(log);
        let mut now = first_delay;
        let mut previous = first_delay;
        for _ in 0..8 {
            let (delay, _) = retry.failed(interval, now);
            assert!(delay >= previous);
            now += delay;
            previous = delay;
        }
        assert_eq!(previous, Duration::from_secs(1));
        // Another output starts with its own retry and diagnostic budget.
        assert_eq!(
            RenderRetry::default().failed(interval, now),
            (first_delay, true)
        );
        retry.succeeded();
        assert_eq!(retry.failed(interval, now), (first_delay, false));
        // Rapid alternating successes and failures must not flood the log.
        for _ in 0..100 {
            retry.succeeded();
            assert_eq!(retry.failed(interval, now), (first_delay, false));
        }
        assert!(retry.failed(interval, Duration::from_secs(10)).1);
    }

    #[test]
    fn parking_cancels_repaint_and_watchdog_timers() {
        for waiting_for_vblank in [false, true] {
            let mut event_loop = EventLoop::<usize>::try_new().unwrap();
            let handle = event_loop.handle();
            let token = handle
                .insert_source(Timer::from_duration(Duration::ZERO), |_, _, calls| {
                    *calls += 1;
                    TimeoutAction::Drop
                })
                .unwrap();
            let mut state = if waiting_for_vblank {
                RedrawState::WaitingForVblank { watchdog: token }
            } else {
                RedrawState::WaitingForTimer { token }
            };
            state.park(&handle);
            assert!(matches!(state, RedrawState::Idle));
            state.park(&handle);
            let mut calls = 0;
            event_loop.dispatch(Duration::ZERO, &mut calls).unwrap();
            assert_eq!(
                calls, 0,
                "parked work must not fire into a paused or removed output"
            );
        }
    }

    #[test]
    fn unexpected_vblank_preserves_scheduled_repaint() {
        let mut event_loop = EventLoop::<(RedrawState, usize)>::try_new().unwrap();
        let handle = event_loop.handle();
        handle.insert_idle(|(state, repaints)| {
            if matches!(state, RedrawState::Scheduled) {
                *state = RedrawState::Idle;
                *repaints += 1;
            }
        });
        let mut data = (RedrawState::Scheduled, 0);
        assert!(data.0.finish_vblank().is_none());
        event_loop.dispatch(Duration::ZERO, &mut data).unwrap();
        assert_eq!(data.1, 1, "the dirty output must still repaint");
    }

    #[test]
    fn unexpected_vblank_preserves_repaint_timer() {
        let mut event_loop = EventLoop::<(RedrawState, usize)>::try_new().unwrap();
        let handle = event_loop.handle();
        let token = handle
            .insert_source(
                Timer::from_duration(Duration::ZERO),
                |_, _, (state, repaints)| {
                    if matches!(state, RedrawState::WaitingForTimer { .. }) {
                        *state = RedrawState::Idle;
                        *repaints += 1;
                    }
                    TimeoutAction::Drop
                },
            )
            .unwrap();
        let mut data = (RedrawState::WaitingForTimer { token }, 0);
        assert!(data.0.finish_vblank().is_none());
        assert!(matches!(data.0, RedrawState::WaitingForTimer { token: saved } if saved == token));
        event_loop.dispatch(Duration::ZERO, &mut data).unwrap();
        assert_eq!(data.1, 1, "the dirty output must still repaint");
    }

    #[test]
    fn expected_vblank_retires_its_watchdog_once() {
        let mut event_loop = EventLoop::<usize>::try_new().unwrap();
        let handle = event_loop.handle();
        let watchdog = handle
            .insert_source(Timer::from_duration(Duration::ZERO), |_, _, calls| {
                *calls += 1;
                TimeoutAction::Drop
            })
            .unwrap();
        let mut state = RedrawState::WaitingForVblank { watchdog };
        handle.remove(state.finish_vblank().unwrap());
        assert!(matches!(state, RedrawState::Idle));
        assert!(state.finish_vblank().is_none());
        let mut calls = 0;
        event_loop.dispatch(Duration::ZERO, &mut calls).unwrap();
        assert_eq!(calls, 0, "a completed flip must cancel its watchdog");
    }

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
    fn presentation_refresh_handles_unknown_fixed_and_variable_modes() {
        for vrr in [false, true] {
            assert_eq!(presentation_refresh(None, vrr), Refresh::Unknown);
            for refresh in [i32::MIN, -1, 0] {
                assert_eq!(
                    presentation_refresh(Some(wl(1920, 1080, refresh)), vrr),
                    Refresh::Unknown
                );
            }
        }
        let mode = Some(wl(1920, 1080, 60_000));
        let interval = Duration::from_nanos(16_666_667);
        assert_eq!(presentation_refresh(mode, false), Refresh::Fixed(interval));
        assert_eq!(
            presentation_refresh(mode, true),
            Refresh::Variable(interval)
        );
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
        for refresh in [i32::MIN, -1, 0, i32::MAX] {
            assert_eq!(
                resolve_connector_mode(&m, wl(1920, 1080, refresh), true),
                None
            );
        }
        assert_eq!(
            resolve_connector_mode(
                &[(wl(1920, 1080, i32::MIN), "invalid")],
                wl(1920, 1080, i32::MAX),
                true
            ),
            None
        );
    }
}
