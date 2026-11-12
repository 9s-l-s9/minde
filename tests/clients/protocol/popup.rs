// SPDX-License-Identifier: GPL-3.0-or-later
use super::{
    focus::Panel,
    input::{Input, evaluate, numbers},
    outputs::Outputs,
    wire::*,
};

struct Popup {
    shell: u32,
    surface: u32,
    xdg: u32,
    role: u32,
    scale: u32,
}

fn fractional(c: &mut Client, surface: u32) -> u32 {
    let manager = c.bind("wp_fractional_scale_manager_v1", 1);
    let scale = c.alloc();
    c.send(manager, 1, &words(&[scale, surface]));
    c.send(manager, 0, &[]);
    scale
}

fn subsurface(c: &mut Client, parent: u32) -> u32 {
    let surface = c.new_surface();
    let scale = fractional(c, surface); // Bind before the surface acquires a parent.
    let manager = c.bind("wl_subcompositor", 1);
    let role = c.alloc();
    c.send(manager, 1, &words(&[role, surface, parent]));
    c.send(parent, 6, &[]);
    c.roundtrip();
    scale
}

fn scales(c: &mut Client, objects: &[u32], expected: u32) -> usize {
    c.roundtrip();
    for &object in objects {
        let event = c.events.iter().rfind(|e| e.is(object, 0)).unwrap();
        assert_eq!(event.word(0), expected, "{}: scale object {object}", c.name);
    }
    c.events
        .iter()
        .filter(|e| objects.contains(&e.object) && e.opcode == 0)
        .count()
}

fn positioner(c: &mut Client, shell: u32, anchor: [i32; 4], adjustments: u32) -> u32 {
    let id = c.alloc();
    c.send(shell, 1, &words(&[id]));
    c.send(id, 1, &words(&[160, 120]));
    c.send(id, 2, &words(&anchor.map(|v| v as u32)));
    c.send(id, 3, &words(&[8])); // bottom-right anchor
    c.send(id, 4, &words(&[8])); // bottom-right gravity
    c.send(id, 5, &words(&[adjustments]));
    id
}

impl Popup {
    fn new(
        c: &mut Client,
        parent: u32,
        layer: Option<u32>,
        anchor: [i32; 4],
        adjustments: u32,
    ) -> (Self, [i32; 4]) {
        let shell = c.bind("xdg_wm_base", 3);
        c.handlers.insert(
            shell,
            Box::new(move |c, e| {
                if e.opcode == 0 {
                    c.send(shell, 3, &e.data);
                }
            }),
        );
        let pos = positioner(c, shell, anchor, adjustments);
        let surface = c.new_surface();
        let scale = fractional(c, surface);
        let xdg = c.alloc();
        let role = c.alloc();
        c.send(shell, 2, &words(&[xdg, surface]));
        c.send(xdg, 2, &words(&[role, parent, pos]));
        if let Some(layer) = layer {
            c.send(layer, 5, &words(&[role]));
        }
        c.send(pos, 0, &[]);
        c.send(surface, 6, &[]);
        let popup = Self {
            shell,
            surface,
            xdg,
            role,
            scale,
        };
        let geometry = popup.configure(c, 0);
        (popup, geometry)
    }

    fn configure(&self, c: &mut Client, start: usize) -> [i32; 4] {
        let event = c.wait(start, |e| e.is(self.role, 0));
        let geometry = std::array::from_fn(|i| event.word(i) as i32);
        let configure = c.wait(start, |e| e.is(self.xdg, 0));
        c.send(self.xdg, 4, &words(&[configure.word(0)]));
        c.paint(
            self.surface,
            geometry[2] as u32,
            geometry[3] as u32,
            0x00ff00,
        );
        c.roundtrip();
        geometry
    }

    fn reposition(&self, c: &mut Client, anchor: [i32; 4], adjustments: u32) -> [i32; 4] {
        let start = c.events.len();
        let pos = positioner(c, self.shell, anchor, adjustments);
        c.send(self.role, 2, &words(&[pos, 42]));
        c.send(pos, 0, &[]);
        assert_eq!(c.wait(start, |e| e.is(self.role, 2)).word(0), 42);
        self.configure(c, start)
    }

    fn destroy(self, c: &mut Client) {
        c.send(self.role, 0, &[]);
        c.send(self.xdg, 0, &[]);
        c.send(self.scale, 0, &[]);
        c.send(self.surface, 0, &[]);
        c.send(self.shell, 0, &[]);
        c.roundtrip();
    }
}

