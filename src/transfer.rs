// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded selection and drag transfers, driven by readiness and deadlines.
//! Queued and active transfers share a quota and payload bytes. Pipes and
//! sockets use the event loop. Non-pollable regular files use at most one
//! worker, so a slow filesystem cannot block the compositor or grow threads.

use smithay::reexports::{
    calloop::{
        self, EventSource, Interest, LoopHandle, Mode, Poll, PostAction, Readiness, Token,
        TokenFactory,
        channel::{self, Event, SyncSender},
        generic::Generic,
        timer::{TimeoutAction, Timer},
    },
    rustix::{
        fs::{FileType, OFlags, fcntl_getfl, fcntl_setfl, fstat},
        io::{Errno, read, write},
    },
};
use std::{
    io,
    os::fd::OwnedFd,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

const MAX_TRANSFERS: usize = 32;
const TIMEOUT: Duration = Duration::from_secs(5);
const CANCEL_INTERVAL: Duration = Duration::from_millis(100);
const PASTE_LIMIT: usize = 64 * 1024;
const WRITE_BUDGET: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub struct Payload {
    bytes: Arc<[u8]>,
    live: Arc<AtomicBool>,
}

impl Payload {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes: bytes.into(),
            live: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn cancel(&self) {
        self.live.store(false, Ordering::Release);
    }
    pub fn alive(&self) -> bool {
        self.live.load(Ordering::Acquire)
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug)]
struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn acquire(count: &Arc<AtomicUsize>) -> io::Result<Self> {
        count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_TRANSFERS).then_some(n + 1)
            })
            .map_err(|_| io::Error::other("too many pending selection transfers"))?;
        Ok(Self(count.clone()))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Debug)]
struct WriteRequest {
    fd: OwnedFd,
    payload: Payload,
    deadline: Instant,
    slot: Slot,
}

impl WriteRequest {
    fn write_file(self) {
        // A disk syscall may block despite O_NONBLOCK. The single file worker
        // keeps it off the compositor thread; check cancellation/deadline
        // between bounded writes. Its request owns the fd and quota slot.
        let mut offset = 0;
        while offset < self.payload.bytes.len()
            && self.payload.alive()
            && Instant::now() < self.deadline
        {
            let end = (offset + WRITE_BUDGET).min(self.payload.bytes.len());
            match write(&self.fd, &self.payload.bytes[offset..end]) {
                Ok(0) => break,
                Ok(n) => offset += n,
                Err(Errno::INTR) => continue,
                Err(_) => break,
            }
        }
    }
}

/// Send + Sync so Smithay's compositor-owned DnD sources can queue writes.
#[derive(Clone, Debug)]
pub(crate) struct Transfers {
    sender: SyncSender<WriteRequest>,
    count: Arc<AtomicUsize>,
}

impl Transfers {
    pub fn new<Data: 'static>(handle: &LoopHandle<'static, Data>) -> calloop::Result<Self> {
        let (sender, channel) = channel::sync_channel::<WriteRequest>(MAX_TRANSFERS);
        let weak = handle.downgrade();
        let mut files: Option<std::sync::mpsc::SyncSender<WriteRequest>> = None;
        handle
            .insert_source(channel, move |event, _, _| {
                if let Event::Msg(request) = event
                    && request.payload.alive()
                    && let Some(handle) = weak.upgrade()
                {
                    if fstat(&request.fd).is_ok_and(|stat| {
                        FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile
                    }) {
                        if files.is_none() {
                            let (sender, receiver) =
                                std::sync::mpsc::sync_channel::<WriteRequest>(1);
                            match std::thread::Builder::new()
                                .name("minde-file-copy".into())
                                .spawn(move || {
                                    for request in receiver {
                                        request.write_file();
                                    }
                                }) {
                                Ok(_) => files = Some(sender),
                                Err(error) => {
                                    tracing::debug!(%error, "file transfer worker could not start")
                                }
                            }
                        }
                        if let Some(sender) = &files {
                            // One worker and one waiting file, both charged to
                            // the shared quota. The event loop never waits.
                            let _ = sender.try_send(request);
                        }
                        return;
                    }
                    let transfer = Transfer::new(
                        request.fd,
                        Operation::Write(request.payload, 0),
                        request.deadline,
                        request.slot,
                    );
                    if let Err(error) = transfer.and_then(|transfer| {
                        handle
                            .insert_source(transfer, |_, _, _| {})
                            .map(|_| ())
                            .map_err(|error| io::Error::other(error.error))
                    }) {
                        tracing::debug!(%error, "selection write could not start");
                    }
                }
            })
            .map_err(|error| error.error)?;
        Ok(Self {
            sender,
            count: Arc::new(AtomicUsize::new(0)),
        })
    }

    pub fn write(&self, fd: OwnedFd, payload: Payload) {
        if !payload.alive() {
            return;
        }
        let Ok(slot) = Slot::acquire(&self.count) else {
            return;
        };
        // try_send never blocks the compositor. Dropping a rejected request
        // closes its fd and releases its quota slot, including on shutdown.
        let _ = self.sender.try_send(WriteRequest {
            fd,
            payload,
            deadline: Instant::now() + TIMEOUT,
            slot,
        });
    }

    pub fn read<Data: 'static>(
        &self,
        handle: &LoopHandle<'static, Data>,
        fd: OwnedFd,
        mut complete: impl FnMut(Vec<u8>, &mut Data) + 'static,
    ) -> io::Result<()> {
        let transfer = Transfer::new(
            fd,
            Operation::Read(Vec::new()),
            Instant::now() + TIMEOUT,
            Slot::acquire(&self.count)?,
        )?;
        handle
            .insert_source(transfer, move |bytes, _, state| {
                if let Some(bytes) = bytes {
                    complete(bytes, state);
                }
            })
            .map(|_| ())
            .map_err(|error| io::Error::other(error.error))
    }
}

