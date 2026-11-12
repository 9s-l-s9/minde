// SPDX-License-Identifier: GPL-3.0-or-later
use super::{
    input::{Input, evaluate},
    wire::*,
};
use std::{
    env,
    fs::{self, File},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::FileExt,
    },
    time::{Duration, Instant},
};
const MIME: &str = "text/plain;charset=utf-8";
const LIMIT: usize = 32;
fn resources() -> (usize, usize) {
    let pid: u32 = env::var("MINDE_TRANSFER_TEST_PID")
        .unwrap()
        .parse()
        .unwrap();
    let status = fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    let threads = status
        .lines()
        .find(|s| s.starts_with("Threads:"))
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    (
        fs::read_dir(format!("/proc/{pid}/fd")).unwrap().count(),
        threads,
    )
}
fn wait_for(mut predicate: impl FnMut() -> bool, seconds: u64, message: &str) {
    let end = Instant::now() + Duration::from_secs(seconds);
    while !predicate() {
        assert!(Instant::now() < end, "{message}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
enum Finish {
    None,
    Eval(String),
    Send(u32, u16, Vec<u8>),
}
struct Selection {
    c: Input,
    control: u32,
    device: u32,
    drag_device: u32,
}
impl Selection {
    fn new() -> Self {
        let mut c = Input::new("selection transfer target");
        c.mapped(100);
        let control = c.bind("zwlr_data_control_manager_v1", 2);
        let seat = c.bind("wl_seat", 1);
        let device = c.alloc();
        c.send(control, 1, &words(&[device, seat]));
        let manager = c.bind("wl_data_device_manager", 3);
        let drag_device = c.alloc();
        c.send(manager, 1, &words(&[drag_device, seat]));
        c.roundtrip();
        Self {
            c,
            control,
            device,
            drag_device,
        }
    }
    fn receive(&self, offer: u32, opcode: u16) -> File {
        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
        let reader = unsafe { File::from_raw_fd(fds[0]) };
        let writer = unsafe { File::from_raw_fd(fds[1]) };
        assert_eq!(
            unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) },
            0
        );
        assert_eq!(
            unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_SETPIPE_SZ, 4096) },
            4096
        );
        self.c
            .send_fd(offer, opcode, &string(MIME), writer.as_raw_fd());
        reader
    }
    fn contents(&mut self, reader: &mut File) -> Vec<u8> {
        let mut data = Vec::new();
        let mut buffer = [0u8; 16384];
        let end = Instant::now() + Duration::from_secs(7);
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => return data,
                Ok(n) => data.extend_from_slice(&buffer[..n]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < end, "selection reader never reached EOF");
                    self.c.roundtrip();
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(e) => panic!("selection read: {e}"),
            }
        }
    }
    fn flood(&mut self, offer: u32, opcode: u16, finish: Finish, label: &str) {
        let (baseline, threads) = resources();
        let mut readers: Vec<_> = (0..96).map(|_| self.receive(offer, opcode)).collect();
        self.c.roundtrip();
        evaluate("#t", "#t");
        let (retained, current_threads) = resources();
        assert!(current_threads <= threads + 2, "{label}: thread growth");
        assert!(
            retained <= baseline + LIMIT + 2,
            "{label}: {} extra fds",
            retained - baseline
        );
        let seconds = if matches!(finish, Finish::None) { 7 } else { 1 };
        match finish {
            Finish::None => {}
            Finish::Eval(expression) => evaluate(&expression, "#t"),
            Finish::Send(object, opcode, data) => self.c.send(object, opcode, &data),
        }
        self.c.roundtrip();
        wait_for(
            || resources().0 <= baseline,
            seconds,
            "stalled writes retained fds",
        );
        for reader in &mut readers {
            self.contents(reader);
        }
        println!("ok - {label} receive flood is bounded and stalled writes close");
    }
    fn clipboard(&mut self) {
        for (setter, opcode) in [("wm-set-clipboard", 1), ("wm-set-primary", 3)] {
            let start = self.c.events.len();
            evaluate(&format!("({setter} (make-string 262144 #\\A))"), "#t");
            let offer = self.c.wait(start, |e| e.is(self.device, opcode)).word(0);
            assert_ne!(offer, 0);
            let mut reader = self.receive(offer, 0);
            assert_eq!(self.contents(&mut reader), vec![b'A'; 262144]);
            drop(reader);
            let file = memfile(0, 0);
            self.c.send_fd(offer, 0, &string(MIME), file.as_raw_fd());
            self.c.roundtrip();
            wait_for(
                || file.metadata().unwrap().len() == 262144,
                7,
                "regular-file selection failed",
            );
            let mut bytes = vec![0u8; 262144];
            file.read_exact_at(&mut bytes, 0).unwrap();
            assert_eq!(bytes, vec![b'A'; 262144]);
            drop(file);
            let finish = if opcode == 1 {
                Finish::Eval(format!("({setter} \"replacement\")"))
            } else {
                Finish::Send(self.device, 2, words(&[0]))
            };
            self.flood(offer, 0, finish, setter);
        }
        let start = self.c.events.len();
        evaluate("(wm-set-clipboard (make-string 262144 #\\T))", "#t");
        let offer = self.c.wait(start, |e| e.is(self.device, 1)).word(0);
        self.flood(offer, 0, Finish::None, "clipboard deadline");
        println!("ok - large clipboard and primary text survive partial writes");
    }
    fn dnd(&mut self) {
        self.c.point();
        let start = self.c.events.len();
        let result = ipc(&format!(
            "(wm-drop-text {} 180 (make-string 262144 #\\D))",
            self.c.x
        ));
        assert!(result.starts_with("(ok "));
        let enter = self.c.wait(start, |e| e.is(self.drag_device, 1));
        let offer = enter.word(4);
        assert_eq!(enter.word(1), self.c.surface);
        assert_ne!(offer, 0);
        self.c
            .send(offer, 0, &[words(&[enter.word(0)]), string(MIME)].concat());
        self.c.send(offer, 4, &words(&[1, 1]));
        self.c.wait(start, |e| e.is(self.drag_device, 4));
        let mut reader = self.receive(offer, 1);
        assert_eq!(self.contents(&mut reader), vec![b'D'; 262144]);
        drop(reader);
        self.flood(offer, 1, Finish::Send(offer, 3, vec![]), "drag payload");
        println!("ok - large drag payload survives partial writes");
    }
    fn paste(&mut self) {
        // Drop prior keymap descriptors before collecting the source's pipes.
        self.c.received_fds.clear();
        let source = self.c.alloc();
        self.c.send(self.control, 0, &words(&[source]));
        self.c.send(source, 0, &string(MIME));
        self.c.send(self.device, 0, &words(&[source]));
        self.c.roundtrip();
        evaluate(
            "(begin (define transfer-pastes '()) \
            (define (handle-paste! text) (set! transfer-pastes (cons text transfer-pastes))) #t)",
            "#t",
        );
        let (baseline, threads) = resources();
        let start = self.c.events.len();
        evaluate(
            "(begin (do ((i 0 (+ i 1))) ((= i 40)) (wm-request-paste)) #t)",
            "#t",
        );
        self.c.roundtrip();
        assert_eq!(
            self.c.events[start..]
                .iter()
                .filter(|e| e.is(source, 0))
                .count(),
            40
        );
        let mut writers: Vec<File> = std::mem::take(&mut self.c.received_fds)
            .into_iter()
            .map(File::from)
            .collect();
        assert_eq!(writers.len(), 40);
        writers[0].write_all(b"partial").unwrap();
        assert!(resources().0 <= baseline + LIMIT);
        assert!(resources().1 <= threads + 2);
        wait_for(
            || ipc("(length transfer-pastes)") == "(ok 40)",
            7,
            "stalled pastes never completed",
        );
        evaluate(
            "(and (= 1 (count (lambda (s) (equal? s \"partial\")) transfer-pastes)) \
                        (= 39 (count string-null? transfer-pastes)))",
            "#t",
        );
        wait_for(
            || resources().0 <= baseline,
            1,
            "paste deadlines retained fds",
        );
        for mut writer in writers {
            assert_eq!(
                writer.write_all(b"late").unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
            );
        }
        self.c.roundtrip();
        evaluate("(= 40 (length transfer-pastes))", "#t");
        evaluate("(begin (wm-request-paste) #t)", "#t");
        self.c.roundtrip();
        assert_eq!(self.c.received_fds.len(), 1);
        let mut writer = File::from(self.c.received_fds.pop().unwrap());
        writer.write_all("normal ✓".as_bytes()).unwrap();
        drop(writer);
        wait_for(
            || ipc("(length transfer-pastes)") == "(ok 41)",
            1,
            "normal paste did not finish",
        );
        evaluate("(equal? (car transfer-pastes) \"normal ✓\")", "#t");
        println!(
            "ok - stalled paste owners release capped fds and deliver partial text exactly once"
        );
    }
}
pub fn run() {
    let mut c = Selection::new();
    for name in env::var("MINDE_TRANSFER_CASES")
        .unwrap_or_else(|_| "clipboard dnd paste".into())
        .split_whitespace()
    {
        match name {
            "clipboard" => c.clipboard(),
            "dnd" => c.dnd(),
            "paste" => c.paste(),
            _ => panic!("unknown transfer case {name}"),
        }
    }
    println!("selection-transfer: all checks passed");
}
