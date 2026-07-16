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
//! [`satisfy_output_captures`], which re-composites the same scene into each
//! pending buffer and signals `ready`.
//!
//! Damage handling is conservative: every capture reports full-buffer damage.
//! Correctness over efficiency for this first version.

use std::time::Duration;

use smithay::{
    backend::{
        allocator::{Fourcc, dmabuf::Dmabuf},
        renderer::{
            Bind, ExportMem, ImportAll, ImportMem, Offscreen, Renderer,
            damage::OutputDamageTracker, element::AsRenderElements, gles::GlesRenderbuffer,
        },
    },
    desktop::{LayerSurface, Space, Window, layer_map_for_output},
    output::Output,
    reexports::wayland_server::protocol::wl_buffer::WlBuffer,
    utils::{Buffer as BufferCoords, Logical, Physical, Point, Rectangle, Scale, Size, Transform},
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

/// A capture frame queued against an output, awaiting the next render.
pub struct PendingCapture {
    pub output: Output,
    pub frame: Frame,
    pub draw_cursor: bool,
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
        let mode = output.current_mode()?;
        let size: Size<i32, BufferCoords> = (mode.size.w, mode.size.h).into();

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
        if !self.space.outputs().any(|o| o == &output) {
            frame.fail(CaptureFailureReason::Unknown);
            return;
        }
        self.pending_captures.push(PendingCapture {
            output,
            frame,
            draw_cursor: session.draw_cursor(),
        });
    }

    fn session_destroyed(&mut self, session: SessionRef) {
        self.capture_sessions.retain(|s| *s != session);
    }
}

// ---------------------------------------------------------------------------
// Scene assembly + buffer rendering
// ---------------------------------------------------------------------------

/// Builds the render-element list for one output's capture, mirroring the
/// udev on-screen assembly: cursor, message overlay, positioned overlays,
/// upper layer surfaces, focus border, windows (front-to-back), then lower
/// layer surfaces. Kept renderer-generic so both backends can reuse it.
#[allow(clippy::too_many_arguments)]
fn output_scene_elements<R>(
    renderer: &mut R,
    output: &Output,
    output_geo: Rectangle<i32, Logical>,
    scale: Scale<f64>,
    int_scale: i32,
    space: &Space<Window>,
    cursor: Option<(&mut CursorState, Point<f64, Logical>)>,
    message: Option<&MessageState>,
    overlays: &[(Point<i32, Logical>, MessageState)],
    focus: Option<Rectangle<i32, Logical>>,
    border_color: [f32; 4],
) -> Vec<MindeRenderElements<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
{
    let mut custom: Vec<MindeRenderElements<R>> = Vec::new();

    // Cursor at the current pointer location (only when this capture asked
    // for cursors and the pointer is over this output).
    if let Some((cursor_state, pointer_location)) = cursor
        && output_geo.to_f64().contains(pointer_location)
    {
        let hotspot = cursor_state.hotspot();
        let cursor_pos = pointer_location - output_geo.loc.to_f64();
        let cursor_phys = (cursor_pos - hotspot.to_f64())
            .to_physical(scale)
            .to_i32_round();
        custom.extend(cursor_state.render_elements(renderer, cursor_phys, scale));
    }

    // Centered message overlay.
    if let Some(msg) = message
        && let Some(elem) = crate::render::message_element(
            renderer,
            msg,
            (output_geo.size.w, output_geo.size.h),
            int_scale,
        )
    {
        custom.push(elem);
    }

    // Positioned overlays landing on this output.
    for (loc, msg) in overlays.iter().filter(|(l, _)| output_geo.contains(*l)) {
        if let Some(elem) =
            crate::render::overlay_element(renderer, msg, *loc - output_geo.loc, int_scale)
        {
            custom.push(elem);
        }
    }

    // Layer surfaces, split into upper (above windows) and lower (below).
    let layer_map = layer_map_for_output(output);
    let (lower, upper): (Vec<&LayerSurface>, Vec<_>) = layer_map
        .layers()
        .partition(|s| matches!(s.layer(), WlrLayer::Background | WlrLayer::Bottom));
    let layer_elements = |surfaces: &[&LayerSurface], renderer: &mut R| {
        let mut out: Vec<MindeRenderElements<R>> = Vec::new();
        for surface in surfaces {
            let loc = layer_map
                .layer_geometry(surface)
                .map(|geo| geo.loc)
                .unwrap_or_default();
            out.extend(AsRenderElements::<R>::render_elements::<
                MindeRenderElements<R>,
            >(
                *surface,
                renderer,
                loc.to_physical_precise_round(scale),
                scale,
                1.0,
            ));
        }
        out
    };
    custom.extend(layer_elements(&upper, renderer));

    // Focus border around the selected frame.
    if let Some(geo) = focus
        && geo.overlaps(output_geo)
    {
        let mut local = geo;
        local.loc -= output_geo.loc;
        let mut border = BorderBuffers::default();
        custom.extend(border.elements(local, int_scale, border_color));
    }

    // Window surfaces, front-to-back (space yields back-to-front).
    let mut all: Vec<MindeRenderElements<R>> = custom;
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
        let phys_loc =
            (loc - window.geometry().loc - output_geo.loc).to_physical_precise_round(scale);
        all.extend(AsRenderElements::<R>::render_elements(
            window, renderer, phys_loc, scale, 1.0,
        ));
    }
    // Background/bottom layers under everything.
    all.extend(layer_elements(&lower, renderer));
    all
}

