// SPDX-License-Identifier: GPL-3.0-or-later
//! Click focus must agree with Scheme policy and layer keyboard interactivity.
use super::{
    input::{Input, evaluate, numbers, token},
    lock::inject_key,
    wire::*,
};

fn mapped(client: &mut Input) {
    client.toplevel(true);
    client.window_id = token(&ipc(&format!(
        "(find (lambda (id) (equal? (window-title id) {:?})) (all-window-ids))",
        client.name
    )));
}
fn select(client: &mut Input) {
    evaluate(&format!("(focus-window-by-id! {})", client.window_id), "#t");
    client.roundtrip();
}
fn selected(id: u32) {
    evaluate("(focused-window-id)", &id.to_string());
    evaluate(
        "(assq-ref (desktop-snapshot) 'focused-window)",
        &id.to_string(),
    );
}
fn keyboard(client: &mut Input, focused: bool) {
    client.roundtrip();
    let event = client
        .events
        .iter()
        .rfind(|e| e.object == client.keyboard && matches!(e.opcode, 1 | 2));
    assert_eq!(
        event.map(|e| e.opcode == 1).unwrap_or(false),
        focused,
        "keyboard focus for {}",
        client.name
    );
}
fn activated(client: &mut Input, expected: bool) {
    client.roundtrip();
    let e = client
        .events
        .iter()
        .rfind(|e| e.is(client.toplevel_object, 0))
        .unwrap();
    let active = (0..e.word(2) as usize / 4).any(|i| e.word(i + 3) == 4);
    assert_eq!(active, expected, "activation for {}", client.name);
}
fn click(sender: &mut Input, pointer: u32, x: i32, y: i32) {
    evaluate(&format!("(wm-warp-pointer {x} {y})"), "#t");
    sender.send(pointer, 2, &words(&[0, 272, 1]));
    sender.send(pointer, 2, &words(&[0, 272, 0]));
    sender.roundtrip();
}
fn center(client: &Input) -> (i32, i32) {
    let rect = numbers(&ipc(&format!("(wm-window-geometry {})", client.window_id)));
    (rect[0] + rect[2] / 2, rect[1] + rect[3] / 2)
}
struct Panel {
    input: Input,
    layer: u32,
}
impl Panel {
    fn new(name: &str, layer: u32, mode: u32) -> Self {
        let mut input = Input::new(name);
        input.get_keyboard();
        input.surface = input.new_surface();
        let shell = input.bind("zwlr_layer_shell_v1", 4);
        let role = input.alloc();
        let mut args = words(&[role, input.surface, 0, layer]);
        args.extend(string(name));
        input.send(shell, 0, &args);
        input.send(role, 0, &words(&[280, 100]));
        input.send(role, 1, &words(&[5])); // top + left
        input.send(role, 3, &words(&[20, 0, 0, 20]));
        input.send(role, 4, &words(&[mode]));
        let surface = input.surface;
        input.send(surface, 6, &[]);
        let configure = input.event(role, 0);
        input.send(role, 6, &words(&[configure.word(0)]));
        input.paint(surface, configure.word(1), configure.word(2), 0x00ff00);
        input.roundtrip();
        Self { input, layer: role }
    }
    fn mode(&mut self, mode: u32) {
        self.input.send(self.layer, 4, &words(&[mode]));
        self.input.send(self.input.surface, 6, &[]);
        self.input.roundtrip();
    }
    fn destroy(mut self) {
        self.input.send(self.layer, 7, &[]);
        self.input.send(self.input.surface, 0, &[]);
        self.input.roundtrip();
    }
}

