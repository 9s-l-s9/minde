// SPDX-License-Identifier: MIT

//! `wlr-screencopy-unstable-v1`: the legacy screen-capture protocol wf-recorder,
//! grim (older builds) and `xdg-desktop-portal-wlr` speak. Superseded by
//! `ext-image-copy-capture-v1` (see [`super::screencopy`]) but still what most
//! of the wlroots ecosystem -- notably the portal used for browser screen
//! sharing -- binds, so sway/river keep serving it and so do we.
//!
//! Smithay ships no server-side module for it, so the `GlobalDispatch`/
//! `Dispatch` impls are hand-written here (like [`super::gamma_control`]). The
//! actual pixel work is *not* duplicated: a queued frame lands in the shared
//! [`MindeState::pending_captures`] queue as a
//! [`CaptureFrame::Wlr`](super::screencopy::CaptureFrame::Wlr) and is satisfied
//! by [`super::screencopy::satisfy_output_captures`] at the next composite,
//! exactly like the ext protocol.
//!
//! Flow: `capture_output`/`capture_output_region` creates a frame; we resolve
//! the output, send a `buffer` event (shm), then on v3 a `linux_dmabuf` event
//! (udev render node) and `buffer_done`. The client attaches a matching buffer
//! and calls `copy`/`copy_with_damage`; we validate it and queue the frame.
//! After the next render we send `flags`, optional `damage`, and `ready` -- or
//! `failed` on any error.

use std::sync::Mutex;
use std::time::Duration;

use smithay::backend::allocator::Fourcc;
use smithay::output::{Output, WeakOutput};
use smithay::reexports::{
    wayland_protocols_wlr::screencopy::v1::server::{
        zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
        zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
    },
    wayland_server::{
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_shm},
    },
};
use smithay::utils::{Logical, Physical, Rectangle, Scale, Size};
use smithay::wayland::shm::{shm_format_to_fourcc, with_buffer_contents};

use super::screencopy::{CaptureFrame, FillOutcome, PendingCapture};
use crate::MindeState;

/// A wlr frame that has had a buffer attached (`copy`) and is waiting on the
/// next composite. Parked in [`MindeState::pending_captures`].
pub struct WlrPending {
    frame: ZwlrScreencopyFrameV1,
    pub buffer: WlBuffer,
    with_damage: bool,
}

impl WlrPending {
    /// Signals the client after the shared renderer filled (or failed to fill)
    /// the buffer: `flags` + optional `damage` + `ready` on success, else
    /// `failed`.
    pub fn complete(self, outcome: FillOutcome, time: Duration) {
        match outcome {
            FillOutcome::Ok(region) => {
                // Rendered upright (Transform::Normal), so no y-invert.
                self.frame.flags(zwlr_screencopy_frame_v1::Flags::empty());
                if self.with_damage && self.frame.version() >= 2 {
                    self.frame.damage(
                        region.loc.x.max(0) as u32,
                        region.loc.y.max(0) as u32,
                        region.size.w.max(0) as u32,
                        region.size.h.max(0) as u32,
                    );
                }
                let secs = time.as_secs();
                self.frame
                    .ready((secs >> 32) as u32, secs as u32, time.subsec_nanos());
            }
            FillOutcome::UnsupportedBuffer | FillOutcome::RenderFailed => self.frame.failed(),
        }
    }
}

/// Per-frame state: which output/region to capture and the buffer parameters
/// we advertised, used to validate the client's attached buffer. `Mutex`
/// because `copy` needs `&mut` to flip `used`, but dispatch hands us `&data`.
pub struct WlrFrameData {
    inner: Mutex<WlrFrameInner>,
}

struct WlrFrameInner {
    output: WeakOutput,
    /// Captured area in output-logical coords (full output for `capture_output`).
    region: Rectangle<i32, Logical>,
    /// Advertised destination buffer size (physical) and shm stride/format.
    size: Size<i32, Physical>,
    shm_stride: u32,
    shm_format: wl_shm::Format,
    overlay_cursor: bool,
    used: bool,
}

impl GlobalDispatch<ZwlrScreencopyManagerV1, ()> for MindeState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for MindeState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _manager: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_screencopy_manager_v1::Request;
        match request {
            Request::CaptureOutput {
                frame,
                overlay_cursor,
                output,
            } => {
                state.create_wlr_frame(frame, overlay_cursor != 0, &output, None, data_init);
            }
            Request::CaptureOutputRegion {
                frame,
                overlay_cursor,
                output,
                x,
                y,
                width,
                height,
            } => {
                let rect = Rectangle::new((x, y).into(), (width.max(0), height.max(0)).into());
                state.create_wlr_frame(frame, overlay_cursor != 0, &output, Some(rect), data_init);
            }
            Request::Destroy => {}
            _ => unreachable!(),
        }
    }
}

