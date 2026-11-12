// SPDX-License-Identifier: GPL-3.0-or-later
use super::wire::*;
use std::{collections::BTreeSet, env, process::Command};

pub struct Outputs {
    pub c: Client,
    pub manager: u32,
    pub head: u32,
    pub mode: u32,
}
impl Outputs {
    pub fn new(name: &str) -> Self {
        let mut c = Client::new(name);
        let manager = c.bind("zwlr_output_manager_v1", 4);
        c.roundtrip();
        let head = c.event(manager, 0).word(0);
        let mode = c.event(head, 3).word(0);
        Self {
            c,
            manager,
            head,
            mode,
        }
    }
    pub fn serial(&self) -> u32 {
        self.c
            .events
            .iter()
            .rfind(|e| e.is(self.manager, 1))
            .unwrap()
            .word(0)
    }
    pub fn configuration(&mut self, enabled: Option<bool>) -> (u32, u32) {
        let config = self.c.alloc();
        self.c
            .send(self.manager, 0, &words(&[config, self.serial()]));
        let head = if enabled == Some(true) {
            let h = self.c.alloc();
            self.c.send(config, 0, &words(&[h, self.head]));
            h
        } else {
            if enabled == Some(false) {
                self.c.send(config, 1, &words(&[self.head]));
            }
            0
        };
        (config, head)
    }
    pub fn result(&mut self, config: u32, opcode: u16, expected: u16) {
        self.c.send(config, opcode, &[]);
        let e = self.c.wait(0, |e| e.object == config && e.opcode <= 2);
        assert_eq!(e.opcode, expected, "configuration result: {e:?}");
        self.c.send(config, 4, &[]);
        self.c.roundtrip();
    }
    fn probe(&mut self) {
        let (config, _) = self.configuration(Some(true));
        self.result(config, 3, 0);
    }
}
fn serials() {
    let mut a = Outputs::new("long-lived output client");
    let serial = a.serial();
    let (config, _) = a.configuration(Some(true));
    let mut b = Outputs::new("new output client");
    a.c.roundtrip();
    assert_eq!(a.serial(), serial, "binding changed serial");
    assert_eq!(b.serial(), serial);
    a.result(config, 3, 0);
    let (config, _) = b.configuration(Some(true));
    b.result(config, 2, 0);
    a.probe();
    let (old, _) = a.configuration(Some(true));
    let (config, head) = b.configuration(Some(true));
    b.c.send(head, 4, &words(&[320]));
    b.result(config, 2, 0);
    a.c.roundtrip();
    assert_eq!(a.serial(), b.serial());
    assert_ne!(a.serial(), serial);
    a.result(old, 3, 2);
    a.probe();
    let (config, head) = b.configuration(Some(true));
    b.c.send(head, 4, &words(&[256]));
    b.result(config, 2, 0);
    println!(
        "ok - binds preserve configurations; real changes notify all clients and cancel stale ones"
    );
}
type Requests = Vec<(u32, u16, Vec<u8>)>;
fn invalid(
    name: &str,
    enabled: Option<bool>,
    on_config: bool,
    code: u32,
    requests: impl FnOnce(&mut Outputs, u32, u32) -> Requests,
) {
    let mut o = Outputs::new(name);
    let (config, head) = o.configuration(enabled);
    let batch = requests(&mut o, config, head);
    o.c.send_batch(&batch);
    o.c.expect_error(if on_config { config } else { head }, code);
    drop(o);
    Outputs::new("healthy client after rejection").probe();
    println!("ok - {name}");
}
fn property(o: &Outputs, opcode: u16) -> Vec<u8> {
    match opcode {
        0 => words(&[o.mode]),
        1 => words(&[800, 600, 0]),
        2 => words(&[0, 0]),
        3 | 5 => words(&[0]),
        4 => words(&[256]),
        _ => unreachable!(),
    }
}
pub fn management(selected: &[String]) {
    let run = |name: &str| selected.is_empty() || selected.iter().any(|s| s == name);
    if run("serials") {
        serials();
    }
    for (name, opcode, payload, code) in [
        ("scale-zero", 4, words(&[0]), 5),
        ("scale-negative", 4, words(&[(-256i32) as u32]), 5),
        ("transform", 3, words(&[99]), 4),
        ("custom-width", 1, words(&[0, 600, 60000]), 3),
        ("custom-height", 1, words(&[800, (-1i32) as u32, 60000]), 3),
        ("custom-refresh", 1, words(&[800, 600, (-1i32) as u32]), 3),
        ("adaptive-sync", 5, words(&[99]), 6),
    ] {
        if run(name) {
            invalid(name, Some(true), false, code, |_, _, h| {
                vec![(h, opcode, payload)]
            });
        }
    }
    if run("foreign-mode") {
        invalid("foreign-mode", Some(true), false, 2, |o, _, h| {
            let manager = o.c.bind("zwlr_output_manager_v1", 4);
            o.c.roundtrip();
            let other_head = o.c.event(manager, 0).word(0);
            let mode = o.c.event(other_head, 3).word(0);
            vec![(h, 0, words(&[mode]))]
        });
    }
    for (name, order) in [("mode-then-custom", [0, 1]), ("custom-then-mode", [1, 0])] {
        if run(name) {
            invalid(name, Some(true), false, 1, |o, _, h| {
                order
                    .into_iter()
                    .map(|op| (h, op, property(o, op)))
                    .collect()
            });
        }
    }
    let properties = [
        "mode",
        "custom-mode",
        "position",
        "transform",
        "scale",
        "adaptive-sync",
    ];
    for (opcode, property_name) in properties.iter().enumerate() {
        let name = format!("duplicate-{property_name}");
        if run(&name) {
            invalid(&name, Some(true), false, 1, |o, _, h| {
                vec![(h, opcode as u16, property(o, opcode as u16)); 2]
            });
        }
    }
    for (action, opcode) in [("test", 3), ("apply", 2)] {
        let name = format!("omitted-{action}");
        if run(&name) {
            invalid(&name, None, true, 2, |_, f, _| vec![(f, opcode, vec![])]);
        }
        for suffix in ["enable", "disable", "test", "apply"]
            .into_iter()
            .chain(properties)
        {
            let name = format!("after-{action}-{suffix}");
            if run(&name) {
                invalid(&name, Some(true), true, 3, |o, f, h| {
                    let request = match suffix {
                        "enable" => (f, 0, words(&[o.c.alloc(), o.head])),
                        "disable" => (f, 1, words(&[o.head])),
                        "test" => (f, 3, vec![]),
                        "apply" => (f, 2, vec![]),
                        _ => {
                            let op = properties.iter().position(|s| *s == suffix).unwrap() as u16;
                            (h, op, property(o, op))
                        }
                    };
                    vec![(f, opcode, vec![]), request]
                });
            }
        }
    }
    for (name, opcode) in [("duplicate-enable", 0), ("duplicate-disable", 1)] {
        if run(name) {
            invalid(name, Some(true), true, 1, |o, f, _| {
                let payload = if opcode == 0 {
                    words(&[o.c.alloc(), o.head])
                } else {
                    words(&[o.head])
                };
                vec![(f, opcode, payload)]
            });
        }
    }
    if run("lifecycle") {
        let mut o = Outputs::new("valid configuration lifecycle");
        for opcode in [3, 2] {
            let (config, head) = o.configuration(Some(true));
            let requests = [0, 2, 3, 4, 5].map(|op| (head, op, property(&o, op)));
            o.c.send_batch(&requests);
            o.result(config, opcode, 0);
            o.c.wait(0, |e| e.is(1, 1) && e.word(0) == head);
        }
        let (config, head) = o.configuration(Some(true));
        o.c.send(head, 5, &words(&[1]));
        o.result(config, 3, 1);
        println!(
            "ok - valid properties succeed, backend refusals stay nonfatal, and destroy retires children"
        );
    }
    if run("custom-unspecified") {
        custom_unspecified();
    }
    if run("resize") {
        resize();
    }
    if selected.iter().any(|s| s == "disabled-scheme") {
        disabled_scheme();
    }
}

