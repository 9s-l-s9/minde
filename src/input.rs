// SPDX-License-Identifier: MIT

use smithay::wayland::seat::WaylandFocus;
use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Device, Event, InputBackend,
        InputEvent, KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
        PointerMotionEvent,
    },
    input::{
        keyboard::{FilterResult, keysyms as xkb},
        pointer::{AxisFrame, ButtonEvent, MotionEvent, RelativeMotionEvent},
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::SERIAL_COUNTER,
};

use crate::guile;
use crate::state::MindeState;

/// X11-style modifier bitmask mirrored for the Scheme side: shift=1, ctrl=4,
/// alt=8, super=64.
fn modifier_bitmask(shift: bool, ctrl: bool, alt: bool, logo: bool) -> u32 {
    let mut mask = 0u32;
    if shift {
        mask |= 1;
    }
    if ctrl {
        mask |= 4;
    }
    if alt {
        mask |= 8;
    }
    if logo {
        mask |= 64;
    }
    mask
}

fn mods_bitmask(mods: &smithay::input::keyboard::ModifiersState) -> u32 {
    modifier_bitmask(mods.shift, mods.ctrl, mods.alt, mods.logo)
}

impl MindeState {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        guile::note_activity();
        // While locked, only keyboard events are processed (they reach the
        // focused lock surface, gated further below so they never hit the
        // Scheme keybinding layer). Pointer, touch, and axis events are
        // dropped so no regular client ever sees them.
        if self.locked && !matches!(event, InputEvent::Keyboard { .. }) {
            return;
        }
        match event {
            InputEvent::Keyboard { event, .. } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);
                let key_state = event.state();

                let key_code = event.key_code();

