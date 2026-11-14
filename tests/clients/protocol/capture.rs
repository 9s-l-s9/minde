// SPDX-License-Identifier: GPL-3.0-or-later
use super::{input::token, wire::*};
pub mod damage;
use std::{
    env,
    fs::File,
    ops::{Deref, DerefMut},
    os::{fd::AsRawFd, unix::fs::FileExt},
    path::Path,
    process::Command,
};

type Request = (u32, u16, Vec<u8>);
type Terminal = (u32, u16, u16);
struct Frame {
    ext: bool,
    id: u32,
    request: Option<Request>,
    buffer: Option<(File, (u32, u32))>,
}
pub(super) struct Capture {
    c: Client,
    output: u32,
    sources: u32,
    ext: u32,
    wlr: u32,
    outputs: u32,
    locks: u32,
    shm: u32,
    head: u32,
    session: u32,
}
impl Deref for Capture {
    type Target = Client;
    fn deref(&self) -> &Client {
        &self.c
    }
}
impl DerefMut for Capture {
    fn deref_mut(&mut self) -> &mut Client {
        &mut self.c
    }
}
impl Capture {
    pub(super) fn new() -> Self {
        let mut c = Client::new("capture lifecycle");
        let output = c.bind("wl_output", 1);
        let sources = c.bind("ext_output_image_capture_source_manager_v1", 1);
        let ext = c.bind("ext_image_copy_capture_manager_v1", 1);
        let wlr = c.bind("zwlr_screencopy_manager_v1", 3);
        let outputs = c.bind("zwlr_output_manager_v1", 4);
        let locks = c.bind("ext_session_lock_manager_v1", 1);
        let shm = c.bind("wl_shm", 1);
        c.roundtrip();
        let head = c.event(outputs, 0).word(0);
        let mut result = Self {
            c,
            output,
            sources,
            ext,
            wlr,
            outputs,
            locks,
            shm,
            head,
            session: 0,
        };
        result.session = result.new_session();
        result
    }
    fn new_session(&mut self) -> u32 {
        let source = self.alloc();
        let session = self.alloc();
        self.send(self.sources, 0, &words(&[source, self.output]));
        self.send(self.ext, 0, &words(&[session, source, 0]));
        self.event(session, 4);
        session
    }
    pub(super) fn image(&mut self, ext: bool, region: Option<[u32; 4]>) -> Image {
        self.image_with_cursor(ext, region, false)
    }
    pub(super) fn cursor_image(&mut self) -> Image {
        self.image_with_cursor(false, None, true)
    }
    fn image_with_cursor(&mut self, ext: bool, region: Option<[u32; 4]>, cursor: bool) -> Image {
        self.roundtrip();
        let frame = if region.is_some() || cursor {
            assert!(!ext);
            let id = self.alloc();
            let mut args = words(&[id, u32::from(cursor), self.output]);
            if let Some(region) = region {
                args.extend(words(&region));
            }
            self.send(self.wlr, u16::from(region.is_some()), &args);
            self.event(id, 6);
            let info = self.event(id, 0);
            let size = (info.word(1), info.word(2));
            assert_eq!(info.word(3), size.0 * 4);
            let file = memfile((size.0 * size.1 * 4 + 64) as usize, 0xa5);
            let buffer = self.buffer_from(size.0, size.1, &file);
            Frame {
                ext,
                id,
                request: Some((id, 0, words(&[buffer]))),
                buffer: Some((file, size)),
            }
        } else {
            self.prepare(ext, None)
        };
        self.send_batch(std::slice::from_ref(frame.request.as_ref().unwrap()));
        let ready = if ext { 3 } else { 2 };
        let failed = if ext { 4 } else { 3 };
        assert_eq!(
            self.wait(0, |e| e.object == frame.id
                && [ready, failed].contains(&e.opcode))
                .opcode,
            ready
        );
        let (file, size) = frame.buffer.as_ref().unwrap();
        let mut bytes = vec![0; (size.0 * size.1 * 4) as usize];
        file.read_exact_at(&mut bytes, 0).unwrap();
        let image = Image {
            width: size.0,
            height: size.1,
            pixels: bytes
                .chunks_exact(4)
                .map(|p| u32::from_ne_bytes(p.try_into().unwrap()) & 0xffffff)
                .collect(),
        };
        self.result(frame, true, None);
        image
    }
    fn size(&self) -> (u32, u32) {
        let e = self.events.iter().rfind(|e| e.is(self.session, 0)).unwrap();
        (e.word(0), e.word(1))
    }
    fn prepare(&mut self, ext: bool, size: Option<(u32, u32)>) -> Frame {
        let id = self.alloc();
        let size = if ext {
            self.send(self.session, 0, &words(&[id]));
            size.unwrap_or_else(|| self.size())
        } else {
            self.send(self.wlr, 0, &words(&[id, 0, self.output]));
            let e = self.wait(0, |e| e.object == id && [3, 6].contains(&e.opcode));
            if e.opcode == 3 {
                return Frame {
                    ext,
                    id,
                    request: None,
                    buffer: None,
                };
            }
            let e = self.event(id, 0);
            (e.word(1), e.word(2))
        };
        let file = memfile((size.0 * size.1 * 4 + 64) as usize, 0xa5);
        let pool = self.alloc();
        let buffer = self.alloc();
        self.send_fd(
            self.shm,
            0,
            &words(&[pool, size.0 * size.1 * 4 + 64]),
            file.as_raw_fd(),
        );
        self.send(pool, 0, &words(&[buffer, 0, size.0, size.1, size.0 * 4, 1]));
        self.send(pool, 1, &[]);
        let request = if ext {
            self.send(id, 1, &words(&[buffer]));
            self.send(id, 2, &words(&[0, 0, size.0, size.1]));
            (id, 3, vec![])
        } else {
            (id, 0, words(&[buffer]))
        };
        Frame {
            ext,
            id,
            request: Some(request),
            buffer: Some((file, size)),
        }
    }
    fn result(&mut self, frame: Frame, success: bool, reason: Option<u32>) -> Terminal {
        let (ready, failed) = if frame.ext { (3, 4) } else { (2, 3) };
        let e = self.wait(0, |e| {
            e.object == frame.id && [ready, failed].contains(&e.opcode)
        });
        assert_eq!(
            e.opcode,
            if success { ready } else { failed },
            "capture result: {e:?}"
        );
        if let Some(reason) = reason.filter(|_| frame.ext) {
            assert_eq!(e.word(0), reason);
        }
        if let Some((file, size)) = frame.buffer {
            let mut bytes = vec![0u8; (size.0 * size.1 * 4 + 64) as usize];
            file.read_exact_at(&mut bytes, 0).unwrap();
            assert!(
                bytes[bytes.len() - 64..].iter().all(|b| *b == 0xa5),
                "capture overwrote guard"
            );
            if success {
                for (x, y) in [(size.0 / 2, size.1 / 2), (size.0 - 2, size.1 - 2)] {
                    let offset = ((y * size.0 + x) * 4) as usize;
                    assert_ne!(
                        &bytes[offset..offset + 3],
                        &[0xa5; 3],
                        "uninitialized captured pixel"
                    );
                }
                if frame.ext {
                    assert_eq!(
                        self.event(frame.id, 0).word(0),
                        0,
                        "capture must be upright"
                    );
                    for e in self.events.iter().filter(|e| e.is(frame.id, 1)) {
                        let (x, y, w, h) = (
                            e.word(0) as i32,
                            e.word(1) as i32,
                            e.word(2) as i32,
                            e.word(3) as i32,
                        );
                        assert!(
                            x >= 0
                                && y >= 0
                                && w > 0
                                && h > 0
                                && x + w <= size.0 as i32
                                && y + h <= size.1 as i32
                        );
                    }
                }
            } else {
                assert!(
                    bytes.iter().all(|b| *b == 0xa5),
                    "failed capture wrote pixels"
                );
            }
        }
        self.send(frame.id, if frame.ext { 0 } else { 1 }, &[]);
        (frame.id, ready, failed)
    }
    fn capture(&mut self, ext: bool, success: bool, size: Option<(u32, u32)>) -> Terminal {
        let frame = self.prepare(ext, size);
        if let Some(request) = &frame.request {
            self.send_batch(std::slice::from_ref(request));
        }
        self.result(frame, success, None)
    }
    fn configuration(&mut self, enabled: bool, transform: Option<u32>, scale: Option<f64>) -> u32 {
        self.roundtrip();
        let serial = self
            .events
            .iter()
            .rfind(|e| e.is(self.outputs, 1))
            .unwrap()
            .word(0);
        let config = self.alloc();
        self.send(self.outputs, 0, &words(&[config, serial]));
        if enabled {
            let head = self.alloc();
            self.send(config, 0, &words(&[head, self.head]));
            if let Some(transform) = transform {
                self.send(head, 3, &words(&[transform]));
            }
            if let Some(scale) = scale {
                self.send(head, 4, &words(&[(scale * 256.0).round() as u32]));
            }
        } else {
            self.send(config, 1, &words(&[self.head]));
        }
        config
    }
    fn applied(&mut self, config: u32) {
        let e = self.wait(0, |e| e.object == config && e.opcode <= 2);
        assert_eq!(e.opcode, 0);
        self.send(config, 4, &[]);
        self.roundtrip();
    }
    fn enable(&mut self) {
        let config = self.configuration(true, None, None);
        self.send(config, 2, &[]);
        self.applied(config);
    }
    fn assert_terminal_once(&mut self, frames: &[Terminal]) {
        self.roundtrip();
        for (id, ready, failed) in frames {
            assert_eq!(
                self.events
                    .iter()
                    .filter(|e| e.object == *id && [*ready, *failed].contains(&e.opcode))
                    .count(),
                1
            );
        }
    }
    fn batch(&mut self, transition: Request) -> [Frame; 2] {
        let captures = [self.prepare(true, None), self.prepare(false, None)];
        let mut requests: Vec<_> = captures
            .iter()
            .map(|f| f.request.clone().unwrap())
            .collect();
        requests.push(transition);
        self.send_batch(&requests);
        captures
    }
}
pub(super) struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}
impl Image {
    pub fn pixel(&self, x: u32, y: u32) -> u32 {
        self.pixels[(y * self.width + x) as usize]
    }
    pub fn crop(&self, [x, y, width, height]: [u32; 4]) -> Vec<u32> {
        (y..y + height)
            .flat_map(|row| {
                let start = (row * self.width + x) as usize;
                self.pixels[start..start + width as usize].iter().copied()
            })
            .collect()
    }
}
fn file_capture(name: &str, then: &str) -> String {
    let directory = env::var("MINDE_SCREENSHOT_DIR")
        .unwrap_or_else(|_| format!("{}/minde/screenshots", env::var("XDG_RUNTIME_DIR").unwrap()));
    let path = format!("{directory}/{name}");
    let id = token(&ipc(&format!(
        "(let ((token (wm-screenshot {path:?}))) {then} token)"
    )));
    assert_eq!(
        ipc(&format!("(wm-automation-status {id})")),
        "(ok (screenshot failed))"
    );
    assert!(!Path::new(&path).exists());
    path
}
fn lock(c: &mut Capture) {
    // Allocate the lock's client ID after creating both frames: new object
    // IDs must appear in allocation order on the wire.
    let captures = [c.prepare(true, None), c.prepare(false, None)];
    let lock = c.alloc();
    let mut requests: Vec<_> = captures
        .iter()
        .map(|f| f.request.clone().unwrap())
        .collect();
    requests.push((c.locks, 1, words(&[lock])));
    c.send_batch(&requests);
    let mut frames: Vec<_> = captures
        .into_iter()
        .map(|f| c.result(f, false, None))
        .collect();
    c.event(lock, 0);
    for _ in 0..8 {
        for ext in [true, false] {
            frames.push(c.capture(ext, false, None));
        }
    }
    let path = file_capture("locked-capture.png", "");
    c.send(lock, 2, &[]);
    c.roundtrip();
    for ext in [true, false] {
        c.capture(ext, true, None);
    }
    c.assert_terminal_once(&frames);
    assert!(!Path::new(&path).exists());
    println!("ok - lock fails queued and new captures; unlock starts fresh");
}
fn remove(c: &mut Capture) {
    let captures = [c.prepare(true, None), c.prepare(false, None)];
    let config = c.configuration(false, None, None);
    let mut requests: Vec<_> = captures
        .iter()
        .map(|f| f.request.clone().unwrap())
        .collect();
    requests.push((config, 2, vec![]));
    c.send_batch(&requests);
    c.applied(config);
    let frames: Vec<_> = captures
        .into_iter()
        .map(|f| c.result(f, false, None))
        .collect();
    let session = c.session;
    c.event(session, 5);
    c.enable();
    c.session = c.new_session();
    for ext in [true, false] {
        c.capture(ext, true, None);
    }
    c.assert_terminal_once(&frames);
    let path = file_capture(
        "removed-capture.png",
        "(configure-output! \"winit\" #:enabled #f)",
    );
    c.enable();
    assert!(!Path::new(&path).exists());
    c.session = c.new_session();
    println!("ok - output removal fails pending captures and stops sessions");
}
fn geometry(c: &mut Capture) {
    let original = c.size();
    assert_ne!(original.0, original.1);
    for (transform, scale) in [(1, 1.0), (3, 1.25), (0, 1.5), (0, 1.0)] {
        let pending = c.prepare(true, None);
        let late = c.prepare(false, None);
        let config = c.configuration(true, Some(transform), Some(scale));
        let start = c.events.len();
        c.send_batch(&[pending.request.clone().unwrap(), (config, 2, vec![])]);
        c.applied(config);
        c.result(pending, false, Some(1));
        c.send_batch(std::slice::from_ref(late.request.as_ref().unwrap()));
        c.result(late, false, None);
        let session = c.session;
        c.wait(start, |e| e.is(session, 4));
        let physical = if [1, 3].contains(&transform) {
            (original.1, original.0)
        } else {
            original
        };
        let rounded = |n| ((f64::from(n) / scale).ceil() * scale).round() as u32;
        let expected = (rounded(physical.0), rounded(physical.1));
        assert_eq!(c.size(), expected);
        c.session = c.new_session();
        assert_eq!(c.size(), expected);
        c.send(c.session, 1, &[]);
        c.session = session;
        c.capture(true, true, None);
        c.capture(false, true, None);
        if transform == 1 {
            c.capture(true, false, Some(original));
        }
    }
    println!("ok - rotation and scale refresh constraints and retire queued frames");
}
fn power(c: &mut Capture) {
    let manager = c.bind("zwlr_output_power_manager_v1", 1);
    let power = c.alloc();
    c.send(manager, 0, &words(&[power, c.output]));
    c.event(power, 0);
    let captures = c.batch((power, 0, words(&[0])));
    let mut frames: Vec<_> = captures
        .into_iter()
        .map(|f| c.result(f, false, None))
        .collect();
    for ext in [true, false] {
        frames.push(c.capture(ext, false, None));
    }
    file_capture("powered-off-capture.png", "");
    c.send(power, 0, &words(&[1]));
    c.roundtrip();
    for ext in [true, false] {
        c.capture(ext, true, None);
    }
    c.assert_terminal_once(&frames);
    c.send(power, 1, &[]);
    println!("ok - DPMS off fails captures without stopping a reusable session");
}
fn resize(c: &mut Capture) {
    let display = env::var("MINDE_CAPTURE_HOST_DISPLAY").unwrap();
    let host = |args: &[&str]| {
        let out = Command::new("timeout")
            .args(["5", "xdotool"])
            .args(args)
            .env("DISPLAY", &display)
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    };
    let window = host(&["search", "--name", "Smithay"])
        .lines()
        .next()
        .unwrap()
        .to_owned();
    let original = c.size();
    for size in [(960, 720), original] {
        let start = c.events.len();
        host(&[
            "windowsize",
            &window,
            &size.0.to_string(),
            &size.1.to_string(),
        ]);
        let session = c.session;
        c.wait(start, |e| {
            e.is(session, 0) && (e.word(0), e.word(1)) == size
        });
        c.wait(start, |e| e.is(session, 4));
        c.capture(true, true, None);
    }
    println!("ok - host mode changes refresh an existing capture session");
}
fn monotonic_ns() -> u64 {
    let mut time: libc::timespec = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) },
        0
    );
    time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64
}
pub fn clock() {
    let mut c = Capture::new();
    let session = c.session;
    c.wait(0, |e| e.is(session, 1) && e.word(0) == 1);
    let mut previous = 0;
    for _ in 0..2 {
        let frame = c.prepare(true, None);
        let before = monotonic_ns();
        c.send_batch(std::slice::from_ref(frame.request.as_ref().unwrap()));
        let id = frame.id;
        c.wait(0, |e| e.object == id && [3, 4].contains(&e.opcode));
        let after = monotonic_ns();
        let time = c.event(id, 2);
        let ns = time.word(2);
        let timestamp = ((u64::from(time.word(0)) << 32) | u64::from(time.word(1))) * 1_000_000_000
            + u64::from(ns);
        assert!(ns < 1_000_000_000 && before <= timestamp && timestamp <= after);
        assert!(timestamp > previous);
        previous = timestamp;
        c.result(frame, true, None);
    }
    println!("ok - capture timestamps match system monotonic time and advance");
}
pub fn run(selected: &[String]) {
    let mut c = Capture::new();
    let defaults: Vec<_> = ["lock", "remove", "geometry", "power", "resize"]
        .map(String::from)
        .into();
    for name in if selected.is_empty() {
        &defaults
    } else {
        selected
    } {
        match name.as_str() {
            "lock" => lock(&mut c),
            "remove" => remove(&mut c),
            "geometry" => geometry(&mut c),
            "power" => power(&mut c),
            "resize" => resize(&mut c),
            _ => panic!("unknown capture case {name}"),
        }
    }
}
