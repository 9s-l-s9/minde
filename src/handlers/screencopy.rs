// SPDX-License-Identifier: MIT

//! `ext-image-copy-capture-v1` + `ext-image-capture-source-v1` (screencopy).
//!
//! This is the protocol grim/`xdg-desktop-portal-wlr` and PipeWire-based
//! screen-cast tools speak to grab a frame of an output. We advertise only
//! **output** capture (no toplevel source manager, no cursor session): those
//! would need `ext-foreign-toplevel-list` and per-cursor render passes we do
//! not have yet, and the protocol is explicit that a compositor must not
//! advertise what it cannot deliver.
//!
//! Both backends offer **shm** capture (grim uses shm); the udev backend
//! additionally offers **dmabuf** constraints from the primary render node so
//! a future zero-copy PipeWire path can allocate GPU buffers. winit only ever
//! offers shm (it has no DRM render node to hand out).
//!
//! ## Flow
//!
//! A client creates an output capture source, then a capture session; we
//! answer with buffer constraints ([`MindeState::capture_constraints`]).
//! The client allocates a matching buffer, creates a frame, and requests
//! capture. We do not render synchronously -- capture must reflect a real
//! composited frame -- so [`ImageCopyCaptureHandler::frame`] just queues the
//! frame against its output. The next on-screen render for that output
//! ([`crate::winit`] redraw / [`crate::udev`] vblank-driven render) then calls
//! [`satisfy_output_captures`], which updates a shared capture scene and copies
//! it into pending buffers before signaling `ready`.
//!
//! Persistent damage tracking avoids unchanged renders/readbacks. Ext sessions
//! supply buffer damage; wlr copy-with-damage waits for a scene change. Capture
//! images and their bounded damage history are shared across matching requests.

mod cache;
pub use cache::{CaptureCache, CaptureHistory};

use smithay::{
    backend::{
        allocator::{Fourcc, dmabuf::Dmabuf},
        renderer::{
            Bind, ExportMem, ImportAll, ImportMem, Offscreen, Renderer,
            damage::OutputDamageTracker, element::AsRenderElements, gles::GlesRenderbuffer,
        },
    },
    desktop::{Space, Window, layer_map_for_output},
    output::{Mode, Output},
    reexports::wayland_server::protocol::wl_buffer::WlBuffer,
    utils::{
        Buffer as BufferCoords, Logical, Monotonic, Physical, Point, Rectangle, Scale, Size, Time,
        Transform,
    },
    wayland::{
        image_capture_source::{
            ImageCaptureSource, ImageCaptureSourceHandler, OutputCaptureSourceHandler,
            OutputCaptureSourceState,
        },
        image_copy_capture::{
            BufferConstraints, CaptureFailureReason, Frame, ImageCopyCaptureHandler,
            ImageCopyCaptureState, Session, SessionRef,
        },
        shell::wlr_layer::Layer as WlrLayer,
        shm::{shm_format_to_fourcc, with_buffer_contents_mut},
    },
};

use crate::MindeState;
use crate::render::{BorderBuffers, CursorState, MessageState, MindeRenderElements};

/// A capture frame queued against an output, awaiting the next render. Holds
/// either an `ext-image-copy-capture-v1` frame or a `wlr-screencopy-unstable-v1`
/// frame -- both flow through the same [`MindeState::pending_captures`] queue
/// and are satisfied by [`satisfy_output_captures`] after the next composite.
pub enum CaptureFrame {
    /// `ext-image-copy-capture-v1` frame (Smithay-driven).
    Ext(Frame),
    /// `wlr-screencopy-unstable-v1` frame (hand-written protocol).
    Wlr(crate::handlers::wlr_screencopy::WlrPending),
    /// `wm-screenshot`: render to an offscreen buffer and write a PNG.
    /// Completion is reported through the automation-result registry.
    File {
        destination: crate::screenshot::Destination,
        token: crate::automation_dnd::AutomationToken,
    },
}

impl CaptureFrame {
    fn fail(self, reason: CaptureFailureReason) {
        match self {
            Self::Ext(frame) => frame.fail(reason),
            Self::Wlr(frame) => frame.fail(),
            Self::File { token, .. } => crate::automation_dnd::record_and_publish(
                token,
                crate::automation_dnd::AutomationOperation::Screenshot,
                crate::automation_dnd::AutomationStatus::Failed,
            ),
        }
    }
}

pub(super) type CaptureGeometry = (Option<Mode>, f64, Transform);

