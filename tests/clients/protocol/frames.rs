// SPDX-License-Identifier: GPL-3.0-or-later
use super::{input::evaluate, window::window_id, wire::*};
use std::{
    env, fs,
    io::Write,
    time::{Duration, Instant},
};

fn request(c: &mut Client, surface: u32) -> u32 {
    let callback = c.alloc();
    c.send(surface, 3, &words(&[callback]));
    c.send(surface, 6, &[]);
    callback
}

fn paint(c: &mut Client, frame: u32) -> u32 {
    // An actual frame-paced raster workload, including buffer allocation and
    // upload. Release each buffer so the measurement does not accumulate shm.
    let (w, h) = (800, 600);
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let color =
                ((x + frame) % 256) << 16 | ((y + frame) % 256) << 8 | ((x ^ y ^ frame) % 256);
            pixels.extend_from_slice(&color.to_ne_bytes());
        }
    }
    let mut file = memfile(pixels.len(), 0);
    file.write_all(&pixels).unwrap();
    let buffer = c.buffer_from(w, h, &file);
    c.handlers.insert(
        buffer,
        Box::new(move |c, e| {
            if e.opcode == 0 {
                c.send(buffer, 0, &[]);
            }
        }),
    );
    c.send(c.surface, 1, &words(&[buffer, 0, 0]));
    c.send(c.surface, 2, &words(&[0, 0, w, h]));
    request(c, c.surface)
}

fn cpu_ticks(pid: &str) -> u64 {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let fields: Vec<_> = stat[stat.rfind(')').unwrap() + 2..]
        .split_whitespace()
        .collect();
    fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
}

fn measure(c: &mut Client, phase: &str, seconds: u64) -> (u32, u32) {
    let wm = env::var("MINDE_TEST_WM_PID").unwrap();
    let cpu_before = [cpu_ticks("self"), cpu_ticks(&wm)];
    let started = Instant::now();
    let deadline = started + Duration::from_secs(seconds);
    let mut count = 0;
    let mut callback = paint(c, count);
    while Instant::now() < deadline {
        match c.read(deadline) {
            Ok(e) if e.is(callback, 0) => {
                count += 1;
                callback = paint(c, count);
            }
            Ok(_) => {}
            Err(Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                break;
            }
            Err(e) => panic!("animation: {e}"),
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
    let cpu = [cpu_ticks("self"), cpu_ticks(&wm)];
    println!(
        "frames {phase}: {count} callbacks in {elapsed:.2}s; client {:.2}% CPU; compositor {:.2}% CPU",
        100.0 * (cpu[0] - cpu_before[0]) as f64 / ticks / elapsed,
        100.0 * (cpu[1] - cpu_before[1]) as f64 / ticks / elapsed
    );
    (count, callback)
}

fn hidden(on: bool) {
    let group = if on { " I " } else { "frame pacing" };
    evaluate(
        &format!("(begin (switch-to-group! {group:?}) (wm-clear-message) #t)"),
        "#t",
    );
}

fn no_callback(c: &mut Client, callback: u32) {
    c.roundtrip();
    assert!(
        !c.events.iter().any(|e| e.is(callback, 0)),
        "unexpected hidden frame callback"
    );
}

pub fn run() {
    let mut anchor = Client::new("visible frame anchor");
    anchor.toplevel(true);
    evaluate(
        "(begin (create-group! \"frame pacing\") (switch-to-group! \"frame pacing\") #t)",
        "#t",
    );
    let mut c = Client::new("frame-paced raster client");
    c.toplevel(true);
    c.map_on_configure = false;
    let id = window_id(&c.name);
    let (visible, callback) = measure(&mut c, "visible", 1);
    assert!(visible >= 10, "visible animation stalled");
    c.event(callback, 0);
    hidden(true);
    evaluate(&format!("(wm-window-geometry {id})"), "#f");
    let (parked, callback) = measure(&mut c, "hidden-idle", 4);
    assert!(
        (2..=5).contains(&parked),
        "hidden animation ran at {parked} callbacks in four seconds"
    );
    c.event(callback, 0);

    // Other visible redraws must not bypass the hidden-window deadline.
    let started = Instant::now();
    let surface = c.surface;
    let callback = request(&mut c, surface);
    for color in 1..=12 {
        anchor.paint(anchor.surface, 64, 64, color * 0x010101);
        std::thread::sleep(Duration::from_millis(20));
    }
    if started.elapsed() < Duration::from_millis(800) {
        no_callback(&mut c, callback);
    }
    c.event(callback, 0);
    assert!(started.elapsed() >= Duration::from_millis(900));

    // Reveal an already waiting window without waiting for the hidden timer.
    let callback = request(&mut c, surface);
    let started = Instant::now();
    hidden(false);
    c.event(callback, 0);
    assert!(
        started.elapsed() < Duration::from_millis(800),
        "revealed window kept its hidden delay"
    );
    let (visible, callback) = measure(&mut c, "visible-again", 1);
    assert!(visible >= 10);
    c.event(callback, 0);
    hidden(true);

    // A pending timer must respect lock and DPMS, and recover without another
    // client commit when those conditions end.
    let warmup = request(&mut c, surface);
    c.event(warmup, 0);
    let callback = request(&mut c, surface);
    let mut owner = Client::new("frame pacing lock owner");
    let lock = owner.lock(true);
    std::thread::sleep(Duration::from_millis(1150));
    no_callback(&mut c, callback);
    owner.send(lock, 2, &[]);
    owner.roundtrip();
    c.event(callback, 0);
    let callback = request(&mut c, surface);
    command("wlopm", &["--off", "winit"]);
    std::thread::sleep(Duration::from_millis(1150));
    no_callback(&mut c, callback);
    command("wlopm", &["--on", "winit"]);
    c.event(callback, 0);

    // A desynchronized child alone can request a hidden frame.
    let child = c.new_surface();
    let subcompositor = c.bind("wl_subcompositor", 1);
    let role = c.alloc();
    c.send(subcompositor, 1, &words(&[role, child, surface]));
    c.send(role, 5, &[]);
    c.paint(child, 32, 32, 0xff00ff);
    c.send(surface, 6, &[]);
    let callback = request(&mut c, child);
    c.event(callback, 0);

    let callback = request(&mut c, surface);
    c.send(surface, 1, &words(&[0, 0, 0]));
    c.send(surface, 6, &[]);
    c.roundtrip();
    std::thread::sleep(Duration::from_millis(1150));
    no_callback(&mut c, callback);
    hidden(true);
    println!("ok - hidden callbacks are paced, survive idle and resume immediately when revealed");
    println!("ok - pending hidden callbacks respect lock, power, subsurfaces and unmap");
}
