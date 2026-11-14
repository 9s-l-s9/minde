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
//!   which drops virtual pointer events while `self.locked` is set. Physical
//!   input targets lock surfaces, while virtual pointer automation stays blocked.
//!
//! No Scheme surface: like the physical pointer, virtual pointer motion is not
//! compositor policy.

use std::sync::Mutex;

use smithay::backend::input::{Axis, AxisSource, ButtonState};
use smithay::desktop::{Space, Window};
use smithay::input::pointer::AxisFrame;
use smithay::output::Output;
use smithay::reexports::{
    wayland_protocols_wlr::virtual_pointer::v1::server::{
        zwlr_virtual_pointer_manager_v1::{self, ZwlrVirtualPointerManagerV1},
        zwlr_virtual_pointer_v1::{self, ZwlrVirtualPointerV1},
    },
    wayland_server::{
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
        backend::{ClientId, GlobalId},
        protocol::wl_pointer,
    },
};
use smithay::utils::{Logical, Point};

use crate::input::PointerButtonSource;
use crate::state::{MindeState, clamp_point_to_rectangles};

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
#[derive(Default)]
pub struct VirtualPointerData {
    pending: Mutex<PendingAxis>,
    output: Option<Output>,
}

impl VirtualPointerData {
    fn absolute_position(
        &self,
        space: &Space<Window>,
        x: u32,
        y: u32,
        x_extent: u32,
        y_extent: u32,
    ) -> Option<Point<f64, Logical>> {
        if x_extent == 0 || y_extent == 0 {
            return None;
        }
        // Resolve current geometry on every event: the requested output can
        // move, scale, rotate, disappear or be re-enabled during this device's
        // lifetime. A disabled/removed target falls back to a mapped output.
        let geo = self
            .output
            .as_ref()
            .and_then(|output| space.output_geometry(output))
            .or_else(|| {
                space
                    .outputs()
                    .find_map(|output| space.output_geometry(output))
            })?;
        let position = geo.loc.to_f64()
            + Point::from((
                x.min(x_extent) as f64 / x_extent as f64 * geo.size.w as f64,
                y.min(y_extent) as f64 / y_extent as f64 * geo.size.h as f64,
            ));
        // Keep the inclusive protocol extent inside the output's exclusive
        // far edge, even when another output begins exactly at that edge.
        Some(clamp_point_to_rectangles(position, [geo]))
    }
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
        let (id, output) = match request {
            zwlr_virtual_pointer_manager_v1::Request::CreateVirtualPointer { id, .. } => (id, None),
            zwlr_virtual_pointer_manager_v1::Request::CreateVirtualPointerWithOutput {
                id,
                output,
                ..
            } => (id, output.as_ref().and_then(Output::from_resource)),
            zwlr_virtual_pointer_manager_v1::Request::Destroy => return,
            _ => unreachable!(),
        };
        // Minde has one seat. Retain the output identity independently of the
        // client's wl_output resource, which it may release after this request.
        data_init.init(
            id,
            VirtualPointerData {
                output,
                ..Default::default()
            },
        );
    }
}

impl Dispatch<ZwlrVirtualPointerV1, VirtualPointerData> for MindeState {
    fn request(
        state: &mut Self,
        _client: &Client,
        pointer: &ZwlrVirtualPointerV1,
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
                let Some(position) = data.absolute_position(&state.space, x, y, x_extent, y_extent)
                else {
                    return;
                };
                state.pointer_absolute_motion(
                    crate::input::AbsolutePointerSource::Virtual(pointer.id()),
                    position,
                    time,
                    (time as u64) * 1000,
                );
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
                state.pointer_button_event(
                    PointerButtonSource::Virtual(pointer.id()),
                    button,
                    button_state,
                    time,
                );
            }
            zwlr_virtual_pointer_v1::Request::Axis { time, axis, value } => {
                if let Some(axis) = wl_axis(pointer, axis) {
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
                let source = match axis_source {
                    WEnum::Value(wl_pointer::AxisSource::Wheel) => AxisSource::Wheel,
                    WEnum::Value(wl_pointer::AxisSource::Finger) => AxisSource::Finger,
                    WEnum::Value(wl_pointer::AxisSource::Continuous) => AxisSource::Continuous,
                    WEnum::Value(wl_pointer::AxisSource::WheelTilt) => AxisSource::WheelTilt,
                    _ => {
                        pointer.post_error(
                            zwlr_virtual_pointer_v1::Error::InvalidAxisSource,
                            "invalid scroll source",
                        );
                        return;
                    }
                };
                let mut p = data.pending.lock().unwrap();
                p.source = Some(source);
                p.dirty = true;
            }
            zwlr_virtual_pointer_v1::Request::AxisStop { time, axis } => {
                if let Some(axis) = wl_axis(pointer, axis) {
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
                if let Some(axis) = wl_axis(pointer, axis) {
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
                    frame = frame.v120(Axis::Horizontal, discrete_v120(d));
                }
                if let Some(d) = pending.vertical_discrete {
                    frame = frame.v120(Axis::Vertical, discrete_v120(d));
                }
                if pending.horizontal_stop {
                    frame = frame.stop(Axis::Horizontal);
                }
                if pending.vertical_stop {
                    frame = frame.stop(Axis::Vertical);
                }
                state.pointer_axis_frame(frame);
            }
            zwlr_virtual_pointer_v1::Request::Destroy => {
                state.release_pointer_buttons(|source| {
                    *source == PointerButtonSource::Virtual(pointer.id())
                });
            }
            _ => unreachable!(),
        }
    }