impl MindeState {
    /// Resolves the output, clips the requested region to it, and sends the
    /// buffer-constraint events. A frame that cannot be captured (no mapped
    /// output, empty region) is still initialized and then `failed`.
    fn create_wlr_frame(
        &mut self,
        id: New<ZwlrScreencopyFrameV1>,
        overlay_cursor: bool,
        wl_output: &WlOutput,
        region: Option<Rectangle<i32, Logical>>,
        data_init: &mut DataInit<'_, MindeState>,
    ) {
        let output = Output::from_resource(wl_output)
            .filter(|o| self.space.outputs().any(|mapped| mapped == o));
        let Some(output) = output else {
            // No mapped output: init the resource (mandatory) then fail it.
            let frame = data_init.init(id, placeholder_frame_data());
            frame.failed();
            return;
        };

        // Full-output geometry in logical coords; region is clipped to it.
        let output_size: Size<i32, Logical> = self
            .space
            .output_geometry(&output)
            .map(|g| g.size)
            .or_else(|| output.current_mode().map(|m| (m.size.w, m.size.h).into()))
            .unwrap_or_default();
        let full = Rectangle::from_size(output_size);
        let region = match region {
            Some(r) => match r.intersection(full) {
                Some(clipped) if !clipped.is_empty() => clipped,
                _ => {
                    let frame = data_init.init(id, placeholder_frame_data());
                    frame.failed();
                    return;
                }
            },
            None => full,
        };

        let scale: Scale<f64> = output.current_scale().fractional_scale().into();
        let phys: Size<i32, Physical> = region.size.to_physical_precise_round(scale);
        // Guard against a degenerate (zero-area) physical size.
        if phys.w <= 0 || phys.h <= 0 {
            let frame = data_init.init(id, placeholder_frame_data());
            frame.failed();
            return;
        }
        let shm_format = wl_shm::Format::Xrgb8888;
        let shm_stride = phys.w as u32 * 4;

        let frame = data_init.init(
            id,
            WlrFrameData {
                inner: Mutex::new(WlrFrameInner {
                    output: output.downgrade(),
                    region,
                    size: phys,
                    shm_stride,
                    shm_format,
                    overlay_cursor,
                    used: false,
                }),
            },
        );

        // shm buffer parameters (always supported).
        frame.buffer(shm_format, phys.w as u32, phys.h as u32, shm_stride);

        // v3: advertise a dmabuf format (udev render node only) and close the
        // enumeration with buffer_done.
        if frame.version() >= 3 {
            if self.dmabuf_capture_constraints().is_some() {
                frame.linux_dmabuf(Fourcc::Xrgb8888 as u32, phys.w as u32, phys.h as u32);
            }
            frame.buffer_done();
        }
    }
}

/// User data for a frame that could never capture (unresolved output / empty
/// region). It is `failed` immediately; its handlers are inert.
fn placeholder_frame_data() -> WlrFrameData {
    WlrFrameData {
        inner: Mutex::new(WlrFrameInner {
            output: WeakOutput::default(),
            region: Rectangle::default(),
            size: (0, 0).into(),
            shm_stride: 0,
            shm_format: wl_shm::Format::Xrgb8888,
            overlay_cursor: false,
            used: true, // reject any copy on a placeholder
        }),
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, WlrFrameData> for MindeState {
    fn request(
        state: &mut Self,
        _client: &Client,
        frame: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        data: &WlrFrameData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_screencopy_frame_v1::Request;
        let (buffer, with_damage) = match request {
            Request::Copy { buffer } => (buffer, false),
            Request::CopyWithDamage { buffer } => (buffer, true),
            Request::Destroy => return,
            _ => unreachable!(),
        };

        let mut inner = data.inner.lock().unwrap();
        if inner.used {
            frame.post_error(
                zwlr_screencopy_frame_v1::Error::AlreadyUsed,
                "frame was already used to copy a buffer",
            );
            return;
        }
        inner.used = true;

        // Validate the attached buffer against what we advertised.
        if let Err(reason) = validate_buffer(&buffer, &inner) {
            frame.post_error(zwlr_screencopy_frame_v1::Error::InvalidBuffer, reason);
            return;
        }

        let Some(output) = inner.output.upgrade() else {
            frame.failed();
            return;
        };
        if !state.space.outputs().any(|o| o == &output) {
            frame.failed();
            return;
        }

        state.pending_captures.push(PendingCapture {
            output,
            frame: CaptureFrame::Wlr(WlrPending {
                frame: frame.clone(),
                buffer,
                with_damage,
            }),
            draw_cursor: inner.overlay_cursor,
            origin: inner.region.loc,
            size: inner.size,
        });
    }
}

/// Checks a client's buffer matches the advertised size (and, for shm, stride
/// and a supported format). Returns the protocol error string on mismatch.
fn validate_buffer(buffer: &WlBuffer, inner: &WlrFrameInner) -> Result<(), &'static str> {
    match smithay::backend::renderer::buffer_type(buffer) {
        Some(smithay::backend::renderer::BufferType::Shm) => {
            with_buffer_contents(buffer, |_ptr, _len, data| {
                if data.width != inner.size.w || data.height != inner.size.h {
                    return Err("shm buffer has the wrong dimensions");
                }
                if data.stride != inner.shm_stride as i32 {
                    return Err("shm buffer has the wrong stride");
                }
                if shm_format_to_fourcc(data.format).is_none() {
                    return Err("shm buffer has an unsupported format");
                }
                let _ = inner.shm_format;
                Ok(())
            })
            .map_err(|_| "attached buffer is not a valid shm buffer")?
        }
        Some(smithay::backend::renderer::BufferType::Dma) => {
            let dmabuf =
                smithay::wayland::dmabuf::get_dmabuf(buffer).map_err(|_| "invalid dmabuf")?;
            use smithay::backend::allocator::Buffer as _;
            if dmabuf.width() != inner.size.w as u32 || dmabuf.height() != inner.size.h as u32 {
                return Err("dmabuf has the wrong dimensions");
            }
            Ok(())
        }
        _ => Err("attached buffer is neither shm nor dmabuf"),
    }
}

/// Creates the manager global (version 3). Registered for both backends;
/// dmabuf constraints are only advertised where a render node exists (udev).
pub fn init_wlr_screencopy_manager(
    dh: &DisplayHandle,
) -> smithay::reexports::wayland_server::backend::GlobalId {
    dh.create_global::<MindeState, ZwlrScreencopyManagerV1, ()>(3, ())
}
