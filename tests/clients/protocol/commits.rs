// SPDX-License-Identifier: GPL-3.0-or-later
use super::{
    focus::Panel,
    input::{Input, evaluate, numbers},
    wire::*,
};
use std::{thread::sleep, time::Duration};

fn counts() -> Vec<i32> {
    numbers(&ipc(
        "(map (lambda (name) (cadr (assq name (wm-timing-stats)))) '(render arrange-layers))",
    ))
}

pub fn run() {
    let mut hidden = Input::new("hidden repaint workload");
    hidden.mapped(400);
    evaluate(
        "(begin (switch-to-group! \" II \") (wm-clear-message) #t)",
        "#t",
    );
    hidden.roundtrip();
    let mut bar = Panel::new("one hertz bar", 2, 0);
    sleep(Duration::from_millis(250));
    let before = counts();
    for frame in 0..60 {
        // Keep committing even without frame callbacks, as a video/timer
        // client can. The ordinary frame-paced hidden workload is tested too.
        let surface = hidden.surface;
        hidden.paint(surface, 300, 250, 0x001100 + frame);
        hidden.roundtrip();
        if frame % 20 == 0 {
            let surface = bar.input.surface;
            bar.input.paint(surface, 280, 100, 0x110000 + frame);
            bar.input.roundtrip();
        }
        sleep(Duration::from_millis(50));
    }
    sleep(Duration::from_millis(100));
    let after = counts();
    let renders = after[0] - before[0];
    let arrangements = after[1] - before[1];
    println!(
        "surface commits: 60 hidden paints + 3 bar paints: {renders} renders, {arrangements} layer arrangements"
    );
    assert!(
        (3..=5).contains(&renders),
        "hidden commits woke the renderer"
    );
    assert_eq!(
        arrangements, 0,
        "content-only bar commits rearranged layers"
    );
    // A real exclusive-zone change still configures the bar and updates
    // the usable area. Its configure acknowledgement is not another layout.
    let before = counts()[1];
    bar.input.send(bar.layer, 0, &words(&[0, 40]));
    bar.input.send(bar.layer, 1, &words(&[13])); // top, left, right
    bar.input.send(bar.layer, 2, &words(&[40]));
    bar.input.send(bar.layer, 3, &words(&[0, 0, 0, 0]));
    let start = bar.input.events.len();
    let surface = bar.input.surface;
    bar.input.send(surface, 6, &[]);
    let role = bar.layer;
    let configure = bar.input.wait(start, |e| e.is(role, 0));
    assert_eq!((configure.word(1), configure.word(2)), (1280, 40));
    bar.input.send(bar.layer, 6, &words(&[configure.word(0)]));
    bar.input.paint(surface, 1280, 40, 0x112233);
    bar.input.roundtrip();
    evaluate(
        "(let ((h (car (heads)))) (list (list-ref h 2) (list-ref h 4)))",
        "(40 760)",
    );
    assert_eq!(
        counts()[1] - before,
        1,
        "layout change/ack arrangement count"
    );
    bar.mode(2);
    assert_eq!(
        counts()[1] - before,
        1,
        "keyboard-only change rearranged layers"
    );
    bar.destroy();
    drop(hidden);
    evaluate(
        "(begin (switch-to-group! \" I \") (wm-clear-message) #t)",
        "#t",
    );
    let mut visible = Input::new("synchronized child owner");
    visible.mapped(400);
    let child = visible.new_surface();
    let manager = visible.bind("wl_subcompositor", 1);
    let role = visible.alloc();
    let parent = visible.surface;
    visible.send(manager, 1, &words(&[role, child, parent]));
    visible.send(parent, 6, &[]);
    visible.roundtrip();
    sleep(Duration::from_millis(200));
    let before = counts()[0];
    visible.paint(child, 40, 40, 0x55aa33);
    visible.roundtrip();
    sleep(Duration::from_millis(100));
    assert_eq!(
        counts()[0],
        before,
        "synchronized child rendered before parent commit"
    );
    let callback = visible.alloc();
    visible.send(parent, 3, &words(&[callback]));
    visible.send(parent, 6, &[]);
    visible.event(callback, 0);
    assert!(
        counts()[0] > before,
        "parent commit did not render synchronized child"
    );
    println!("ok - layout changes and synchronized children retain their protocol timing");
}
