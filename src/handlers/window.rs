// SPDX-License-Identifier: MIT

//! Shared cleanup of mapped native and X11 windows.

use crate::{
    MindeState,
    grabs::{MoveSurfaceGrab, ResizeSurfaceGrab},
    guile,
    input::PointerButtonSource,
};
use smithay::{
    desktop::{PopupManager, Window},
    input::pointer::MotionEvent,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::SERIAL_COUNTER,
};

impl MindeState {
    pub(crate) fn unmap_window(&mut self, window: &Window) {
        self.space.unmap_elem(window);
        self.schedule_redraw();
        // Unmapped native and X11 windows may retain live surfaces. Grabs cannot
        // rely on surface destruction to end, and a stale move grab could even
        // map the old Window back into Space on its next motion.
        let mut surfaces = Vec::new();
        window.with_surfaces(|surface, _| surfaces.push(surface.clone()));
        let owns = |surface: &WlSurface| surfaces.contains(surface);
        if let Some(pointer) = self.seat.get_pointer() {
            let grabbed = pointer
                .with_grab(|_, grab| {
                    grab.start_data()
                        .focus
                        .as_ref()
                        .is_some_and(|(surface, _)| owns(surface))
                        || grab
                            .downcast_ref::<MoveSurfaceGrab>()
                            .is_some_and(|grab| &grab.window == window)
                        || grab
                            .downcast_ref::<ResizeSurfaceGrab>()
                            .is_some_and(|grab| &grab.window == window)
                })
                .unwrap_or(false);
            let automation = self
                .active_automation_dnd
                .as_ref()
                .filter(|_| grabbed)
                .map(|source| source.token());
            if grabbed || pointer.current_focus().as_ref().is_some_and(owns) {
                let serial = SERIAL_COUNTER.next_serial();
                let time = self.start_time.elapsed().as_millis() as u32;
                if grabbed {
                    pointer.unset_grab(self, serial, time);
                }
                pointer.motion(
                    self,
                    None,
                    &MotionEvent {
                        location: self.pointer_location,
                        serial,
                        time,
                    },
                );
                pointer.frame(self);
                if let Some(token) = automation {
                    // Release cancelled synthetic presses with no focused
                    // surface, before policy focuses the next desktop window.
                    self.release_pointer_buttons(|owner| {
                        *owner == PointerButtonSource::Automation(token)
                    });
                }
            }
        }
        if let Some(keyboard) = self.seat.get_keyboard() {
            if keyboard
                .grab_start_data()
                .and_then(|start| start.focus)
                .as_ref()
                .is_some_and(|focus| owns(focus.surface()))
            {
                keyboard.unset_grab(self);
            }
            if self.keyboard_focus().as_ref().is_some_and(owns) {
                self.set_keyboard_focus(None, SERIAL_COUNTER.next_serial());
            }
        }
        if let Some(touch) = self.seat.get_touch()
            && touch
                .grab_start_data()
                .and_then(|start| start.focus)
                .is_some_and(|(surface, _)| owns(&surface))
        {
            self.cancel_touch_input();
        }
        if let Some(toplevel) = window.toplevel() {
            let root = toplevel.wl_surface();
            let popups: Vec<_> = PopupManager::popups_for_surface(root).collect();
            for (popup, _) in popups {
                let _ = PopupManager::dismiss_popup(root, &popup);
            }
        }
        if let Some(id) = self.unregister_window(window) {
            guile::on_window_unmap(id);
            tracing::info!(
                component = "window",
                protocol = if window.x11_surface().is_some() {
                    "xwayland"
                } else {
                    "wayland"
                },
                window_id = id,
                "managed toplevel unmapped"
            );
        }
    }
}