pub fn fractional_scales() {
    let mut output = Outputs::new("fractional popup output");
    let mut desktop = Input::new("fractional popup parent");
    desktop.mapped(100);
    let root = desktop.surface;
    let root_scale = fractional(&mut desktop.c, root);
    let root_child = subsurface(&mut desktop.c, root);
    let parent = desktop.xdg_surface;
    let (popup, _) = Popup::new(&mut desktop.c, parent, None, [20, 20, 10, 10], 12);
    let popup_child = subsurface(&mut desktop.c, popup.surface);
    let (child, _) = Popup::new(&mut desktop.c, popup.xdg, None, [20, 20, 10, 10], 12);
    let desktop_scales = [
        root_scale,
        root_child,
        popup.scale,
        popup_child,
        child.scale,
    ];

    let mut panel = Panel::new("fractional layer parent", 3, 0);
    let surface = panel.input.surface;
    let panel_scale = fractional(&mut panel.input.c, surface);
    let (menu, _) = Popup::new(
        &mut panel.input.c,
        0,
        Some(panel.layer),
        [20, 20, 10, 10],
        12,
    );
    let menu_child = subsurface(&mut panel.input.c, menu.surface);
    let (submenu, _) = Popup::new(&mut panel.input.c, menu.xdg, None, [20, 20, 10, 10], 12);
    let panel_scales = [panel_scale, menu.scale, menu_child, submenu.scale];
    scales(&mut desktop.c, &desktop_scales, 120);
    scales(&mut panel.input.c, &panel_scales, 120);

    for (fixed_scale, expected) in [(384, 180), (256, 120)] {
        let (config, head) = output.configuration(Some(true));
        output.c.send(head, 4, &words(&[fixed_scale]));
        output.result(config, 2, 0);
        let count = scales(&mut desktop.c, &desktop_scales, expected);
        let panel_count = scales(&mut panel.input.c, &panel_scales, expected);
        evaluate(
            &format!(
                "(begin (update-floating-window-geometry! {} 150 100 300 250) (sync-frames!) #t)",
                desktop.window_id
            ),
            "#t",
        );
        let root = desktop.surface;
        desktop.send(root, 6, &[]);
        panel.input.send(surface, 6, &[]);
        assert_eq!(
            scales(&mut desktop.c, &desktop_scales, expected),
            count,
            "same-output movement repeats preferred_scale"
        );
        assert_eq!(
            scales(&mut panel.input.c, &panel_scales, expected),
            panel_count
        );
        let late = subsurface(&mut desktop.c, popup.surface);
        scales(&mut desktop.c, &[late], expected);
    }
    child.destroy(&mut desktop.c);
    popup.destroy(&mut desktop.c);
    submenu.destroy(&mut panel.input.c);
    menu.destroy(&mut panel.input.c);
    panel.destroy();
    drop(desktop);
    until(
        || ipc("(all-window-ids)") == "(ok ())",
        "fractional parent remained mapped",
    );
    evaluate("(begin (collapse-to-one-frame!) #t)", "#t");
    println!(
        "ok - fractional scale follows toplevels, layers, nested popups and subsurfaces without duplicate events"
    );
}

fn nested(c: &mut Client, parent: u32) {
    let (popup, geometry) = Popup::new(c, parent, None, [150, 110, 10, 10], 12);
    assert_eq!(geometry, [-10, -10, 160, 120], "nested popup constraint");
    popup.destroy(c);
}

pub fn run() {
    let mut output = Outputs::new("popup output placement");
    let mode = output.c.event(output.mode, 0);
    let (width, height) = (mode.word(0) as i32, mode.word(1) as i32);
    let (config, head) = output.configuration(Some(true));
    output.c.send(head, 2, &words(&[400, (-200i32) as u32]));
    output.result(config, 2, 0);

    let mut desktop = Input::new("popup toplevel parent");
    desktop.mapped(450);
    evaluate(
        &format!(
            "(begin (update-floating-window-geometry! {} {} {} 300 250) (sync-frames!) #t)",
            desktop.window_id,
            400 + width - 300,
            -200 + height - 250
        ),
        "#t",
    );
    desktop.roundtrip();
    let rect = numbers(&ipc(&format!("(wm-window-geometry {})", desktop.window_id)));
    assert_eq!(rect, [400 + width - 300, -200 + height - 250, 300, 250]);
    let parent = desktop.xdg_surface;
    let anchor = [rect[2] - 20, rect[3] - 20, 10, 10];
    let (popup, geometry) = Popup::new(&mut desktop.c, parent, None, anchor, 12);
    assert_eq!(geometry, [rect[2] - 180, rect[3] - 140, 160, 120]);
    nested(&mut desktop.c, popup.xdg);
    let geometry = popup.reposition(&mut desktop.c, anchor, 3); // slide x/y
    assert_eq!(
        geometry,
        [
            400 + width - rect[0] - 160,
            -200 + height - rect[1] - 120,
            160,
            120
        ]
    );
    popup.destroy(&mut desktop.c);
    drop(desktop);
    until(
        || ipc("(all-window-ids)") == "(ok ())",
        "popup parent remained mapped",
    );
    println!("ok - toplevel popups flip, slide and nest on an offset output");

    let mut panel = Panel::new("popup layer parent", 3, 0);
    panel.input.send(panel.layer, 1, &words(&[10])); // bottom + right
    panel.input.send(panel.layer, 3, &words(&[0, 20, 20, 0]));
    let surface = panel.input.surface;
    panel.input.send(surface, 6, &[]);
    panel.input.roundtrip();
    let (popup, geometry) = Popup::new(
        &mut panel.input.c,
        0,
        Some(panel.layer),
        [250, 80, 10, 10],
        12,
    );
    assert_eq!(geometry, [90, -40, 160, 120], "layer popup constraint");
    nested(&mut panel.input.c, popup.xdg);
    assert_eq!(
        popup.reposition(&mut panel.input.c, [270, 90, 10, 10], 12),
        [110, -30, 160, 120]
    );
    popup.destroy(&mut panel.input.c);
    let (raw, geometry) = Popup::new(
        &mut panel.input.c,
        0,
        Some(panel.layer),
        [250, 80, 10, 10],
        0,
    );
    assert_eq!(
        geometry,
        [260, 90, 160, 120],
        "client forbids constraint adjustments"
    );
    raw.destroy(&mut panel.input.c);
    panel.destroy();
    println!(
        "ok - layer popups constrain after attachment, including nested and repositioned menus"
    );

    let (config, head) = output.configuration(Some(true));
    output.c.send(head, 2, &words(&[0, 0]));
    output.result(config, 2, 0);
    evaluate("(begin (collapse-to-one-frame!) #t)", "#t");
}
