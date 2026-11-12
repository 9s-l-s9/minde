// SPDX-License-Identifier: GPL-3.0-or-later
use super::{
    focus::{Panel, keyboard},
    input::{Input, evaluate},
    lock::inject_key,
    outputs::Outputs,
    wire::*,
};

pub fn unlock() {
    let mut desktop = Input::new("desktop behind exclusive layer");
    desktop.mapped(350);
    let mut panel = Panel::new("exclusive across lock", 3, 1);
    keyboard(&mut panel.input, true);
    let mut owner = Client::new("layer focus locker");
    let lock = owner.lock(true);
    owner.lock_surface(lock, true);
    inject_key(&mut owner, &mut [&mut desktop.c, &mut panel.input.c]);
    let start = desktop.events.len();
    owner.send(lock, 2, &[]);
    owner.roundtrip();
    locked(false);
    keyboard(&mut panel.input, true);
    desktop.roundtrip();
    assert!(desktop.keyboard_events(start, &[1, 3]).is_empty());
    inject_key(&mut panel.input.c, &mut [&mut desktop.c, &mut owner]);
    panel.destroy();
    inject_key(&mut desktop.c, &mut [&mut owner]);
    println!("ok - unlock restores the exclusive layer before the desktop window");
}

pub fn output_lifecycle() {
    command("sh", &["-c", "xdotool search --name Smithay windowfocus"]);
    let mut output = Outputs::new("layer output configuration");
    let mut desktop = Input::new("desktop behind removed output layer");
    desktop.mapped(350);
    let mut panel = Panel::new("layer on disabled output", 3, 1);
    let mut retained = Input::new("layer on retained disabled output binding");
    let binding = retained.bind("wl_output", 1);
    retained.roundtrip();

    let (config, _) = output.configuration(Some(false));
    output.result(config, 3, 0); // Test-only must leave the layer alive.
    let (config, head) = output.configuration(Some(true));
    output.c.send(head, 5, &words(&[1])); // Winit rejects adaptive sync.
    output.result(config, 2, 1);
    keyboard(&mut panel.input, true);
    assert!(!panel.input.events.iter().any(|e| e.is(panel.layer, 1)));
    println!("ok - tested and rejected output changes preserve layer ownership");

    let (config, _) = output.configuration(Some(false));
    output.result(config, 2, 0);
    panel.input.event(panel.layer, 1); // closed
    keyboard(&mut panel.input, false);
    inject_key(&mut desktop.c, &mut [&mut panel.input.c]);
    panel.mode(1); // A retained closed role cannot regain keyboard focus.
    keyboard(&mut panel.input, false);

    for (client, binding) in [
        (Input::new("layer without any output"), 0),
        (retained, binding),
    ] {
        let mut rejected = Panel::pending(client, 3, 1, binding);
        rejected.input.event(rejected.layer, 1);
        rejected.input.roundtrip();
        assert!(
            !rejected
                .input
                .events
                .iter()
                .any(|e| e.is(rejected.layer, 0))
        );
    }
    println!("ok - disabled and missing outputs close new layer roles promptly");

    let (config, _) = output.configuration(Some(true));
    output.result(config, 2, 0);
    panel.mode(1);
    keyboard(&mut panel.input, false);
    keyboard(&mut desktop, true);
    let mut replacement = Panel::new("replacement after output enable", 3, 1);
    inject_key(
        &mut replacement.input.c,
        &mut [&mut desktop.c, &mut panel.input.c],
    );
    replacement.destroy();
    inject_key(&mut desktop.c, &mut [&mut panel.input.c]);
    panel.destroy();
    drop(desktop);
    until(
        || ipc("(all-window-ids)") == "(ok ())",
        "layer lifecycle desktop remained mapped",
    );
    evaluate("(begin (collapse-to-one-frame!) #t)", "#t");
    println!("ok - re-enabling an output accepts fresh layers without reviving closed ones");
}
