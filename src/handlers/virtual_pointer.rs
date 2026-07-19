// SPDX-License-Identifier: MIT

//! `zwlr_virtual_pointer_manager_v1` / `zwlr_virtual_pointer_v1` (v2):
//! wtype/ydotool/wlrctl-style pointer automation and accessibility tools.
//!
//! Smithay ships a server module for the virtual *keyboard* but not for the
//! wlroots virtual *pointer*, so the `GlobalDispatch`/`Dispatch` impls are
//! hand-written here on the `wayland-protocols-wlr` bindings, following the
//! same pattern as [`super::gamma_control`], [`super::wlr_screencopy`] and
//! [`super::output_management`]. The userdata types deliberately do not
//! implement `Dispatch2`, so the crate-wide `delegate_dispatch2!` blanket does
//! not collide with the explicit impls below.
//!
//! # Integration with the real input path
//!
//! Virtual pointer requests are translated into the *exact same* pointer
//! processing a physical pointer uses: [`MindeState::pointer_relative_motion`],
//! [`MindeState::pointer_absolute_motion`], [`MindeState::pointer_button_event`]
//! and [`MindeState::pointer_axis_frame`] in `src/input.rs`. That means
//! virtual input honestly respects:
//!
//! - **pointer constraints** (a locked pointer parks, a confined pointer
//!   clamps) -- the motion helpers call `constrain_pointer` like real motion;
//! - **idle activity** -- every request resets the idle-notify timers via
//!   [`MindeState::notify_idle_activity`], so automation keeps the seat
//!   "active" exactly like a physical device;
//! - **the session lock** -- each request is gated by [`Self::virtual_input_gate`],
//!   which drops pointer events while `self.locked` is set, mirroring the
//!   `process_input_event` locked gate. Virtual pointer input can never reach
//!   a regular client while the screen is locked.
//!
//! No Scheme surface: like the physical pointer, virtual pointer motion is not
//! compositor policy.

use std::sync::Mutex;

use smithay::backend::input::{Axis, AxisSource, ButtonState};
use smithay::input::pointer::AxisFrame;
use smithay::reexports::{
    wayland_protocols_wlr::virtual_pointer::v1::server::{
        zwlr_virtual_pointer_manager_v1::{self, ZwlrVirtualPointerManagerV1},
        zwlr_virtual_pointer_v1::{self, ZwlrVirtualPointerV1},
    },
    wayland_server::{
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, WEnum, backend::GlobalId,
        protocol::wl_pointer,
    },
};
use smithay::utils::Point;

use crate::state::MindeState;

/// Registers the `zwlr_virtual_pointer_manager_v1` global (v2). The returned
/// id is kept alive by the display; callers discard it like the other
/// hand-rolled wlr globals.
pub fn init_virtual_pointer_manager(display: &DisplayHandle) -> GlobalId {
    display.create_global::<MindeState, ZwlrVirtualPointerManagerV1, _>(2, ())
}

/// Per-virtual-pointer axis (scroll) accumulator. A client sends
/// `axis`/`axis_source`/`axis_discrete`/`axis_stop` and then `frame`; we
/// collect the pieces and flush one [`AxisFrame`] on `frame`, exactly as the
/// real axis path builds one frame per libinput axis event.
#[derive(Default)]
struct PendingAxis {
    source: Option<AxisSource>,
    time: u32,
    horizontal: f64,
    vertical: f64,
    horizontal_discrete: Option<i32>,
    vertical_discrete: Option<i32>,
    horizontal_stop: bool,
    vertical_stop: bool,
    dirty: bool,
}

/// Userdata for a `zwlr_virtual_pointer_v1` object.
pub struct VirtualPointerData {
    pending: Mutex<PendingAxis>,
}

impl GlobalDispatch<ZwlrVirtualPointerManagerV1, ()> for MindeState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrVirtualPointerManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwlrVirtualPointerManagerV1, ()> for MindeState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _manager: &ZwlrVirtualPointerManagerV1,
        request: zwlr_virtual_pointer_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwlr_virtual_pointer_manager_v1::Request::CreateVirtualPointer { id, .. }
            | zwlr_virtual_pointer_manager_v1::Request::CreateVirtualPointerWithOutput {
                id, ..
            } => {
                // We only run a single seat; the seat/output hints are ignored
                // and the pointer drives the one compositor pointer.
                data_init.init(
                    id,
                    VirtualPointerData {
                        pending: Mutex::new(PendingAxis::default()),
                    },
                );
            }
            zwlr_virtual_pointer_manager_v1::Request::Destroy => {}
            _ => unreachable!(),
        }
    }
}

