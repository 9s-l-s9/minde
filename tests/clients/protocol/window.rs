// SPDX-License-Identifier: GPL-3.0-or-later
use super::{
    input::{evaluate, token},
    wire::*,
};
pub fn window_id(name: &str) -> u32 {
    token(&ipc(&format!(
        "(find (lambda (id) (equal? (window-title id) {name:?})) (all-window-ids))"
    )))
}
fn grabbed_popup(c: &mut Client, seat: u32, serial: u32) -> (u32, u32, u32) {
    let shell = c.bind("xdg_wm_base", 1);
    c.handlers.insert(
        shell,
        Box::new(move |c, e| {
            if e.opcode == 0 {
                c.send(shell, 3, &e.data);
            }
        }),
    );
    let positioner = c.alloc();
    c.send(shell, 1, &words(&[positioner]));
    c.send(positioner, 1, &words(&[100, 80]));
    c.send(positioner, 2, &words(&[10, 10, 20, 20]));
    let surface = c.new_surface();
    let xdg = c.alloc();
    let popup = c.alloc();
    c.send(shell, 2, &words(&[xdg, surface]));
    c.send(xdg, 2, &words(&[popup, c.xdg_surface, positioner]));
    c.send(popup, 1, &words(&[seat, serial]));
    c.handlers.insert(
        xdg,
        Box::new(move |c, e| {
            if e.opcode == 0 {
                c.send(xdg, 4, &e.data);
                c.paint(surface, 100, 80, 0x00ff00);
            }
        }),
    );
    c.send(surface, 6, &[]);
    c.event(xdg, 0);
    c.roundtrip();
    (surface, xdg, popup)
}
pub struct Observer {
    pub c: Client,
    manager: u32,
}
impl Observer {
    pub fn new() -> Self {
        let mut c = Client::new("window lifecycle observer");
        let manager = c.bind("zwlr_foreign_toplevel_manager_v1", 3);
        c.roundtrip();
        Self { c, manager }
    }
    pub fn live_handles(&mut self) -> Vec<u32> {
        self.c.roundtrip();
        self.c
            .events
            .iter()
            .filter(|e| e.is(self.manager, 0))
            .map(|e| e.word(0))
            .filter(|id| !self.c.events.iter().any(|e| e.is(*id, 6)))
            .collect()
    }
    pub fn handle(&mut self, title: &str) -> u32 {
        let live = self.live_handles();
        self.c
            .events
            .iter()
            .rev()
            .find(|e| live.contains(&e.object) && e.opcode == 0 && text(&e.data, 0) == title)
            .unwrap_or_else(|| panic!("no foreign handle for {title}"))
            .object
    }
}
pub fn run() {
    let mut observer = Observer::new();
    let mut anchor = Client::new("anchor");
    let mut target = Client::new("pending");
    evaluate(
        "(begin (define lifecycle-maps '()) (define lifecycle-unmaps '()) \
        (add-event-hook! 'new-window (lambda (id title app) (set! lifecycle-maps (cons id lifecycle-maps)))) \
        (add-event-hook! 'destroy-window (lambda (id) (set! lifecycle-unmaps (cons id lifecycle-unmaps)))) #t)",
        "#t",
    );
    anchor.toplevel(true);
    let anchor_id = window_id("anchor");
    target.toplevel(false);
    let seat = target.bind("wl_seat", 5);
    let pointer = target.alloc();
    target.send(seat, 0, &words(&[pointer]));
    let manager = target.bind("zwlr_virtual_pointer_manager_v1", 1);
    let virtual_pointer = target.alloc();
    target.send(manager, 0, &words(&[0, virtual_pointer]));
    let anchor_seat = anchor.bind("wl_seat", 5);
    let anchor_pointer = anchor.alloc();
    anchor.send(anchor_seat, 0, &words(&[anchor_pointer]));
    target.roundtrip();
    anchor.roundtrip();
    evaluate("(length (all-window-ids))", "1");
    evaluate("(length lifecycle-maps)", "1");
    assert_eq!(observer.live_handles().len(), 1);
    println!("ok - configuring a bufferless toplevel creates no managed window");
    let mut old_id = 0;
    for cycle in 0..6 {
        let name = format!("lifecycle-{cycle}");
        target.send(target.toplevel_object, 2, &string(&name));
        target.send(target.toplevel_object, 3, &string("org.minde.lifecycle"));
        if cycle == 1 {
            target.send(target.toplevel_object, 11, &words(&[0]));
            target.roundtrip();
            assert_eq!(target.configured_size, (1280, 800));
        }
        target.map_on_configure = true;
        target.paint(
            target.surface,
            target.configured_size.0,
            target.configured_size.1,
            0xff00ff,
        );
        target.roundtrip();
        let current = window_id(&name);
        assert!(current > old_id);
        evaluate("(length (all-window-ids))", "2");
        evaluate("(focused-window-id)", &current.to_string());
        evaluate("(length lifecycle-maps)", &(cycle + 2).to_string());
        evaluate(
            &format!("(window-app-id {current})"),
            "\"org.minde.lifecycle\"",
        );
        if cycle == 1 {
            evaluate("(fullscreen-window)", &current.to_string());
        }
        let handle = observer.handle(&name);
        assert_eq!(observer.live_handles().len(), 2);
        target.paint(
            target.surface,
            target.configured_size.0,
            target.configured_size.1,
            0x00ffff,
        );
        target.send(target.surface, 6, &[]);
        target.roundtrip();
        evaluate("(length lifecycle-maps)", &(cycle + 2).to_string());
        if cycle == 5 {
            evaluate(
                &format!(
                    "(begin (float-window! {current} (list 160 120 220 180)) (sync-frames!) #t)"
                ),
                "#t",
            );
            target.roundtrip();
            target.roundtrip();
            evaluate(
                &format!("(wm-window-geometry {current})"),
                "(160 120 220 180)",
            );
        }
        let mut popup_objects = None;
        if (2..=4).contains(&cycle) {
            evaluate(
                &format!(
                    "(begin (float-window! {current} (list 100 100 300 250)) (sync-frames!) (wm-warp-pointer 200 180))"
                ),
                "#t",
            );
            target.roundtrip();
            let start = target.events.len();
            target.send(virtual_pointer, 2, &words(&[0, 272, 1]));
            let e = target.wait(start, |e| {
                e.is(pointer, 3) && e.word(2) == 272 && e.word(3) == 1
            });
            match cycle {
                2 => popup_objects = Some(grabbed_popup(&mut target, seat, e.word(0))),
                3 => target.send(target.toplevel_object, 5, &words(&[seat, e.word(0)])),
                _ => target.send(target.toplevel_object, 6, &words(&[seat, e.word(0), 5])),
            }
            target.roundtrip();
        }
        anchor.roundtrip();
        target.roundtrip();
        let anchor_start = anchor.events.len();
        let target_start = target.events.len();
        target.map_on_configure = false;
        target.send(target.surface, 1, &words(&[0, 0, 0]));
        target.send(target.surface, 6, &[]);
        target.roundtrip();
        evaluate("(length (all-window-ids))", "1");
        evaluate("(focused-window-id)", &anchor_id.to_string());
        evaluate("(fullscreen-window)", "#f");
        evaluate(&format!("(wm-window-geometry {current})"), "#f");
        evaluate(&format!("(wm-window-title {current})"), "#f");
        evaluate("(length lifecycle-unmaps)", &(cycle + 1).to_string());
        let keyboard = anchor.keyboard;
        anchor.wait(anchor_start, |e| e.is(keyboard, 1));
        let keyboard = target.keyboard;
        target.wait(target_start, |e| e.is(keyboard, 2));
        observer.c.event(handle, 6);
        assert_eq!(observer.live_handles().len(), 1);
        if let Some((surface, xdg, popup)) = popup_objects {
            target.wait(target_start, |e| e.is(popup, 1));
            target.send(popup, 0, &[]);
            target.send(xdg, 0, &[]);
            target.send(surface, 0, &[]);
            target.roundtrip();
        }
        if (2..=4).contains(&cycle) {
            evaluate("(wm-warp-pointer 50 50)", "#t");
            anchor.wait(anchor_start, |e| e.is(anchor_pointer, 0));
            target.send(virtual_pointer, 2, &words(&[0, 272, 0]));
            target.roundtrip();
        }
        target.send(target.surface, 6, &[]);
        target.roundtrip();
        evaluate("(length lifecycle-unmaps)", &(cycle + 1).to_string());
        old_id = current;
    }
    println!(
        "ok - repeated null-buffer unmaps clear layout, focus, fullscreen and foreign handles"
    );
    println!("ok - remaps get fresh IDs and honor pre-map metadata and fullscreen");
    println!("ok - hiding a window dismisses popups and ends popup/move/resize grabs");
    println!("ok - remapping after a cancelled resize uses the new geometry");
    target.send(target.toplevel_object, 0, &[]);
    target.send(target.xdg_surface, 0, &[]);
    target.send(target.surface, 0, &[]);
    target.roundtrip();
    evaluate("(length lifecycle-unmaps)", "6");
    anchor.map_on_configure = false;
    anchor.send(anchor.toplevel_object, 0, &[]);
    anchor.send(anchor.xdg_surface, 0, &[]);
    anchor.send(anchor.surface, 0, &[]);
    anchor.roundtrip();
    evaluate("(all-window-ids)", "()");
    evaluate("(focused-window-id)", "#f");
    evaluate("(length lifecycle-unmaps)", "7");
    assert!(observer.live_handles().is_empty());
    println!("ok - mapped and unmapped role destruction emit unmap exactly once");
}
