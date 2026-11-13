// SPDX-License-Identifier: GPL-3.0-or-later
use super::{
    input::{Input, evaluate},
    lock::HostPixels,
    window::Observer,
    wire::*,
};
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};
fn managed_id(title: &str) -> Option<u32> {
    let reply = ipc(&format!(
        "(find (lambda (id) (equal? (window-title id) {title:?})) (all-window-ids))"
    ));
    reply
        .strip_prefix("(ok ")
        .and_then(|s| s.strip_suffix(')'))
        .and_then(|s| s.parse().ok())
}
fn await_id(title: &str) -> u32 {
    let mut id = None;
    until(
        || {
            id = managed_id(title);
            id.is_some()
        },
        "X11 window never mapped",
    );
    id.unwrap()
}
fn spawn_display(out: &Path, label: &str, expected: &str) {
    let path = out.join(format!("{label}.display"));
    let quoted = format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''"));
    let command = "printf '%s' \"${DISPLAY-unset}\" > ".to_owned() + &quoted;
    evaluate(&format!("(wm-spawn {command:?})"), "#t");
    until(
        || fs::read_to_string(&path).is_ok_and(|s| s == expected),
        "spawned DISPLAY did not match",
    );
}
fn start_drag(anchor: &mut Input, pointer: u32, id: u32, button: u32) {
    anchor.focus();
    let start = anchor.events.len();
    command("xdotool", &["keydown", "Super_L"]);
    anchor.key(125, 1, start);
    evaluate(
        &format!("(begin (wm-focus-window {id}) (wm-warp-pointer 200 180))"),
        "#t",
    );
    anchor.send(pointer, 2, &words(&[0, button, 1]));
    anchor.roundtrip();
    command("xdotool", &["keyup", "Super_L"]);
}
struct X11Client {
    title: String,
    log: PathBuf,
    process: Child,
    window_id: u32,
}
impl X11Client {
    fn new(executable: &Path, title: &str) -> Self {
        let log = executable.parent().unwrap().join(format!("{title}.log"));
        let output = fs::File::create(&log).unwrap();
        let process = Command::new(executable)
            .args(["normal", title])
            .stdin(Stdio::piped())
            .stdout(output.try_clone().unwrap())
            .stderr(output)
            .env("DISPLAY", env::var("MINDE_X11_DISPLAY").unwrap())
            .spawn()
            .unwrap();
        let mut result = Self {
            title: title.into(),
            log,
            process,
            window_id: 0,
        };
        result.window_id = await_id(title);
        result
    }
    fn command(&mut self, command: &str) {
        let marker = format!("done {command}\n");
        let before = fs::read_to_string(&self.log)
            .unwrap()
            .matches(&marker)
            .count();
        writeln!(self.process.stdin.as_mut().unwrap(), "{command}").unwrap();
        self.process.stdin.as_mut().unwrap().flush().unwrap();
        until(
            || {
                fs::read_to_string(&self.log)
                    .unwrap()
                    .matches(&marker)
                    .count()
                    > before
            },
            "X11 command did not finish",
        );
    }
}
impl Drop for X11Client {
    fn drop(&mut self) {
        self.process.stdin.take();
        if self.process.try_wait().ok().flatten().is_none() {
            let _ = self.process.kill();
        }
        let _ = self.process.wait();
    }
}
pub fn metadata() {
    use smithay::reexports::x11rb::{
        COPY_DEPTH_FROM_PARENT,
        connection::Connection,
        protocol::xproto::{AtomEnum, ConnectionExt, CreateWindowAux, PropMode, WindowClass},
        wrapper::ConnectionExt as _,
    };
    let display = env::var("MINDE_X11_DISPLAY").unwrap();
    let (connection, screen) = smithay::reexports::x11rb::connect(Some(&display)).unwrap();
    let root = connection.setup().roots[screen].root;
    let window = connection.generate_id().unwrap();
    connection
        .create_window(
            COPY_DEPTH_FROM_PARENT,
            window,
            root,
            0,
            0,
            300,
            200,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new(),
        )
        .unwrap();
    let net_name = connection
        .intern_atom(false, b"_NET_WM_NAME")
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    let utf8 = connection
        .intern_atom(false, b"UTF8_STRING")
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    let name = u32::from(AtomEnum::WM_NAME);
    let class = u32::from(AtomEnum::WM_CLASS);
    let string_type = u32::from(AtomEnum::STRING);
    let set = |property, kind, value: &[u8]| {
        connection
            .change_property8(PropMode::REPLACE, window, property, kind, value)
            .unwrap();
    };
    set(name, string_type, b"X11 metadata initial");
    set(class, string_type, b"instance\0org.minde.initial\0");
    let mut observer = Observer::new();
    connection.map_window(window).unwrap();
    connection.flush().unwrap();
    let id = await_id("X11 metadata initial");
    let handle = observer.handle("X11 metadata initial");
    evaluate(
        "(begin (define x11-metadata-events '()) \
        (define (record-x11-metadata id title app) \
          (set! x11-metadata-events (cons (list id title app) x11-metadata-events))) \
        (add-event-hook! 'window-title-changed record-x11-metadata) #t)",
        "#t",
    );
    for (index, (property, kind, value, title, app, opcode)) in [
        (
            name,
            string_type,
            Some(b"X11 legacy changed".as_slice()),
            "X11 legacy changed",
            "org.minde.initial",
            0,
        ),
        (
            net_name,
            utf8,
            Some("X11 Grüße".as_bytes()),
            "X11 Grüße",
            "org.minde.initial",
            0,
        ),
        (
            class,
            string_type,
            Some(b"instance\0org.minde.changed\0".as_slice()),
            "X11 Grüße",
            "org.minde.changed",
            1,
        ),
        (
            net_name,
            utf8,
            None,
            "X11 legacy fallback",
            "org.minde.changed",
            0,
        ),
        (class, string_type, None, "X11 legacy fallback", "", 1),
        (name, string_type, None, "", "", 0),
    ]
    .into_iter()
    .enumerate()
    {
        observer.c.roundtrip();
        let start = observer.c.events.len();
        // WM_NAME must not supersede _NET_WM_NAME. The following class
        // notification orders this negative assertion through the XWM.
        if index == 2 {
            set(name, string_type, b"X11 legacy fallback");
        }
        // Repeated property notifications must not repeat Scheme or
        // foreign-toplevel updates. No repaint is requested by this client.
        for _ in 0..2 {
            if let Some(value) = value {
                set(property, kind, value);
            } else {
                connection.delete_property(window, property).unwrap();
            }
        }
        connection.flush().unwrap();
        let expected = if opcode == 0 { title } else { app };
        observer.c.wait(start, |e| {
            e.is(handle, opcode) && text(&e.data, 0) == expected
        });
        observer.c.wait(start, |e| e.is(handle, 5));
        observer.c.roundtrip();
        let events: Vec<_> = observer.c.events[start..]
            .iter()
            .filter(|e| e.object == handle)
            .map(|e| e.opcode)
            .collect();
        assert_eq!(
            events,
            [opcode, 5],
            "X11 metadata must have one property and done"
        );
        evaluate("(length x11-metadata-events)", &(index + 1).to_string());
        // Compare Scheme values: its printer may escape non-ASCII text.
        evaluate(
            &format!("(equal? (car x11-metadata-events) '({id} {title:?} {app:?}))"),
            "#t",
        );
        evaluate(
            &format!("(equal? (wm-window-title {id}) '({title:?} . {app:?}))"),
            "#t",
        );
    }
    connection.unmap_window(window).unwrap();
    connection.flush().unwrap();
    observer.c.event(handle, 6);
    until(
        || ipc(&format!("(wm-window-title {id})")) == "(ok #f)",
        "X11 metadata window stayed registered",
    );
    set(name, string_type, b"X11 metadata remapped");
    set(class, string_type, b"instance\0org.minde.remapped\0");
    connection.map_window(window).unwrap();
    connection.flush().unwrap();
    let remapped = await_id("X11 metadata remapped");
    assert!(remapped > id);
    let new_handle = observer.handle("X11 metadata remapped");
    assert_ne!(new_handle, handle);
    assert!(
        observer
            .c
            .events
            .iter()
            .any(|e| e.is(new_handle, 1) && text(&e.data, 0) == "org.minde.remapped")
    );
    evaluate("(length x11-metadata-events)", "6");
    connection.destroy_window(window).unwrap();
    connection.flush().unwrap();
    observer.c.event(new_handle, 6);
    evaluate(
        "(begin (remove-event-hook! 'window-title-changed record-x11-metadata) #t)",
        "#t",
    );
    println!(
        "ok - X11 title/class changes notify Scheme and foreign handles without repaint or duplicates"
    );
    println!("ok - X11 title precedence, property deletion and remap metadata stay synchronized");
}
pub fn run() {
    struct ReleaseSuper;
    impl Drop for ReleaseSuper {
        fn drop(&mut self) {
            let _ = Command::new("timeout")
                .args(["5", "xdotool", "keyup", "Super_L"])
                .status();
        }
    }
    let _release = ReleaseSuper;
    let executable = PathBuf::from(env::var("MINDE_X11_TEST_CLIENT").unwrap());
    let out = executable.parent().unwrap();
    let runtime = PathBuf::from(env::var("XDG_RUNTIME_DIR").unwrap());
    let startup = runtime.join("startup-display");
    until(|| startup.exists(), "startup probe did not run");
    assert_eq!(fs::read_to_string(startup).unwrap(), "unset");
    println!("ok - autostart has no DISPLAY before Xwayland is ready");
    let wm_pid: u32 = env::var("MINDE_TEST_COMPOSITOR_PID")
        .unwrap()
        .parse()
        .unwrap();
    let xdisplay = env::var("MINDE_X11_DISPLAY").unwrap();
    let host_window = command("xdotool", &["search", "--sync", "--name", "Smithay"])
        .lines()
        .next()
        .unwrap()
        .to_owned();
    command("xdotool", &["windowfocus", &host_window]);
    let pixels = HostPixels::for_window(host_window.parse().unwrap());
    let mut anchor = Input::new("native lifecycle anchor");
    let mut observer = Observer::new();
    anchor.mapped(700);
    let pointer = anchor.virtual_pointer();
    let mut target = X11Client::new(&executable, "x11-lifecycle-target");
    spawn_display(out, "ready", &xdisplay);
    let cases = env::var("MINDE_XWAYLAND_CASES").unwrap_or_else(|_| "unmap loss".into());
    if cases.split_whitespace().any(|s| s == "unmap") {
        for button in [272, 273] {
            let current = target.window_id;
            let handle = observer.handle(&target.title);
            evaluate(
                &format!("(begin (float-window! {current} '(100 100 300 250)) (sync-frames!) #t)"),
                "#t",
            );
            start_drag(&mut anchor, pointer, current, button);
            target.command("unmap");
            until(
                || managed_id(&target.title).is_none(),
                "X11 unmap stayed registered",
            );
            let start = anchor.events.len();
            evaluate("(wm-warp-pointer 800 180)", "#t");
            let anchor_pointer = anchor.pointer;
            anchor.wait(start, |e| e.is(anchor_pointer, 0));
            observer.c.event(handle, 6);
            evaluate(&format!("(wm-window-geometry {current})"), "#f");
            evaluate("(length (all-window-ids))", "1");
            anchor.send(pointer, 2, &words(&[0, button, 0]));
            anchor.roundtrip();
            target.command("map");
            target.window_id = await_id(&target.title);
            assert!(target.window_id > current);
            assert_eq!(observer.live_handles().len(), 2);
        }
        println!("ok - X11 unmap cancels move/resize grabs and remaps with a fresh ID");
    }
    if cases.split_whitespace().any(|s| s == "loss") {
        let second = X11Client::new(&executable, "x11-lifecycle-second");
        let handles = [
            observer.handle(&target.title),
            observer.handle(&second.title),
        ];
        evaluate(
            &format!(
                "(begin (unfloat-window! {}) (float-window! {} '(100 100 300 250)) \
                            (float-window! {} '(100 450 300 250)) (sync-frames!) #t)",
                anchor.window_id, target.window_id, second.window_id
            ),
            "#t",
        );
        evaluate("(wm-set-clipboard \"native clipboard survives\")", "#t");
        start_drag(&mut anchor, pointer, target.window_id, 272);
        let pointer_start = anchor.events.len();
        target.command("menu");
        until(
            || pixels.samples().contains(&0x00ff00),
            "override-redirect menu not rendered",
        );
        // Only signal a verified direct child of this private compositor
        // serving its expected X display.
        let children =
            fs::read_to_string(format!("/proc/{wm_pid}/task/{wm_pid}/children")).unwrap();
        let mut servers = Vec::new();
        for child in children.split_whitespace() {
            let pid: i32 = child.parse().unwrap();
            if fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|s| s.trim() == "Xwayland")
            {
                let args = fs::read(format!("/proc/{pid}/cmdline")).unwrap();
                assert!(
                    args.split(|b| *b == 0).any(|a| a == xdisplay.as_bytes()),
                    "unexpected X display"
                );
                servers.push(pid);
            }
        }
        assert_eq!(servers.len(), 1);
        assert_eq!(unsafe { libc::kill(servers[0], libc::SIGKILL) }, 0);
        until(
            || ipc("(cadr (wm-runtime-info))") == "(ok \"failed\")",
            "dead Xwayland reported ready",
        );
        evaluate("(caddr (wm-runtime-info))", "-1");
        evaluate("(all-window-ids)", &format!("({})", anchor.window_id));
        evaluate("(focused-window-id)", &anchor.window_id.to_string());
        for handle in handles {
            observer.c.event(handle, 6);
        }
        assert_eq!(observer.live_handles().len(), 1);
        until(
            || !pixels.samples().contains(&0x00ff00),
            "dead X11 menu remained rendered",
        );
        evaluate("(wm-warp-pointer 800 180)", "#t");
        let anchor_pointer = anchor.pointer;
        anchor.wait(pointer_start, |e| e.is(anchor_pointer, 0));
        anchor.send(pointer, 2, &words(&[0, 272, 0]));
        anchor.roundtrip();
        spawn_display(out, "failed", "unset");
        let clipboard = Command::new("timeout")
            .args(["5", "wl-paste", "--no-newline"])
            .env(
                "WAYLAND_DISPLAY",
                env::var("NESTED_WAYLAND_DISPLAY").unwrap(),
            )
            .output()
            .unwrap();
        assert!(clipboard.status.success());
        assert_eq!(clipboard.stdout, b"native clipboard survives");
        let status = runtime.join("minde/status.json");
        until(
            || {
                Command::new("timeout")
                    .args([
                        "5",
                        "jq",
                        "-e",
                        ".runtime.xwayland == \"failed\" and .runtime.xdisplay == null",
                    ])
                    .arg(&status)
                    .output()
                    .unwrap()
                    .status
                    .success()
            },
            "published runtime kept dead display",
        );
        anchor.focus();
        let start = anchor.events.len();
        evaluate("(wm-send-string \"a\" 0)", "#t");
        anchor.key(30, 0, start);
        println!(
            "ok - Xwayland loss removes X11 windows, handles and menus; native input survives"
        );
        println!("ok - failed Xwayland clears status and DISPLAY in newly spawned children");
    }
}
