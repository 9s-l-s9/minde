// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::{focus::Panel, input::evaluate};
use std::time::{Duration, Instant};

struct Buffer {
    file: File,
    id: u32,
    size: (u32, u32),
}
impl Buffer {
    fn new(c: &mut Capture) -> Self {
        let size = c.size();
        let file = memfile((size.0 * size.1 * 4) as usize, 0xa5);
        let id = c.buffer_from(size.0, size.1, &file);
        Self { file, id, size }
    }
    fn bytes(&self) -> Vec<u8> {
        let mut bytes = vec![0; (self.size.0 * self.size.1 * 4) as usize];
        self.file.read_exact_at(&mut bytes, 0).unwrap();
        bytes
    }
    fn pixel(&self, x: u32, y: u32) -> u32 {
        let mut bytes = [0; 4];
        self.file
            .read_exact_at(&mut bytes, u64::from((y * self.size.0 + x) * 4))
            .unwrap();
        u32::from_ne_bytes(bytes) & 0xffffff
    }
}

fn ext(c: &mut Capture, session: u32, buffer: &Buffer, repair: &[[u32; 4]]) -> u32 {
    let frame = c.alloc();
    c.send(session, 0, &words(&[frame]));
    c.send(frame, 1, &words(&[buffer.id]));
    for rect in repair {
        c.send(frame, 2, &words(rect));
    }
    c.send(frame, 3, &[]);
    frame
}

fn wlr(c: &mut Capture, buffer: &Buffer, with_damage: bool) -> u32 {
    let frame = c.alloc();
    c.send(c.wlr, 0, &words(&[frame, 0, c.output]));
    c.event(frame, 6);
    c.send(frame, if with_damage { 2 } else { 0 }, &words(&[buffer.id]));
    frame
}

fn ready(c: &mut Capture, frame: u32, ext: bool) {
    let ready = if ext { 3 } else { 2 };
    let failed = if ext { 4 } else { 3 };
    assert_eq!(
        c.wait(0, |e| e.object == frame
            && [ready, failed].contains(&e.opcode))
            .opcode,
        ready
    );
    c.send(frame, if ext { 0 } else { 1 }, &[]);
}

fn quiet(c: &mut Capture, frame: u32) {
    let end = Instant::now() + Duration::from_millis(150);
    while Instant::now() < end {
        match c.read(end) {
            Ok(_) => {}
            Err(Error::Io(err))
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                break;
            }
            Err(err) => panic!("capture connection failed: {err}"),
        }
    }
    assert!(
        !c.events
            .iter()
            .any(|e| e.object == frame && [2, 3].contains(&e.opcode)),
        "copy-with-damage completed without a scene change"
    );
}

fn counts() -> Vec<i32> {
    crate::input::numbers(&ipc(
        "(map (lambda (name) (cadr (assq name (wm-timing-stats)))) '(capture-render capture-readback))",
    ))
}

fn damage(c: &Capture, frame: u32, ext: bool) -> Vec<[u32; 4]> {
    c.events
        .iter()
        .filter(|e| e.is(frame, if ext { 1 } else { 4 }))
        .map(|e| [e.word(0), e.word(1), e.word(2), e.word(3)])
        .collect()
}

