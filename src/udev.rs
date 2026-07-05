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
//! - no DRM leasing, no fps/debug overlays, no screencopy/wlr protocols,
//!   no profiling, no presentation-time feedback (this compositor doesn't
//!   expose `wp_presentation`, so DRM output user-data is just `()`).

use std::{collections::HashMap, path::Path, time::Duration};

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
            DrmNode, NodeType,
            compositor::FrameFlags,
            exporter::gbm::GbmFramebufferExporter,
            output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
        },
        egl::{EGLContext, EGLDevice, EGLDisplay, context::ContextPriority},
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            ImportDma,
            gles::{Capability, GlesRenderer},
            multigpu::{GpuManager, MultiRenderer, gbm::GbmGlesBackend},
        },
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{UdevBackend, UdevEvent, primary_gpu},
    },
    output::{Mode as WlMode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{
            EventLoop, RegistrationToken,
            timer::{TimeoutAction, Timer},
        },
        drm::control::{ModeTypeFlags, connector, crtc},
        input::Libinput,
        rustix::fs::OFlags,
        wayland_server::{DisplayHandle, backend::GlobalId},
    },
    utils::{DeviceFd, Transform},
    wayland::dmabuf::{
        DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier,
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

type GbmDrmOutputManager = DrmOutputManager<
    GbmAllocator<DrmDeviceFd>,
    GbmFramebufferExporter<DrmDeviceFd>,
    Option<()>,
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
        Option<()>,
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

    state.session = Some(session.clone());
    state.udev_data = Some(UdevBackendData {
        session: session.clone(),
        primary_gpu,
        gpus,
        devices: HashMap::new(),
        started: false,
        dmabuf_state: None,
    });

    let udev_backend = UdevBackend::new(&seat_name)?;

    let mut libinput_context =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput_context.udev_assign_seat(&seat_name).unwrap();
    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());

    event_loop
        .handle()
        .insert_source(libinput_backend, |event, _, state| {
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

fn device_added(
    state: &mut MindeState,
    node: DrmNode,
    path: &Path,
) -> Result<(), DeviceAddError> {
    let handle = state.handle.clone();
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
        let _ = metadata;
        let Some(udev) = self.udev_data.as_mut() else {
            return;
        };
        let Some(device) = udev.devices.get_mut(&node) else {
            return;
        };
        if let Some(surface) = device.surfaces.get_mut(&crtc) {
            let _ = surface.drm_output.frame_submitted();
        } else {
            return; // output vanished (unplug); don't reschedule repaints
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

    fn handle_repaint_now(&mut self, node: DrmNode, crtc: crtc::Handle) {
        self.render_now(node, crtc);
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
                output.current_scale().integer_scale(),
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
            if let Some(elem) = crate::render::overlay_element(
                &mut renderer,
                msg,
                *loc - output_geo.loc,
                output.current_scale().integer_scale(),
            ) {
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
            custom.extend(output_surface.border_buffers.elements(
                local,
                output.current_scale().integer_scale(),
                self.border_color,
            ));
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

        let queued = !render_result.is_empty;
        if queued {
            output_surface
                .drm_output
                .queue_frame(None)
                .map_err(Into::<SwapBuffersError>::into)?;
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
        let _ = self.display_handle.flush_clients();

        Ok(queued)
    }
}