/// Re-composites the given elements into a client capture buffer (shm or
/// dmabuf) and completes the frame. Rendered upright (`Transform::Normal`)
/// and reported as such, so the captured image is the logical desktop
/// regardless of any physical output transform.
fn render_capture_frame<R>(
    renderer: &mut R,
    elements: &[MindeRenderElements<R>],
    frame: Frame,
    size: Size<i32, Physical>,
    scale: Scale<f64>,
    time: Duration,
) where
    R: Renderer + ImportAll + ImportMem + ExportMem + Offscreen<GlesRenderbuffer> + Bind<Dmabuf>,
    R::TextureId: Send + Clone + 'static,
    R::Error: Send + Sync + 'static,
{
    let buffer = frame.buffer();
    let buffer_size: Size<i32, BufferCoords> = (size.w, size.h).into();
    let region = Rectangle::from_size(buffer_size);
    let mut tracker = OutputDamageTracker::new(size, scale, Transform::Normal);
    let clear = [0.1, 0.1, 0.1, 1.0];

    match smithay::backend::renderer::buffer_type(&buffer) {
        Some(smithay::backend::renderer::BufferType::Dma) => {
            match render_into_dmabuf(renderer, elements, &buffer, &mut tracker, clear) {
                Ok(()) => frame.success(Transform::Normal, vec![region], time),
                Err(err) => {
                    tracing::warn!(%err, "screencopy: dmabuf capture failed");
                    frame.fail(CaptureFailureReason::Unknown);
                }
            }
        }
        Some(smithay::backend::renderer::BufferType::Shm) => {
            match render_into_shm(
                renderer,
                elements,
                &buffer,
                &mut tracker,
                clear,
                buffer_size,
                region,
            ) {
                Ok(()) => frame.success(Transform::Normal, vec![region], time),
                Err(err) => {
                    tracing::warn!(%err, "screencopy: shm capture failed");
                    frame.fail(CaptureFailureReason::Unknown);
                }
            }
        }
        _ => frame.fail(CaptureFailureReason::BufferConstraints),
    }
}