impl Dispatch<ZwlrVirtualPointerV1, VirtualPointerData> for MindeState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _pointer: &ZwlrVirtualPointerV1,
        request: zwlr_virtual_pointer_v1::Request,
        data: &VirtualPointerData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwlr_virtual_pointer_v1::Request::Motion { time, dx, dy } => {
                if !state.virtual_input_gate() {
                    return;
                }
                let delta: Point<f64, _> = (dx, dy).into();
                // No unaccelerated delta from a virtual device; report the
                // same value (as winit does for synthesized relative motion).
                state.pointer_relative_motion(delta, delta, time, (time as u64) * 1000);
            }
            zwlr_virtual_pointer_v1::Request::MotionAbsolute {
                time,
                x,
                y,
                x_extent,
                y_extent,
            } => {
                if !state.virtual_input_gate() {
                    return;
                }
                if x_extent == 0 || y_extent == 0 {
                    return;
                }
                // Map the client's abstract extent onto the first output's
                // geometry (the same output the real absolute path uses).
                let Some(output) = state.space.outputs().next() else {
                    return;
                };
                let Some(geo) = state.space.output_geometry(output) else {
                    return;
                };
                let px = geo.loc.x as f64 + (x as f64 / x_extent as f64) * geo.size.w as f64;
                let py = geo.loc.y as f64 + (y as f64 / y_extent as f64) * geo.size.h as f64;
                state.pointer_absolute_motion((px, py).into(), time, (time as u64) * 1000);
            }
            zwlr_virtual_pointer_v1::Request::Button {
                time,
                button,
                state: btn_state,
            } => {
                if !state.virtual_input_gate() {
                    return;
                }
                let pressed = matches!(btn_state, WEnum::Value(wl_pointer::ButtonState::Pressed));
                let button_state = if pressed {
                    ButtonState::Pressed
                } else {
                    ButtonState::Released
                };
                state.pointer_button_event(button, button_state, time);
            }
            zwlr_virtual_pointer_v1::Request::Axis { time, axis, value } => {
                if let Some(axis) = wl_axis(axis) {
                    let mut p = data.pending.lock().unwrap();
                    p.time = time;
                    p.dirty = true;
                    match axis {
                        Axis::Horizontal => p.horizontal += value,
                        Axis::Vertical => p.vertical += value,
                    }
                }
            }
            zwlr_virtual_pointer_v1::Request::AxisSource { axis_source } => {
                if let WEnum::Value(src) = axis_source {
                    let source = match src {
                        wl_pointer::AxisSource::Wheel => AxisSource::Wheel,
                        wl_pointer::AxisSource::Finger => AxisSource::Finger,
                        wl_pointer::AxisSource::Continuous => AxisSource::Continuous,
                        wl_pointer::AxisSource::WheelTilt => AxisSource::WheelTilt,
                        _ => AxisSource::Wheel,
                    };
                    let mut p = data.pending.lock().unwrap();
                    p.source = Some(source);
                    p.dirty = true;
                }
            }
            zwlr_virtual_pointer_v1::Request::AxisStop { time, axis } => {
                if let Some(axis) = wl_axis(axis) {
                    let mut p = data.pending.lock().unwrap();
                    p.time = time;
                    p.dirty = true;
                    match axis {
                        Axis::Horizontal => p.horizontal_stop = true,
                        Axis::Vertical => p.vertical_stop = true,
                    }
                }
            }
            zwlr_virtual_pointer_v1::Request::AxisDiscrete {
                time,
                axis,
                value,
                discrete,
            } => {
                if let Some(axis) = wl_axis(axis) {
                    let mut p = data.pending.lock().unwrap();
                    p.time = time;
                    p.dirty = true;
                    match axis {
                        Axis::Horizontal => {
                            p.horizontal += value;
                            p.horizontal_discrete = Some(discrete);
                        }
                        Axis::Vertical => {
                            p.vertical += value;
                            p.vertical_discrete = Some(discrete);
                        }
                    }
                }
            }
            zwlr_virtual_pointer_v1::Request::Frame => {
                // Motion and button requests already framed themselves; only a
                // pending axis batch needs flushing here.
                let pending = {
                    let mut p = data.pending.lock().unwrap();
                    if !p.dirty {
                        return;
                    }
                    std::mem::take(&mut *p)
                };
                if !state.virtual_input_gate() {
                    return;
                }
                let mut frame = AxisFrame::new(pending.time)
                    .source(pending.source.unwrap_or(AxisSource::Wheel));
                if pending.horizontal != 0.0 {
                    frame = frame.value(Axis::Horizontal, pending.horizontal);
                }
                if pending.vertical != 0.0 {
                    frame = frame.value(Axis::Vertical, pending.vertical);
                }
                if let Some(d) = pending.horizontal_discrete {
                    frame = frame.v120(Axis::Horizontal, d * 120);
                }
                if let Some(d) = pending.vertical_discrete {
                    frame = frame.v120(Axis::Vertical, d * 120);
                }
                if pending.horizontal_stop {
                    frame = frame.stop(Axis::Horizontal);
                }
                if pending.vertical_stop {
                    frame = frame.stop(Axis::Vertical);
                }
                state.pointer_axis_frame(frame);
            }
            zwlr_virtual_pointer_v1::Request::Destroy => {}
            _ => unreachable!(),
        }
    }
}

/// Translate a `wl_pointer.axis` enum value into Smithay's [`Axis`].
fn wl_axis(axis: WEnum<wl_pointer::Axis>) -> Option<Axis> {
    match axis {
        WEnum::Value(wl_pointer::Axis::HorizontalScroll) => Some(Axis::Horizontal),
        WEnum::Value(wl_pointer::Axis::VerticalScroll) => Some(Axis::Vertical),
        _ => None,
    }
}

impl MindeState {
    /// Note user activity (resetting idle timers) and report whether virtual
    /// pointer input is currently allowed to flow. Drops events while the
    /// session is locked, mirroring the `process_input_event` locked gate so
    /// virtual input can never reach a regular client on a locked screen.
    fn virtual_input_gate(&mut self) -> bool {
        crate::guile::note_activity();
        self.notify_idle_activity();
        !self.locked
    }
}