fn custom_unspecified() {
    let mut o = Outputs::new("custom mode without refresh");
    let size = o.c.event(o.mode, 0).data.clone();
    let before = ipc("(output-heads)");
    for opcode in [3, 2] {
        let (config, head) = o.configuration(Some(true));
        let mut custom = size.clone();
        custom.extend(words(&[0]));
        o.c.send(head, 1, &custom);
        o.result(config, opcode, 0);
        assert_eq!(ipc("(output-heads)"), before);
    }
    println!("ok - unspecified refresh tests and applies as the advertised mode");
}

fn single_mode(o: &mut Outputs, size: (u32, u32)) -> u32 {
    let mut modes: BTreeSet<_> =
        o.c.events
            .iter()
            .filter(|e| e.is(o.head, 3))
            .map(|e| e.word(0))
            .collect();
    modes.retain(|id| !o.c.events.iter().any(|e| e.is(*id, 3)));
    assert_eq!(modes.len(), 1, "obsolete output modes remain advertised");
    let mode = *modes.first().unwrap();
    assert_eq!(o.c.event(mode, 0).data, words(&[size.0, size.1]));
    o.c.event(mode, 2); // Preferred.
    assert_eq!(
        o.c.events
            .iter()
            .rfind(|e| e.is(o.head, 5))
            .unwrap()
            .word(0),
        mode
    );
    mode
}

