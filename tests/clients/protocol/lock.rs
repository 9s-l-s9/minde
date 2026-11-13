// SPDX-License-Identifier: GPL-3.0-or-later
use super::wire::*;
use smithay::reexports::x11rb::{
    connection::Connection,
    protocol::xproto::{ConnectionExt, ImageFormat, ImageOrder},
    rust_connection::RustConnection,
};
use std::{
    env,
    io::{self, BufRead, BufReader},
    os::unix::net::UnixStream,
    time::Instant,
};

fn event_policy_while_locked() {
    let socket = UnixStream::connect(format!(
        "{}/minde-events.sock",
        env::var("XDG_RUNTIME_DIR").unwrap()
    ))
    .unwrap();
    let mut subscriber = BufReader::new(socket);
    let start = Instant::now();
    while ipc("(wm-events-active?)") != "(ok #t)" {
        assert!(
            start.elapsed() < TIMEOUT,
            "event subscriber was not accepted"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let reply = ipc("(begin
          (define lock-urgent-id (car (all-window-ids)))
          (define lock-urgent-cursor (wm-event-cursor))
          (handle-urgent-window! lock-urgent-id)
          (run-event-hook! 'private-extension \"private payload\")
          (run-event-hook! 'focus-frame 901 902 903 904)
          lock-urgent-id)");
    let id: u32 = reply
        .strip_prefix("(ok ")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap()
        .parse()
        .unwrap();
    let expected = format!("(urgent-window {id})");
    let mut urgent = false;
    let start = Instant::now();
    loop {
        let remaining = TIMEOUT
            .checked_sub(start.elapsed())
            .expect("locked event stream did not reach its marker");
        subscriber
            .get_ref()
            .set_read_timeout(Some(remaining))
            .unwrap();
        let mut line = String::new();
        assert!(subscriber.read_line(&mut line).unwrap() > 0);
        let line = line.trim_end();
        assert!(
            !line.starts_with("(message "),
            "locked stream exposed a message"
        );
        assert!(
            !line.starts_with("(private-extension "),
            "locked stream exposed an unknown event"
        );
        urgent |= line == expected;
        if line == "(focus-frame 901 902 903 904)" {
            break;
        }
    }
    assert!(urgent, "live locked stream omitted the urgency event");
    assert_eq!(
        ipc("(let* ((reply (events-since lock-urgent-cursor))
                     (events (vector->list (assq-ref reply 'events))))
               (and (not (assq-ref reply 'gap))
                    (any (lambda (event)
                           (and (eq? (assq-ref event 'name) 'urgent-window)
                                (equal? (assq-ref event 'arguments) (vector lock-urgent-id)))) events)
                    (not (any (lambda (event)
                                (memq (assq-ref event 'name) '(message private-extension))) events))))"),
        "(ok #t)",
        "locked journal and live stream disagree"
    );
    assert_eq!(ipc("(begin (clear-urgent! lock-urgent-id) #t)"), "(ok #t)");
    println!("ok - locked live events and journal retain urgency and suppress private payloads");
}

pub struct HostPixels {
    connection: RustConnection,
    window: u32,
}
impl HostPixels {
    pub fn new() -> Self {
        let window = env::var("MINDE_SESSION_LOCK_TEST_WINDOW")
            .unwrap()
            .parse()
            .unwrap();
        Self::for_window(window)
    }
    pub fn for_window(window: u32) -> Self {
        let (connection, _) = RustConnection::connect(Some(&env::var("DISPLAY").unwrap())).unwrap();
        Self { connection, window }
    }
    pub fn samples(&self) -> Vec<u32> {
        let geometry = self
            .connection
            .get_geometry(self.window)
            .unwrap()
            .reply()
            .unwrap();
        let image = self
            .connection
            .get_image(
                ImageFormat::Z_PIXMAP,
                self.window,
                0,
                0,
                geometry.width,
                geometry.height,
                u32::MAX,
            )
            .unwrap()
            .reply()
            .unwrap();
        let format = self
            .connection
            .setup()
            .pixmap_formats
            .iter()
            .find(|f| f.depth == image.depth)
            .unwrap();
        assert_eq!(format.bits_per_pixel, 32, "private Xvfb pixel format");
        let stride = image.data.len() / geometry.height as usize;
        (1..4)
            .flat_map(|x| (1..4).map(move |y| (x, y)))
            .map(|(x, y)| {
                let offset =
                    geometry.height as usize * y / 4 * stride + geometry.width as usize * x / 4 * 4;
                let pixel = image.data[offset..offset + 4].try_into().unwrap();
                let value = if self.connection.setup().image_byte_order == ImageOrder::LSB_FIRST {
                    u32::from_le_bytes(pixel)
                } else {
                    u32::from_be_bytes(pixel)
                };
                value & 0xffffff
            })
            .collect()
    }
    pub fn pixel(&self, x: i16, y: i16) -> u32 {
        let image = self
            .connection
            .get_image(ImageFormat::Z_PIXMAP, self.window, x, y, 1, 1, u32::MAX)
            .unwrap()
            .reply()
            .unwrap();
        let pixel = image.data[..4].try_into().unwrap();
        let value = if self.connection.setup().image_byte_order == ImageOrder::LSB_FIRST {
            u32::from_le_bytes(pixel)
        } else {
            u32::from_be_bytes(pixel)
        };
        value & 0xffffff
    }
    pub fn wait_black(&self) {
        until(
            || self.samples().iter().all(|p| *p == 0),
            "lock surface did not turn black",
        );
    }
    pub fn wait_desktop(&self) {
        until(
            || self.samples().contains(&0xff00ff),
            "desktop not presented",
        );
    }
}
pub fn inject_key(recipient: &mut Client, excluded: &mut [&mut Client]) {
    recipient.roundtrip();
    for c in excluded.iter_mut() {
        c.roundtrip();
        let focus = c.keyboard_events(0, &[1, 2]);
        assert!(
            focus.last().is_none_or(|e| e.opcode == 2),
            "{} retained focus while locked",
            c.name
        );
    }
    let start = recipient.events.len();
    let starts: Vec<_> = excluded.iter().map(|c| c.events.len()).collect();
    command("xdotool", &["key", "--clearmodifiers", "a"]);
    let keyboard = recipient.keyboard;
    recipient.wait(start, |e| {
        e.is(keyboard, 3) && e.word(2) == 30 && e.word(3) == 0
    });
    let keys = recipient.keyboard_events(start, &[3]);
    for state in [0, 1] {
        assert!(
            keys.iter().any(|e| e.word(2) == 30 && e.word(3) == state),
            "incomplete key: {keys:?}"
        );
    }
    for (c, start) in excluded.iter_mut().zip(starts) {
        c.roundtrip();
        assert!(
            c.keyboard_events(start, &[1, 3]).is_empty(),
            "{} received lock-screen input",
            c.name
        );
    }
}
fn unlock(owner: &mut Client, lock: u32, desktop: &mut Client, ordinary: &mut Client) {
    let starts = [desktop.events.len(), ordinary.events.len()];
    owner.send(lock, 2, &[]);
    owner.roundtrip();
    locked(false);
    for (c, start) in [desktop, ordinary].into_iter().zip(starts) {
        c.roundtrip();
        if c.keyboard_events(start, &[1, 2])
            .last()
            .is_some_and(|e| e.opcode == 1)
        {
            inject_key(c, &mut []);
            return;
        }
    }
    panic!("unlock failed to restore ordinary focus");
}
pub fn run() {
    let pixels = HostPixels::new();
    locked(false);
    let mut desktop = Client::new("desktop before lock");
    desktop.toplevel(true);
    desktop.event(desktop.keyboard, 1);
    inject_key(&mut desktop, &mut []);
    pixels.wait_desktop();
    assert_eq!(
        ipc("(begin (split-frame-horizontal!)
          (define lock-selection-layout (dump-frames))
          (set-key-state! (expose-mode!)) #t)"),
        "(ok #t)"
    );
    let mut owner = Client::new("active lock owner");
    let lock = owner.lock(true);
    assert!(
        pixels.samples().iter().all(|p| *p == 0),
        "lock confirmed before black frame"
    );
    locked(true);
    assert_eq!(
        ipc("(and (eq? %key-state 'normal)
                  (equal? lock-selection-layout (dump-frames)))"),
        "(ok #t)",
        "locking must cancel expose and restore its layout"
    );
    desktop.roundtrip();
    println!("ok - lock confirmation observes a cleared nested framebuffer");
    event_policy_while_locked();

    let mut attacker = Client::new("denied lock surface attacker");
    let denied = attacker.lock(false);
    let denied_surface = attacker.lock_surface(denied, false);
    attacker.roundtrip();
    assert!(!attacker.events.iter().any(|e| e.is(denied_surface, 0)));
    assert!(attacker.keyboard_events(0, &[1, 3]).is_empty());
    locked(true);
    owner.lock_surface(lock, true);
    owner.event(owner.keyboard, 1);
    inject_key(&mut owner, &mut [&mut desktop, &mut attacker]);
    println!("ok - live owner cannot be replaced; denied surface cannot steal input");

    let active_surface = owner.surface;
    let denied = owner.lock(false);
    let start = owner.events.len();
    let denied_surface = owner.lock_surface(denied, false);
    owner.roundtrip();
    assert!(!owner.events.iter().any(|e| e.is(denied_surface, 0)));
    assert!(owner.keyboard_events(start, &[1]).is_empty());
    owner.surface = active_surface;
    owner.send(denied, 0, &[]);
    owner.roundtrip();
    locked(true);
    inject_key(&mut owner, &mut [&mut desktop, &mut attacker]);
    println!("ok - second lock object on owner's connection has no ownership");

    let mut ordinary = Client::new("ordinary toplevel created while locked");
    ordinary.toplevel(true);
    assert!(ordinary.keyboard_events(0, &[1, 3]).is_empty());
    inject_key(
        &mut owner,
        &mut [&mut desktop, &mut ordinary, &mut attacker],
    );
    println!("ok - mapped ordinary toplevel receives no lock-screen keyboard input");

    let mut ime = Client::new("input method grabbing while locked");
    let manager = ime.bind("zwp_input_method_manager_v2", 1);
    let seat = ime.bind("wl_seat", 1);
    let method = ime.alloc();
    ime.send(manager, 0, &words(&[seat, method]));
    ime.roundtrip();
    assert!(!ime.events.iter().any(|e| e.is(method, 6)));
    let grab = ime.alloc();
    ime.send(method, 5, &words(&[grab]));
    ime.roundtrip();
    assert!(ime.events.iter().any(|e| e.is(grab, 0)));
    let start = ime.events.len();
    inject_key(
        &mut owner,
        &mut [&mut desktop, &mut ordinary, &mut attacker],
    );
    ime.roundtrip();
    assert!(!ime.events[start..].iter().any(|e| e.is(grab, 1)));
    locked(true);
    ime.send(grab, 0, &[]);
    ime.send(method, 6, &[]);
    ime.roundtrip();
    inject_key(
        &mut owner,
        &mut [&mut desktop, &mut ordinary, &mut attacker],
    );
    println!("ok - IME keyboard grab cannot capture lock-screen input");

    let mut unlock_attacker = Client::new("denied lock unlock attacker");
    let denied = unlock_attacker.lock(false);
    unlock_attacker.send(denied, 2, &[]);
    unlock_attacker.rejected();
    locked(true);
    owner.roundtrip();
    inject_key(
        &mut owner,
        &mut [&mut desktop, &mut ordinary, &mut attacker],
    );
    println!("ok - denied unlock cannot unlock another client's session");
    unlock(&mut owner, lock, &mut desktop, &mut ordinary);
    println!("ok - owner unlock restores ordinary keyboard input");

    let mut surface_owner = Client::new("owner destroying focused wl_surface");
    let surface_lock = surface_owner.lock(true);
    surface_owner.lock_surface(surface_lock, true);
    let inhibitor = super::focus::inhibitor(&mut surface_owner);
    surface_owner.event(inhibitor, 0);
    inject_key(
        &mut surface_owner,
        &mut [&mut desktop, &mut ordinary, &mut attacker],
    );
    surface_owner.send(surface_owner.surface, 0, &[]);
    surface_owner.roundtrip();
    surface_owner.event(inhibitor, 1);
    locked(true);
    pixels.wait_black();
    let start = surface_owner.events.len();
    let starts = [desktop.events.len(), ordinary.events.len()];
    command("xdotool", &["key", "--clearmodifiers", "a"]);
    surface_owner.roundtrip();
    locked(true);
    for (c, from) in [&mut desktop, &mut ordinary].into_iter().zip(starts) {
        c.roundtrip();
        assert!(
            c.keyboard_events(from, &[1, 3]).is_empty(),
            "destroyed lock surface exposed input"
        );
    }
    assert!(surface_owner.keyboard_events(start, &[1, 3]).is_empty());
    unlock(
        &mut surface_owner,
        surface_lock,
        &mut desktop,
        &mut ordinary,
    );
    println!("ok - focused wl_surface destruction keeps live owner locked and able to unlock");

    let mut abandoned = Client::new("owner that disconnects");
    let abandoned_lock = abandoned.lock(true);
    abandoned.lock_surface(abandoned_lock, true);
    desktop.roundtrip();
    ordinary.roundtrip();
    let starts = [desktop.events.len(), ordinary.events.len()];
    drop(abandoned);
    ordinary.roundtrip();
    locked(true);
    ordinary.roundtrip();
    let mut takeover = Client::new("abandoned lock takeover");
    let takeover_lock = takeover.lock(true);
    assert!(pixels.samples().iter().all(|p| *p == 0));
    locked(true);
    for (c, start) in [&mut desktop, &mut ordinary].into_iter().zip(starts) {
        c.roundtrip();
        assert!(c.keyboard_events(start, &[1, 3]).is_empty());
    }
    takeover.lock_surface(takeover_lock, true);
    inject_key(&mut takeover, &mut [&mut desktop, &mut ordinary]);
    unlock(&mut takeover, takeover_lock, &mut desktop, &mut ordinary);
    println!("ok - owner death remains locked; replacement owner can unlock");

    let mut duplicate = Client::new("duplicate output lock owner");
    let duplicate_lock = duplicate.lock(true);
    duplicate.lock_surface(duplicate_lock, true);
    duplicate.lock_surface(duplicate_lock, false);
    duplicate.rejected();
    locked(true);
    let mut replacement = Client::new("replacement after duplicate output");
    let replacement_lock = replacement.lock(true);
    replacement.lock_surface(replacement_lock, true);
    inject_key(
        &mut replacement,
        &mut [&mut desktop, &mut ordinary, &mut attacker],
    );
    replacement.send(replacement_lock, 2, &[]);
    replacement.roundtrip();
    locked(false);
    println!("ok - duplicate physical output through a second wl_output bind is rejected");

    let mut same = Client::new("same-client unauthorized unlock");
    let same_lock = same.lock(true);
    same.lock_surface(same_lock, true);
    let denied = same.lock(false);
    same.send(denied, 2, &[]);
    same.rejected();
    locked(true);
    let mut recovery = Client::new("recovery after same-client unlock");
    let recovery_lock = recovery.lock(true);
    recovery.lock_surface(recovery_lock, true);
    inject_key(
        &mut recovery,
        &mut [&mut desktop, &mut ordinary, &mut attacker],
    );
    recovery.send(recovery_lock, 2, &[]);
    recovery.roundtrip();
    locked(false);
    println!("ok - denied lock on owner's connection cannot unlock session");

    let mut pending = Client::new("owner aborting unconfirmed lock");
    let compositor = pending.bind("wl_compositor", 1);
    let output = pending.bind("wl_output", 1);
    let manager = pending.bind("ext_session_lock_manager_v1", 1);
    pending.roundtrip();
    let surface = pending.alloc();
    let lock = pending.alloc();
    let role = pending.alloc();
    pending.send_batch(&[
        (compositor, 0, words(&[surface])),
        (manager, 1, words(&[lock])),
        (lock, 1, words(&[role, surface, output])),
        (role, 0, vec![]),
        (surface, 0, vec![]),
        (lock, 0, vec![]),
    ]);
    pending.roundtrip();
    assert!(!pending.events.iter().any(|e| e.is(lock, 0)));
    locked(true);
    let retry = pending.lock(false);
    pending.send(retry, 0, &[]);
    pending.roundtrip();
    locked(true);
    let mut recovery = Client::new("fresh connection after pending-lock abort");
    let recovery_lock = recovery.lock(true);
    recovery.lock_surface(recovery_lock, true);
    inject_key(
        &mut recovery,
        &mut [&mut desktop, &mut ordinary, &mut attacker],
    );
    recovery.send(recovery_lock, 2, &[]);
    recovery.roundtrip();
    locked(false);
    println!("ok - pending lock abort requires reconnection and fresh owner can recover");
    println!("session-lock-e2e: all checks passed");
}
pub fn emergency_exit() {
    let mut owner = Client::new("emergency exit while locked");
    let lock = owner.lock(true);
    owner.lock_surface(lock, true);
    owner.event(owner.keyboard, 1);
    locked(true);
    command("xdotool", &["key", "ctrl+alt+BackSpace"]);
    let end = Instant::now() + TIMEOUT;
    loop {
        match owner.read(end) {
            Ok(_) => {}
            Err(Error::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset
                ) =>
            {
                break;
            }
            error => panic!("emergency exit failed: {error:?}"),
        }
    }
}
