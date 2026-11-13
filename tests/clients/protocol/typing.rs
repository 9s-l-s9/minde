// SPDX-License-Identifier: GPL-3.0-or-later
use super::{
    input::{Input, evaluate},
    wire::*,
};
use smithay::input::keyboard::xkb;
use std::{
    cell::RefCell,
    fs::File,
    io::{Read, Seek, SeekFrom},
    rc::Rc,
    time::Duration,
};

#[derive(Default)]
struct Trace {
    state: Option<xkb::State>,
    keymap: String,
    text: String,
    keys: Vec<(u32, u32)>,
    modifiers: [u32; 4],
}

fn observe(client: &mut Client, keyboard: u32) -> Rc<RefCell<Trace>> {
    let trace = Rc::new(RefCell::new(Trace::default()));
    let observed = trace.clone();
    client.handlers.insert(
        keyboard,
        Box::new(move |client, event| {
            let mut trace = observed.borrow_mut();
            match event.opcode {
                0 => {
                    assert_eq!(event.word(0), 1);
                    let mut keymap = String::new();
                    let mut file = File::from(client.received_fds.remove(0));
                    file.seek(SeekFrom::Start(0)).unwrap();
                    file.take(event.word(1) as u64)
                        .read_to_string(&mut keymap)
                        .unwrap();
                    let keymap = keymap.trim_end_matches('\0').to_owned();
                    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
                    let compiled = xkb::Keymap::new_from_string(
                        &context,
                        keymap.clone(),
                        xkb::KEYMAP_FORMAT_TEXT_V1,
                        xkb::KEYMAP_COMPILE_NO_FLAGS,
                    )
                    .unwrap();
                    trace.state = Some(xkb::State::new(&compiled));
                    trace.keymap = keymap;
                }
                3 => {
                    trace.keys.push((event.word(2), event.word(3)));
                    if event.word(3) == 1 {
                        let text = trace
                            .state
                            .as_ref()
                            .unwrap()
                            .key_get_utf8(xkb::Keycode::new(event.word(2) + 8));
                        trace.text.push_str(&text);
                    }
                }
                4 => {
                    let mods = [event.word(1), event.word(2), event.word(3), event.word(4)];
                    trace
                        .state
                        .as_mut()
                        .unwrap()
                        .update_mask(mods[0], mods[1], mods[2], 0, 0, mods[3]);
                    trace.modifiers = mods;
                }
                _ => (),
            }
        }),
    );
    trace
}

pub fn run() {
    struct ReleaseShift;
    impl Drop for ReleaseShift {
        fn drop(&mut self) {
            command("xdotool", &["keyup", "Shift_L"]);
        }
    }
    let _release = ReleaseShift;
    let mut target = Input::new("Unicode typing target");
    let keyboard = target.get_keyboard();
    let legacy = observe(&mut target, keyboard);
    target.roundtrip();
    let seat = target.bind("wl_seat", 8);
    let keyboard = target.alloc();
    let modern = observe(&mut target, keyboard);
    target.send(seat, 1, &words(&[keyboard]));
    target.roundtrip();
    target.mapped(100);
    let original = legacy.borrow().keymap.clone();
    let layouts = ipc("(wm-keyboard-layouts)");
    evaluate("(wm-set-clipboard \"untouched\")", "#t");
    // Both protocol generations must see the same complete characters and
    // restored map, including Latin-1, astral and combining codepoints.
    let text = "🦀naïve-日本語-é";
    evaluate(&format!("(wm-type \"{text}\" 0)"), "#t");
    until(
        || {
            target.roundtrip();
            modern.borrow().text == text
        },
        "Unicode text incomplete",
    );
    for trace in [&legacy, &modern] {
        let trace = trace.borrow();
        assert_eq!(trace.text, text);
        assert_eq!(trace.keymap, original);
        assert_eq!(trace.modifiers, [0; 4]);
        let unicode: Vec<_> = trace
            .keys
            .iter()
            .copied()
            .filter(|(code, _)| *code == 1)
            .collect();
        assert_eq!(unicode, [(1, 1), (1, 0)].repeat(6));
    }
    assert_eq!(ipc("(wm-keyboard-layouts)"), layouts);
    let display = format!(
        "WAYLAND_DISPLAY={}",
        std::env::var("NESTED_WAYLAND_DISPLAY").unwrap()
    );
    assert_eq!(
        command("env", &[&display, "wl-paste", "--no-newline"]),
        "untouched"
    );
    println!(
        "ok - legacy and modern keyboards decode Unicode and restore keymaps without clipboard writes"
    );

    // Locked modifiers and the nondefault layout must survive; holding a
    // physical Shift must not modify the Unicode character or be released.
    evaluate("(wm-set-keyboard-layout! 1)", "#t");
    let layouts = ipc("(wm-keyboard-layouts)");
    let start = target.events.len();
    command("xdotool", &["key", "Caps_Lock"]);
    command("xdotool", &["keydown", "Shift_L"]);
    target.key(42, 1, start);
    target.roundtrip();
    let before = legacy.borrow().modifiers;
    assert_eq!(before[3], 1, "test requires a second layout");
    assert_ne!(before[0] & 1, 0);
    assert_ne!(before[2] & 2, 0);
    for trace in [&legacy, &modern] {
        trace.borrow_mut().text.clear();
    }
    evaluate("(wm-type \"🦀ï\" 0)", "#t");
    until(
        || {
            target.roundtrip();
            modern.borrow().text == "🦀ï"
        },
        "modified Unicode text incomplete",
    );
    for trace in [&legacy, &modern] {
        let trace = trace.borrow();
        assert_eq!(trace.text, "🦀ï");
        assert_eq!(trace.keymap, original);
        assert_eq!(trace.modifiers, before);
    }
    assert_eq!(ipc("(wm-keyboard-layouts)"), layouts);
    command("xdotool", &["keyup", "Shift_L"]);
    command("xdotool", &["key", "Caps_Lock"]);
    evaluate("(wm-set-keyboard-layout! 0)", "#t");
    assert_eq!(target.modifiers(), [0; 3]);
    println!("ok - Unicode preserves physical Shift, Caps Lock and the active layout group");

    let mut second = Input::new("Unicode cancellation target");
    second.mapped(700);
    target.focus();
    legacy.borrow_mut().text.clear();
    let start = second.events.len();
    evaluate(
        &format!(
            "(begin (wm-type \"🦀🦀\" 100) (rust-call 'wm-focus-window {}))",
            second.window_id
        ),
        "#t",
    );
    std::thread::sleep(Duration::from_millis(200));
    target.roundtrip();
    second.roundtrip();
    assert_eq!(legacy.borrow().text, "🦀");
    assert_eq!(legacy.borrow().keymap, original);
    assert!(second.keyboard_events(start, &[3]).is_empty());
    assert_eq!(second.modifiers(), [0; 3]);
    println!("ok - focus cancellation discards remaining Unicode without transferring a held key");

    target.focus();
    evaluate("(wm-type (make-string 40 #\\🦀) 100)", "#t");
    let mut owner = Client::new("Unicode lock cancellation");
    let lock = owner.lock(true);
    owner.lock_surface(lock, true);
    target.roundtrip();
    let before = legacy.borrow().text.clone();
    std::thread::sleep(Duration::from_millis(250));
    target.roundtrip();
    assert_eq!(legacy.borrow().text, before);
    assert_eq!(legacy.borrow().keymap, original);
    owner.send(lock, 2, &[]);
    owner.roundtrip();
    locked(false);
    println!("ok - session locking cancels Unicode typing with the configured keymap restored");
}
