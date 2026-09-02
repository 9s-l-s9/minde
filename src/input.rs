// SPDX-License-Identifier: MIT

use smithay::wayland::seat::WaylandFocus;
use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Device, Event, InputBackend,
        InputEvent, KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
        PointerMotionEvent, ProximityState, TabletToolAxisEvent, TabletToolButtonEvent,
        TabletToolProximityEvent, TabletToolTipEvent, TabletToolTipState, TouchEvent,
    },
    input::{
        keyboard::{FilterResult, keysyms as xkb},
        pointer::{AxisFrame, ButtonEvent, MotionEvent, RelativeMotionEvent},
        touch::{DownEvent, MotionEvent as TouchMotionEvent, UpEvent},
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::tablet_manager::{TabletDescriptor, TabletSeatTrait},
};

/// BTN_LEFT/BTN_RIGHT in the libinput/evdev button namespace. Emulated as the tool's
/// tip-down/up "click" for tablet-unaware clients.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

/// Whether a tablet tool's input drives the real tablet protocol (the surface
/// under the tool is tablet-aware) or falls back to emulating the pointer so a
/// stylus still points and clicks in tablet-unaware apps. Factored out so the
/// decision is unit-testable without a live seat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolRoute {
    Tablet,
    Pointer,
}

/// The tool routes to the real tablet protocol only when a tool resource is
/// registered *and* the focused surface's client has bound it; otherwise the
/// event is emulated on the pointer.
fn tool_route(tool_registered: bool, focus_is_tablet_aware: bool) -> ToolRoute {
    if tool_registered && focus_is_tablet_aware {
        ToolRoute::Tablet
    } else {
        ToolRoute::Pointer
    }
}

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
        // Reset the ext-idle-notify-v1 timers on any user activity (keyboard,
        // pointer motion/button/axis, touch). Done before the locked-state
        // gate so activity on the lock screen still counts as activity. See
        // handlers::idle.
        self.notify_idle_activity();
        // While locked, only keyboard events are processed (they reach the
        // focused lock surface, gated further below so they never hit the
        // Scheme keybinding layer). Pointer, touch, tablet-tool, and axis
        // events are all dropped -- the allowlist is Keyboard-only, so every
        // TabletTool* event is covered -- so no regular client ever sees them.
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

                            // keyboard-shortcuts-inhibit: while an inhibitor is
                            // active for the focused surface, the compositor's
                            // own bindings (the prefix-key grab and every Scheme
                            // shortcut) are bypassed and keys are delivered
                            // straight to the client (remote-desktop/VM clients).
                            // An inhibitor is only ever marked active while its
                            // surface holds keyboard focus (see
                            // handlers::keyboard_shortcuts_inhibit), and this
                            // runs *after* the locked gate above, so it can
                            // never take effect on the lock surface.
                            let inhibited = {
                                use smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitorSeat;
                                data.seat.keyboard_shortcuts_inhibited()
                            };
                            if inhibited {
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
                self.pointer_relative_motion(
                    event.delta(),
                    event.delta_unaccel(),
                    event.time_msec(),
                    event.time(),
                );
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
                self.pointer_absolute_motion(proposed, event.time_msec(), event.time());
            }
            InputEvent::PointerButton { event, .. } => {
                self.pointer_button_event(event.button_code(), event.state(), event.time_msec());
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

                self.pointer_axis_frame(frame);
            }
            InputEvent::TouchDown { event, .. } => {
                let Some(touch) = self.seat.get_touch() else {
                    return;
                };
                let Some(location) = self.touch_global_position(&event) else {
                    return;
                };
                let serial = SERIAL_COUNTER.next_serial();
                let under = self.surface_under(location);

                // Tap-to-focus: a touch-down on an unfocused toplevel raises
                // and focuses it, mirroring click-to-focus in the
                // PointerButton arm.
                self.focus_toplevel_under(location, serial);

                touch.down(
                    self,
                    under,
                    &DownEvent {
                        slot: event.slot(),
                        location,
                        serial,
                        time: event.time_msec(),
                    },
                );
            }
            InputEvent::TouchMotion { event, .. } => {
                let Some(touch) = self.seat.get_touch() else {
                    return;
                };
                let Some(location) = self.touch_global_position(&event) else {
                    return;
                };
                let under = self.surface_under(location);
                touch.motion(
                    self,
                    under,
                    &TouchMotionEvent {
                        slot: event.slot(),
                        location,
                        time: event.time_msec(),
                    },
                );
            }
            InputEvent::TouchUp { event, .. } => {
                let Some(touch) = self.seat.get_touch() else {
                    return;
                };
                let serial = SERIAL_COUNTER.next_serial();
                touch.up(
                    self,
                    &UpEvent {
                        slot: event.slot(),
                        serial,
                        time: event.time_msec(),
                    },
                );
            }
            InputEvent::TouchFrame { .. } => {
                if let Some(touch) = self.seat.get_touch() {
                    touch.frame(self);
                }
            }
            InputEvent::TabletToolAxis { event, .. } => {
                self.on_tablet_tool_axis(&event);
            }
            InputEvent::TabletToolProximity { event, .. } => {
                self.on_tablet_tool_proximity(&event);
            }
            InputEvent::TabletToolTip { event, .. } => {
                self.on_tablet_tool_tip(&event);
            }
            InputEvent::TabletToolButton { event, .. } => {
                self.on_tablet_tool_button(&event);
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

    /// Shared relative-motion pointer processing used by the real input path
    /// (udev/libinput deltas) and the hand-rolled virtual pointer
    /// (`zwlr_virtual_pointer_v1` motion). Forwards relative motion (games
    /// read deltas even while locked), then applies pointer
    /// lock/confinement before emitting absolute motion.
    pub fn pointer_relative_motion(
        &mut self,
        delta: Point<f64, Logical>,
        delta_unaccel: Point<f64, Logical>,
        time_msec: u32,
        utime: u64,
    ) {
        let mut proposed = self.pointer_location + delta;
        proposed = self.clamp_to_outputs(proposed);
        // One hit test for the pre-move position serves both the constraint
        // check and the relative-motion focus.
        let under_current = self.surface_under(self.pointer_location);
        // Apply any active pointer lock/confinement before moving.
        let (new_pos, locked) = self.constrain_pointer(under_current.as_ref(), proposed);

        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };

        // Relative motion always flows -- this is the whole point of
        // the relative-pointer protocol for pointer-lock games, which
        // read deltas while the cursor itself stays parked.
        pointer.relative_motion(
            self,
            under_current,
            &RelativeMotionEvent {
                delta,
                delta_unaccel,
                utime,
            },
        );

        if locked {
            // Cursor stays put; no absolute motion is sent so the
            // client's pointer never leaves its locked position.
            pointer.frame(self);
            return;
        }

        let old_pos = self.pointer_location;
        self.pointer_location = new_pos;
        crate::automation_observe::set_pointer_position(new_pos.x, new_pos.y);
        // The cursor moved: repaint the heads it left and entered.
        self.schedule_redraw_at(&[old_pos, new_pos]);
        let serial = SERIAL_COUNTER.next_serial();
        let under = self.surface_under(new_pos);
        pointer.motion(
            self,
            under.clone(),
            &MotionEvent {
                location: new_pos,
                serial,
                time: time_msec,
            },
        );
        // Activate a constraint the pointer just entered (focus-driven).
        self.activate_pointer_constraint_if_entered(&under);
        pointer.frame(self);
    }

    /// Shared absolute-motion pointer processing (winit absolute motion and
    /// `zwlr_virtual_pointer_v1` motion_absolute). Synthesizes a relative
    /// delta from consecutive positions so the relative-pointer protocol
    /// still works, then applies constraints.
    pub fn pointer_absolute_motion(
        &mut self,
        proposed: Point<f64, Logical>,
        time_msec: u32,
        utime: u64,
    ) {
        let under_current = self.surface_under(self.pointer_location);
        // Apply any active pointer lock/confinement before moving.
        let (new_pos, locked) = self.constrain_pointer(under_current.as_ref(), proposed);

        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };

        // Winit only ever delivers absolute motion, but pointer-lock
        // clients (games) need relative_pointer events. Synthesize the
        // delta from consecutive absolute positions and forward it so
        // the relative-pointer protocol is honestly usable nested; the
        // libinput backend supplies true unaccelerated deltas instead.
        let delta = proposed - self.pointer_location;
        pointer.relative_motion(
            self,
            under_current,
            &RelativeMotionEvent {
                delta,
                delta_unaccel: delta,
                utime,
            },
        );

        if locked {
            pointer.frame(self);
            return;
        }

        let old_pos = self.pointer_location;
        self.pointer_location = new_pos;
        crate::automation_observe::set_pointer_position(new_pos.x, new_pos.y);
        // The cursor moved: repaint the heads it left and entered.
        self.schedule_redraw_at(&[old_pos, new_pos]);
        let serial = SERIAL_COUNTER.next_serial();
        let under = self.surface_under(new_pos);
        pointer.motion(
            self,
            under.clone(),
            &MotionEvent {
                location: new_pos,
                serial,
                time: time_msec,
            },
        );
        self.activate_pointer_constraint_if_entered(&under);
        pointer.frame(self);
    }

    /// Shared axis (scroll) processing: submit a fully-built frame to the
    /// pointer. Used by the real input path and the virtual pointer.
    pub fn pointer_axis_frame(&mut self, frame: AxisFrame) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        pointer.axis(self, frame);
        pointer.frame(self);
    }

    /// Shared button processing (real pointer buttons and
    /// `zwlr_virtual_pointer_v1` button). Runs the same focus/raise and
    /// super+drag move/resize logic as a physical click so virtual clicks
    /// behave identically.
    pub fn pointer_button_event(&mut self, button: u32, button_state: ButtonState, time_msec: u32) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };

        let serial = SERIAL_COUNTER.next_serial();

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
                            time: time_msec,
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
                self.set_text_input_focus(None);
                self.update_keyboard_shortcuts_inhibitors(None);
            }
        };

        pointer.button(
            self,
            &ButtonEvent {
                button,
                state: button_state,
                serial,
                time: time_msec,
            },
        );
        pointer.frame(self);
    }

    /// Map an absolute touch event to a point in global compositor space.
    /// Touchscreens report normalized absolute coordinates, so we transform
    /// against the first output's geometry exactly like the
    /// PointerMotionAbsolute arm does. Returns `None` if there is no output.
    fn touch_global_position<I: InputBackend>(
        &self,
        event: &impl AbsolutePositionEvent<I>,
    ) -> Option<Point<f64, Logical>> {
        let output = self.space.outputs().next()?;
        let output_geo = self.space.output_geometry(output)?;
        Some(map_absolute_to_global(
            event.position_transformed(output_geo.size),
            output_geo.loc,
        ))
    }

    /// Tap-to-focus: raise and give keyboard focus to the toplevel under the
    /// given point, matching the click-to-focus behavior in the
    /// PointerButton arm. Does nothing (keeping current focus) if no window
    /// element is under the point.
    fn focus_toplevel_under(
        &mut self,
        location: Point<f64, Logical>,
        serial: smithay::utils::Serial,
    ) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        if let Some(window) = self.space.element_under(location).map(|(w, _)| w.clone()) {
            self.space.raise_element(&window, true);
            if let Some(surface) = window.wl_surface().map(|s| s.into_owned()) {
                keyboard.set_focus(self, Some(surface), serial);
            }
            self.space.elements().for_each(|window| {
                if let Some(t) = window.toplevel() {
                    t.send_pending_configure();
                }
            });
        }
    }

    /// Tablet tool axis (motion + pressure/tilt/distance/etc.). Transforms the
    /// absolute tool position into global space, always keeps the visible
    /// cursor following the stylus, and either delivers a real tablet motion to
    /// a tablet-aware client or emulates pointer motion for a tablet-unaware
    /// one. The tablet/tool must already be registered (tablet from
    /// DeviceAdded, tool from proximity) for the tablet path; otherwise this is
    /// pure pointer emulation.
    fn on_tablet_tool_axis<I: InputBackend>(&mut self, event: &impl TabletToolAxisEvent<I>) {
        let Some(location) = self.touch_global_position(event) else {
            return;
        };
        let tablet_seat = self.seat.tablet_seat();
        let tablet = tablet_seat.get_tablet(&TabletDescriptor::from(&event.device()));
        let tool = tablet_seat.get_tool(&event.tool());
        let under = self.surface_under(location);
        // Decide against the surface's client *before* handing `under` to
        // motion() (which consumes it).
        let aware = under
            .as_ref()
            .map(|(s, _)| self.client_is_tablet_aware(s))
            .unwrap_or(false);

        let route = if let (Some(tablet), Some(tool)) = (tablet, tool) {
            // Queue the axes so they ride the next motion frame (the tool
            // handle batches pending axes until motion is emitted).
            if event.pressure_has_changed() {
                tool.pressure(event.pressure());
            }
            if event.distance_has_changed() {
                tool.distance(event.distance());
            }
            if event.tilt_has_changed() {
                tool.tilt(event.tilt());
            }
            if event.slider_has_changed() {
                tool.slider_position(event.slider_position());
            }
            if event.rotation_has_changed() {
                tool.rotation(event.rotation());
            }
            if event.wheel_has_changed() {
                tool.wheel(event.wheel_delta(), event.wheel_delta_discrete());
            }
            let serial = SERIAL_COUNTER.next_serial();
            // motion() internally drives proximity_in/out; delivery to the
            // client is a no-op when it never bound the tool.
            tool.motion(location, under, &tablet, serial, event.time_msec());
            tool_route(true, aware)
        } else {
            tool_route(false, false)
        };

        match route {
            ToolRoute::Tablet => {
                // Tablet-aware client owns the tool events; move only the
                // visible cursor (no wl_pointer events, which would double up).
                let old_pos = self.pointer_location;
                self.pointer_location = location;
                crate::automation_observe::set_pointer_position(location.x, location.y);
                self.schedule_redraw_at(&[old_pos, location]);
            }
            ToolRoute::Pointer => {
                // Emulate the pointer so the stylus points in unaware apps.
                // This reuses the shared pointer path, so the cursor follows and
                // hover/enter-leave are delivered; constraints only ever apply
                // to real pointer focus, which a stylus never grabs.
                self.pointer_absolute_motion(location, event.time_msec(), event.time());
            }
        }
    }

    /// Tablet tool proximity in/out. On the first proximity-in the tool is
    /// registered with the seat (per the protocol, tools are advertised on
    /// use), and the tablet is ensured to exist (winit has no DeviceAdded
    /// tablet). Proximity-out notifies the focused client the tool left.
    fn on_tablet_tool_proximity<I: InputBackend>(
        &mut self,
        event: &impl TabletToolProximityEvent<I>,
    ) {
        let tablet_seat = self.seat.tablet_seat();
        let dh = self.display_handle.clone();
        let tablet_desc = TabletDescriptor::from(&event.device());
        // Idempotent: ensures a tablet exists even under winit / before a
        // DeviceAdded was seen for it.
        tablet_seat.add_tablet::<Self>(&dh, &tablet_desc);

        match event.state() {
            ProximityState::In => {
                let tool = tablet_seat.add_tool::<Self>(self, &dh, &event.tool());
                // proximity_in requires a focused surface and an initial
                // motion; drive it through motion() when a surface is under the
                // tool so the client gets proximity_in + motion in one go.
                if let Some(location) = self.touch_global_position(event) {
                    let under = self.surface_under(location);
                    if let Some(tablet) = tablet_seat.get_tablet(&tablet_desc) {
                        let serial = SERIAL_COUNTER.next_serial();
                        tool.motion(location, under, &tablet, serial, event.time_msec());
                    }
                }
            }
            ProximityState::Out => {
                if let Some(tool) = tablet_seat.get_tool(&event.tool()) {
                    tool.proximity_out(event.time_msec());
                }
            }
        }
    }

    /// Tablet tool tip down/up. Tip-down focuses the toplevel under the tool
    /// (tap-to-focus, like touch and click), then either sends a real tablet
    /// tip event to a tablet-aware client or emulates a BTN_LEFT press. Tip-up
    /// mirrors it so an emulated button is always released.
    fn on_tablet_tool_tip<I: InputBackend>(&mut self, event: &impl TabletToolTipEvent<I>) {
        let tablet_seat = self.seat.tablet_seat();
        let tool = tablet_seat.get_tool(&event.tool());
        let serial = SERIAL_COUNTER.next_serial();
        match event.tip_state() {
            TabletToolTipState::Down => {
                // The tool sits at the cursor (kept following the stylus by
                // the axis arm). The route is decided at tip-down and
                // remembered in `stylus_tip_emulated`: tip-up must release
                // iff the press was emulated, even if awareness or the
                // surface under the tool changes mid-stroke -- recomputing on
                // tip-up could leave a stuck emulated button.
                let aware = self
                    .surface_under(self.pointer_location)
                    .map(|(s, _)| self.client_is_tablet_aware(&s))
                    .unwrap_or(false);
                let route = tool_route(tool.is_some(), aware);
                self.focus_toplevel_under(self.pointer_location, serial);
                if let Some(tool) = tool.as_ref() {
                    tool.tip_down(serial, event.time_msec());
                }
                self.stylus_tip_emulated = route == ToolRoute::Pointer;
                if self.stylus_tip_emulated {
                    self.pointer_button_event(BTN_LEFT, ButtonState::Pressed, event.time_msec());
                }
            }
            TabletToolTipState::Up => {
                if let Some(tool) = tool.as_ref() {
                    tool.tip_up(event.time_msec());
                }
                if self.stylus_tip_emulated {
                    self.stylus_tip_emulated = false;
                    self.pointer_button_event(BTN_LEFT, ButtonState::Released, event.time_msec());
                }
            }
        }
    }

    /// Tablet tool barrel button. Delivered to tablet-aware clients as a tool
    /// button; there is no sensible pointer mapping for a stylus barrel button,
    /// so unaware clients simply don't see it (no-op when no tool resource).
    fn on_tablet_tool_button<I: InputBackend>(&mut self, event: &impl TabletToolButtonEvent<I>) {
        let tablet_seat = self.seat.tablet_seat();
        if let Some(tool) = tablet_seat.get_tool(&event.tool()) {
            let serial = SERIAL_COUNTER.next_serial();
            tool.button(
                event.button(),
                event.button_state(),
                serial,
                event.time_msec(),
            );
        }
    }

    /// Whether the surface's client is tablet-aware, i.e. it bound
    /// `zwp_tablet_manager_v2` and holds a `zwp_tablet_tool_v2` resource. Drives
    /// the pointer-emulation fallback: unaware clients get emulated pointer
    /// input so a stylus still points and clicks. Smithay exposes no per-client
    /// tablet-binding query, so we inspect the client's live protocol objects.
    fn client_is_tablet_aware(&self, surface: &WlSurface) -> bool {
        use smithay::reexports::wayland_server::Resource;
        let Ok(client) = self.display_handle.get_client(surface.id()) else {
            return false;
        };
        let mut aware = false;
        // `interface()` reads the object's own metadata (no backend lock), so it
        // is safe to call inside the enumeration closure.
        let _ = self
            .display_handle
            .backend_handle()
            .with_all_objects_for(client.id(), |obj| {
                if obj.interface().name == "zwp_tablet_tool_v2" {
                    aware = true;
                }
            });
        aware
    }
}

