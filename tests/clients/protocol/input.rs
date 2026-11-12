// SPDX-License-Identifier: GPL-3.0-or-later
use super::{lock::inject_key, wire::*};
use std::{
    cell::RefCell,
    env,
    ops::{Deref, DerefMut},
    rc::Rc,
    time::Duration,
};

pub fn evaluate(expression: &str, expected: &str) {
    assert_eq!(ipc(expression), format!("(ok {expected})"), "{expression}");
}
pub fn token(reply: &str) -> u32 {
    reply
        .strip_prefix("(ok ")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap()
        .parse()
        .unwrap()
}
pub fn numbers(reply: &str) -> Vec<i32> {
    reply
        .strip_prefix("(ok (")
        .and_then(|s| s.strip_suffix("))"))
        .unwrap()
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect()
}
pub struct Input {
    pub c: Client,
    pub pointer: u32,
    pub window_id: u32,
    pub x: i32,
}
impl Deref for Input {
    type Target = Client;
    fn deref(&self) -> &Client {
        &self.c
    }
}
impl DerefMut for Input {
    fn deref_mut(&mut self) -> &mut Client {
        &mut self.c
    }
}
impl Input {
    pub fn new(name: &str) -> Self {
        let mut c = Client::new(name);
        let seat = c.bind("wl_seat", 5);
        let pointer = c.alloc();
        c.send(seat, 0, &words(&[pointer]));
        Self {
            c,
            pointer,
            window_id: 0,
            x: 0,
        }
    }
    pub fn mapped(&mut self, x: i32) {
        self.toplevel(true);
        self.window_id = token(&ipc(&format!(
            "(find (lambda (id) (equal? (window-title id) {:?})) (all-window-ids))",
            self.name
        )));
        evaluate(
            &format!(
                "(begin (float-window! {} (list {x} 100 300 250)) (sync-frames!) (rust-call 'wm-focus-window {}))",
                self.window_id, self.window_id
            ),
            "#t",
        );
        self.c.event(self.c.keyboard, 1);
        self.x = x + 100;
        self.roundtrip();
    }
    pub fn focus(&mut self) {
        evaluate(
            &format!("(rust-call 'wm-focus-window {})", self.window_id),
            "#t",
        );
        self.roundtrip();
    }
    pub fn point(&mut self) {
        evaluate(
            &format!("(rust-call 'wm-warp-pointer {} 180)", self.x),
            "#t",
        );
        self.roundtrip();
        let focus = self
            .events
            .iter()
            .rfind(|e| e.object == self.pointer && e.opcode <= 1)
            .unwrap();
        assert_eq!(
            (focus.opcode, focus.word(1)),
            (0, self.surface),
            "pointer focus stuck"
        );
    }
    pub fn key(&mut self, code: u32, pressed: u32, start: usize) {
        let keyboard = self.keyboard;
        self.wait(start, |e| {
            e.is(keyboard, 3) && e.word(2) == code && e.word(3) == pressed
        });
    }
    pub fn modifiers(&mut self) -> [u32; 3] {
        self.roundtrip();
        let events = self.keyboard_events(0, &[4]);
        let e = events.last().unwrap();
        [e.word(1), e.word(2), e.word(3)]
    }
    pub fn buttons(&self, start: usize) -> Vec<(u32, u32)> {
        self.events[start..]
            .iter()
            .filter(|e| e.is(self.pointer, 3))
            .map(|e| (e.word(2), e.word(3)))
            .collect()
    }
    pub fn button(&mut self, code: u32, pressed: u32, start: usize) -> Event {
        let pointer = self.pointer;
        self.wait(start, |e| {
            e.is(pointer, 3) && e.word(2) == code && e.word(3) == pressed
        })
    }
    pub fn virtual_pointer(&mut self) -> u32 {
        let manager = self.bind("zwlr_virtual_pointer_manager_v1", 1);
        let pointer = self.alloc();
        self.send(manager, 0, &words(&[0, pointer]));
        self.roundtrip();
        pointer
    }
}
#[derive(Clone)]
struct Configure {
    serial: u32,
    size: (u32, u32),
    states: Vec<u32>,
}
struct Resize {
    input: Input,
    seat: u32,
    virtual_pointer: u32,
    configures: Rc<RefCell<Vec<Configure>>>,
}
impl Deref for Resize {
    type Target = Input;
    fn deref(&self) -> &Input {
        &self.input
    }
}
impl DerefMut for Resize {
    fn deref_mut(&mut self) -> &mut Input {
        &mut self.input
    }
}
impl Resize {
    fn new(name: &str) -> Self {
        let mut input = Input::new(name);
        input.mapped(100);
        input.roundtrip();
        let seat = input.bind("wl_seat", 5);
        let virtual_pointer = input.virtual_pointer();
        let configures = Rc::new(RefCell::new(Vec::new()));
        let states = Rc::new(RefCell::new(Vec::new()));
        let top_states = states.clone();
        let top = input.toplevel_object;
        input.handlers.insert(
            top,
            Box::new(move |c, e| {
                if e.opcode == 0 {
                    c.configured_size = (e.word(0), e.word(1));
                    *top_states.borrow_mut() =
                        (0..e.word(2) as usize / 4).map(|i| e.word(i + 3)).collect();
                }
            }),
        );
        let list = configures.clone();
        let xdg = input.xdg_surface;
        input.handlers.insert(
            xdg,
            Box::new(move |c, e| {
                if e.opcode == 0 {
                    list.borrow_mut().push(Configure {
                        serial: e.word(0),
                        size: c.configured_size,
                        states: states.borrow().clone(),
                    });
                }
            }),
        );
        Self {
            input,
            seat,
            virtual_pointer,
            configures,
        }
    }
    fn configured(&mut self, resizing: bool, start: usize) -> Configure {
        let configs = self.configures.clone();
        let xdg = self.xdg_surface;
        let from = self.events.len();
        self.wait(from, |e| {
            e.is(xdg, 0)
                && configs.borrow()[start..]
                    .iter()
                    .any(|c| c.states.contains(&3) == resizing)
        });
        self.configures.borrow()[start..]
            .iter()
            .find(|c| c.states.contains(&3) == resizing)
            .unwrap()
            .clone()
    }
    fn commit(&mut self, config: &Configure, size: Option<(u32, u32)>) {
        self.send(self.xdg_surface, 4, &words(&[config.serial]));
        let size = size.unwrap_or(config.size);
        let surface = self.surface;
        self.paint(surface, size.0, size.1, 0x00ffff);
        self.roundtrip();
    }
    fn start_resize(&mut self, edge: u32) {
        self.point();
        let start = self.events.len();
        self.send(self.virtual_pointer, 2, &words(&[0, 272, 1]));
        let event = self.button(272, 1, start);
        self.send(
            self.toplevel_object,
            6,
            &words(&[self.seat, event.word(0), edge]),
        );
        self.roundtrip();
    }
}
fn rectangle(edge: u32, size: (u32, u32)) -> Vec<i32> {
    vec![
        100 + if edge & 4 != 0 {
            300 - size.0 as i32
        } else {
            0
        },
        100 + if edge & 1 != 0 {
            250 - size.1 as i32
        } else {
            0
        },
        size.0 as i32,
        size.1 as i32,
    ]
}
fn check_rectangle(target: &Input, rect: &[i32]) {
    for query in ["wm-window-geometry", "float-geometry"] {
        assert_eq!(
            numbers(&ipc(&format!("({query} {})", target.window_id))),
            rect
        );
    }
}
fn resize() {
    for edge in [4, 1, 5, 6, 9, 10] {
        let mut target = Resize::new(&format!("delayed resize edge {edge}"));
        target.start_resize(edge);
        let start = target.configures.borrow().len();
        evaluate("(wm-warp-pointer 220 190)", "#t");
        let earlier = target.configured(true, start);
        let start = target.configures.borrow().len();
        evaluate("(wm-warp-pointer 240 210)", "#t");
        let latest = target.configured(true, start);
        let start = target.configures.borrow().len();
        target.send(target.virtual_pointer, 2, &words(&[0, 272, 0]));
        let final_config = target.configured(false, start);
        assert_eq!(final_config.size, latest.size);
        assert_eq!(
            numbers(&ipc(&format!("(float-geometry {})", target.window_id))),
            rectangle(edge, final_config.size)
        );
        target.commit(&earlier, None);
        let surface = target.surface;
        target.paint(surface, earlier.size.0, earlier.size.1, 0xff0000);
        target.roundtrip();
        assert_eq!(
            numbers(&ipc(&format!("(wm-window-geometry {})", target.window_id))),
            rectangle(edge, earlier.size)
        );
        let size = if edge == 9 {
            (final_config.size.0 - 7, final_config.size.1 + 9)
        } else {
            final_config.size
        };
        target.commit(&final_config, Some(size));
        check_rectangle(&target, &rectangle(edge, size));
        evaluate("(begin (sync-frames!) #t)", "#t");
        target.roundtrip();
        check_rectangle(&target, &rectangle(edge, size));
        evaluate(
            &format!(
                "(begin (set-float-geometry! {} '(160 120 220 180)) (sync-frames!) #t)",
                target.window_id
            ),
            "#t",
        );
        target.roundtrip();
        let latest = target.configures.borrow().last().unwrap().clone();
        target.commit(&latest, None);
        check_rectangle(&target, &[160, 120, 220, 180]);
    }
    println!("ok - delayed resize commits preserve anchored edges and final policy geometry");
    println!("ok - client-chosen resize sizes survive relayout without stale positioning");
}
fn resize_lock() {
    let mut target = Resize::new("resize interrupted by lock");
    target.start_resize(5);
    let start = target.configures.borrow().len();
    evaluate("(wm-warp-pointer 220 190)", "#t");
    let config = target.configured(true, start);
    target.commit(&config, None);
    let start = target.configures.borrow().len();
    let mut owner = Client::new("resize cancellation lock owner");
    let lock = owner.lock(true);
    owner.lock_surface(lock, true);
    let final_config = target.configured(false, start);
    target.commit(&final_config, None);
    target.send(target.virtual_pointer, 2, &words(&[0, 272, 0]));
    target.roundtrip();
    owner.send(lock, 2, &[]);
    owner.roundtrip();
    locked(false);
    evaluate("(begin (sync-frames!) #t)", "#t");
    target.roundtrip();
    check_rectangle(&target, &[120, 110, 280, 240]);
    assert!(
        target
            .configures
            .borrow()
            .iter()
            .filter(|c| c.serial >= final_config.serial)
            .all(|c| !c.states.contains(&3))
    );
    println!(
        "ok - lock cancellation sends a final resize configure and preserves committed geometry"
    );
}
fn typing(first: &mut Input, second: &mut Input) {
    for (operation, released) in [
        ("wm-send-string \"AAAA\" 0", vec![30, 42]),
        ("wm-send-key 4 \"v\"", vec![29]),
        ("wm-send-string \"🦀\" 0", vec![47, 29]),
    ] {
        first.focus();
        second.roundtrip();
        let start = second.events.len();
        evaluate(
            &format!(
                "(begin ({operation}) (rust-call 'wm-focus-window {}) #t)",
                second.window_id
            ),
            "#t",
        );
        for code in released {
            second.key(code, 0, start);
        }
        assert!(
            second
                .keyboard_events(start, &[3])
                .iter()
                .all(|e| e.word(3) == 0)
        );
        assert_eq!(second.modifiers(), [0; 3]);
        let start = second.events.len();
        evaluate("(wm-send-string \"xyz\" 0)", "#t");
        second.key(44, 0, start);
        assert_eq!(second.modifiers(), [0; 3]);
    }
    println!("ok - focus cancellation releases Shift, Ctrl and clipboard-fallback keys");
    first.focus();
    let start = first.events.len();
    command("xdotool", &["keydown", "Shift_L"]);
    first.key(42, 1, start);
    second.roundtrip();
    let start = second.events.len();
    evaluate(
        &format!(
            "(begin (wm-send-string \"A\" 0) (rust-call 'wm-focus-window {}) #t)",
            second.window_id
        ),
        "#t",
    );
    second.key(30, 0, start);
    assert_ne!(
        second.modifiers()[0] & 1,
        0,
        "physical Shift released by cancellation"
    );
    command("xdotool", &["keyup", "Shift_L"]);
    second.key(42, 0, start);
    assert_eq!(second.modifiers(), [0; 3]);
    println!("ok - synthetic typing borrows an already held physical modifier");
}
fn pointer(first: &mut Input, second: &mut Input) {
    for disconnect in [false, true] {
        second.point();
        first.point();
        let mut producer = Input::new("virtual pointer producer");
        let pointer = producer.virtual_pointer();
        let start = first.events.len();
        producer.send(pointer, 2, &words(&[0, 272, 1]));
        first.button(272, 1, start);
        if disconnect {
            drop(producer);
        } else {
            producer.send(pointer, 8, &[]);
            producer.roundtrip();
        }
        first.button(272, 0, start);
        second.point();
    }
    println!("ok - virtual pointer destroy and disconnect release buttons and implicit grabs");
    first.point();
    let mut one = Input::new("first pointer");
    let mut two = Input::new("second pointer");
    let vp1 = one.virtual_pointer();
    let vp2 = two.virtual_pointer();
    let start = first.events.len();
    for (c, v) in [(&mut one, vp1), (&mut two, vp2)] {
        c.send(v, 2, &words(&[0, 272, 1]));
        c.roundtrip();
    }
    one.send(vp1, 8, &[]);
    one.roundtrip();
    first.roundtrip();
    assert_eq!(first.buttons(start), [(272, 1)]);
    two.send(vp2, 8, &[]);
    two.roundtrip();
    first.button(272, 0, start);
    assert_eq!(first.buttons(start), [(272, 1), (272, 0)]);
    second.point();
    println!("ok - shared virtual buttons release only when the last owner is gone");
    let seat = first.bind("wl_seat", 8);
    let modern = first.alloc();
    first.send(seat, 0, &words(&[modern]));
    first.roundtrip();
    first.point();
    let mut producer = Input::new("extreme scroll producer");
    let pointer = producer.virtual_pointer();
    let limit = (i32::MAX - 119) / 120;
    for axis in [0, 1] {
        for steps in [i32::MAX, i32::MAX, i32::MIN, i32::MIN, -2, 1] {
            let start = first.events.len();
            producer.send(pointer, 7, &words(&[0, axis, 256, steps as u32]));
            producer.send(pointer, 4, &[]);
            let bounded = steps.clamp(-limit, limit);
            for (object, opcode, expected) in
                [(first.pointer, 8, bounded), (modern, 9, bounded * 120)]
            {
                first.wait(start, |e| {
                    e.is(object, opcode) && e.word(0) == axis && e.word(1) as i32 == expected
                });
            }
        }
    }
    println!("ok - extreme scroll stays bounded for legacy and v8 pointers");
}
fn lock(first: &mut Input, second: &mut Input) {
    second.point();
    first.point();
    first.focus();
    let mut producer = Input::new("pointer held across lock");
    let pointer = producer.virtual_pointer();
    let start = first.events.len();
    producer.send(pointer, 2, &words(&[0, 272, 1]));
    first.button(272, 1, start);
    evaluate(
        r#"(begin (wm-send-string (make-string 80 #\A) 200) (wm-click 1 10) #t)"#,
        "#t",
    );
    let mut owner = Input::new("input lifecycle lock owner");
    let lock = owner.lock(true);
    owner.lock_surface(lock, true);
    owner.c.event(owner.c.keyboard, 1);
    first.roundtrip();
    second.roundtrip();
    let starts = [first.events.len(), second.events.len()];
    let position = ipc("(rust-call 'wm-pointer-position)");
    for operation in [
        "wm-click 1",
        "wm-send-string \"secret\" 0",
        "wm-send-key 4 \"v\"",
        "wm-paste",
        "wm-scroll 1 2",
        "wm-warp-pointer 900 500",
        "wm-warp-pointer-relative 20 20",
    ] {
        evaluate(&format!("(rust-call '{operation})"), "#f");
    }
    assert_eq!(owner.modifiers(), [0; 3]);
    producer.send(pointer, 2, &words(&[0, 272, 0]));
    producer.roundtrip();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(ipc("(rust-call 'wm-pointer-position)"), position);
    for (c, from) in [&mut *first, &mut *second].into_iter().zip(starts) {
        c.roundtrip();
        assert!(c.keyboard_events(from, &[1, 3]).is_empty() && c.buttons(from).is_empty());
    }
    inject_key(&mut owner, &mut [first, second]);
    owner.send(lock, 2, &[]);
    owner.roundtrip();
    locked(false);
    second.point();
    let start = second.events.len();
    producer.send_batch(&[
        (pointer, 2, words(&[0, 273, 1])),
        (pointer, 2, words(&[0, 273, 0])),
    ]);
    second.button(273, 0, start);
    first.point();
    first.roundtrip();
    second.roundtrip();
    let starts = [first.events.len(), second.events.len()];
    std::thread::sleep(Duration::from_millis(1500));
    for (c, from) in [&mut *first, &mut *second].into_iter().zip(starts) {
        c.roundtrip();
        assert!(c.buttons(from).is_empty() && c.keyboard_events(from, &[3]).is_empty());
    }
    println!("ok - lock rejects new synthetic input, cancels queues and releases held buttons");
    first.point();
    let id = token(&ipc(&format!(
        "(wm-drop-text {} 180 \"cancelled drag\")",
        first.x
    )));
    let mut drag_owner = Client::new("lock during drag");
    let drag_lock = drag_owner.lock(true);
    evaluate(
        &format!("(rust-call 'wm-automation-status {id})"),
        "(drop-text cancelled)",
    );
    drag_owner.send(drag_lock, 2, &[]);
    drag_owner.roundtrip();
    locked(false);
    let position = ipc("(rust-call 'wm-pointer-position)");
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(37));
        assert_eq!(ipc("(rust-call 'wm-pointer-position)"), position);
    }
    println!("ok - cancelled drag timers do not resume after unlock");
    let mut dying = Input::new("drag target that disconnects");
    dying.mapped(400);
    let id = token(&ipc(&format!(
        "(wm-drop-text {} 180 \"cancelled target\")",
        dying.x
    )));
    drop(dying);
    until(
        || ipc(&format!("(rust-call 'wm-automation-status {id})")) == "(ok (drop-text cancelled))",
        "destroyed drag target was not cancelled",
    );
    std::thread::sleep(Duration::from_millis(70));
    second.point();
    let start = second.events.len();
    producer.send_batch(&[
        (pointer, 2, words(&[0, 273, 1])),
        (pointer, 2, words(&[0, 273, 0])),
    ]);
    second.button(273, 0, start);
    first.point();
    println!("ok - disappearing drag targets release the automation button");
}
fn reject_drops(first: &mut Input, second: &Input, payload: &str) {
    let position = ipc("(wm-pointer-position)");
    let focus = ipc("(focused-window-id)");
    first.roundtrip();
    let start = first.events.len();
    for (operation, argument) in [
        ("drop-text", "\"must not arrive\"".into()),
        ("drop-files", format!("(list {payload:?})")),
    ] {
        let id = token(&ipc(&format!(
            "(wm-{operation} {} 180 {argument})",
            second.x
        )));
        evaluate(
            &format!("(wm-automation-status {id})"),
            &format!("({operation} cancelled)"),
        );
        assert_eq!(ipc("(wm-pointer-position)"), position);
        assert_eq!(ipc("(focused-window-id)"), focus);
    }
    first.roundtrip();
    assert!(first.buttons(start).is_empty());
}
fn geometry(c: &Input) -> Vec<i32> {
    numbers(&ipc(&format!("(wm-window-geometry {})", c.window_id)))
}
fn drag(first: &mut Input, second: &mut Input) {
    let mut producer = Input::new("drag button producer");
    let pointer = producer.virtual_pointer();
    let payload = format!("{}/drop-payload.txt", env::var("XDG_RUNTIME_DIR").unwrap());
    std::fs::write(&payload, "file drop regression\n").unwrap();
    for button in [272, 273] {
        first.point();
        let start = first.events.len();
        producer.send(pointer, 2, &words(&[0, button, 1]));
        first.button(button, 1, start);
        reject_drops(first, second, &payload);
        producer.send(pointer, 2, &words(&[0, button, 0]));
        first.button(button, 0, start);
        assert_eq!(first.buttons(start), [(button, 1), (button, 0)]);
        second.point();
    }
    println!("ok - automation drops preserve held left and right clicks");
    for button in [272, 273] {
        first.point();
        first.focus();
        let mut expected = geometry(first);
        let start = first.events.len();
        command("xdotool", &["keydown", "Super_L"]);
        first.key(125, 1, start);
        producer.send(pointer, 2, &words(&[0, button, 1]));
        producer.roundtrip();
        command("xdotool", &["keyup", "Super_L"]);
        first.key(125, 0, start);
        reject_drops(first, second, &payload);
        evaluate(&format!("(wm-warp-pointer {} 200)", first.x + 20), "#t");
        let offset = if button == 272 { 0 } else { 2 };
        expected[offset] += 20;
        expected[offset + 1] += 20;
        until(
            || {
                first.roundtrip();
                geometry(first) == expected
            },
            "move/resize grab stopped working",
        );
        producer.send(pointer, 2, &words(&[0, button, 0]));
        producer.roundtrip();
        second.point();
    }
    println!("ok - automation drops preserve working compositor move/resize grabs");
    let mut devices = Vec::new();
    for target in [&mut *first, &mut *second] {
        let manager = target.bind("wl_data_device_manager", 3);
        let seat = target.bind("wl_seat", 5);
        let device = target.alloc();
        target.send(manager, 1, &words(&[device, seat]));
        target.roundtrip();
        devices.push((manager, device));
    }
    let (manager, origin) = devices[0];
    let target_device = devices[1].1;
    first.point();
    let start = first.events.len();
    producer.send(pointer, 2, &words(&[0, 272, 1]));
    let press = first.button(272, 1, start);
    let source = first.alloc();
    first.send(manager, 0, &words(&[source]));
    first.send(source, 0, &string("text/plain;charset=utf-8"));
    first.send(source, 2, &words(&[1]));
    first.send(
        origin,
        0,
        &words(&[source, first.surface, 0, press.word(0)]),
    );
    first.roundtrip();
    evaluate(&format!("(wm-warp-pointer {} 180)", first.x + 1), "#t");
    first.wait(start, |e| e.is(origin, 1));
    reject_drops(first, second, &payload);
    first.roundtrip();
    assert!(!first.events.iter().any(|e| e.is(source, 2)));
    let start = second.events.len();
    evaluate(&format!("(wm-warp-pointer {} 180)", second.x), "#t");
    let enter = second.wait(start, |e| e.is(target_device, 1));
    let offer = enter.word(4);
    assert_eq!(enter.word(1), second.surface);
    assert_ne!(offer, 0);
    second.send(
        offer,
        0,
        &[words(&[enter.word(0)]), string("text/plain;charset=utf-8")].concat(),
    );
    second.send(offer, 4, &words(&[1, 1]));
    second.roundtrip();
    producer.send(pointer, 2, &words(&[0, 272, 0]));
    second.wait(start, |e| e.is(target_device, 4));
    first.event(source, 3);
    second.send(offer, 3, &[]);
    first.event(source, 4);
    assert!(!first.events.iter().any(|e| e.is(source, 2)));
    println!("ok - rejected automation leaves client drag negotiation and completion intact");
    second.point();
    let start = second.events.len();
    let id = token(&ipc(&format!(
        "(wm-drop-text {} 180 \"idle drop\")",
        second.x
    )));
    let enter = second.wait(start, |e| e.is(target_device, 1));
    let offer = enter.word(4);
    second.send(
        offer,
        0,
        &[words(&[enter.word(0)]), string("text/plain;charset=utf-8")].concat(),
    );
    second.send(offer, 4, &words(&[1, 1]));
    second.wait(start, |e| e.is(target_device, 4));
    second.send(offer, 3, &[]);
    second.roundtrip();
    evaluate(
        &format!("(wm-automation-status {id})"),
        "(drop-text accepted)",
    );
    println!("ok - automation drops succeed once the pointer is idle");
}
pub fn run() {
    struct ReleaseModifiers;
    impl Drop for ReleaseModifiers {
        fn drop(&mut self) {
            command("xdotool", &["keyup", "Shift_L", "Control_L", "Super_L"]);
        }
    }
    let _release = ReleaseModifiers;
    locked(false);
    let mut first = Input::new("first input target");
    let mut second = Input::new("second input target");
    first.mapped(100);
    second.mapped(700);
    for name in env::var("MINDE_INPUT_LIFECYCLE_CASES")
        .unwrap_or_else(|_| "typing pointer lock drag resize resize-lock".into())
        .split_whitespace()
    {
        match name {
            "typing" => typing(&mut first, &mut second),
            "pointer" => pointer(&mut first, &mut second),
            "lock" => lock(&mut first, &mut second),
            "drag" => drag(&mut first, &mut second),
            "resize" => resize(),
            "resize-lock" => resize_lock(),
            _ => panic!("unknown input scenario {name}"),
        }
    }
    println!("input-lifecycle: all checks passed");
}
