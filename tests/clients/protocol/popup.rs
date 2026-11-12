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
        c.send(self.surface, 0, &[]);
        c.send(self.shell, 0, &[]);
        c.roundtrip();
    }
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