    fn destroyed(
        state: &mut Self,
        _client: ClientId,
        pointer: &ZwlrVirtualPointerV1,
        _data: &VirtualPointerData,
    ) {
        state.release_pointer_buttons(|source| {
            *source == PointerButtonSource::Virtual(pointer.id())
        });
    }
}

/// Keep whole wheel steps representable, including the partial step Smithay
/// retains for pre-v8 pointers. Its accumulator adds up to 119 before `abs()`.
fn discrete_v120(steps: i32) -> i32 {
    const MAX_STEPS: i32 = (i32::MAX - 119) / 120;
    steps.clamp(-MAX_STEPS, MAX_STEPS) * 120
}

/// Translate a `wl_pointer.axis` enum value into Smithay's [`Axis`].
fn wl_axis(pointer: &ZwlrVirtualPointerV1, axis: WEnum<wl_pointer::Axis>) -> Option<Axis> {
    match axis {
        WEnum::Value(wl_pointer::Axis::HorizontalScroll) => Some(Axis::Horizontal),
        WEnum::Value(wl_pointer::Axis::VerticalScroll) => Some(Axis::Vertical),
        _ => {
            pointer.post_error(
                zwlr_virtual_pointer_v1::Error::InvalidAxis,
                "invalid scroll axis",
            );
            None
        }
    }
}

impl MindeState {
    /// Note user activity (resetting idle timers) and report whether virtual
    /// pointer input is currently allowed to flow. Drops events while the
    /// session is locked; physical input uses separate lock-surface routing.
    fn virtual_input_gate(&mut self) -> bool {
        crate::guile::note_activity();
        self.notify_idle_activity();
        !self.locked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::output::{Mode, PhysicalProperties, Scale, Subpixel};
    use smithay::utils::Transform;

    fn output(name: &str, size: (i32, i32)) -> Output {
        let output = Output::new(
            name.into(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "test".into(),
                model: "test".into(),
                serial_number: name.into(),
            },
        );
        output.change_current_state(
            Some(Mode {
                size: size.into(),
                refresh: 60_000,
            }),
            None,
            None,
            None,
        );
        output
    }

    #[test]
    fn absolute_mapping_keeps_the_requested_output_through_layout_changes() {
        let first = output("first", (1920, 1080));
        let target = output("target", (800, 600));
        let mut space = Space::default();
        space.map_output(&first, (0, 0));
        space.map_output(&target, (-800, 200));
        let pointer = VirtualPointerData {
            output: Some(target.clone()),
            ..Default::default()
        };
        let middle = |space: &Space<Window>| pointer.absolute_position(space, 50, 50, 100, 100);
        assert_eq!(middle(&space), Some((-400.0, 500.0).into()));
        assert_eq!(
            VirtualPointerData::default().absolute_position(&space, 50, 50, 100, 100),
            Some((960.0, 540.0).into())
        );

        target.change_current_state(None, Some(Transform::_90), Some(Scale::Integer(2)), None);
        space.map_output(&target, (2000, -400));
        assert_eq!(middle(&space), Some((2150.0, -200.0).into()));
        // Unmapping models disable/removal; retaining the Output identity
        // makes re-enabling it recover the original association.
        space.unmap_output(&target);
        assert_eq!(middle(&space), Some((960.0, 540.0).into()));
        space.map_output(&target, (2000, -400));
        assert_eq!(middle(&space), Some((2150.0, -200.0).into()));
        space.unmap_output(&first);
        space.unmap_output(&target);
        assert_eq!(middle(&space), None);
    }

    #[test]
    fn virtual_extents_stay_inside_the_target_including_shared_edges() {
        let first = output("first", (800, 600));
        let adjacent = output("adjacent", (1024, 768));
        let mut space = Space::default();
        space.map_output(&first, (-800, -600));
        space.map_output(&adjacent, (0, -600));
        let pointer = VirtualPointerData {
            output: Some(first.clone()),
            ..Default::default()
        };
        let geometry = space.output_geometry(&first).unwrap().to_f64();
        for (x, y, x_extent, y_extent) in [
            (0, 0, 100, 100),
            (100, 100, 100, 100),
            (1000, 1000, 1, 1),
            (u32::MAX, u32::MAX, 1, 1),
        ] {
            let point = pointer
                .absolute_position(&space, x, y, x_extent, y_extent)
                .unwrap();
            assert!(geometry.contains(point), "{point:?} escaped {geometry:?}");
            if x > 0 {
                assert!(point.x > -0.01 && point.y > -0.01);
            }
        }
        for (x_extent, y_extent) in [(0, 0), (0, 100), (100, 0)] {
            assert_eq!(
                pointer.absolute_position(&space, 1, 1, x_extent, y_extent),
                None
            );
        }
    }

    #[test]
    fn wheel_steps_leave_room_for_legacy_fractional_accumulation() {
        for steps in [i32::MIN, -17_895_697, -2, 0, 1, 17_895_697, i32::MAX] {
            let value = discrete_v120(steps);
            assert_eq!(value % 120, 0);
            for remainder in -119..=119 {
                assert!(
                    value
                        .checked_add(remainder)
                        .unwrap()
                        .checked_abs()
                        .is_some()
                );
            }
        }
        assert_eq!(discrete_v120(-2), -240);
        assert_eq!(discrete_v120(1), 120);
    }
}
