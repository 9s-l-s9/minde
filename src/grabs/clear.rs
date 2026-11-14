// SPDX-License-Identifier: MIT
//! Cancel grabs without restoring their saved desktop focus.
use crate::MindeState;
use smithay::{
    input::pointer::*,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, SERIAL_COUNTER},
};

struct ClearGrab(GrabStartData<MindeState>);
impl PointerGrab<MindeState> for ClearGrab {
    fn motion(
        &mut self,
        data: &mut MindeState,
        handle: &mut PointerInnerHandle<'_, MindeState>,
        _: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.unset_grab(self, data, event.serial, event.time, false);
        handle.motion(data, None, event);
    }
    fn button(
        &mut self,
        _: &mut MindeState,
        _: &mut PointerInnerHandle<'_, MindeState>,
        _: &ButtonEvent,
    ) {
    }
    crate::grabs::forward_pointer_events!();
    fn start_data(&self) -> &GrabStartData<MindeState> {
        &self.0
    }
    fn unset(&mut self, _: &mut MindeState) {}
}

impl MindeState {
    pub(crate) fn clear_pointer_focus(&mut self) {
        if let Some(pointer) = self.seat.get_pointer() {
            let serial = SERIAL_COUNTER.next_serial();
            let location = self.pointer_location;
            let time = self.start_time.elapsed().as_millis() as u32;
            // PointerHandle::unset_grab restores pending focus. Replace the
            // old grab first, then synchronously remove this temporary grab
            // through PointerInnerHandle's non-restoring path. No external
            // event can arrive between set_grab and this motion.
            pointer.set_grab(
                self,
                ClearGrab(GrabStartData {
                    focus: None,
                    button: 0,
                    location,
                }),
                serial,
                Focus::Clear,
            );
            pointer.motion(
                self,
                None,
                &MotionEvent {
                    location,
                    serial,
                    time,
                },
            );
            pointer.frame(self);
        }
        self.release_pointer_buttons(|_| true);
        self.cursor_state
            .set_status(CursorImageStatus::default_named());
    }
}
