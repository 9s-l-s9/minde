// SPDX-License-Identifier: MIT
//! Track physical contacts so cancellation reaches every held Smithay slot.

use std::collections::HashMap;

use smithay::{
    backend::input::TouchSlot,
    input::{
        SeatHandler,
        touch::{DownEvent, MotionEvent, TouchHandle},
    },
};

#[derive(Default)]
pub(crate) struct TouchContacts {
    held: HashMap<TouchSlot, MotionEvent>,
    pending_frame: bool,
}

impl TouchContacts {
    pub(crate) fn down(&mut self, event: &DownEvent) {
        self.held.insert(
            event.slot,
            MotionEvent {
                slot: event.slot,
                location: event.location,
                time: event.time,
            },
        );
        self.pending_frame = true;
    }

    pub(crate) fn motion(&mut self, event: &MotionEvent) -> bool {
        let Some(held) = self.held.get_mut(&event.slot) else {
            return false;
        };
        *held = event.clone();
        self.pending_frame = true;
        true
    }

    pub(crate) fn up(&mut self, slot: TouchSlot) -> bool {
        if self.held.remove(&slot).is_none() {
            return false;
        }
        self.pending_frame = true;
        true
    }

    pub(crate) fn take_frame(&mut self) -> bool {
        std::mem::take(&mut self.pending_frame)
    }

    /// Consume the old sequence before callbacks can start another one.
    pub(crate) fn cancel<D: SeatHandler + 'static>(mut self, touch: &TouchHandle<D>, data: &mut D) {
        // A DnD grab can swallow cancel/motion. Unset it first, which cancels
        // its source, then clear the seat's ordinary per-slot focus as well.
        touch.unset_grab(data);
        if self.take_frame() {
            // Complete already delivered ups, including their deferred focus,
            // before cancelling the contacts that are still held.
            touch.frame(data);
        }
        if self.held.is_empty() {
            return;
        }
        // Smithay 3021f619's cancel needs a pending frame and skips slots that
        // did not change in it. Repeat each held position to mark every slot;
        // cancel terminates this batch without a frame or a position change.
        for event in self.held.values() {
            touch.motion(data, None, event);
        }
        touch.cancel(data);
    }
}

#[cfg(test)]
mod tests;
