// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded raw Wayland clients for the private nested test compositor.
mod capture;
mod constraints;
mod emergency;
mod focus;
mod input;
mod keyboard;
mod layer;
mod lock;
mod outputs;
mod popup;
mod render;
mod scene;
mod selection;
mod virtual_pointer;
mod window;
mod wire;
mod xwayland;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("output-management") => outputs::management(&args[1..]),
        Some("output-mode") => outputs::mode_limits(),
        Some("session-lock") => lock::run(),
        Some("emergency-exit") => lock::emergency_exit(),
        Some("emergency-config") => emergency::run(args.get(1).expect("locked or inhibited")),
        Some("input-lifecycle") => input::run(),
        Some("keyboard-filter") => keyboard::run(),
        Some("pointer-constraints") => constraints::run(),
        Some("pointer-focus") => focus::run(),
        Some("x11-focus-restore") => focus::restore_x11(),
        Some("layer-unlock") => layer::unlock(),
        Some("layer-output") => layer::output_lifecycle(),
        Some("popup-placement") => popup::run(),
        Some("fractional-popups") => popup::fractional_scales(),
        Some("scene-layers") => scene::layers(),
        Some("scene-message") => scene::message(),
        Some("scene-cursor") => scene::cursor(),
        Some("virtual-pointer") => virtual_pointer::run(),
        Some("virtual-pointer-errors") => virtual_pointer::errors(),
        Some("capture-clock") => capture::clock(),
        Some("capture-lifecycle") => capture::run(&args[1..]),
        Some("winit-errors") => render::run(args.get(1).expect("render or swap")),
        Some("window-lifecycle") => window::run(),
        Some("xwayland-lifecycle") => xwayland::run(),
        Some("x11-metadata") => xwayland::metadata(),
        Some("selection-transfer") => selection::run(),
        _ => panic!("select a protocol test scenario"),
    }
}