fn render_into_dmabuf<R>(
    renderer: &mut R,
    elements: &[MindeRenderElements<R>],
    buffer: &WlBuffer,
    tracker: &mut OutputDamageTracker,
    clear: [f32; 4],
) -> Result<(), Box<dyn std::error::Error>>
where
    R: Renderer + ImportAll + ImportMem + Bind<Dmabuf>,
    R::TextureId: Send + Clone + 'static,
    R::Error: Send + Sync + 'static,
{
    let mut dmabuf = smithay::wayland::dmabuf::get_dmabuf(buffer)
        .map_err(|_| "attached buffer is not a dmabuf")?
        .clone();
    let mut framebuffer = renderer.bind(&mut dmabuf)?;
    tracker.render_output(renderer, &mut framebuffer, 0, elements, clear)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_into_shm<R>(
    renderer: &mut R,
    elements: &[MindeRenderElements<R>],
    buffer: &WlBuffer,
    tracker: &mut OutputDamageTracker,
    clear: [f32; 4],
    size: Size<i32, BufferCoords>,
    region: Rectangle<i32, BufferCoords>,
) -> Result<(), Box<dyn std::error::Error>>
where
    R: Renderer + ImportAll + ImportMem + ExportMem + Offscreen<GlesRenderbuffer>,
    R::TextureId: Send + Clone + 'static,
    R::Error: Send + Sync + 'static,
{
    // Copy back in whatever pixel format the client's shm buffer uses.
    let dst_format = with_buffer_contents_mut(buffer, |_, _, data| data.format)
        .map_err(|err| format!("shm buffer access: {err:?}"))?;
    let fourcc = shm_format_to_fourcc(dst_format).ok_or("unsupported shm format")?;

    // Render into an offscreen GL renderbuffer, then read it back and copy
    // into the client's shm mapping (GlesRenderer cannot scan out to shm).
    let mut offscreen = renderer.create_buffer(Fourcc::Argb8888, size)?;
    let mut framebuffer = renderer.bind(&mut offscreen)?;
    tracker.render_output(renderer, &mut framebuffer, 0, elements, clear)?;
    let mapping = renderer.copy_framebuffer(&framebuffer, region, fourcc)?;
    drop(framebuffer);
    let pixels = renderer.map_texture(&mapping)?.to_vec();

    with_buffer_contents_mut(buffer, |ptr, len, data| {
        let bpp = 4usize;
        let src_stride = size.w as usize * bpp;
        let dst_stride = data.stride as usize;
        let rows = (data.height as usize).min(size.h as usize);
        let copy_w = src_stride.min(dst_stride);
        for row in 0..rows {
            let src_off = row * src_stride;
            let dst_off = data.offset as usize + row * dst_stride;
            if src_off + copy_w > pixels.len() || dst_off + copy_w > len {
                break;
            }
            // Safety: bounds checked above; ptr is the client pool mapping.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    pixels.as_ptr().add(src_off),
                    ptr.add(dst_off),
                    copy_w,
                );
            }
        }
    })
    .map_err(|err| format!("shm buffer access: {err:?}"))?;
    Ok(())
}

/// Satisfies every capture frame queued for `output`, re-compositing the
/// scene into each pending buffer. Called from the render loops after the
/// on-screen frame is drawn. All arguments are disjoint fields of
/// [`MindeState`] so the udev caller can hand them out while the renderer
/// borrows `udev_data`.
#[allow(clippy::too_many_arguments)]
pub fn satisfy_output_captures<R>(
    renderer: &mut R,
    output: &Output,
    output_geo: Rectangle<i32, Logical>,
    scale: Scale<f64>,
    int_scale: i32,
    size: Size<i32, Physical>,
    time: Duration,
    pending: &mut Vec<PendingCapture>,
    space: &Space<Window>,
    cursor_state: &mut CursorState,
    pointer_location: Point<f64, Logical>,
    message: Option<&MessageState>,
    overlays: &[(Point<i32, Logical>, MessageState)],
    focus: Option<Rectangle<i32, Logical>>,
    border_color: [f32; 4],
) where
    R: Renderer + ImportAll + ImportMem + ExportMem + Offscreen<GlesRenderbuffer> + Bind<Dmabuf>,
    R::TextureId: Send + Clone + 'static,
    R::Error: Send + Sync + 'static,
{
    // Pull out the frames for this output (Frame is not Clone; move them).
    let mut mine: Vec<PendingCapture> = Vec::new();
    let mut i = 0;
    while i < pending.len() {
        if &pending[i].output == output {
            mine.push(pending.remove(i));
        } else {
            i += 1;
        }
    }
    if mine.is_empty() {
        return;
    }

    for capture in mine {
        let cursor = if capture.draw_cursor {
            Some((&mut *cursor_state, pointer_location))
        } else {
            None
        };
        let elements = output_scene_elements(
            renderer,
            output,
            output_geo,
            scale,
            int_scale,
            space,
            cursor,
            message,
            overlays,
            focus,
            border_color,
        );
        render_capture_frame(renderer, &elements, capture.frame, size, scale, time);
    }
}
