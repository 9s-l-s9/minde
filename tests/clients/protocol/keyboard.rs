// SPDX-License-Identifier: GPL-3.0-or-later
//! Press/release ownership at the actual native keyboard filter boundary.
use super::{
    input::{Input, evaluate, token},
    wire::*,
};
use std::time::Duration;

fn key(action: &str, name: &str) {
    command("xdotool", &[action, name]);
}

fn keys(target: &mut Input, start: usize) -> Vec<(u32, u32)> {
    // An ordinary sentinel key orders preceding XTest input before the
    // assertion. A Wayland sync cannot order the separate X connection.
    key("key", "F12");
    target.key(88, 0, start);
    target
        .keyboard_events(start, &[3])
        .iter()
        .filter(|event| event.word(2) != 88)
        .map(|event| (event.word(2), event.word(3)))
        .collect()
}

fn enter_keys(target: &Input) -> Vec<u32> {
    let events = target.keyboard_events(0, &[1]);
    let event = events.last().unwrap();
    event.data[12..12 + event.word(2) as usize]
        .chunks_exact(4)
        .map(|bytes| u32::from_ne_bytes(bytes.try_into().unwrap()))
        .collect()
}

fn shortcut_down() {
    let before = token(&ipc("keyboard-filter-count"));
    key("keydown", "F8");
    until(
        || token(&ipc("keyboard-filter-count")) > before,
        "shortcut press not consumed",
    );
}