enum Operation {
    Read(Vec<u8>),
    Write(Payload, usize),
}

struct Transfer {
    io: Generic<OwnedFd>,
    timer: Timer,
    deadline: Instant,
    operation: Operation,
    _slot: Slot,
}

impl Transfer {
    fn new(fd: OwnedFd, operation: Operation, deadline: Instant, slot: Slot) -> io::Result<Self> {
        fcntl_setfl(&fd, fcntl_getfl(&fd)? | OFlags::NONBLOCK)?;
        let (interest, wake) = match operation {
            Operation::Read(_) => (Interest::READ, deadline),
            Operation::Write(..) => (
                Interest::WRITE,
                deadline.min(Instant::now() + CANCEL_INTERVAL),
            ),
        };
        Ok(Self {
            io: Generic::new(fd, interest, Mode::Level),
            timer: Timer::from_deadline(wake),
            deadline,
            operation,
            _slot: slot,
        })
    }

    fn cancelled(&self) -> bool {
        matches!(&self.operation, Operation::Write(payload, _) if !payload.alive())
    }
}

impl EventSource for Transfer {
    type Event = Option<Vec<u8>>;
    type Metadata = ();
    type Ret = ();
    type Error = io::Error;

    fn process_events<F>(
        &mut self,
        readiness: Readiness,
        token: Token,
        mut callback: F,
    ) -> io::Result<PostAction>
    where
        F: FnMut(Self::Event, &mut ()),
    {
        let mut done = self.cancelled() || Instant::now() >= self.deadline;
        if !done {
            let operation = &mut self.operation;
            self.io.process_events(readiness, token, |_, fd| {
                let result = match operation {
                    Operation::Read(bytes) => {
                        let mut buffer = [0; 4096];
                        let remaining = (PASTE_LIMIT - bytes.len()).min(buffer.len());
                        read(&**fd, &mut buffer[..remaining]).map(|n| {
                            bytes.extend_from_slice(&buffer[..n]);
                            n == 0 || bytes.len() == PASTE_LIMIT
                        })
                    }
                    Operation::Write(payload, offset) => {
                        let end = (*offset + WRITE_BUDGET).min(payload.bytes.len());
                        write(&**fd, &payload.bytes[*offset..end]).map(|n| {
                            *offset += n;
                            n == 0 || *offset == payload.bytes.len()
                        })
                    }
                };
                done = match result {
                    Ok(done) => done,
                    Err(Errno::AGAIN | Errno::INTR) => false,
                    Err(_) => true, // Closed peer or unusable fd: release it.
                };
                Ok(PostAction::Continue)
            })?;
        }
        if done {
            let bytes = match &mut self.operation {
                Operation::Read(bytes) => Some(std::mem::take(bytes)),
                Operation::Write(..) => None,
            };
            callback(bytes, &mut ());
            return Ok(PostAction::Remove);
        }
        self.timer.process_events(readiness, token, |_, _| {
            TimeoutAction::ToInstant(self.deadline.min(Instant::now() + CANCEL_INTERVAL))
        })
    }

    fn register(&mut self, poll: &mut Poll, tokens: &mut TokenFactory) -> calloop::Result<()> {
        self.io.register(poll, tokens)?;
        self.timer.register(poll, tokens)
    }

    fn reregister(&mut self, poll: &mut Poll, tokens: &mut TokenFactory) -> calloop::Result<()> {
        self.io.reregister(poll, tokens)?;
        self.timer.reregister(poll, tokens)
    }