pub(super) fn capture_geometry(output: &Output) -> CaptureGeometry {
    (
        output.current_mode(),
        output.current_scale().fractional_scale(),
        output.current_transform(),
    )
}

/// A capture queued against an output, awaiting the next render.
pub struct PendingCapture {
    pub output: Output,
    pub frame: CaptureFrame,
    pub draw_cursor: bool,
    /// Top-left of the captured area in output-logical coords. `(0, 0)` for
    /// full-output captures (all ext frames, plain wlr `capture_output`); a
    /// non-zero origin comes from wlr `capture_output_region`, and shifts the
    /// whole scene so a sub-rectangle lands in a region-sized buffer.
    pub origin: Point<i32, Logical>,
    /// Physical size of the destination buffer.
    pub size: Size<i32, Physical>,
    geometry: CaptureGeometry,
    history: Option<CaptureHistory>,
}

impl PendingCapture {
    pub fn new(
        output: Output,
        frame: CaptureFrame,
        draw_cursor: bool,
        origin: Point<i32, Logical>,
        size: Size<i32, Physical>,
    ) -> Self {
        let geometry = capture_geometry(&output);
        let history = match &frame {
            CaptureFrame::Wlr(frame) => Some(frame.history.clone()),
            _ => None,
        };
        Self {
            output,
            frame,
            draw_cursor,
            origin,
            size,
            geometry,
            history,
        }
    }
}

impl MindeState {
    pub(crate) fn capture_output_available(&self, output: &Output) -> bool {
        !self.locked && self.output_enabled(output) && self.output_power_is_on(output)
    }

    fn capture_size(&self, output: &Output) -> Option<Size<i32, Physical>> {
        let size = self
            .space
            .output_geometry(output)?
            .size
            .to_physical_precise_round(output.current_scale().fractional_scale());
        (size.w > 0 && size.h > 0).then_some(size)
    }

    /// Complete requests before a lock or unavailable output can strand them.
    pub(crate) fn fail_pending_captures(
        &mut self,
        output: Option<&Output>,
        reason: CaptureFailureReason,
    ) {
        self.capture_cache.clear(output);
        let pending = std::mem::take(&mut self.pending_captures);
        for capture in pending {
            if output.is_none_or(|output| output == &capture.output) {
                capture.frame.fail(reason);
            } else {
                self.pending_captures.push(capture);
            }
        }
    }

    pub(crate) fn capture_output_removed(&mut self, output: &Output) {
        self.fail_pending_captures(Some(output), CaptureFailureReason::Stopped);
        // Dropping an owned Session sends stopped and retires its active frames.
        self.capture_sessions.retain(|session| {
            session
                .source()
                .user_data()
                .get::<smithay::output::WeakOutput>()
                .and_then(|weak| weak.upgrade())
                .is_some_and(|source| &source != output)
        });
    }