                let Some(keyboard) = self.seat.get_keyboard() else {
                    tracing::warn!("keyboard event received before keyboard initialization");
                    return;
                };
                keyboard.input::<(), _>(
                    self,
                    key_code,
                    key_state,
                    serial,
                    time,
                    |data, mods, keysym_handle| {
                        // Only intercept on key press; let releases and
                        // repeats always reach the client so held keys don't
                        // get stuck if a binding consumed the press.
                        if key_state == KeyState::Pressed {
                            // Any new press ends the previous compositor-side
                            // repeat (prompts/armed keymaps; re-armed below
                            // if this press is consumed while repeat is on).
                            data.cancel_key_repeat();
                            let keysym = keysym_handle.modified_sym();

                            // VT switching: only meaningful (and only
                            // wired up) under the udev/libseat backend.
                            // xkb::KEY_XF86Switch_VT_1..=12 map to VTs 1..12.
                            if (xkb::KEY_XF86Switch_VT_1..=xkb::KEY_XF86Switch_VT_12)
                                .contains(&keysym.raw())
                            {
                                let vt = (keysym.raw() - xkb::KEY_XF86Switch_VT_1 + 1) as i32;
                                if let Some(session) = data.session.as_mut() {
                                    use smithay::backend::session::Session;
                                    if let Err(err) = session.change_vt(vt) {
                                        tracing::error!(vt, %err, "failed to switch vt");
                                    }
                                    return FilterResult::Intercept(());
                                }
                            }

                            // While locked, keys never reach the Scheme
                            // keybinding layer -- forward them straight to the
                            // focused lock surface (VT switching above still
                            // works, so you can leave for another TTY).
                            // Otherwise the WM prefix key would operate on the
                            // lock screen.
                            if data.locked {
                                return FilterResult::Forward;
                            }

                            // xkbcommon's name ("t", "Return"), not the
                            // xkeysym constant name ("XK_t") from `.name()`.
                            let name = smithay::input::keyboard::xkb::keysym_get_name(keysym);
                            // The text this key produces under the active
                            // keymap (empty for Return/Backspace/etc.) --
                            // what the native input prompt inserts.
                            let mut utf8 = smithay::input::keyboard::xkb::keysym_to_utf8(keysym);
                            utf8.retain(|c| !c.is_control());
                            let consumed =
                                guile::handle_key(mods_bitmask(mods), keysym.raw(), &name, &utf8);
                            if consumed {
                                // Compositor-side auto-repeat: consumed keys
                                // never come back from libinput as repeats,
                                // so while a prompt/mode asked for repeat
                                // (wm-set-key-repeat) re-fire the handler
                                // from a timer until the key is released.
                                if data.key_repeat_enabled {
                                    let code = key_code.raw();
                                    let (m, k) = (mods_bitmask(mods), keysym.raw());
                                    let timer =
                                        smithay::reexports::calloop::timer::Timer::from_duration(
                                            std::time::Duration::from_millis(600),
                                        );
                                    let n = name.clone();
                                    let u = utf8.clone();
                                    let token =
                                        data.handle.insert_source(timer, move |_, _, state| {
                                            use smithay::reexports::calloop::timer::TimeoutAction;
                                            let held = state
                                                .key_repeat
                                                .as_ref()
                                                .is_some_and(|(c, _)| *c == code);
                                            if state.key_repeat_enabled
                                                && held
                                                && guile::handle_key(m, k, &n, &u)
                                            {
                                                TimeoutAction::ToDuration(
                                                    std::time::Duration::from_millis(40),
                                                )
                                            } else {
                                                if held {
                                                    state.key_repeat = None;
                                                }
                                                TimeoutAction::Drop
                                            }
                                        });
                                    if let Ok(token) = token {
                                        data.key_repeat = Some((code, token));
                                    }
                                }
                                return FilterResult::Intercept(());
                            }
                        } else if data
                            .key_repeat
                            .as_ref()
                            .is_some_and(|(c, _)| *c == key_code.raw())
                        {
                            // The repeated key was released: stop the timer.
                            data.cancel_key_repeat();
                        }
                        FilterResult::Forward
                    },
                );
            }
            InputEvent::PointerMotion { event, .. } => {
                // Relative motion: only generated by the udev/libinput
                // backend (winit only ever produces absolute motion).
                let mut proposed = self.pointer_location + event.delta();
                proposed = self.clamp_to_outputs(proposed);
                // Apply any active pointer lock/confinement before moving.
                let (new_pos, locked) = self.constrain_pointer(proposed);

                let Some(pointer) = self.seat.get_pointer() else {
                    return;
                };

                // Relative motion always flows -- this is the whole point of
                // the relative-pointer protocol for pointer-lock games, which
                // read deltas while the cursor itself stays parked.
                let under_current = self.surface_under(self.pointer_location);
                pointer.relative_motion(
                    self,
                    under_current,
                    &RelativeMotionEvent {
                        delta: event.delta(),
                        delta_unaccel: event.delta_unaccel(),
                        utime: event.time(),
                    },
                );

                if locked {
                    // Cursor stays put; no absolute motion is sent so the
                    // client's pointer never leaves its locked position.
                    pointer.frame(self);
                    return;
                }

                self.pointer_location = new_pos;
                let serial = SERIAL_COUNTER.next_serial();
                let under = self.surface_under(new_pos);
                pointer.motion(
                    self,
                    under.clone(),
                    &MotionEvent {
                        location: new_pos,
                        serial,
                        time: event.time_msec(),
                    },
                );
                // Activate a constraint the pointer just entered (focus-driven).
                self.activate_pointer_constraint_if_entered(&under);
                pointer.frame(self);
            }
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let Some(output) = self.space.outputs().next() else {
                    return;
                };
                let Some(output_geo) = self.space.output_geometry(output) else {
                    return;
                };

                let proposed =
                    event.position_transformed(output_geo.size) + output_geo.loc.to_f64();
                // Apply any active pointer lock/confinement before moving.
                let (new_pos, locked) = self.constrain_pointer(proposed);

                let Some(pointer) = self.seat.get_pointer() else {
                    return;
                };

                // Winit only ever delivers absolute motion, but pointer-lock
                // clients (games) need relative_pointer events. Synthesize the
                // delta from consecutive absolute positions and forward it so
                // the relative-pointer protocol is honestly usable nested; the
                // libinput backend supplies true unaccelerated deltas instead.
                let delta = proposed - self.pointer_location;
                let under_current = self.surface_under(self.pointer_location);
                pointer.relative_motion(
                    self,
                    under_current,
                    &RelativeMotionEvent {
                        delta,
                        delta_unaccel: delta,
                        utime: event.time(),
                    },
                );

                if locked {
                    pointer.frame(self);
                    return;
                }

                self.pointer_location = new_pos;
                let serial = SERIAL_COUNTER.next_serial();
                let under = self.surface_under(new_pos);
                pointer.motion(
                    self,
                    under.clone(),
                    &MotionEvent {
                        location: new_pos,
                        serial,
                        time: event.time_msec(),
                    },
                );
                self.activate_pointer_constraint_if_entered(&under);
                pointer.frame(self);
            }
            InputEvent::PointerButton { event, .. } => {
                let Some(pointer) = self.seat.get_pointer() else {
                    return;
                };
                let Some(keyboard) = self.seat.get_keyboard() else {
                    return;
                };

                let serial = SERIAL_COUNTER.next_serial();

                let button = event.button_code();

                let button_state = event.state();

                if ButtonState::Pressed == button_state && !pointer.is_grabbed() {
                    if let Some((window, _loc)) = self
                        .space
                        .element_under(pointer.current_location())
                        .map(|(w, l)| (w.clone(), l))
                    {
                        // Super+drag on a floating window: left = move,
                        // right = resize from the bottom-right corner.
                        // The grab consumes the click (Focus::Clear) but
                        // the press is still forwarded through
                        // pointer.button below so current_pressed()
                        // tracks the drag button.
                        const BTN_LEFT: u32 = 0x110;
                        const BTN_RIGHT: u32 = 0x111;
                        let floating = self
                            .id_for_window(&window)
                            .map(|id| self.floating_ids.contains(&id))
                            .unwrap_or(false);
                        if floating
                            && keyboard.modifier_state().logo
                            && (button == BTN_LEFT || button == BTN_RIGHT)
                        {
                            let start_data = smithay::input::pointer::GrabStartData {
                                focus: None,
                                button,
                                location: pointer.current_location(),
                            };
                            self.space.raise_element(&window, true);
                            if let Some(rect) = self.space.element_geometry(&window) {
                                if button == BTN_LEFT {
                                    let grab = crate::grabs::MoveSurfaceGrab {
                                        start_data,
                                        window,
                                        initial_window_location: rect.loc,
                                    };
                                    pointer.set_grab(
                                        self,
                                        grab,
                                        serial,
                                        smithay::input::pointer::Focus::Clear,
                                    );
                                } else {
                                    let grab = crate::grabs::ResizeSurfaceGrab::start(
                                        start_data,
                                        window,
                                        crate::grabs::ResizeEdge::BOTTOM_RIGHT,
                                        rect,
                                    );
                                    pointer.set_grab(
                                        self,
                                        grab,
                                        serial,
                                        smithay::input::pointer::Focus::Clear,
                                    );
                                }
                            }
                            pointer.button(
                                self,
                                &ButtonEvent {
                                    button,
                                    state: button_state,
                                    serial,
                                    time: event.time_msec(),
                                },
                            );
                            pointer.frame(self);
                            return;
                        }
                        self.space.raise_element(&window, true);
                        // X11-backed windows expose a wl_surface too (once
                        // the xwayland-shell association happened).
                        if let Some(surface) = window.wl_surface().map(|s| s.into_owned()) {
                            keyboard.set_focus(self, Some(surface), serial);
                        }
                        self.space.elements().for_each(|window| {
                            if let Some(t) = window.toplevel() {
                                t.send_pending_configure();
                            }
                        });
                    } else {
                        self.space.elements().for_each(|window| {
                            window.set_activated(false);
                            if let Some(t) = window.toplevel() {
                                t.send_pending_configure();
                            }
                        });
                        keyboard.set_focus(self, Option::<WlSurface>::None, serial);
                    }
                };

                pointer.button(
                    self,
                    &ButtonEvent {
                        button,
                        state: button_state,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerAxis { event, .. } => {
                let source = event.source();

                let horizontal_amount = event.amount(Axis::Horizontal).unwrap_or_else(|| {
                    event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.
                });
                let vertical_amount = event.amount(Axis::Vertical).unwrap_or_else(|| {
                    event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.
                });
                let horizontal_amount_discrete = event.amount_v120(Axis::Horizontal);
                let vertical_amount_discrete = event.amount_v120(Axis::Vertical);

                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                if horizontal_amount != 0.0 {
                    frame = frame.value(Axis::Horizontal, horizontal_amount);
                    if let Some(discrete) = horizontal_amount_discrete {
                        frame = frame.v120(Axis::Horizontal, discrete as i32);
                    }
                }
                if vertical_amount != 0.0 {
                    frame = frame.value(Axis::Vertical, vertical_amount);
                    if let Some(discrete) = vertical_amount_discrete {
                        frame = frame.v120(Axis::Vertical, discrete as i32);
                    }
                }

                if source == AxisSource::Finger {
                    if event.amount(Axis::Horizontal) == Some(0.0) {
                        frame = frame.stop(Axis::Horizontal);
                    }
                    if event.amount(Axis::Vertical) == Some(0.0) {
                        frame = frame.stop(Axis::Vertical);
                    }
                }

                let pointer = self.seat.get_pointer().unwrap();
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            InputEvent::DeviceAdded { device } => {
                tracing::info!(name = device.name(), "input device added");
            }
            InputEvent::DeviceRemoved { device } => {
                tracing::info!(name = device.name(), "input device removed");
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::modifier_bitmask;

    #[test]
    fn modifier_translation_matches_the_scheme_contract() {
        assert_eq!(modifier_bitmask(false, false, false, false), 0);
        assert_eq!(modifier_bitmask(true, false, false, false), 1);
        assert_eq!(modifier_bitmask(false, true, true, false), 4 | 8);
        assert_eq!(modifier_bitmask(true, true, true, true), 1 | 4 | 8 | 64);
    }
}
