// SPDX-License-Identifier: GPL-3.0-or-later
use std::{
    collections::HashMap,
    env,
    fs::File,
    io::{self, Read, Write},
    net::Shutdown,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::net::UnixStream,
    },
    process::Command,
    time::{Duration, Instant},
};

pub const TIMEOUT: Duration = Duration::from_secs(5);
pub fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|n| n.to_ne_bytes()).collect()
}
pub fn word(data: &[u8], index: usize) -> u32 {
    u32::from_ne_bytes(data[index * 4..index * 4 + 4].try_into().unwrap())
}
pub fn string(value: &str) -> Vec<u8> {
    let mut out = words(&[(value.len() + 1) as u32]);
    out.extend_from_slice(value.as_bytes());
    out.push(0);
    out.resize(out.len().next_multiple_of(4), 0);
    out
}
pub fn text(data: &[u8], offset: usize) -> String {
    let len = word(&data[offset..], 0) as usize;
    String::from_utf8_lossy(&data[offset + 4..offset + 4 + len.saturating_sub(1)]).into_owned()
}
pub fn ipc(expression: &str) -> String {
    let mut socket = UnixStream::connect(format!(
        "{}/minde-ipc.sock",
        env::var("XDG_RUNTIME_DIR").unwrap()
    ))
    .unwrap();
    socket.set_read_timeout(Some(TIMEOUT)).unwrap();
    socket.set_write_timeout(Some(TIMEOUT)).unwrap();
    socket.write_all(expression.as_bytes()).unwrap();
    socket.shutdown(Shutdown::Write).unwrap();
    let mut reply = String::new();
    socket.read_to_string(&mut reply).unwrap();
    reply.trim().into()
}
pub fn locked(expected: bool) {
    assert_eq!(
        ipc("(wm-session-locked?)"),
        if expected { "(ok #t)" } else { "(ok #f)" }
    );
}
pub fn command(program: &str, args: &[&str]) -> String {
    let output = Command::new("timeout")
        .args(["5", program])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
pub fn until(mut condition: impl FnMut() -> bool, message: &str) {
    let end = Instant::now() + TIMEOUT;
    while !condition() {
        assert!(Instant::now() < end, "{message}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
pub fn memfile(size: usize, fill: u8) -> File {
    // A fresh anonymous file, owned by File and closed on every exit path.
    let fd = unsafe { libc::memfd_create(c"minde-protocol-test".as_ptr(), libc::MFD_CLOEXEC) };
    assert!(fd >= 0, "memfd_create: {}", io::Error::last_os_error());
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.set_len(size as u64).unwrap();
    if fill != 0 {
        file.write_all(&vec![fill; size]).unwrap();
    }
    file
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub object: u32,
    pub opcode: u16,
    pub data: Vec<u8>,
}
impl Event {
    pub fn is(&self, object: u32, opcode: u16) -> bool {
        self.object == object && self.opcode == opcode
    }
    pub fn word(&self, index: usize) -> u32 {
        word(&self.data, index)
    }
}
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Protocol {
        object: u32,
        code: u32,
        message: String,
    },
}
impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Protocol {
                object,
                code,
                message,
            } => write!(f, "object {object}, code {code}: {message}"),
        }
    }
}
pub type Handler = Box<dyn FnMut(&mut Client, &Event)>;
pub struct Client {
    pub name: String,
    socket: UnixStream,
    buffer: Vec<u8>,
    next_id: u32,
    pub registry: u32,
    pub globals: HashMap<String, (u32, u32)>,
    pub events: Vec<Event>,
    pub handlers: HashMap<u32, Handler>,
    pub received_fds: Vec<OwnedFd>,
    pub keyboard: u32,
    pub surface: u32,
    pub xdg_surface: u32,
    pub toplevel_object: u32,
    pub configured_size: (u32, u32),
    pub map_on_configure: bool,
}
impl Client {
    pub fn new(name: &str) -> Self {
        let display = env::var("NESTED_WAYLAND_DISPLAY")
            .or_else(|_| env::var("WAYLAND_DISPLAY"))
            .unwrap();
        let path = format!("{}/{}", env::var("XDG_RUNTIME_DIR").unwrap(), display);
        let socket = UnixStream::connect(path).unwrap();
        socket.set_write_timeout(Some(TIMEOUT)).unwrap();
        let mut c = Self {
            name: name.into(),
            socket,
            buffer: Vec::new(),
            next_id: 2,
            registry: 0,
            globals: HashMap::new(),
            events: Vec::new(),
            handlers: HashMap::new(),
            received_fds: Vec::new(),
            keyboard: 0,
            surface: 0,
            xdg_surface: 0,
            toplevel_object: 0,
            configured_size: (640, 480),
            map_on_configure: true,
        };
        c.registry = c.alloc();
        c.send(1, 1, &words(&[c.registry]));
        c.roundtrip();
        c
    }
    pub fn alloc(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    pub fn send(&self, object: u32, opcode: u16, data: &[u8]) {
        self.send_batch(&[(object, opcode, data.to_vec())]);
    }
    pub fn send_batch(&self, requests: &[(u32, u16, Vec<u8>)]) {
        let mut bytes = Vec::new();
        for (object, opcode, data) in requests {
            bytes.extend(words(&[
                *object,
                ((8 + data.len()) as u32) << 16 | *opcode as u32,
            ]));
            bytes.extend_from_slice(data);
        }
        (&self.socket).write_all(&bytes).unwrap();
    }
    pub fn send_fd(&self, object: u32, opcode: u16, data: &[u8], fd: RawFd) {
        let mut bytes = words(&[object, ((8 + data.len()) as u32) << 16 | opcode as u32]);
        bytes.extend_from_slice(data);
        // Word-aligned ancillary storage, sized by libc's platform ABI.
        unsafe {
            let size = libc::CMSG_SPACE(size_of::<RawFd>() as u32) as usize;
            let mut control = vec![0usize; size.div_ceil(size_of::<usize>())];
            let mut iov = libc::iovec {
                iov_base: bytes.as_mut_ptr().cast(),
                iov_len: bytes.len(),
            };
            let mut msg: libc::msghdr = std::mem::zeroed();
            msg.msg_iov = &mut iov;
            msg.msg_iovlen = 1;
            msg.msg_control = control.as_mut_ptr().cast();
            msg.msg_controllen = size;
            let header = libc::CMSG_FIRSTHDR(&msg);
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            (*header).cmsg_len = libc::CMSG_LEN(size_of::<RawFd>() as u32) as usize;
            std::ptr::write_unaligned(libc::CMSG_DATA(header).cast::<RawFd>(), fd);
            let sent = libc::sendmsg(self.socket.as_raw_fd(), &msg, libc::MSG_NOSIGNAL);
            assert!(sent >= 0, "sendmsg: {}", io::Error::last_os_error());
            (&self.socket).write_all(&bytes[sent as usize..]).unwrap();
        }
    }
    pub fn read(&mut self, deadline: Instant) -> Result<Event, Error> {
        loop {
            if self.buffer.len() >= 8 {
                let size = (word(&self.buffer, 1) >> 16) as usize;
                assert!(
                    size >= 8 && size.is_multiple_of(4),
                    "malformed Wayland event"
                );
                if self.buffer.len() >= size {
                    break;
                }
            }
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "Wayland event deadline"))?;
            self.socket.set_read_timeout(Some(remaining))?;
            let mut bytes = [0u8; 65536];
            // recvmsg owns each received descriptor immediately, including on
            // error. Extra descriptors are dropped when the client is dropped.
            unsafe {
                let size = libc::CMSG_SPACE(4096) as usize;
                let mut control = vec![0usize; size.div_ceil(size_of::<usize>())];
                let mut iov = libc::iovec {
                    iov_base: bytes.as_mut_ptr().cast(),
                    iov_len: bytes.len(),
                };
                let mut msg: libc::msghdr = std::mem::zeroed();
                msg.msg_iov = &mut iov;
                msg.msg_iovlen = 1;
                msg.msg_control = control.as_mut_ptr().cast();
                msg.msg_controllen = size;
                let count =
                    libc::recvmsg(self.socket.as_raw_fd(), &mut msg, libc::MSG_CMSG_CLOEXEC);
                if count < 0 {
                    return Err(io::Error::last_os_error().into());
                }
                let mut header = libc::CMSG_FIRSTHDR(&msg);
                while !header.is_null() {
                    if (*header).cmsg_level == libc::SOL_SOCKET
                        && (*header).cmsg_type == libc::SCM_RIGHTS
                    {
                        let length = (*header).cmsg_len - libc::CMSG_LEN(0) as usize;
                        for i in 0..length / size_of::<RawFd>() {
                            let fd = std::ptr::read_unaligned(
                                libc::CMSG_DATA(header).cast::<RawFd>().add(i),
                            );
                            self.received_fds.push(OwnedFd::from_raw_fd(fd));
                        }
                    }
                    header = libc::CMSG_NXTHDR(&msg, header);
                }
                assert_eq!(msg.msg_flags & libc::MSG_CTRUNC, 0, "truncated descriptors");
                if count == 0 {
                    return Err(
                        io::Error::new(io::ErrorKind::UnexpectedEof, "Wayland closed").into(),
                    );
                }
                self.buffer.extend_from_slice(&bytes[..count as usize]);
            }
        }
        let header = word(&self.buffer, 1);
        let event = Event {
            object: word(&self.buffer, 0),
            opcode: header as u16,
            data: self.buffer[8..(header >> 16) as usize].to_vec(),
        };
        self.buffer.drain(..(header >> 16) as usize);
        if event.is(1, 0) {
            return Err(Error::Protocol {
                object: event.word(0),
                code: event.word(1),
                message: text(&event.data, 8),
            });
        }
        if event.is(self.registry, 0) {
            let name = text(&event.data, 4);
            let version = word(&event.data, 2 + (event.word(1) as usize).div_ceil(4));
            self.globals.insert(name, (event.word(0), version));
        }
        self.events.push(event.clone());
        if let Some(mut handler) = self.handlers.remove(&event.object) {
            handler(self, &event);
            self.handlers.entry(event.object).or_insert(handler);
        }
        Ok(event)
    }
    pub fn wait_result(
        &mut self,
        start: usize,
        predicate: impl Fn(&Event) -> bool,
    ) -> Result<Event, Error> {
        let end = Instant::now() + TIMEOUT;
        let mut from = start;
        loop {
            if let Some(event) = self.events[from..].iter().find(|e| predicate(e)) {
                return Ok(event.clone());
            }
            from = self.events.len();
            self.read(end)?;
        }
    }
    pub fn wait(&mut self, start: usize, predicate: impl Fn(&Event) -> bool) -> Event {
        self.wait_result(start, predicate)
            .unwrap_or_else(|e| panic!("{}: {e}", self.name))
    }
    pub fn event(&mut self, object: u32, opcode: u16) -> Event {
        self.wait(0, |e| e.is(object, opcode))
    }
    pub fn roundtrip(&mut self) {
        let callback = self.alloc();
        self.send(1, 0, &words(&[callback]));
        self.event(callback, 0);
    }
    pub fn bind(&mut self, interface: &str, version: u32) -> u32 {
        let (name, available) = *self
            .globals
            .get(interface)
            .unwrap_or_else(|| panic!("missing {interface}"));
        let object = self.alloc();
        let data = [
            words(&[name]),
            string(interface),
            words(&[version.min(available), object]),
        ]
        .concat();
        self.send(self.registry, 0, &data);
        object
    }
    pub fn get_keyboard(&mut self) -> u32 {
        if self.keyboard == 0 {
            let seat = self.bind("wl_seat", 1);
            self.keyboard = self.alloc();
            self.send(seat, 1, &words(&[self.keyboard]));
        }
        self.keyboard
    }
    pub fn new_surface(&mut self) -> u32 {
        let compositor = self.bind("wl_compositor", 1);
        let surface = self.alloc();
        self.send(compositor, 0, &words(&[surface]));
        surface
    }
    pub fn buffer_from(&mut self, width: u32, height: u32, file: &File) -> u32 {
        let shm = self.bind("wl_shm", 1);
        let pool = self.alloc();
        let buffer = self.alloc();
        self.send_fd(
            shm,
            0,
            &words(&[pool, width * height * 4]),
            file.as_raw_fd(),
        );
        self.send(pool, 0, &words(&[buffer, 0, width, height, width * 4, 1]));
        self.send(pool, 1, &[]);
        buffer
    }
    pub fn buffer_for(&mut self, width: u32, height: u32, color: u32) -> u32 {
        let mut file = memfile((width * height * 4) as usize, 0);
        if color != 0 {
            file.write_all(&color.to_ne_bytes().repeat((width * height) as usize))
                .unwrap();
        }
        self.buffer_from(width, height, &file)
    }
    pub fn paint(&mut self, surface: u32, width: u32, height: u32, color: u32) {
        let buffer = self.buffer_for(width, height, color);
        self.send(surface, 1, &words(&[buffer, 0, 0]));
        self.send(surface, 2, &words(&[0, 0, width, height]));
        self.send(surface, 6, &[]);
    }
    pub fn toplevel(&mut self, map: bool) {
        self.get_keyboard();
        self.map_on_configure = map;
        self.surface = self.new_surface();
        let shell = self.bind("xdg_wm_base", 1);
        self.handlers.insert(
            shell,
            Box::new(move |c, e| {
                if e.opcode == 0 {
                    c.send(shell, 3, &e.data);
                }
            }),
        );
        self.xdg_surface = self.alloc();
        self.toplevel_object = self.alloc();
        self.send(shell, 2, &words(&[self.xdg_surface, self.surface]));
        self.send(self.xdg_surface, 1, &words(&[self.toplevel_object]));
        self.send(self.toplevel_object, 2, &string(&self.name));
        self.handlers.insert(
            self.toplevel_object,
            Box::new(|c, e| {
                if e.opcode == 0 {
                    if e.word(0) != 0 {
                        c.configured_size.0 = e.word(0);
                    }
                    if e.word(1) != 0 {
                        c.configured_size.1 = e.word(1);
                    }
                }
            }),
        );
        self.handlers.insert(
            self.xdg_surface,
            Box::new(|c, e| {
                if e.opcode == 0 {
                    c.send(c.xdg_surface, 4, &e.data);
                    if c.map_on_configure {
                        c.paint(
                            c.surface,
                            c.configured_size.0,
                            c.configured_size.1,
                            0xff00ff,
                        );
                    }
                }
            }),
        );
        self.send(self.surface, 6, &[]);
        self.event(self.xdg_surface, 0);
        self.roundtrip();
    }
    pub fn request_lock(&mut self) -> u32 {
        let manager = self.bind("ext_session_lock_manager_v1", 1);
        let lock = self.alloc();
        self.send(manager, 1, &words(&[lock]));
        lock
    }
    pub fn lock(&mut self, accepted: bool) -> u32 {
        let lock = self.request_lock();
        let event = self.wait(0, |e| e.object == lock && e.opcode <= 1);
        assert_eq!(event.opcode, if accepted { 0 } else { 1 }, "lock ownership");
        if !accepted {
            self.roundtrip();
            assert!(!self.events.iter().any(|e| e.is(lock, 0)));
        }
        lock
    }
    pub fn lock_surface(&mut self, lock: u32, commit: bool) -> u32 {
        self.get_keyboard();
        self.surface = self.new_surface();
        let output = self.bind("wl_output", 1);
        let surface = self.alloc();
        self.send(lock, 1, &words(&[surface, self.surface, output]));
        if commit {
            let e = self.event(surface, 0);
            assert!(e.word(1) > 0 && e.word(2) > 0);
            self.send(surface, 1, &words(&[e.word(0)]));
            self.paint(self.surface, e.word(1), e.word(2), 0x004080);
            self.roundtrip();
        }
        surface
    }
    pub fn keyboard_events(&self, start: usize, opcodes: &[u16]) -> Vec<Event> {
        self.events[start..]
            .iter()
            .filter(|e| e.object == self.keyboard && opcodes.contains(&e.opcode))
            .cloned()
            .collect()
    }
    pub fn rejected(&mut self) {
        // The invalid request is already sent. Reading avoids a race with
        // the server closing its receive side before we could send sync.
        match self.wait_result(self.events.len(), |_| false) {
            Err(Error::Protocol { .. }) => {}
            Err(Error::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::UnexpectedEof
                        | io::ErrorKind::ConnectionReset
                        | io::ErrorKind::BrokenPipe
                ) => {}
            result => panic!("malicious request not rejected: {result:?}"),
        }
    }
    pub fn expect_error(&mut self, object: u32, code: u32) {
        match self.wait_result(self.events.len(), |_| false) {
            Err(Error::Protocol {
                object: got,
                code: actual,
                ..
            }) => assert_eq!((got, actual), (object, code)),
            result => panic!("expected protocol error {object}/{code}, got {result:?}"),
        }
    }
}