fn prompt_and_menu(target: &mut Input) {
    for (open, close) in [
        ("(read-one-line \"key test: \" (lambda (_) #t))", "Return"),
        (
            "(select-from-menu '(\"one\" \"two\") (lambda (_) #t))",
            "KP_Enter",
        ),
    ] {
        evaluate(&format!("(begin {open} #t)"), "#t");
        let start = target.events.len();
        key("keydown", "Shift_L");
        target.key(42, 1, start);
        assert_eq!(
            target.modifiers()[0] & 1,
            1,
            "Shift was hidden by the modal UI"
        );
        key("key", close);
        until(
            || ipc("(or (input-active?) (menu-active?))") == "(ok #f)",
            "modal submit did not close",
        );
        key("key", "a");
        target.key(30, 1, start);
        assert_eq!(
            target.modifiers()[0] & 1,
            1,
            "first client letter lost held Shift"
        );
        key("keyup", "Shift_L");
        assert_eq!(keys(target, start), [(42, 1), (30, 1), (30, 0), (42, 0)]);
        assert_eq!(target.modifiers(), [0; 3]);
    }
    println!("ok - prompt/menu modifiers stay synchronized; keypad Enter selects and closes");

    let start = target.events.len();
    key("keydown", "a");
    target.key(30, 1, start);
    evaluate(
        "(begin (read-one-line \"held: \" (lambda (_) #t)) #t)",
        "#t",
    );
    key("keyup", "a");
    target.key(30, 0, start);
    key("key", "Escape");
    assert_eq!(keys(target, start), [(30, 1), (30, 0)]);

    evaluate(
        "(begin (define keyboard-filter-submit #f)
      (read-one-line \"held: \" (lambda (text) (set! keyboard-filter-submit text))) #t)",
        "#t",
    );
    let start = target.events.len();
    key("keydown", "b");
    key("key", "Return");
    until(
        || ipc("keyboard-filter-submit") == "(ok \"b\")",
        "prompt text not submitted",
    );
    key("keyup", "b");
    assert!(keys(target, start).is_empty());
    println!("ok - prompt transitions preserve both forwarded and consumed releases");
}

fn focus_changes(first: &mut Input, second: &mut Input) {
    first.focus();
    shortcut_down();
    second.focus();
    assert!(!enter_keys(second).contains(&66));
    let start = second.events.len();
    key("keyup", "F8");
    assert!(keys(second, start).is_empty());

    first.focus();
    let start = first.events.len();
    key("keydown", "a");
    first.key(30, 1, start);
    second.focus();
    assert!(enter_keys(second).contains(&30));
    let start = second.events.len();
    key("keyup", "a");
    assert_eq!(keys(second, start), [(30, 0)]);
    println!("ok - focus changes transfer forwarded held keys without exposing consumed keys");
}

fn synthetic_chords(first: &mut Input, second: &mut Input) {
    // Inspect modifiers at both edges of the target key, then transfer focus
    // while the source modifier remains physically held. A shortcut must
    // neither inherit that modifier nor release it on behalf of the user.
    for (name, code, held, wanted) in [
        ("Control_L", 29, 4, 0),
        ("Control_R", 97, 4, 1),
        ("Shift_L", 42, 1, 4),
        ("Alt_L", 56, 8, 64),
        ("Super_L", 125, 64, 8),
    ] {
        first.focus();
        let start = first.events.len();
        key("keydown", name);
        first.key(code, 1, start);
        let before = first.modifiers();
        assert_eq!(before[0], held);
        let start = first.events.len();
        evaluate(&format!("(wm-send-key {wanted} \"Down\")"), "#t");
        // A second queued stroke orders the first stroke's modifier releases.
        evaluate("(wm-send-key 0 \"F11\")", "#t");
        first.key(87, 0, start);
        let after = first.modifiers();
        let mut modifiers = before;
        let mut edges = Vec::new();
        for event in first.keyboard_events(start, &[3, 4]) {
            if event.opcode == 4 {
                modifiers = [event.word(1), event.word(2), event.word(3)];
            } else if event.word(2) == 108 {
                assert_eq!(modifiers, [wanted, 0, before[2]], "held {name}");
                edges.push(event.word(3));
            } else if event.word(2) == code {
                panic!("synthetic chord changed physical {name}");
            }
        }
        assert_eq!(edges, [1, 0]);
        assert_eq!(after, before, "physical {name} not restored");
        second.focus();
        assert!(enter_keys(second).contains(&code));
        assert_eq!(second.modifiers(), before);
        let start = second.events.len();
        key("keyup", name);
        second.key(code, 0, start);
        assert_eq!(second.modifiers(), [0; 3]);
    }
    println!("ok - synthetic chords use requested modifiers and preserve held keys across focus");
}

fn inhibitors(target: &mut Input) {
    target.focus();
    shortcut_down();
    let manager = target.bind("zwp_keyboard_shortcuts_inhibit_manager_v1", 1);
    let seat = target.bind("wl_seat", 1);
    let inhibitor = target.alloc();
    let surface = target.surface;
    target.send(manager, 1, &words(&[inhibitor, surface, seat]));
    target.event(inhibitor, 0);
    let start = target.events.len();
    key("keyup", "F8");
    assert!(keys(target, start).is_empty());
    let start = target.events.len();
    key("keydown", "F8");
    target.key(66, 1, start);
    target.send(inhibitor, 0, &[]);
    target.roundtrip();
    key("keyup", "F8");
    assert_eq!(keys(target, start), [(66, 1), (66, 0)]);
    let start = target.events.len();
    key("key", "F8");
    assert!(keys(target, start).is_empty());
    println!("ok - inhibitor activation and removal do not change held-key delivery");
}

fn lock_transition(target: &mut Input) {
    target.focus();
    shortcut_down();
    let mut owner = Input::new("keyboard filter lock owner");
    let lock = owner.lock(true);
    owner.lock_surface(lock, true);
    owner.c.event(owner.c.keyboard, 1);
    assert!(!enter_keys(&owner).contains(&66));
    let start = owner.events.len();
    key("keyup", "F8");
    assert!(keys(&mut owner, start).is_empty());
    let start = owner.events.len();
    key("keydown", "F8");
    owner.key(66, 1, start);
    owner.send(lock, 2, &[]);
    owner.roundtrip();
    locked(false);
    target.focus();
    assert!(enter_keys(target).contains(&66));
    let start = target.events.len();
    key("keyup", "F8");
    assert_eq!(keys(target, start), [(66, 0)]);
    let start = target.events.len();
    key("key", "F8");
    assert!(keys(target, start).is_empty());
    println!("ok - consumed desktop keys stay hidden on lock; locker keys release after unlock");
}

fn repeat(target: &mut Input) {
    evaluate(
        "(begin
      (define keyboard-filter-handler wm-handle-key)
      (define keyboard-filter-repeats 0)
      (set! wm-handle-key (lambda args
        (when (equal? (list-ref args 2) \"a\")
          (set! keyboard-filter-repeats (+ keyboard-filter-repeats 1)))
        (apply keyboard-filter-handler args)))
      (read-one-line \"repeat: \" (lambda (_) #t)) #t)",
        "#t",
    );
    let start = target.events.len();
    key("keydown", "a");
    until(
        || token(&ipc("keyboard-filter-repeats")) >= 2,
        "compositor repeat did not start",
    );
    key("keyup", "a");
    key("key", "Escape");
    assert!(keys(target, start).is_empty());
    let count = ipc("keyboard-filter-repeats");
    std::thread::sleep(Duration::from_millis(160));
    assert_eq!(ipc("keyboard-filter-repeats"), count);
    evaluate(
        "(begin (set! wm-handle-key keyboard-filter-handler) #t)",
        "#t",
    );
    println!("ok - consumed repeats stop on release without leaking a release event");
}

pub fn run() {
    locked(false);
    evaluate(
        "(begin (cancel-keymap!) (set-prefix-key! '() \"Print\")
      (define keyboard-filter-count 0)
      (define keyboard-filter-bindings (copy-hash-table %keybindings))
      (define keyboard-filter-docs (copy-hash-table %global-binding-docs))
      (bind-key! '() \"F8\" (lambda () (set! keyboard-filter-count (+ keyboard-filter-count 1))))
      (bind-key! '() \"F9\" (lambda () (error \"expected keyboard regression error\"))) #t)",
        "#t",
    );
    let mut first = Input::new("first keyboard filter target");
    let mut second = Input::new("second keyboard filter target");
    first.mapped(100);
    second.mapped(700);
    first.focus();
    let start = first.events.len();
    for name in ["Print", "Escape", "F8", "F9"] {
        key("key", name);
    }
    assert!(
        keys(&mut first, start).is_empty(),
        "consumed key leaked to client"
    );
    println!("ok - prefix, cancelled maps and failed shortcuts consume both press and release");
    prompt_and_menu(&mut first);
    synthetic_chords(&mut first, &mut second);
    focus_changes(&mut first, &mut second);
    inhibitors(&mut second);
    lock_transition(&mut first);
    repeat(&mut first);
    evaluate(
        "(begin (set! %keybindings keyboard-filter-bindings)
      (set! %global-binding-docs keyboard-filter-docs) #t)",
        "#t",
    );
    println!("keyboard-filter: all checks passed");
}
