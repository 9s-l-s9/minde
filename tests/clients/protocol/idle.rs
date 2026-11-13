// SPDX-License-Identifier: GPL-3.0-or-later
use super::wire::*;
use std::{
    env, fs, io,
    process::{Child, Command},
    time::{Duration, Instant},
};

fn idle_ms() -> u64 {
    ipc("(wm-idle-ms)")
        .strip_prefix("(ok ")
        .unwrap()
        .strip_suffix(')')
        .unwrap()
        .parse()
        .unwrap()
}

fn clock() {
    let startup = super::input::numbers(&ipc("(list idle-startup-before idle-startup-after)"));
    assert!(
        startup[1] >= startup[0] + 250,
        "idle time did not advance before backend creation: {startup:?}"
    );
    let advances = || {
        let started = Instant::now();
        let before = idle_ms();
        std::thread::sleep(Duration::from_millis(300));
        let after = idle_ms();
        assert!(
            after >= before + 250 && after - before <= started.elapsed().as_millis() as u64 + 20,
            "idle time did not advance with elapsed time: {before} -> {after}"
        );
    };
    // Queries themselves must not reset activity after startup either.
    advances();
    let mut c = Client::new("idle clock activity");
    let manager = c.bind("zwlr_virtual_pointer_manager_v1", 1);
    let pointer = c.alloc();
    c.send(manager, 0, &words(&[0, pointer]));
    c.roundtrip();
    let started = Instant::now();
    c.send(pointer, 0, &words(&[0, 256, 256]));
    c.roundtrip();
    let after = idle_ms();
    assert!(
        after <= started.elapsed().as_millis() as u64 + 20,
        "pointer activity did not reset idle time: {after}"
    );
    advances();
    super::input::evaluate("(<= (abs (- (wm-idle-ms) (idle-ms))) 1)", "#t");
    println!("ok - Scheme idle time advances from startup and resets on input");
}

struct Observer {
    c: Client,
    manager: u32,
    seat: u32,
    notification: u32,
}
impl Observer {
    fn new() -> Self {
        let mut c = Client::new("idle lifecycle observer");
        let manager = c.bind("ext_idle_notifier_v1", 2);
        let seat = c.bind("wl_seat", 1);
        let notification = c.alloc();
        c.send(manager, 1, &words(&[notification, 100, seat]));
        c.roundtrip();
        Self {
            c,
            manager,
            seat,
            notification,
        }
    }
    fn states(&mut self, expected: &[u16]) {
        self.c.roundtrip();
        let actual: Vec<_> = self
            .c
            .events
            .iter()
            .filter(|e| e.object == self.notification)
            .map(|e| e.opcode)
            .collect();
        assert_eq!(actual, expected, "idle/resumed event history");
    }
    fn suppressed(&mut self, expected: &[u16]) {
        // Input-only notifications ignore inhibitors. Their timer proves the
        // ordinary notification's timeout elapsed without injecting any input.
        let probe = self.c.alloc();
        self.c
            .send(self.manager, 2, &words(&[probe, 250, self.seat]));
        self.c.event(probe, 0);
        self.states(expected);
        self.c.send(probe, 0, &[]);
    }
    fn next(&mut self, opcode: u16) {
        let start = self.c.events.len();
        self.c.wait(start, |e| e.is(self.notification, opcode));
    }
}

fn inhibit(c: &mut Client, surface: u32) -> u32 {
    let manager = c.bind("zwp_idle_inhibit_manager_v1", 1);
    let inhibitor = c.alloc();
    c.send(manager, 1, &words(&[inhibitor, surface]));
    c.send(manager, 0, &[]);
    c.roundtrip();
    inhibitor
}
fn release(c: &mut Client, inhibitor: u32) {
    c.send(inhibitor, 0, &[]);
    c.roundtrip();
}

