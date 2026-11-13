// SPDX-License-Identifier: GPL-3.0-or-later
//! Per-surface tearing-control ownership and protocol-level destruction.
use super::wire::*;

const MANAGER: &str = "wp_tearing_control_manager_v1";

fn surface(client: &mut Client, compositor: u32) -> u32 {
    let surface = client.alloc();
    client.send(compositor, 0, &words(&[surface]));
    surface
}

fn control(client: &mut Client, manager: u32, surface: u32) -> u32 {
    let control = client.alloc();
    client.send(manager, 1, &words(&[control, surface]));
    control
}

fn lifecycle() {
    let mut client = Client::new("tearing-control lifecycle");
    let compositor = client.bind("wl_compositor", 4);
    let manager = client.bind(MANAGER, 1);
    let a = surface(&mut client, compositor);
    let b = surface(&mut client, compositor);
    let mut first = control(&mut client, manager, a);
    let second = control(&mut client, manager, b);
    client.roundtrip();
    // Factory destruction leaves both controls usable and associated.
    client.send(manager, 0, &[]);
    let replacement = client.bind(MANAGER, 1);
    for _ in 0..3 {
        client.send(first, 0, &words(&[1])); // async hint
        client.send(second, 0, &words(&[0])); // vsync hint
        client.send(first, 1, &[]); // control.destroy
        first = control(&mut client, replacement, a);
        client.roundtrip();
    }
    // A control becomes inert if the surface is destroyed first.
    client.send(a, 0, &[]);
    client.send(first, 0, &words(&[1]));
    client.send(first, 1, &[]);
    client.send(second, 1, &[]);
    client.send(b, 0, &[]);
    client.roundtrip();
    println!("ok - separate surfaces, factory destruction, repeated recreation and inert controls");
}

fn duplicate(rebind: bool, destroy_manager: bool) {
    let mut client = Client::new("duplicate tearing control");
    let compositor = client.bind("wl_compositor", 4);
    let manager = client.bind(MANAGER, 1);
    let surface = surface(&mut client, compositor);
    control(&mut client, manager, surface);
    if destroy_manager {
        client.send(manager, 0, &[]);
    }
    let second_manager = if rebind {
        client.bind(MANAGER, 1)
    } else {
        manager
    };
    control(&mut client, second_manager, surface);
    // Error 0 belongs to the manager receiving the duplicate request.
    client.expect_error(second_manager, 0);
    Client::new("healthy client after tearing error").roundtrip();
    println!(
        "ok - duplicate control rejected on manager (rebind={rebind}, destroy={destroy_manager})"
    );
}

fn surface_id_reuse() {
    let mut client = Client::new("tearing control after surface ID reuse");
    let compositor = client.bind("wl_compositor", 4);
    let manager = client.bind(MANAGER, 1);
    let surface = surface(&mut client, compositor);
    let old = control(&mut client, manager, surface);
    client.send(surface, 0, &[]);
    client.roundtrip();
    assert!(
        client
            .events
            .iter()
            .any(|e| e.is(1, 1) && e.word(0) == surface)
    );
    client.send(compositor, 0, &words(&[surface]));
    control(&mut client, manager, surface);
    // Destroying an old, inert control must not release the new surface's slot.
    client.send(old, 0, &words(&[1]));
    client.send(old, 1, &[]);
    client.roundtrip();
    control(&mut client, manager, surface);
    client.expect_error(manager, 0);
    Client::new("healthy client after surface ID reuse").roundtrip();
    println!("ok - an inert control cannot release a new surface with a reused protocol ID");
}

pub fn run() {
    lifecycle();
    duplicate(false, false);
    duplicate(true, false);
    duplicate(true, true);
    surface_id_reuse();
    println!("tearing-control: all checks passed");
}
