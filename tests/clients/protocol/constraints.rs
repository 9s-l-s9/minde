// SPDX-License-Identifier: GPL-3.0-or-later
//! Pointer focus, constraint resource ownership and committed unlock hints.
use super::{
    input::{Input, evaluate},
    wire::*,
};

fn position() -> [f64; 2] {
    let reply = ipc("(wm-pointer-position)");
    let values: Vec<f64> = reply
        .strip_prefix("(ok (")
        .unwrap()
        .strip_suffix("))")
        .unwrap()
        .split_whitespace()
        .map(|value| value.parse().unwrap())
        .collect();
    [values[0], values[1]]
}
fn at(expected: [f64; 2]) {
    let actual = position();
    assert!(
        (actual[0] - expected[0]).abs() < 0.01 && (actual[1] - expected[1]).abs() < 0.01,
        "pointer at {actual:?}, expected {expected:?}"
    );
}
fn motion(client: &mut Input, pointer: u32, absolute: bool, x: i32, y: i32) {
    if absolute {
        client.send(pointer, 1, &words(&[0, x as u32, y as u32, 1280, 800]));
    } else {
        let old = position();
        client.send(
            pointer,
            0,
            &words(&[
                0,
                ((x as f64 - old[0]) * 256.0) as i32 as u32,
                ((y as f64 - old[1]) * 256.0) as i32 as u32,
            ]),
        );
    }
    client.roundtrip();
}
fn constraint(client: &mut Input, locked: bool, oneshot: bool) -> u32 {
    let manager = client.bind("zwp_pointer_constraints_v1", 1);
    let id = client.alloc();
    client.send(
        manager,
        if locked { 1 } else { 2 },
        &words(&[
            id,
            client.surface,
            client.pointer,
            0,
            if oneshot { 1 } else { 2 },
        ]),
    );
    client.roundtrip();
    id
}
fn destroy(client: &mut Input, id: u32) {
    client.send(id, 0, &[]);
    client.roundtrip();
}
fn hint(client: &mut Input, id: u32, x: i32, y: i32, commit: bool) {
    client.send(id, 1, &words(&[(x * 256) as u32, (y * 256) as u32]));
    if commit {
        client.send(client.surface, 6, &[]);
    }
    client.roundtrip();
}
fn region(client: &mut Input, x: i32, y: i32, width: i32, height: i32) -> u32 {
    let compositor = client.bind("wl_compositor", 1);
    let id = client.alloc();
    client.send(compositor, 1, &words(&[id]));
    client.send(
        id,
        1,
        &words(&[x as u32, y as u32, width as u32, height as u32]),
    );
    id
}
fn no_activation(client: &mut Input, id: u32) {
    client.roundtrip();
    assert!(
        !client.events.iter().any(|event| event.is(id, 0)),
        "unfocused constraint activated"
    );
}
fn button(client: &mut Input, pointer: u32, pressed: bool) {
    client.send(pointer, 2, &words(&[0, 272, u32::from(pressed)]));
    client.roundtrip();
}
fn reset(client: &mut Input, x: i32) {
    evaluate(
        &format!(
            "(begin (set-float-geometry! {} (list {x} 100 300 250)) (sync-frames!) #t)",
            client.window_id
        ),
        "#t",
    );
    until(
        || {
            client.roundtrip();
            ipc(&format!("(wm-window-geometry {})", client.window_id))
                == format!("(ok ({x} 100 300 250))")
        },
        "float reset did not reach the requested geometry",
    );
}

fn implicit_grabs(first: &mut Input, second: &mut Input, pointer: u32) {
    for absolute in [false, true] {
        for locked in [false, true] {
            for create_during in [false, true] {
                first.point();
                let mut id = if create_during {
                    0
                } else {
                    constraint(second, locked, false)
                };
                button(first, pointer, true);
                motion(first, pointer, absolute, 800, 180);
                at([800.0, 180.0]);
                if create_during {
                    id = constraint(second, locked, false);
                }
                no_activation(second, id);
                motion(first, pointer, absolute, 850, 180);
                at([850.0, 180.0]);
                motion(first, pointer, absolute, 200, 180);
                button(first, pointer, false);
                second.point();
                second.event(id, 0);
                if locked {
                    motion(first, pointer, absolute, 810, 180);
                    at([800.0, 180.0]);
                }
                destroy(second, id);
            }
        }
    }
    println!("ok - relative/absolute drags never activate an unfocused lock or confinement");
}

fn compositor_grabs(first: &mut Input, pointer: u32) {
    let seat = first.bind("wl_seat", 5);
    for resize in [false, true] {
        for locked in [false, true] {
            reset(first, 100);
            first.point();
            let start = first.events.len();
            button(first, pointer, true);
            let press = first.button(272, 1, start);
            let mut args = vec![seat, press.word(0)];
            if resize {
                args.push(10);
            } // bottom-right
            first.send(
                first.toplevel_object,
                if resize { 6 } else { 5 },
                &words(&args),
            );
            first.roundtrip();
            motion(first, pointer, false, 205, 180);
            let last = first
                .events
                .iter()
                .rfind(|event| event.object == first.pointer && event.opcode <= 1)
                .unwrap();
            assert_eq!(
                last.opcode, 1,
                "compositor grab retained client pointer focus"
            );
            let id = constraint(first, locked, false);
            no_activation(first, id);
            motion(first, pointer, false, 225, 180);
            at([225.0, 180.0]);
            no_activation(first, id);
            destroy(first, id);
            button(first, pointer, false);
        }
    }
    reset(first, 100);
    println!("ok - move/resize grabs with no pointer focus reject constraint activation");
}