fn shared() {
    let mut owner = Client::new("shared idle inhibitors");
    let surface = owner.new_surface();
    let a = inhibit(&mut owner, surface);
    let b = inhibit(&mut owner, surface);
    let mut observer = Observer::new();
    observer.suppressed(&[]);
    release(&mut owner, a);
    observer.suppressed(&[]);
    release(&mut owner, b);
    observer.next(0);
    observer.states(&[0]);
    let a = inhibit(&mut owner, surface);
    let b = inhibit(&mut owner, surface);
    observer.next(1);
    release(&mut owner, b);
    observer.suppressed(&[0, 1]);
    release(&mut owner, a);
    observer.next(0);
    observer.states(&[0, 1, 0]);
    println!("ok - each inhibitor on a shared surface retains its own lifetime");
}

fn surface_destroyed() {
    let mut owner = Client::new("destroyed inhibiting surface");
    let surface = owner.new_surface();
    let a = inhibit(&mut owner, surface);
    let b = inhibit(&mut owner, surface);
    let mut observer = Observer::new();
    observer.suppressed(&[]);
    owner.send(surface, 0, &[]);
    owner.roundtrip();
    observer.next(0);
    let replacement = owner.new_surface();
    let live = inhibit(&mut owner, replacement);
    observer.next(1);
    release(&mut owner, a);
    release(&mut owner, b);
    observer.suppressed(&[0, 1]);
    release(&mut owner, live);
    observer.next(0);
    observer.states(&[0, 1, 0]);
    println!(
        "ok - surface destruction releases all inhibitors; late destroys cannot affect a replacement"
    );
}

fn disconnected() {
    let mut peer = Client::new("surviving inhibitor client");
    let peer_surface = peer.new_surface();
    let live = inhibit(&mut peer, peer_surface);
    let mut owner = Client::new("disconnected inhibitor client");
    let surface = owner.new_surface();
    inhibit(&mut owner, surface);
    inhibit(&mut owner, surface);
    let mut observer = Observer::new();
    drop(owner);
    observer.suppressed(&[]);
    release(&mut peer, live);
    observer.next(0);
    let mut owner = Client::new("last disconnected inhibitor client");
    let surface = owner.new_surface();
    inhibit(&mut owner, surface);
    observer.next(1);
    drop(owner);
    observer.next(0);
    observer.states(&[0, 1, 0]);
    println!(
        "ok - disconnect preserves other clients' inhibitors and releases the last one without input"
    );
}

struct Holder(Child);
impl Drop for Holder {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn killed() {
    let ready = format!(
        "{}/idle-holder-{}.ready",
        env::var("XDG_RUNTIME_DIR").unwrap(),
        std::process::id()
    );
    let mut child = Holder(
        Command::new(env::current_exe().unwrap())
            .args(["idle-inhibitor-holder", &ready])
            .spawn()
            .unwrap(),
    );
    until(
        || fs::exists(&ready).unwrap(),
        "inhibitor holder did not become ready",
    );
    fs::remove_file(&ready).unwrap();
    let mut observer = Observer::new();
    observer.suppressed(&[]);
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    observer.next(0);
    observer.states(&[0]);
    println!("ok - SIGKILL releases idle inhibition without any input event");
}

pub fn holder(ready: &str) {
    let mut c = Client::new("killed idle inhibitor holder");
    let surface = c.new_surface();
    inhibit(&mut c, surface);
    inhibit(&mut c, surface);
    fs::write(ready, "ready").unwrap();
    loop {
        match c.read(Instant::now() + Duration::from_secs(2)) {
            Ok(_) => {}
            Err(Error::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) => {}
            Err(_) => return,
        }
    }
}

pub fn run(selected: &[String]) {
    for (name, run) in [
        ("clock", clock as fn()),
        ("shared", shared),
        ("surface", surface_destroyed),
        ("disconnect", disconnected),
        ("killed", killed),
    ] {
        if selected.is_empty() || selected.iter().any(|s| s == name) {
            run();
        }
    }
}
