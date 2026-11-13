// SPDX-License-Identifier: GPL-3.0-or-later
use super::{
    capture::Capture,
    focus::Panel,
    input::{Input, evaluate},
    lock::HostPixels,
    outputs::Outputs,
    wire::*,
};

fn paint(panel: &mut Panel, color: u32) {
    let surface = panel.input.surface;
    panel.input.paint(surface, 280, 100, color);
    panel.input.roundtrip();
}
fn panel(name: &str, layer: u32, color: u32) -> Panel {
    let mut panel = Panel::new(name, layer, 0);
    paint(&mut panel, color);
    panel
}
fn verify(capture: &mut Capture, host: &HostPixels, target: &mut Input, color: u32) {
    // Drain configure events and wait for the buffer before hit-testing a
    // fullscreen transition; the pending configure is not the rendered state.
    target.roundtrip();
    until(
        || host.pixel(180, 70) == color,
        "live layer stacking differs from input ownership",
    );
    evaluate("(wm-warp-pointer 10 10)", "#t");
    evaluate("(wm-warp-pointer 30 30)", "#t");
    target.roundtrip();
    let enter = target
        .events
        .iter()
        .rfind(|e| e.object == target.pointer && e.opcode <= 1)
        .unwrap();
    assert_eq!(
        (enter.opcode, enter.word(1)),
        (0, target.surface),
        "visible layer must receive input"
    );
    for ext in [true, false] {
        assert_eq!(
            capture.image(ext, None).pixel(180, 70),
            color,
            "captured layer order (ext={ext})"
        );
    }
}

pub fn layers() {
    evaluate("(wm-clear-message)", "#t");
    let window = command("xdotool", &["search", "--name", "Smithay"]);
    let host = HostPixels::for_window(window.trim().parse().unwrap());
    let mut capture = Capture::new();
    let background = panel("older background", 0, 0x224466);
    let mut bottom = panel("newer bottom", 1, 0xee2244);
    verify(&mut capture, &host, &mut bottom.input, 0xee2244);
    let mut top = panel("older top", 2, 0x33aa55);
    let mut overlay = panel("newer overlay", 3, 0x4455ee);
    verify(&mut capture, &host, &mut overlay.input, 0x4455ee);
    let mut newer = panel("newest overlay", 3, 0xeedd44);
    verify(&mut capture, &host, &mut newer.input, 0xeedd44);
    paint(&mut overlay, 0x4455ee);
    verify(&mut capture, &host, &mut newer.input, 0xeedd44);
    newer.destroy();
    overlay.destroy();
    verify(&mut capture, &host, &mut top.input, 0x33aa55);
    let mut newer = panel("newest top", 2, 0x4488aa);
    verify(&mut capture, &host, &mut newer.input, 0x4488aa);
    newer.destroy();

    let mut desktop = Input::new("fullscreen behind overlay");
    desktop.mapped(100);
    evaluate("(begin (fullscreen!) #t)", "#t");
    verify(&mut capture, &host, &mut desktop, 0xff00ff);
    let mut overlay = panel("overlay above fullscreen", 3, 0x4455ee);
    verify(&mut capture, &host, &mut overlay.input, 0x4455ee);
    overlay.destroy();
    drop(desktop);
    until(
        || ipc("(all-window-ids)") == "(ok ())",
        "fullscreen scene client remained mapped",
    );
    verify(&mut capture, &host, &mut top.input, 0x33aa55);
    top.destroy();
    bottom.destroy();
    background.destroy();
    evaluate("(begin (collapse-to-one-frame!) #t)", "#t");
    println!(
        "ok - live, ext and wlr captures match layer hit-testing, mapping order and fullscreen policy"
    );
}

pub fn message() {
    let mut capture = Capture::new();
    let mut output = Outputs::new("message region output");
    for scale in [1.0, 1.5] {
        let (config, head) = output.configuration(Some(true));
        output.c.send(head, 4, &words(&[(scale * 256.0) as u32]));
        output.result(config, 2, 0);
        evaluate("(wm-clear-message)", "#t");
        let empty = capture.image(false, None);
        evaluate("(wm-message \"Region capture origin\" 0)", "#t");
        let full = capture.image(false, None);
        let (lw, lh) = (
            (full.width as f64 / scale).round() as u32,
            (full.height as f64 / scale).round() as u32,
        );
        let (w, h) = ((lw / 2) & !1, (lh / 2) & !1);
        let region = [((lw - w) / 2) & !1, ((lh - h) / 2) & !1, w, h];
        let physical = region.map(|v| (v as f64 * scale).round() as u32);
        let expected = full.crop(physical);
        assert!(
            expected != empty.crop(physical),
            "message never reached the selected region"
        );
        let crop = capture.image(false, Some(region));
        assert_eq!((crop.width, crop.height), (physical[2], physical[3]));
        assert!(
            crop.pixels == expected,
            "message region differs from full-output crop at scale {scale}"
        );
    }
    evaluate("(wm-clear-message)", "#t");
    let (config, head) = output.configuration(Some(true));
    output.c.send(head, 4, &words(&[256]));
    output.result(config, 2, 0);
    println!("ok - message regions equal full-output crops at integer and fractional scales");
}

pub fn cursor() {
    const COLOR: u32 = 0x12e0a7;
    evaluate("(wm-clear-message)", "#t");
    let mut capture = Capture::new();
    let mut output = Outputs::new("cursor hotspot output");
    // Fixture hotspots are (13, 9), (25, 17), (37, 25) at buffer
    // scales 1, 2, 3. Expected bounds include per-axis physical rounding.
    for (scale, expected) in [
        (1.0, [187, 191, 24, 24]),
        (1.25, [234, 239, 30, 30]),
        (1.5, [281, 287, 36, 36]),
        (1.75, [328, 335, 42, 42]),
        (2.0, [375, 383, 48, 48]),
        (2.5, [469, 479, 60, 60]),
        (1.5, [281, 287, 36, 36]),
        (1.0, [187, 191, 24, 24]),
    ] {
        let (config, head) = output.configuration(Some(true));
        output.c.send(head, 4, &words(&[(scale * 256.0) as u32]));
        output.result(config, 2, 0);
        evaluate("(wm-warp-pointer 200 200)", "#t");
        let image = capture.cursor_image();
        let mut bounds = [u32::MAX, u32::MAX, 0, 0];
        let mut count = 0;
        for (index, &pixel) in image.pixels.iter().enumerate() {
            if pixel == COLOR {
                let (px, py) = (index as u32 % image.width, index as u32 / image.width);
                bounds[0] = bounds[0].min(px);
                bounds[1] = bounds[1].min(py);
                bounds[2] = bounds[2].max(px);
                bounds[3] = bounds[3].max(py);
                count += 1;
            }
        }
        assert!(count > 0, "themed cursor fixture was not rendered");
        bounds[2] = bounds[2] - bounds[0] + 1;
        bounds[3] = bounds[3] - bounds[1] + 1;
        assert_eq!(bounds, expected, "captured cursor hotspot at scale {scale}");
        assert_eq!(count, expected[2] * expected[3]);
        assert!(
            !capture.image(false, None).pixels.contains(&COLOR),
            "cursor-free capture includes the cursor"
        );
    }
    evaluate("(wm-warp-pointer 10 10)", "#t");
    println!("ok - themed cursor captures align odd hotspots across integer and fractional scales");
}