fn resize() {
    let mut o = Outputs::new("mode retirement after host resize");
    let output = o.c.bind("wl_output", 4);
    o.c.roundtrip();
    let initial = o.c.event(o.mode, 0);
    let original = (initial.word(0), initial.word(1));
    let host = |args: &[&str]| {
        let out = Command::new("timeout")
            .args(["5", "xdotool"])
            .args(args)
            .env("DISPLAY", env::var("MINDE_OUTPUT_HOST_DISPLAY").unwrap())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "xdotool: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    let window = host(&["search", "--name", "Smithay"])
        .lines()
        .next()
        .unwrap()
        .to_owned();
    for size in [(700, 500), (800, 600), (900, 650), original] {
        let previous =
            o.c.events
                .iter()
                .rfind(|e| e.is(o.head, 5))
                .unwrap()
                .word(0);
        let (stale, _) = o.configuration(Some(true));
        o.c.roundtrip();
        let start = o.c.events.len();
        host(&[
            "windowsize",
            &window,
            &size.0.to_string(),
            &size.1.to_string(),
        ]);
        o.c.wait(start, |e| e.is(o.manager, 1));
        o.c.roundtrip();
        single_mode(&mut o, size);
        assert_eq!(
            o.c.events[start..]
                .iter()
                .filter(|e| e.is(previous, 3))
                .count(),
            1
        );
        let current = o.c.events[start..]
            .iter()
            .rfind(|e| e.is(output, 1))
            .unwrap();
        assert_eq!(
            current.data,
            words(&[3, size.0, size.1, 60_000]),
            "wl_output current/preferred flags"
        );
        let mut fresh = Outputs::new("new client after host resize");
        let mode = single_mode(&mut fresh, size);
        let (config, head) = fresh.configuration(Some(true));
        fresh.c.send(head, 0, &words(&[mode]));
        fresh.result(config, 2, 0);
        o.result(stale, 3, 2);
    }
    println!("ok - resizes retire modes and update current/preferred for existing and new clients");
}

fn scheme_wait(expression: &str) {
    until(|| ipc(expression) == "(ok #t)", expression);
}

fn disabled_scheme() {
    assert_eq!(ipc("(assq-ref (car (output-heads)) 'enabled)"), "(ok #f)");
    ipc("(begin
      (define %disabled-before (output-heads))
      (define %disabled-failures '())
      (define %disabled-successes 0)
      (define %saved-configure-failed handle-output-configure-failed!)
      (define %saved-configured handle-output-configured!)
      (define (handle-output-configure-failed! name reason)
        (set! %disabled-failures (cons (list name reason) %disabled-failures)))
      (define (handle-output-configured!)
        (set! %disabled-successes (+ 1 %disabled-successes))))");
    let settings = [
        "#:mode '(1280 800)",
        "#:mode '(799 599)",
        "#:position '(100 20)",
        "#:scale 2",
        "#:transform 90",
        "#:adaptive-sync #f",
        "#:enabled #f #:scale 2",
    ];
    for (index, setting) in settings.iter().enumerate() {
        assert_eq!(
            ipc(&format!("(configure-output! \"winit\" {setting})")),
            "(ok #t)"
        );
        scheme_wait(&format!("(= (length %disabled-failures) {})", index + 1));
        assert_eq!(ipc("(equal? %disabled-before (output-heads))"), "(ok #t)");
    }
    assert_eq!(ipc("%disabled-successes"), "(ok 0)");
    assert_eq!(
        ipc("%disabled-failures").matches("#:enabled #t").count(),
        settings.len()
    );
    ipc("(configure-output! \"winit\" #:enabled #t #:scale 2 #:position '(100 20) #:transform 90)");
    scheme_wait("(= %disabled-successes 1)");
    assert_eq!(
        ipc("(let ((h (car (output-heads))))
      (and (assq-ref h 'enabled) (= (assq-ref h 'scale) 2)
           (equal? (assq-ref h 'position) '(100 20))
           (equal? (symbol->string (assq-ref h 'transform)) \"90\")))"),
        "(ok #t)"
    );
    // Combining disable with settings must also fail while the head is on.
    ipc("(configure-output! \"winit\" #:enabled #f #:scale 1)");
    scheme_wait(&format!(
        "(= (length %disabled-failures) {})",
        settings.len() + 1
    ));
    assert_eq!(ipc("(assq-ref (car (output-heads)) 'enabled)"), "(ok #t)");
    assert_eq!(ipc("%disabled-successes"), "(ok 1)");
    ipc("(configure-output! \"winit\" #:scale 1 #:position '(0 0) #:transform 'normal)");
    scheme_wait("(= %disabled-successes 2)");
    ipc("(configure-output! \"winit\" #:enabled #f)");
    scheme_wait("(= %disabled-successes 3)");
    ipc("(configure-output! \"winit\" #:enabled #f)");
    scheme_wait("(= %disabled-successes 4)");
    ipc(
        "(begin (set! handle-output-configure-failed! %saved-configure-failed)
                (set! handle-output-configured! %saved-configured))",
    );
    println!("ok - disabled-head settings fail without mutation; explicit enable applies them");
}

pub fn mode_limits() {
    let before = ipc("(output-heads)");
    for refresh in [i32::MIN, -1, 0, i32::MAX, 60000] {
        let mut o = Outputs::new("custom refresh limits");
        let (config, head) = o.configuration(Some(true));
        o.c.send_batch(&[
            (head, 1, words(&[1280, 800, refresh as u32])),
            (config, 2, vec![]),
        ]);
        if refresh < 0 {
            o.c.expect_error(head, 3);
        } else {
            let event = o.c.wait(0, |e| e.object == config && e.opcode <= 2);
            assert_eq!(
                event.opcode,
                if refresh == 0 || refresh == 60000 {
                    0
                } else {
                    1
                }
            );
        }
        assert_eq!(ipc("(output-heads)"), before);
    }
    println!("ok - custom mode protocol rejects negative refresh and survives integer limits");
}
