// SPDX-License-Identifier: GPL-3.0-or-later
//! Real compositor input dispatch and Wayland events, without a GPU or host input.
use crate::MindeState;
use smithay::{
    backend::input::*,
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{EventLoop, channel},
        wayland_server::Display,
    },
    utils::Transform,
};
use std::{
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[allow(dead_code)]
#[path = "../../../tests/clients/protocol/wire.rs"]
mod wire;
use wire::{Client, words};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Device;
impl smithay::backend::input::Device for Device {
    fn id(&self) -> String {
        "test-panel".into()
    }
    fn name(&self) -> String {
        self.id()
    }
    fn has_capability(&self, _: DeviceCapability) -> bool {
        true
    }
    fn usb_id(&self) -> Option<(u32, u32)> {
        None
    }
    fn syspath(&self) -> Option<std::path::PathBuf> {
        None
    }
}
struct Backend;
impl InputBackend for Backend {
    type Device = Device;
    type KeyboardKeyEvent = UnusedEvent;
    type PointerAxisEvent = Sample;
    type PointerButtonEvent = Sample;
    type PointerMotionEvent = UnusedEvent;
    type PointerMotionAbsoluteEvent = Sample;
    type GestureSwipeBeginEvent = UnusedEvent;
    type GestureSwipeUpdateEvent = UnusedEvent;
    type GestureSwipeEndEvent = UnusedEvent;
    type GesturePinchBeginEvent = UnusedEvent;
    type GesturePinchUpdateEvent = UnusedEvent;
    type GesturePinchEndEvent = UnusedEvent;
    type GestureHoldBeginEvent = UnusedEvent;
    type GestureHoldEndEvent = UnusedEvent;
    type TouchDownEvent = Sample;
    type TouchUpEvent = Sample;
    type TouchMotionEvent = Sample;
    type TouchCancelEvent = Sample;
    type TouchFrameEvent = Sample;
    type TabletToolAxisEvent = UnusedEvent;
    type TabletToolProximityEvent = UnusedEvent;
    type TabletToolTipEvent = UnusedEvent;
    type TabletToolButtonEvent = UnusedEvent;
    type SwitchToggleEvent = UnusedEvent;
    type SpecialEvent = UnusedEvent;
}
#[derive(Clone, Copy)]
struct Sample {
    x: f64,
    y: f64,
    slot: u32,
    pressed: bool,
}
impl Sample {
    fn at(x: f64, y: f64) -> Self {
        Self {
            x,
            y,
            slot: 0,
            pressed: true,
        }
    }
}
impl Event<Backend> for Sample {
    fn time(&self) -> u64 {
        1000
    }
    fn device(&self) -> Device {
        Device
    }
}
impl AbsolutePositionEvent<Backend> for Sample {
    fn x(&self) -> f64 {
        self.x
    }
    fn y(&self) -> f64 {
        self.y
    }
    fn x_transformed(&self, width: i32) -> f64 {
        self.x * width as f64
    }
    fn y_transformed(&self, height: i32) -> f64 {
        self.y * height as f64
    }
}
impl PointerMotionAbsoluteEvent<Backend> for Sample {}
impl PointerButtonEvent<Backend> for Sample {
    fn button_code(&self) -> u32 {
        0x110
    }
    fn state(&self) -> ButtonState {
        if self.pressed {
            ButtonState::Pressed
        } else {
            ButtonState::Released
        }
    }
}
impl PointerAxisEvent<Backend> for Sample {
    fn amount(&self, axis: Axis) -> Option<f64> {
        (axis == Axis::Vertical).then_some(15.0)
    }
    fn amount_v120(&self, _: Axis) -> Option<f64> {
        None
    }
    fn source(&self) -> AxisSource {
        AxisSource::Wheel
    }
    fn relative_direction(&self, _: Axis) -> AxisRelativeDirection {
        AxisRelativeDirection::Identical
    }
}
impl TouchEvent<Backend> for Sample {
    fn slot(&self) -> TouchSlot {
        Some(self.slot).into()
    }
}
impl TouchDownEvent<Backend> for Sample {}
impl TouchMotionEvent<Backend> for Sample {}
impl TouchUpEvent<Backend> for Sample {}
impl TouchCancelEvent<Backend> for Sample {}
impl TouchFrameEvent<Backend> for Sample {}

type Action = Box<dyn FnOnce(&mut MindeState) + Send>;
fn act(sender: &channel::Sender<Action>, action: impl FnOnce(&mut MindeState) + Send + 'static) {
    let (done, result) = std::sync::mpsc::channel();
    sender
        .send(Box::new(move |state| {
            action(state);
            done.send(()).unwrap();
        }))
        .unwrap();
    result.recv_timeout(Duration::from_secs(5)).unwrap();
}
fn input(sender: &channel::Sender<Action>, event: InputEvent<Backend>) {
    act(sender, move |state| state.process_input_event(event));
}
fn devices(client: &mut Client) -> (u32, u32) {
    let seat = client.bind("wl_seat", 8);
    let pointer = client.alloc();
    let touch = client.alloc();
    client.send(seat, 0, &words(&[pointer]));
    client.send(seat, 2, &words(&[touch]));
    client.roundtrip();
    (pointer, touch)
}
fn lock_surface(client: &mut Client, lock: u32, output_name: u32) -> u32 {
    let output = client.alloc();
    let mut bind = words(&[output_name]);
    bind.extend(wire::string("wl_output"));
    bind.extend(words(&[1, output]));
    client.send(client.registry, 0, &bind);
    let surface = client.new_surface();
    let role = client.alloc();
    client.send(lock, 1, &words(&[role, surface, output]));
    let configure = client.event(role, 0);
    client.send(role, 1, &words(&[configure.word(0)]));
    client.paint(surface, configure.word(1), configure.word(2), 0x004080);
    client.roundtrip();
    surface
}

fn exercise(sender: channel::Sender<Action>) {
    let sample = Sample::at(0.3, 0.4); // (160, 110) globally, (60, 60) locally.
    let mut desktop = Client::new("desktop");
    let outputs: Vec<_> = desktop
        .events
        .iter()
        .filter(|event| event.is(desktop.registry, 0) && wire::text(&event.data, 4) == "wl_output")
        .map(|event| event.word(0))
        .collect();
    assert_eq!(outputs.len(), 2);
    let (desktop_pointer, desktop_touch) = devices(&mut desktop);
    desktop.toplevel(true);
    act(&sender, |state| {
        let window = state.windows.values().next().unwrap().clone();
        state.space.map_element(window, (100, 50), true);
    });
    input(&sender, InputEvent::PointerMotionAbsolute { event: sample });
    input(&sender, InputEvent::PointerButton { event: sample });
    input(&sender, InputEvent::TouchDown { event: sample });
    input(&sender, InputEvent::TouchFrame { event: sample });
    desktop.roundtrip();
    assert!(
        desktop
            .events
            .iter()
            .any(|event| event.is(desktop_touch, 0))
    );
    assert!(
        desktop
            .events
            .iter()
            .any(|event| event.is(desktop_pointer, 3))
    );
    let start = desktop.events.len();
    let mut owner = Client::new("lock owner");
    let (pointer, touch) = devices(&mut owner);
    let lock = owner.lock(true);
    // No committed lock surface: black output receives no client input.
    input(&sender, InputEvent::PointerMotionAbsolute { event: sample });
    input(&sender, InputEvent::PointerAxis { event: sample });
    input(&sender, InputEvent::TouchMotion { event: sample });
    input(&sender, InputEvent::TouchUp { event: sample });
    desktop.roundtrip();
    assert!(
        desktop.events[start..]
            .iter()
            .any(|event| event.is(desktop_touch, 4))
    );
    assert!(
        !desktop.events[start..]
            .iter()
            .any(|event| (event.object == desktop_pointer
                && matches!(event.opcode, 0 | 2 | 3 | 4))
                || (event.object == desktop_touch && matches!(event.opcode, 0 | 1)))
    );
    let left = lock_surface(&mut owner, lock, outputs[0]);
    let start = owner.events.len();
    input(
        &sender,
        InputEvent::PointerMotionAbsolute {
            event: Sample::at(2.3, 0.4),
        },
    );
    input(&sender, InputEvent::PointerAxis { event: sample });
    owner.roundtrip();
    assert!(
        !owner.events[start..]
            .iter()
            .any(|event| event.is(pointer, 0) || event.is(pointer, 4)),
        "output without a lock surface delivered input"
    );
    input(&sender, InputEvent::PointerMotionAbsolute { event: sample });
    let right = lock_surface(&mut owner, lock, outputs[1]);
    owner.roundtrip();
    let enter = owner
        .events
        .iter()
        .rev()
        .find(|event| event.is(pointer, 0))
        .unwrap();
    assert_eq!(
        (enter.word(1), enter.word(2), enter.word(3)),
        (left, 60 * 256, 60 * 256)
    );
    let start = owner.events.len();
    input(
        &sender,
        InputEvent::PointerButton {
            event: Sample {
                pressed: false,
                ..sample
            },
        },
    );
    owner.roundtrip();
    assert!(
        !owner.events[start..]
            .iter()
            .any(|event| event.is(pointer, 3)),
        "old desktop release reached locker"
    );
    // Physical clicks and scroll now reach only the owner.
    input(&sender, InputEvent::PointerButton { event: sample });
    input(&sender, InputEvent::PointerAxis { event: sample });
    input(
        &sender,
        InputEvent::PointerButton {
            event: Sample {
                pressed: false,
                ..sample
            },
        },
    );
    owner.roundtrip();
    assert_eq!(
        owner.events[start..]
            .iter()
            .filter(|event| event.is(pointer, 3))
            .count(),
        2
    );
    assert!(
        owner.events[start..]
            .iter()
            .any(|event| event.is(pointer, 4))
    );
    // Absolute input uses the same output origins as rendering/hit testing.
    input(
        &sender,
        InputEvent::PointerMotionAbsolute {
            event: Sample::at(2.3, 0.4),
        },
    );
    owner.roundtrip();
    let enter = owner
        .events
        .iter()
        .rev()
        .find(|event| event.is(pointer, 0))
        .unwrap();
    assert_eq!((enter.word(1), enter.word(2)), (right, 60 * 256));
    act(&sender, |state| {
        state
            .absolute_inputs
            .configure("test-panel".into(), "panel", Some("right".into()), &[])
    });
    let start = owner.events.len();
    act(&sender, |state| state.winit_powered_off = true);
    input(&sender, InputEvent::TouchDown { event: sample });
    act(&sender, |state| {
        assert!(
            !state.winit_powered_off,
            "touch did not wake locked outputs"
        )
    });
    input(
        &sender,
        InputEvent::TouchMotion {
            event: Sample::at(0.4, 0.5),
        },
    );
    input(&sender, InputEvent::TouchFrame { event: sample });
    input(&sender, InputEvent::TouchUp { event: sample });
    input(&sender, InputEvent::TouchFrame { event: sample });
    owner.roundtrip();
    let down = owner.events[start..]
        .iter()
        .find(|event| event.is(touch, 0))
        .unwrap();
    assert_eq!(
        (down.word(2), down.word(4), down.word(5)),
        (right, 60 * 256, 60 * 256)
    );
    for opcode in [1, 2, 3] {
        assert!(
            owner.events[start..]
                .iter()
                .any(|event| event.is(touch, opcode))
        );
    }
    // Subsurface focus is cleared before destruction, then returns to the
    // root without a leave referring to a destroyed protocol object.
    let child = owner.new_surface();
    let subcompositor = owner.bind("wl_subcompositor", 1);
    let subsurface = owner.alloc();
    owner.send(subcompositor, 1, &words(&[subsurface, child, right]));
    owner.send(subsurface, 1, &words(&[20, 20]));
    owner.paint(child, 80, 80, 0xff0000);
    owner.send(right, 6, &[]);
    owner.roundtrip();
    assert_eq!(
        owner
            .events
            .iter()
            .rev()
            .find(|event| event.is(pointer, 0))
            .unwrap()
            .word(1),
        child
    );
    let destroyed_from = owner.events.len();
    owner.send(child, 0, &[]);
    owner.roundtrip();
    owner.wait(destroyed_from, |event| {
        event.is(pointer, 0) && event.word(1) == right
    });
    // Multiple contacts cancel as one sequence and cannot leak after unlock.
    input(&sender, InputEvent::TouchDown { event: sample });
    input(
        &sender,
        InputEvent::TouchDown {
            event: Sample { slot: 1, ..sample },
        },
    );
    input(&sender, InputEvent::TouchFrame { event: sample });
    input(&sender, InputEvent::PointerButton { event: sample });
    owner.roundtrip();
    let start = owner.events.len();
    let unrelated = desktop.new_surface();
    desktop.send(unrelated, 0, &[]);
    desktop.roundtrip();
    owner.roundtrip();
    assert!(
        !owner.events[start..].iter().any(|event| event.is(touch, 4)),
        "unrelated desktop destruction cancelled locker touch"
    );
    let start = owner.events.len();
    owner.send(lock, 2, &[]);
    owner.roundtrip();
    assert!(owner.events[start..].iter().any(|event| event.is(touch, 4)));
    desktop.roundtrip();
    let start = desktop.events.len();
    input(
        &sender,
        InputEvent::PointerButton {
            event: Sample {
                pressed: false,
                ..sample
            },
        },
    );
    input(&sender, InputEvent::TouchUp { event: sample });
    input(
        &sender,
        InputEvent::TouchUp {
            event: Sample { slot: 1, ..sample },
        },
    );
    input(&sender, InputEvent::TouchFrame { event: sample });
    desktop.roundtrip();
    assert!(
        !desktop.events[start..]
            .iter()
            .any(|event| event.is(desktop_pointer, 3) || event.object == desktop_touch)
    );
    // A destroyed surface on a live connection must lose focus before its ID
    // becomes invalid, then a disconnected owner must leave no active contacts.
    let lock = owner.lock(true);
    let right = lock_surface(&mut owner, lock, outputs[1]);
    input(&sender, InputEvent::PointerMotionAbsolute { event: sample });
    input(&sender, InputEvent::TouchDown { event: sample });
    input(&sender, InputEvent::TouchFrame { event: sample });
    owner.send(right, 0, &[]);
    owner.roundtrip();
    let start = owner.events.len();
    input(&sender, InputEvent::PointerAxis { event: sample });
    input(&sender, InputEvent::TouchMotion { event: sample });
    input(&sender, InputEvent::TouchUp { event: sample });
    owner.roundtrip();
    assert!(
        !owner.events[start..]
            .iter()
            .any(|event| event.object == pointer || event.object == touch)
    );
    drop(owner);
    act(&sender, |state| {
        state.enforce_lock_focus();
        assert!(state.locked);
    });
    let mut replacement = Client::new("replacement owner");
    let (_, touch) = devices(&mut replacement);
    let lock = replacement.lock(true);
    lock_surface(&mut replacement, lock, outputs[1]);
    input(&sender, InputEvent::TouchDown { event: sample });
    input(&sender, InputEvent::TouchFrame { event: sample });
    replacement.roundtrip();
    assert!(replacement.events.iter().any(|event| event.is(touch, 0)));
    replacement.send(lock, 2, &[]);
    replacement.roundtrip();
}

#[test]
fn lock_pointer_and_touch() {
    const CHILD: &str = "MINDE_LOCK_INPUT_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // Guile and compositor globals belong to one thread/process. Isolate
        // this real-state fixture from the unit runner's other Guile tests.
        let dir = std::env::temp_dir().join(format!("minde-lock-input-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "handlers::session_lock::input_tests::lock_pointer_and_touch",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("XDG_RUNTIME_DIR", &dir)
            .env("WAYLAND_DISPLAY", "wayland-1")
            .env_remove("NESTED_WAYLAND_DISPLAY")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() > deadline {
                child.kill().unwrap();
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let result = child.wait_with_output().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    crate::guile::boot();
    let mut event_loop = EventLoop::try_new().unwrap();
    let display = Display::new().unwrap();
    let mut state = MindeState::new(&mut event_loop, display).unwrap();
    for (name, x) in [("left", 100), ("right", 500)] {
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
        let mode = Mode {
            size: (200, 150).into(),
            refresh: 60000,
        };
        output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            None,
            Some((x, 50).into()),
        );
        output.set_preferred(mode);
        output.create_global::<MindeState>(&state.display_handle);
        state.space.map_output(&output, (x, 50));
    }
    let (sender, receiver) = channel::channel::<Action>();
    event_loop
        .handle()
        .insert_source(receiver, |event, _, state| {
            if let channel::Event::Msg(action) = event {
                action(state);
            }
        })
        .unwrap();
    let client = std::thread::spawn(move || exercise(sender));
    let deadline = Instant::now() + Duration::from_secs(20);
    while !client.is_finished() {
        assert!(
            Instant::now() < deadline,
            "headless lock input fixture timed out"
        );
        event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut state)
            .unwrap();
        state.enforce_lock_focus();
        state.maybe_confirm_lock();
        state.display_handle.flush_clients().unwrap();
    }
    client.join().unwrap();
}