/// Add the output's origin to an output-relative transformed point to obtain a
/// point in the global compositor coordinate space. Factored out so the
/// absolute-coordinate mapping is unit-testable without a live backend.
fn map_absolute_to_global(
    transformed: Point<f64, Logical>,
    output_loc: Point<i32, Logical>,
) -> Point<f64, Logical> {
    transformed + output_loc.to_f64()
}

#[cfg(test)]
mod tests {
    use super::{ToolRoute, map_absolute_to_global, modifier_bitmask, tool_route};
    use smithay::utils::Point;

    #[test]
    fn modifier_translation_matches_the_scheme_contract() {
        assert_eq!(modifier_bitmask(false, false, false, false), 0);
        assert_eq!(modifier_bitmask(true, false, false, false), 1);
        assert_eq!(modifier_bitmask(false, true, true, false), 4 | 8);
        assert_eq!(modifier_bitmask(true, true, true, true), 1 | 4 | 8 | 64);
    }

    #[test]
    fn absolute_touch_maps_into_the_output_global_space() {
        // On the origin output the transformed point is already global.
        let p = map_absolute_to_global(Point::from((10.0, 20.0)), Point::from((0, 0)));
        assert_eq!(p, Point::from((10.0, 20.0)));

        // A touch on a second output offset to the right lands in global
        // space by adding the output origin.
        let p = map_absolute_to_global(Point::from((5.5, 7.25)), Point::from((1920, 0)));
        assert_eq!(p, Point::from((1925.5, 7.25)));
    }

    #[test]
    fn tablet_tool_routes_to_tablet_only_when_registered_and_aware() {
        // The real tablet protocol is used only when a tool resource exists and
        // the focused client has bound it.
        assert_eq!(tool_route(true, true), ToolRoute::Tablet);
        // A tablet-unaware client (bound no tool) falls back to the pointer,
        // so the stylus still points and clicks.
        assert_eq!(tool_route(true, false), ToolRoute::Pointer);
        // No registered tool yet (proximity not seen) is pure pointer emulation,
        // regardless of the awareness flag.
        assert_eq!(tool_route(false, false), ToolRoute::Pointer);
        assert_eq!(tool_route(false, true), ToolRoute::Pointer);
    }
}