pub fn run() {
    command("sh", &["-c", "xdotool search --name Smithay windowfocus"]);
    let mut a = Input::new("click frame A");
    mapped(&mut a);
    evaluate(
        "(begin (split-frame-horizontal!) (focus-next-frame!) #t)",
        "#t",
    );
    let mut b = Input::new("click frame B");
    mapped(&mut b);
    let pointer = a.virtual_pointer();
    select(&mut a);
    evaluate(
        "(begin (define click-focus-events '()) (define (record-click-focus id) (set! click-focus-events (cons id click-focus-events))) (add-event-hook! 'focus-window record-click-focus) #t)",
        "#t",
    );
    let (bx, by) = center(&b);
    click(&mut a, pointer, bx, by);
    selected(b.window_id);
    evaluate("(current-frame-window)", &b.window_id.to_string());
    evaluate(
        &format!("(and (member {} click-focus-events) #t)", b.window_id),
        "#t",
    );
    keyboard(&mut b, true);
    keyboard(&mut a, false);
    activated(&mut b, true);
    activated(&mut a, false);
    inject_key(&mut b.c, &mut [&mut a.c]);
    let start = b.events.len();
    evaluate("(begin (close-current-window!) #t)", "#t");
    let top = b.toplevel_object;
    b.wait(start, |e| e.is(top, 1));
    println!("ok - clicks synchronize keyboard, activation, frame, snapshot, events and commands");

    let mut float = Input::new("click floating window");
    float.mapped(600);
    select(&mut a);
    let (fx, fy) = center(&float);
    click(&mut a, pointer, fx, fy);
    selected(float.window_id);
    keyboard(&mut float, true);
    activated(&mut float, true);
    evaluate(
        "(begin (define saved-pointer-focus handle-pointer-focus!) (set! handle-pointer-focus! (lambda (id) #f)) #t)",
        "#t",
    );
    click(&mut a, pointer, bx, 500); // below the floating window
    selected(float.window_id);
    keyboard(&mut float, true);
    keyboard(&mut b, false);
    evaluate(
        "(begin (set! handle-pointer-focus! saved-pointer-focus) #t)",
        "#t",
    );
    println!("ok - floating focus uses policy, and the hook can disable click-to-focus");

    select(&mut a);
    evaluate("(begin (clear-current-frame!) #t)", "#t");
    select(&mut float);
    click(&mut a, pointer, 30, 500);
    selected(float.window_id);
    keyboard(&mut float, true);
    select(&mut a);
    select(&mut float);
    let mut panel = Panel::new("click panel", 3, 0);
    let start = panel.input.events.len();
    click(&mut a, pointer, 50, 50);
    panel.input.roundtrip();
    assert_eq!(panel.input.buttons(start), vec![(272, 1), (272, 0)]);
    selected(float.window_id);
    keyboard(&mut float, true);
    keyboard(&mut panel.input, false);
    panel.mode(2);
    click(&mut a, pointer, 50, 50);
    keyboard(&mut panel.input, true);
    keyboard(&mut float, false);
    selected(float.window_id); // a layer does not change the desktop selection
    inject_key(&mut panel.input.c, &mut [&mut a.c, &mut b.c, &mut float.c]);
    click(&mut a, pointer, bx, 500);
    selected(b.window_id);
    keyboard(&mut b, true);
    keyboard(&mut panel.input, false);
    println!(
        "ok - empty space and noninteractive panels preserve focus; on-demand panels take and release it"
    );

    select(&mut a);
    panel.mode(1);
    keyboard(&mut panel.input, true);
    click(&mut a, pointer, bx, 500);
    selected(a.window_id);
    keyboard(&mut panel.input, true);
    keyboard(&mut b, false);
    evaluate("(begin (sync-frames!) #t)", "#t");
    keyboard(&mut panel.input, true);
    let mut lower = Panel::new("lower exclusive panel", 2, 1);
    keyboard(&mut panel.input, true);
    keyboard(&mut lower.input, false);
    panel.destroy();
    keyboard(&mut lower.input, true);
    lower.destroy();
    keyboard(&mut a, true);
    selected(a.window_id);
    println!("ok - exclusive layers retain keyboard focus and restore the next eligible owner");
    evaluate(
        "(begin (remove-event-hook! 'focus-window record-click-focus) #t)",
        "#t",
    );
    drop((a, b, float));
    until(
        || ipc("(all-window-ids)") == "(ok ())",
        "focus clients remained mapped",
    );
    evaluate("(begin (collapse-to-one-frame!) #t)", "#t");
    println!("pointer-focus: all checks passed");
}
