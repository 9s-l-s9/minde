// SPDX-License-Identifier: MIT

pub mod move_grab;
pub use move_grab::MoveSurfaceGrab;

pub mod resize_grab;
pub use resize_grab::{ResizeEdge, ResizeSurfaceGrab};

/// The `PointerGrab` methods both grabs forward unchanged to the inner
/// handle (relative motion, axis, frame and every gesture event). Expanded
/// inside each `impl PointerGrab<MindeState>`; the invoking module supplies
/// the smithay imports the signatures name.
macro_rules! forward_pointer_events {
    () => {
        fn relative_motion(
            &mut self,
            data: &mut MindeState,
            handle: &mut PointerInnerHandle<'_, MindeState>,
            focus: Option<(WlSurface, Point<f64, Logical>)>,
            event: &RelativeMotionEvent,
        ) {
            handle.relative_motion(data, focus, event);
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
    };
}
pub(crate) use forward_pointer_events;