fn dnd_grab(first: &mut Input, second: &mut Input, pointer: u32) {
    for locked in [false, true] {
        first.point();
        let id = constraint(second, locked, false);
        let manager = first.bind("wl_data_device_manager", 3);
        let seat = first.bind("wl_seat", 5);
        let device = first.alloc();
        first.send(manager, 1, &words(&[device, seat]));
        let start = first.events.len();
        button(first, pointer, true);
        let press = first.button(272, 1, start);
        let source = first.alloc();
        first.send(manager, 0, &words(&[source]));
        first.send(source, 0, &string("text/plain;charset=utf-8"));
        first.send(source, 2, &words(&[1]));
        first.send(
            device,
            0,
            &words(&[source, first.surface, 0, press.word(0)]),
        );
        first.roundtrip();
        motion(first, pointer, false, 800, 180);
        no_activation(second, id);
        motion(first, pointer, false, 850, 180);
        at([850.0, 180.0]);
        destroy(second, id);
        button(first, pointer, false);
        first.send(source, 1, &[]);
        first.send(device, 2, &[]);
        first.roundtrip();
    }
    println!("ok - drag-and-drop does not activate constraints on its unfocused target");
}

fn hints(first: &mut Input, second: &mut Input, pointer: u32) {
    first.point();
    let id = constraint(first, true, false);
    first.event(id, 0);
    hint(first, id, 50, 60, true);
    destroy(first, id);
    at([150.0, 160.0]);
    for (x, y, commit) in [(100000, 100000, true), (-10, 30, true), (50, 60, false)] {
        first.point();
        let id = constraint(first, true, false);
        hint(first, id, x, y, commit);
        destroy(first, id);
        at([200.0, 180.0]);
    }
    first.point();
    let id = constraint(first, true, false);
    let input = region(first, 0, 0, 300, 250);
    first.send(input, 2, &words(&[40, 50, 30, 30]));
    first.send(first.surface, 5, &words(&[input]));
    hint(first, id, 50, 60, true);
    destroy(first, id);
    at([200.0, 180.0]);
    first.send(first.surface, 5, &words(&[0]));
    first.send(first.surface, 6, &[]);
    destroy(first, input);
    println!("ok - unlock honors only committed hints inside the current surface input region");

    for locked in [false, true] {
        first.point();
        let owner = constraint(first, true, false);
        hint(first, owner, 50, 60, true);
        let unrelated = constraint(second, locked, false);
        destroy(second, unrelated);
        destroy(first, owner);
        at([150.0, 160.0]);
    }
    println!("ok - removing another client's lock or confinement preserves the owner's hint");

    second.point();
    let inactive = constraint(first, true, false);
    hint(first, inactive, 50, 60, true);
    first.point();
    first.event(inactive, 0);
    destroy(first, inactive);
    at([200.0, 180.0]);
    first.point();
    let inactive = constraint(first, true, false);
    hint(first, inactive, 50, 60, true);
    second.point();
    first.event(inactive, 1);
    destroy(first, inactive);
    at([800.0, 180.0]);
    first.point();
    let reentered = constraint(first, true, false);
    hint(first, reentered, 50, 60, true);
    second.point();
    first.point();
    destroy(first, reentered);
    at([200.0, 180.0]);
    println!("ok - inactive hints and hints from a previous activation never warp on unlock");

    first.point();
    let old = constraint(first, true, true);
    hint(first, old, 50, 60, true);
    let excluded = region(first, 0, 0, 5, 5);
    first.send(old, 2, &words(&[excluded]));
    first.send(first.surface, 6, &[]);
    first.roundtrip();
    motion(first, pointer, false, 201, 180);
    first.event(old, 1);
    let replacement = constraint(first, true, false);
    first.event(replacement, 0);
    hint(first, old, 70, 80, true);
    first.send(old, 2, &words(&[0]));
    first.send(first.surface, 6, &[]);
    destroy(first, old);
    motion(first, pointer, false, 220, 180);
    at([201.0, 180.0]);
    destroy(first, replacement);
    at([201.0, 180.0]);
    destroy(first, excluded);
    println!("ok - retired oneshot requests and destruction cannot affect a replacement lock");

    first.point();
    let id = constraint(first, true, false);
    reset(second, 100);
    evaluate(
        &format!("(rust-call 'wm-raise-window {})", second.window_id),
        "#t",
    );
    motion(first, pointer, false, 230, 180);
    at([200.0, 180.0]);
    reset(second, 700);
    destroy(first, id);
    println!("ok - a focused lock stays effective when another window covers the parked pointer");
}

pub fn run() {
    let mut first = Input::new("constraint owner");
    let mut second = Input::new("constraint drag target");
    first.mapped(100);
    second.mapped(700);
    let pointer = first.virtual_pointer();
    implicit_grabs(&mut first, &mut second, pointer);
    compositor_grabs(&mut first, pointer);
    dnd_grab(&mut first, &mut second, pointer);
    hints(&mut first, &mut second, pointer);
    drop(first);
    drop(second);
    until(
        || ipc("(all-window-ids)") == "(ok ())",
        "constraint clients did not unmap",
    );
    println!("pointer-constraints: all checks passed");
}
