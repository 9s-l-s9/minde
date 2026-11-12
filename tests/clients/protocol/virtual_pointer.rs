// SPDX-License-Identifier: GPL-3.0-or-later
//! Virtual-pointer output hints, bounds, scroll frames and protocol errors.
use super::{input::Input, outputs::Outputs, wire::*};

fn pointer(client: &mut Client, version: u32, output: Option<u32>) -> u32 {
    let manager = client.bind("zwlr_virtual_pointer_manager_v1", version);
    let pointer = client.alloc();
    if let Some(output) = output {
        client.send(manager, 2, &words(&[0, output, pointer]));
    } else {
        client.send(manager, 0, &words(&[0, pointer]));
    }
    client.roundtrip();
    pointer
}
fn position() -> Vec<f64> {
    ipc("(wm-pointer-position)")
        .strip_prefix("(ok (")
        .unwrap()
        .strip_suffix("))")
        .unwrap()
        .split_whitespace()
        .map(|value| value.parse().unwrap())
        .collect()
}
fn at(x: f64, y: f64) {
    let actual = position();
    assert!(
        (actual[0] - x).abs() < 0.01 && (actual[1] - y).abs() < 0.01,
        "pointer at {actual:?}, expected {x}, {y}"
    );
}
fn absolute(client: &mut Client, pointer: u32, values: &[u32; 4]) {
    client.send(
        pointer,
        1,
        &words(&[0, values[0], values[1], values[2], values[3]]),
    );
    client.roundtrip();
}
fn bounds(client: &mut Client, pointer: u32, origin: [f64; 2], size: [f64; 2]) {
    absolute(client, pointer, &[50, 50, 100, 100]);
    at(origin[0] + size[0] / 2.0, origin[1] + size[1] / 2.0);
    for extents in [[1, 1, 0, 1], [1, 1, 1, 0], [0, 0, 0, 0]] {
        absolute(client, pointer, &extents);
        at(origin[0] + size[0] / 2.0, origin[1] + size[1] / 2.0);
    }
    absolute(client, pointer, &[0, 0, 100, 100]);
    at(origin[0], origin[1]);
    for values in [
        [100, 100, 100, 100],
        [1000, 1000, 1, 1],
        [u32::MAX, u32::MAX, 1, 1],
    ] {
        absolute(client, pointer, &values);
        // The control snapshot rounds to integer pixels. Unit tests inspect
        // the floating-point position and require strict output membership.
        at(origin[0] + size[0], origin[1] + size[1]);
    }
    client.send(
        pointer,
        0,
        &words(&[0, (-2560_i32) as u32, (-2560_i32) as u32]),
    );
    client.roundtrip();
    at(origin[0] + size[0] - 10.0, origin[1] + size[1] - 10.0);
}
fn geometry(outputs: &mut Outputs, moved: bool) {
    let (config, head) = outputs.configuration(Some(true));
    outputs.c.send(
        head,
        2,
        &words(if moved {
            &[(-600_i32) as u32, (-400_i32) as u32]
        } else {
            &[0, 0]
        }),
    );
    outputs.c.send(head, 3, &words(&[u32::from(moved)]));
    outputs
        .c
        .send(head, 4, &words(&[if moved { 512 } else { 256 }]));
    outputs.result(config, 2, 0);
}
fn scrolling() {
    let mut target = Input::new("virtual scroll target");
    target.send(target.pointer, 1, &[]);
    let seat = target.bind("wl_seat", 8);
    target.pointer = target.alloc();
    target.send(seat, 0, &words(&[target.pointer]));
    target.mapped(100);
    target.point();
    let device = pointer(&mut target, 2, None);
    for source in 0..=3 {
        let start = target.events.len();
        target.send(device, 5, &words(&[source]));
        target.send(device, 3, &words(&[7, 0, 256]));
        target.send(device, 3, &words(&[7, 1, 512]));
        target.roundtrip();
        assert!(
            !target.events[start..]
                .iter()
                .any(|e| e.object == target.pointer)
        );
        target.send(device, 4, &[]);
        target.roundtrip();
        let events: Vec<_> = target.events[start..]
            .iter()
            .filter(|e| e.object == target.pointer)
            .collect();
        assert!(events.iter().any(|e| e.opcode == 6 && e.word(0) == source));
        for axis in 0..=1 {
            assert!(
                events
                    .iter()
                    .any(|e| e.opcode == 4 && e.word(1) == axis && e.word(2) == (axis + 1) * 256)
            );
        }
        assert_eq!(events.iter().filter(|e| e.opcode == 5).count(), 1);
        let start = target.events.len();
        target.send(device, 4, &[]);
        target.roundtrip();
        assert!(
            !target.events[start..]
                .iter()
                .any(|e| e.object == target.pointer)
        );
    }
    let start = target.events.len();
    target.send(device, 5, &words(&[1]));
    for axis in 0..=1 {
        target.send(device, 6, &words(&[8, axis]));
    }
    target.send(device, 4, &[]);
    target.roundtrip();
    for axis in 0..=1 {
        assert!(
            target.events[start..]
                .iter()
                .any(|e| e.is(target.pointer, 7) && e.word(1) == axis)
        );
    }
    let start = target.events.len();
    target.send(device, 5, &words(&[0]));
    target.send(device, 7, &words(&[9, 0, 768, 2]));
    target.send(device, 4, &[]);
    target.roundtrip();
    assert!(
        target.events[start..]
            .iter()
            .any(|e| e.is(target.pointer, 9) && e.word(0) == 0 && e.word(1) == 240)
    );
    drop(target);
    until(
        || ipc("(all-window-ids)") == "(ok ())",
        "scroll target did not unmap",
    );
    println!(
        "ok - valid scroll sources, both axes, stops and discrete steps retain frame boundaries"
    );
}
pub fn run() {
    let mut client = Client::new("virtual pointer coordinates");
    let output = client.bind("wl_output", 3);
    let devices = [
        pointer(&mut client, 1, None),
        pointer(&mut client, 2, Some(0)),
        pointer(&mut client, 2, Some(output)),
    ];
    client.send(output, 0, &[]); // wl_output.release must not lose the association.
    client.roundtrip();
    for device in devices {
        bounds(&mut client, device, [0.0, 0.0], [1280.0, 800.0]);
    }
    let mut outputs = Outputs::new("virtual pointer output geometry");
    geometry(&mut outputs, true);
    for device in devices {
        bounds(&mut client, device, [-600.0, -400.0], [400.0, 640.0]);
    }
    geometry(&mut outputs, false);
    println!(
        "ok - absolute extents stay on the current output across scale, rotation and negative origins"
    );
    scrolling();
    println!("virtual-pointer: all checks passed");
}
pub fn errors() {
    for version in [1, 2] {
        for bad in [99, u32::MAX] {
            for (opcode, arguments, code) in [
                (3, vec![0, bad, 256], 0),
                (6, vec![0, bad], 0),
                (7, vec![0, bad, 256, 1], 0),
                (5, vec![bad], 1),
            ] {
                let mut client = Client::new("invalid virtual pointer scroll enum");
                let device = pointer(&mut client, version, None);
                client.send(device, opcode, &words(&arguments));
                client.expect_error(device, code);
                Client::new("healthy client after virtual pointer error").roundtrip();
            }
        }
    }
    println!(
        "ok - v1/v2 invalid axis, stop, discrete and source requests report the exact protocol error"
    );
}