pub fn run() {
    evaluate("(wm-clear-message)", "#t");
    let mut panel = Panel::new("capture damage", 3, 0);
    let surface = panel.input.surface;
    panel.input.paint(surface, 280, 100, 0x224466);
    panel.input.roundtrip();
    let mut c = Capture::new();
    let first = c.capture(true, true, None).0;
    let size = c.size();
    assert_eq!(damage(&c, first, true), vec![[0, 0, size.0, size.1]]);
    let unchanged = c.capture(true, true, None).0;
    assert!(
        damage(&c, unchanged, true).is_empty(),
        "unchanged ext frame reports damage: {:?}",
        damage(&c, unchanged, true)
    );
    println!("ok - unchanged ext capture has no source damage and initializes a fresh buffer");

    // Reuse one source buffer so a small wl_surface.damage remains small.
    let source = memfile(280 * 100 * 4, 0);
    source
        .write_all_at(&0x224466u32.to_ne_bytes().repeat(280 * 100), 0)
        .unwrap();
    let source_buffer = panel.input.buffer_from(280, 100, &source);
    panel.input.send(surface, 1, &words(&[source_buffer, 0, 0]));
    panel.input.send(surface, 2, &words(&[0, 0, 280, 100]));
    panel.input.send(surface, 6, &[]);
    panel.input.roundtrip();
    let session = c.session;
    let second_session = c.new_session();
    let a = Buffer::new(&mut c);
    let b = Buffer::new(&mut c);
    let full = [0, 0, size.0, size.1];
    let first = ext(&mut c, session, &a, &[full]);
    let second = ext(&mut c, second_session, &b, &[full]);
    ready(&mut c, first, true);
    ready(&mut c, second, true);
    assert_eq!(a.bytes(), b.bytes());
    assert_eq!(
        damage(&c, second, true),
        vec![full],
        "new session must report full damage"
    );

    let before = counts();
    // Client-side repair is outside the source damage and must not be reported
    // as a change in the scene. Other buffers need only the source damage.
    let repair = [400, 300, 7, 5];
    for row in repair[1]..repair[1] + repair[3] {
        a.file
            .write_all_at(&[0xa5; 7 * 4], u64::from((row * size.0 + repair[0]) * 4))
            .unwrap();
    }
    for row in 20..32 {
        source
            .write_all_at(&0xee3355u32.to_ne_bytes().repeat(16), (row * 280 + 30) * 4)
            .unwrap();
    }
    panel.input.send(surface, 1, &words(&[source_buffer, 0, 0]));
    panel.input.send(surface, 2, &words(&[30, 20, 16, 12]));
    panel.input.send(surface, 6, &[]);
    panel.input.roundtrip();
    let first = ext(&mut c, session, &a, &[repair]);
    let second = ext(&mut c, second_session, &b, &[]);
    ready(&mut c, first, true);
    ready(&mut c, second, true);
    let after = counts();
    assert_eq!(damage(&c, first, true), vec![[50, 40, 16, 12]]);
    assert_eq!(damage(&c, second, true), damage(&c, first, true));
    assert_eq!(
        a.pixel(55, 45),
        0xee3355,
        "partial GPU readback has wrong coordinates"
    );
    assert_eq!(
        a.pixel(25, 25),
        0x224466,
        "unchanged source pixels were lost"
    );
    assert_eq!(
        a.bytes(),
        b.bytes(),
        "buffer repair did not restore stale pixels"
    );
    assert_eq!(
        after
            .iter()
            .zip(&before)
            .map(|(a, b)| a - b)
            .collect::<Vec<_>>(),
        vec![1, 1],
        "matching clients repeated the capture render/readback"
    );
    let before = counts();
    let unchanged = ext(&mut c, session, &a, &[]);
    ready(&mut c, unchanged, true);
    assert!(damage(&c, unchanged, true).is_empty());
    assert_eq!(
        counts(),
        before,
        "unchanged capture rendered or read back pixels"
    );
    println!(
        "ok - two sessions share one 16x12 update/readback, repair reused buffers and skip unchanged work"
    );

    let wlr_buffer = Buffer::new(&mut c);
    let initial = wlr(&mut c, &wlr_buffer, true);
    ready(&mut c, initial, false);
    assert_eq!(damage(&c, initial, false), vec![full]);
    // A new buffer is fully populated even though only the small changed
    // portion is read back from the GPU and reported to the encoder.
    wlr_buffer
        .file
        .write_all_at(&vec![0xa5; (size.0 * size.1 * 4) as usize], 0)
        .unwrap();
    let waiting = wlr(&mut c, &wlr_buffer, true);
    quiet(&mut c, waiting);
    for row in 20..32 {
        source
            .write_all_at(&0x33ee55u32.to_ne_bytes().repeat(16), (row * 280 + 30) * 4)
            .unwrap();
    }
    panel.input.send(surface, 1, &words(&[source_buffer, 0, 0]));
    panel.input.send(surface, 2, &words(&[30, 20, 16, 12]));
    panel.input.send(surface, 6, &[]);
    panel.input.roundtrip();
    ready(&mut c, waiting, false);
    assert_eq!(damage(&c, waiting, false), vec![[50, 40, 16, 12]]);
    assert_eq!(wlr_buffer.pixel(55, 45), 0x33ee55);
    assert_eq!(wlr_buffer.pixel(25, 25), 0x224466);
    // Immediate copy is allowed on a static scene; damage-aware copy waits.
    let immediate = wlr(&mut c, &wlr_buffer, false);
    ready(&mut c, immediate, false);
    let abandoned = wlr(&mut c, &wlr_buffer, true);
    quiet(&mut c, abandoned);
    c.send(abandoned, 1, &[]);
    c.roundtrip();
    println!("ok - wlr damage waits for changes, reports their extent and accepts cancellation");

    for color in 1u32..=18 {
        for row in 20..32 {
            source
                .write_all_at(&color.to_ne_bytes().repeat(16), (row * 280 + 30) * 4)
                .unwrap();
        }
        panel.input.send(surface, 1, &words(&[source_buffer, 0, 0]));
        panel.input.send(surface, 2, &words(&[30, 20, 16, 12]));
        panel.input.send(surface, 6, &[]);
        panel.input.roundtrip();
        let frame = ext(&mut c, session, &a, &[]);
        ready(&mut c, frame, true);
    }
    let slow = ext(&mut c, second_session, &b, &[]);
    ready(&mut c, slow, true);
    assert_eq!(
        damage(&c, slow, true),
        vec![full],
        "expired damage history must require a full repair"
    );
    assert_eq!(a.bytes(), b.bytes());
    // Different regions are different cache images. Eviction must invalidate
    // the session's old image stamp even when the client reuses its buffer.
    for x in 0..5 {
        c.image(false, Some([x * 20, 0, 40, 40]));
    }
    let evicted = ext(&mut c, session, &a, &[]);
    ready(&mut c, evicted, true);
    assert_eq!(damage(&c, evicted, true), vec![full]);
    assert_eq!(a.bytes(), b.bytes());
    println!("ok - slow sessions and evicted capture images recover with full damage");
    panel.destroy();
}