    fn unregister(&mut self, poll: &mut Poll) -> calloop::Result<()> {
        let io = self.io.unregister(poll);
        let timer = self.timer.unregister(poll);
        io.and(timer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::reexports::{
        calloop::EventLoop,
        rustix::pipe::{PipeFlags, pipe_with},
    };

    fn pipe() -> (OwnedFd, OwnedFd) {
        pipe_with(PipeFlags::NONBLOCK | PipeFlags::CLOEXEC).unwrap()
    }

    fn drive<Data>(
        event_loop: &mut EventLoop<Data>,
        state: &mut Data,
        mut done: impl FnMut(&Data) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !done(state) {
            assert!(Instant::now() < deadline, "transfer did not finish");
            event_loop
                .dispatch(Duration::from_millis(10), state)
                .unwrap();
        }
    }

    #[test]
    fn queued_and_active_writes_share_a_limit_and_cancel_without_readiness() {
        let mut event_loop = EventLoop::<()>::try_new().unwrap();
        let transfers = Transfers::new(&event_loop.handle()).unwrap();
        let payload = Payload::new(vec![42; 128 * 1024]);
        let mut readers = Vec::new();
        for _ in 0..MAX_TRANSFERS * 3 {
            let (read, write) = pipe();
            transfers.write(write, payload.clone());
            readers.push(read);
        }
        assert_eq!(transfers.count.load(Ordering::Acquire), MAX_TRANSFERS);
        for _ in 0..5 {
            event_loop.dispatch(Duration::ZERO, &mut ()).unwrap();
        }
        assert_eq!(transfers.count.load(Ordering::Acquire), MAX_TRANSFERS);
        payload.cancel();
        drive(&mut event_loop, &mut (), |_| {
            transfers.count.load(Ordering::Acquire) == 0
        });
        let mut bytes = [0; 4096];
        for fd in readers {
            while read(&fd, &mut bytes).unwrap() != 0 {}
        }
        // New writes still work after every slot was occupied and released.
        let (reader, writer) = pipe();
        transfers.write(writer, Payload::new(b"after cancellation".to_vec()));
        drive(&mut event_loop, &mut (), |_| {
            transfers.count.load(Ordering::Acquire) == 0
        });
        let n = read(&reader, &mut bytes).unwrap();
        assert_eq!(&bytes[..n], b"after cancellation");
    }

    #[test]
    fn partial_writes_deliver_every_byte_and_closed_peers_release_slots() {
        let mut event_loop = EventLoop::<()>::try_new().unwrap();
        let transfers = Transfers::new(&event_loop.handle()).unwrap();
        let payload: Vec<_> = (0..200_000).map(|i| (i % 251) as u8).collect();
        let (reader, writer) = pipe();
        transfers.write(writer, Payload::new(payload.clone()));
        let mut received = Vec::new();
        let mut buffer = [0; 4093];
        drive(&mut event_loop, &mut (), |_| {
            match read(&reader, &mut buffer) {
                Ok(0) => return true,
                Ok(n) => received.extend_from_slice(&buffer[..n]),
                Err(Errno::AGAIN) => {}
                Err(error) => panic!("{error}"),
            }
            false
        });
        assert_eq!(received, payload);
        assert_eq!(transfers.count.load(Ordering::Acquire), 0);
        let (reader, writer) = pipe();
        drop(reader);
        transfers.write(writer, Payload::new(vec![1; 128 * 1024]));
        drive(&mut event_loop, &mut (), |_| {
            transfers.count.load(Ordering::Acquire) == 0
        });
    }

    #[test]
    fn read_deadline_delivers_partial_text_once_and_releases_the_pipe() {
        let mut event_loop = EventLoop::<Vec<Vec<u8>>>::try_new().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let (reader, writer) = pipe();
        write(&writer, b"partial").unwrap();
        let transfer = Transfer::new(
            reader,
            Operation::Read(Vec::new()),
            Instant::now() + Duration::from_millis(30),
            Slot::acquire(&count).unwrap(),
        )
        .unwrap();
        event_loop
            .handle()
            .insert_source(transfer, |bytes, _, state| state.push(bytes.unwrap()))
            .unwrap();
        let mut results = Vec::new();
        drive(&mut event_loop, &mut results, |results| !results.is_empty());
        assert_eq!(results, [b"partial".to_vec()]);
        assert_eq!(write(&writer, b"late"), Err(Errno::PIPE));
        assert_eq!(count.load(Ordering::Acquire), 0);
        event_loop.dispatch(Duration::ZERO, &mut results).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn read_size_limit_and_registration_failure_release_resources() {
        let mut event_loop = EventLoop::<Vec<Vec<u8>>>::try_new().unwrap();
        let transfers = Transfers::new(&event_loop.handle()).unwrap();
        let (reader, writer) = pipe();
        transfers
            .read(&event_loop.handle(), reader, |bytes, state| {
                state.push(bytes)
            })
            .unwrap();
        transfers.write(writer, Payload::new(vec![7; PASTE_LIMIT * 2]));
        let mut results = Vec::new();
        drive(&mut event_loop, &mut results, |results| !results.is_empty());
        assert_eq!(results[0], vec![7; PASTE_LIMIT]);
        drive(&mut event_loop, &mut results, |_| {
            transfers.count.load(Ordering::Acquire) == 0
        });
        let fd = std::fs::File::open("/dev/null").unwrap().into();
        assert!(
            transfers
                .read(&event_loop.handle(), fd, |_, _| panic!(
                    "unregistered read callback"
                ))
                .is_err()
        );
        assert_eq!(transfers.count.load(Ordering::Acquire), 0);
    }
}
