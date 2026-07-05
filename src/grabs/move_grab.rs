// SPDX-License-Identifier: MIT

//! Move grab is the state of a composer during which the client window is being dragged around.
//!
//! eg. Usually whenever a user clicks on the app's titlebar and starts dragging, the compositors
//! enters a MoveSurfaceGrab state.

use crate::MindeState;
use smithay::{
    desktop::Window,
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
        GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
        GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData as PointerGrabStartData,
        MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point},
};

pub struct MoveSurfaceGrab {
    pub start_data: PointerGrabStartData<MindeState>,
    pub window: Window,
    pub initial_window_location: Point<i32, Logical>,
}

impl PointerGrab<MindeState> for MoveSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut MindeState,
        handle: &mut PointerInnerHandle<'_, MindeState>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        // While the grab is active, no client has pointer focus
        handle.motion(data, None, event);

        let delta = event.location - self.start_data.location;
        let new_location = self.initial_window_location.to_f64() + delta;
        data.space
            .map_element(self.window.clone(), new_location.to_i32_round(), true);
    }

    fn relative_motion(
        &mut self,
        data: &mut MindeState,
        handle: &mut PointerInnerHandle<'_, MindeState>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        data: &mut MindeState,
        handle: &mut PointerInnerHandle<'_, MindeState>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);

        // Release once no buttons are held (the grab may have been started
        // by any button: client titlebar drags use left, super+drag resize
        // uses right).
        if handle.current_pressed().is_empty() {
            handle.unset_grab(self, data, event.serial, event.time, true);
            // Tell Scheme where the drag ended, so `%floating` geometry
            // stays authoritative across syncs.
            if let (Some(id), Some(geo)) = (
                data.id_for_window(&self.window),
                data.space.element_geometry(&self.window),
            ) {
                crate::guile::on_window_moved(id, geo.loc.x, geo.loc.y, geo.size.w, geo.size.h);
            }
        }
    }

    fn axis(
        &mut self,
        data: &mut MindeState,
        handle: &mut PointerInnerHandle<'_, MindeState>,
        details: AxisFrame,
    ) {
        handle.axis(data, details)
    }

    fn frame(
        &mut self,
        data: &mut MindeState,
        handle: &mut PointerInnerHandle<'_, MindeState>,
    ) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut MindeState,
        handle: &mut PointerInnerHandle<'_, MindeState>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event)
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut MindeState,
        handle: &mut PointerInnerHandle<'_, MindeState>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event)
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut MindeState,
        handle: &mut PointerInnerHandle<'_, MindeState>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event)
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut MindeState,
        handle: &mut PointerInnerHandle<'_, MindeState>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event)
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut MindeState,
        handle: &mut PointerInnerHandle<'_, MindeState>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event)
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut MindeState,
        handle: &mut PointerInnerHandle<'_, MindeState>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event)
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut MindeState,
        handle: &mut PointerInnerHandle<'_, MindeState>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event)
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut MindeState,
        handle: &mut PointerInnerHandle<'_, MindeState>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event)
    }

    fn start_data(&self) -> &PointerGrabStartData<MindeState> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut MindeState) {}
}