    /// Called with the final layout after resize, hotplug or configuration.
    pub(crate) fn refresh_capture_constraints(&mut self) {
        let pending = std::mem::take(&mut self.pending_captures);
        for capture in pending {
            if !self.capture_output_available(&capture.output) {
                capture.frame.fail(CaptureFailureReason::Unknown);
            } else if capture.geometry != capture_geometry(&capture.output) {
                capture.frame.fail(CaptureFailureReason::BufferConstraints);
            } else {
                self.pending_captures.push(capture);
            }
        }
        let sessions = std::mem::take(&mut self.capture_sessions);
        for session in sessions {
            if let Some(constraints) = self.capture_constraints(&session.source()) {
                session.update_constraints(constraints);
                self.capture_sessions.push(session);
            } else {
                session.stop();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Handler impls
// ---------------------------------------------------------------------------

impl ImageCaptureSourceHandler for MindeState {
    fn source_destroyed(&mut self, _source: ImageCaptureSource) {}
}

impl OutputCaptureSourceHandler for MindeState {
    fn output_capture_source_state(&mut self) -> &mut OutputCaptureSourceState {
        &mut self.output_capture_source_state
    }

    fn output_source_created(&mut self, source: ImageCaptureSource, output: &Output) {
        // Stash the (weak) output on the source so the constraints/frame
        // handlers can resolve it back to a mapped output.
        source.user_data().insert_if_missing(|| output.downgrade());
    }
}

impl ImageCopyCaptureHandler for MindeState {
    fn image_copy_capture_state(&mut self) -> &mut ImageCopyCaptureState {
        &mut self.image_copy_capture_state
    }

    fn capture_constraints(&mut self, source: &ImageCaptureSource) -> Option<BufferConstraints> {
        let output = source
            .user_data()
            .get::<smithay::output::WeakOutput>()
            .and_then(|weak| weak.upgrade())?;
        // Only capture outputs still mapped into the space (an unplugged
        // monitor's source lingers until the client drops it).
        if !self.space.outputs().any(|o| o == &output) {
            return None;
        }
        let size = self.capture_size(&output)?;
        let size: Size<i32, BufferCoords> = (size.w, size.h).into();

        use smithay::reexports::wayland_server::protocol::wl_shm;
        let shm = vec![wl_shm::Format::Argb8888, wl_shm::Format::Xrgb8888];

        // dmabuf capture is offered only where we own a DRM render node
        // (udev). It feeds a future zero-copy screen-cast path; grim itself
        // takes the shm route above.
        let dma = self.dmabuf_capture_constraints();

        Some(BufferConstraints { size, shm, dma })
    }

    fn new_session(&mut self, session: Session) {
        // The protocol state keeps the SessionRef alive; we only need the
        // owned Session to stay alive for the capture's duration, so hand it
        // to the state's session list by leaking it into the manager. Since
        // ImageCopyCaptureState already retains a SessionRef clone, dropping
        // the owned Session here would send `stopped`. Keep it parked.
        self.capture_sessions.push(session);
    }

    fn frame(&mut self, session: &SessionRef, frame: Frame) {
        let Some(output) = session
            .source()
            .user_data()
            .get::<smithay::output::WeakOutput>()
            .and_then(|weak| weak.upgrade())
        else {
            frame.fail(CaptureFailureReason::Unknown);
            return;
        };
        if !self.capture_output_available(&output) {
            frame.fail(CaptureFailureReason::Unknown);
            return;
        }
        let Some(size) = self.capture_size(&output) else {
            frame.fail(CaptureFailureReason::Unknown);
            return;
        };
        session
            .user_data()
            .insert_if_missing(CaptureHistory::default);
        let mut capture = PendingCapture::new(
            output,
            CaptureFrame::Ext(frame),
            session.draw_cursor(),
            (0, 0).into(),
            size,
        );
        capture.history = session.user_data().get::<CaptureHistory>().cloned();
        self.pending_captures.push(capture);
        // Captures are satisfied by the next composite; make sure one runs.
        self.schedule_redraw();
    }

    fn session_destroyed(&mut self, session: SessionRef) {
        self.capture_sessions.retain(|s| *s != session);
    }

    fn frame_aborted(&mut self, frame: smithay::wayland::image_copy_capture::FrameRef) {
        self.pending_captures.retain(|capture| {
            !matches!(&capture.frame,
            CaptureFrame::Ext(pending) if pending == &frame)
        });
    }
}

// ---------------------------------------------------------------------------
// Scene assembly + buffer rendering
// ---------------------------------------------------------------------------

/// Builds the render-element list for one output: cursor, message overlay,
/// positioned overlays, upper layer surfaces, focus border, windows
/// (front-to-back), then lower layer surfaces. The single scene assembly
/// shared by the on-screen backends (`udev`, `winit`) and the capture
/// paths, so they cannot drift apart. Renderer-generic.
///
/// `border_buffers` should be persistent per output for on-screen use (stable
/// element ids keep damage tracking incremental); each cached capture scene
/// retains its own buffers for the same reason.
/// The returned list borrows nothing: the layer-map guard is released before
/// returning, so callers may open their own afterwards.
#[allow(clippy::too_many_arguments)]
pub(crate) fn output_scene_elements<R>(
    renderer: &mut R,
    output: &Output,
    output_geo: Rectangle<i32, Logical>,
    origin: Point<i32, Logical>,
    scale: Scale<f64>,
    space: &Space<Window>,
    cursor: Option<(&mut CursorState, Point<f64, Logical>)>,
    message: Option<&MessageState>,
    overlays: &[(Point<i32, Logical>, MessageState)],
    focus: Option<Rectangle<i32, Logical>>,
    border_color: [f32; 4],
    border_buffers: &mut BorderBuffers,
) -> Vec<MindeRenderElements<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
{
    // Every element is positioned relative to the output origin; shifting that
    // origin by the capture `origin` slides the whole scene so a sub-rectangle
    // (wlr region capture) lands at the top-left of the destination buffer.
    // Full-output captures pass `(0, 0)` and are unaffected.
    let base = output_geo.loc + origin;
    let mut all: Vec<MindeRenderElements<R>> = Vec::new();

    // Cursor at the current pointer location (only when this capture asked
    // for cursors and the pointer is over this output).
    if let Some((cursor_state, pointer_location)) = cursor
        && output_geo.to_f64().contains(pointer_location)
    {
        let hotspot = cursor_state.hotspot();
        let cursor_pos = pointer_location - base.to_f64();
        let cursor_phys = (cursor_pos - hotspot.to_f64())
            .to_physical(scale)
            .to_i32_round();
        all.extend(cursor_state.render_elements(renderer, cursor_phys, scale));
    }

    // Centered message overlay.
    if let Some(msg) = message {
        let center = Point::<i32, Logical>::from((
            (output_geo.size.w - msg.size.0) / 2,
            (output_geo.size.h - msg.size.1) / 2,
        ));
        if let Some(elem) = crate::render::overlay_element(renderer, msg, center - origin, scale) {
            all.push(elem);
        }
    }

    // Positioned overlays landing on this output.
    for (loc, msg) in overlays.iter().filter(|(l, _)| output_geo.contains(*l)) {
        if let Some(elem) = crate::render::overlay_element(renderer, msg, *loc - base, scale) {
            all.push(elem);
        }
    }

    // Front-to-back order matches hit-testing: higher levels first, newest
    // mapping first within each level. Borrowed iteration avoids partitions
    // and temporary element lists on every frame and capture.
    let fullscreen = crate::state::output_has_fullscreen(space, output_geo);
    let layer_map = layer_map_for_output(output);
    let append_layers = |lower: bool, renderer: &mut R, out: &mut Vec<MindeRenderElements<R>>| {
        let levels = if lower {
            [WlrLayer::Bottom, WlrLayer::Background]
        } else {
            [WlrLayer::Overlay, WlrLayer::Top]
        };
        for level in levels {
            if fullscreen && level == WlrLayer::Top {
                continue;
            }
            for surface in layer_map.layers_on(level).rev() {
                let loc = layer_map
                    .layer_geometry(surface)
                    .map(|geo| geo.loc)
                    .unwrap_or_default()
                    - origin;
                out.extend(AsRenderElements::<R>::render_elements::<
                    MindeRenderElements<R>,
                >(
                    surface,
                    renderer,
                    loc.to_physical_precise_round(scale),
                    scale,
                    1.0,
                ));
            }
        }
    };
    append_layers(false, renderer, &mut all);

    // Focus border around the selected frame.
    if let Some(geo) = focus
        && !fullscreen
        && geo.overlaps(output_geo)
    {
        let mut local = geo;
        local.loc -= base;
        all.extend(border_buffers.elements(local, scale, border_color));
    }

    // Window surfaces, front-to-back (space yields back-to-front).
    for window in space.elements().rev() {
        let Some(loc) = space.element_location(window) else {
            continue;
        };
        if !space
            .element_geometry(window)
            .map(|g| g.overlaps(output_geo))
            .unwrap_or(false)
        {
            continue;
        }
        let phys_loc = (loc - window.geometry().loc - base).to_physical_precise_round(scale);
        all.extend(AsRenderElements::<R>::render_elements(
            window, renderer, phys_loc, scale, 1.0,
        ));
    }
    // Background/bottom layers under everything.
    append_layers(true, renderer, &mut all);
    all
}

/// Protocol-independent result of copying a capture image.
pub enum FillOutcome {
    Ok(Vec<Rectangle<i32, BufferCoords>>),
    UnsupportedBuffer,
    RenderFailed,
}

/// Update each distinct capture scene once, then fan it out to its clients.
/// Source damage and client buffer repair are independent: a fresh buffer
/// still needs all pixels even when the scene itself has not changed.
#[allow(clippy::too_many_arguments)]
pub fn satisfy_output_captures<R>(
    renderer: &mut R,
    output: &Output,
    output_geo: Rectangle<i32, Logical>,
    scale: Scale<f64>,
    time: Time<Monotonic>,
    pending: &mut Vec<PendingCapture>,
    cache: &mut CaptureCache,
    space: &Space<Window>,
    cursor_state: &mut CursorState,
    pointer_location: Point<f64, Logical>,
    message: Option<&MessageState>,
    overlays: &[(Point<i32, Logical>, MessageState)],
    focus: Option<Rectangle<i32, Logical>>,
    border_color: [f32; 4],
) where
    R: Renderer
        + ImportAll
        + ImportMem
        + ExportMem
        + Offscreen<GlesRenderbuffer>
        + Bind<Dmabuf>
        + smithay::backend::renderer::Blit,
    R::TextureId: Send + Clone + 'static,
    R::Error: Send + Sync + 'static,
{
    let (mine, rest): (Vec<_>, Vec<_>) = pending.drain(..).partition(|c| &c.output == output);
    *pending = rest;
    let mut groups: Vec<(cache::Key, Vec<PendingCapture>)> = Vec::new();
    for capture in mine {
        if capture.geometry != capture_geometry(output) {
            capture.frame.fail(CaptureFailureReason::BufferConstraints);
            continue;
        }
        let key = cache::Key {
            output: output.clone(),
            geometry: capture.geometry,
            output_geo,
            origin: capture.origin,
            size: capture.size,
            cursor: capture.draw_cursor,
            context: renderer.context_id().erased(),
        };
        if let Some((_, frames)) = groups.iter_mut().find(|(existing, _)| existing == &key) {
            frames.push(capture);
        } else {
            groups.push((key, vec![capture]));
        }
    }
    for (key, captures) in groups {
        let mut scene = cache.take(key, scale);
        let cursor = scene
            .key
            .cursor
            .then_some((&mut *cursor_state, pointer_location));
        let elements = output_scene_elements(
            renderer,
            output,
            output_geo,
            scene.key.origin,
            scale,
            space,
            cursor,
            message,
            overlays,
            focus,
            border_color,
            &mut scene.borders,
        );
        if let Err(err) = scene.render(renderer, &elements) {
            tracing::warn!(%err, "screencopy: scene render failed");
            for capture in captures {
                capture.frame.fail(CaptureFailureReason::Unknown);
            }
            continue; // Never reuse a partially rendered image/damage history.
        }
        for capture in captures {
            let damage = scene.changed_since(capture.history.as_ref());
            if damage.is_empty()
                && matches!(&capture.frame, CaptureFrame::Wlr(frame) if frame.with_damage)
            {
                pending.push(capture);
                continue; // Reconsider on the next scene change, without a polling timer.
            }
            match capture.frame {
                CaptureFrame::Ext(frame) => {
                    let mut repair = damage.clone();
                    repair.extend(frame.damage());
                    match scene.copy(renderer, &frame.buffer(), &repair) {
                        FillOutcome::Ok(_) => {
                            scene.delivered(capture.history.as_ref());
                            frame.success(Transform::Normal, damage, time);
                        }
                        FillOutcome::UnsupportedBuffer => {
                            frame.fail(CaptureFailureReason::BufferConstraints)
                        }
                        FillOutcome::RenderFailed => frame.fail(CaptureFailureReason::Unknown),
                    }
                }
                CaptureFrame::Wlr(frame) => {
                    // wlr provides no buffer age/damage: every supplied buffer
                    // must receive the whole image, but shared CPU pixels need
                    // GPU readback only for the newly damaged source region.
                    let full = Rectangle::from_size((capture.size.w, capture.size.h).into());
                    let outcome = match scene.copy(renderer, &frame.buffer, &[full]) {
                        FillOutcome::Ok(_) => {
                            scene.delivered(capture.history.as_ref());
                            FillOutcome::Ok(damage)
                        }
                        error => error,
                    };
                    frame.complete(outcome, time.into());
                }
                CaptureFrame::File { destination, token } => {
                    use crate::automation_dnd::{
                        AutomationOperation, AutomationStatus, record_and_publish,
                    };
                    match scene.rgba(renderer) {
                        Ok(pixels) => {
                            let size = capture.size;
                            std::thread::spawn(move || {
                                let status = match destination.write_rgba(
                                    size.w as u32,
                                    size.h as u32,
                                    &pixels,
                                ) {
                                    Ok(()) => AutomationStatus::Done,
                                    Err(err) => {
                                        tracing::warn!(%err, path = %destination.path().display(), "wm-screenshot failed");
                                        AutomationStatus::Failed
                                    }
                                };
                                record_and_publish(token, AutomationOperation::Screenshot, status);
                            });
                        }
                        Err(err) => {
                            tracing::warn!(%err, "wm-screenshot readback failed");
                            record_and_publish(
                                token,
                                AutomationOperation::Screenshot,
                                AutomationStatus::Failed,
                            );
                        }
                    }
                }
            }
        }
        cache.put(scene);
    }
}
