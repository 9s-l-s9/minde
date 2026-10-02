// SPDX-License-Identifier: GPL-3.0-or-later

//! Read-only event push socket: a second Unix socket where every subscriber
//! receives each fired compositor event as one s-expression line.
//!
//! The eval socket (src/ipc.rs) is request/response and forces a polling
//! consumer; this socket lets an external agent react in real time. The Scheme
//! hook glue (event-stream.scm, called from `run-event-hook!`) serializes each
//! event and hands the finished line to the `wm-publish-event` gsubr, which
//! calls [`publish_line`]. Serialization, sanitization and lock-privacy policy
//! all live in Scheme; this module only owns the socket, the subscriber set and
//! the non-blocking, slow-consumer-safe delivery.
//!
//! Delivery is fan-out with a bounded per-subscriber backlog. Writes are
//! non-blocking so a stalled reader can never block the compositor event loop;
//! a subscriber whose unsent backlog grows past [`MAX_BACKLOG_BYTES`] is
//! evicted (its connection closed) and the eviction logged. A line is written
//! straight to each socket and only the unwritten remainder is buffered, so a
//! healthy subscriber never costs a backlog copy. Write readiness drains pending
//! bytes when readers resume, and writer sources are removed once drained.
//! Scheme asks
//! [`has_subscribers`] (the `wm-events-active?` gsubr) before serialising at
//! all, so an idle socket costs nothing per hook firing.

use std::collections::VecDeque;
use std::io::{ErrorKind, Write};
use std::net::Shutdown;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use smithay::reexports::calloop::{
    EventLoop, Interest, LoopHandle, Mode, PostAction, RegistrationToken,
    generic::Generic,
    ping::{Ping, make_ping},
};

use crate::MindeState;

const MAX_SUBSCRIBERS: usize = 16;
const MAX_BACKLOG_BYTES: usize = 256 * 1024;

struct Subscriber {
    id: u64,
    stream: UnixStream,
    pending: VecDeque<u8>,
    watcher: Option<RegistrationToken>,
    writer: Option<RegistrationToken>,
}

#[derive(Default)]
struct SubscriberSet {
    subscribers: Vec<Subscriber>,
    next_id: u64,
    ping: Option<Ping>,
    retired: Vec<RegistrationToken>,
}

type SharedSubscribers = Arc<Mutex<SubscriberSet>>;

// Publishing may originate on a REPL thread. Only the event-loop callbacks
// manipulate calloop sources; the ping schedules those changes on that thread.
static SUBSCRIBERS: OnceLock<SharedSubscribers> = OnceLock::new();

fn subscribers() -> &'static SharedSubscribers {
    SUBSCRIBERS.get_or_init(|| Arc::new(Mutex::new(SubscriberSet::default())))
}

pub fn socket_path() -> std::io::Result<PathBuf> {
    crate::runtime_dir::socket_path("minde-events.sock")
}

fn try_flush(subscriber: &mut Subscriber) -> std::io::Result<()> {
    while !subscriber.pending.is_empty() {
        let (head, _) = subscriber.pending.as_slices();
        match subscriber.stream.write(head) {
            Ok(0) => return Err(ErrorKind::WriteZero.into()),
            Ok(written) => {
                subscriber.pending.drain(..written);
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub fn has_subscribers() -> bool {
    !subscribers().lock().unwrap().subscribers.is_empty()
}

/// Preserve order and enforce the ceiling before growing the pending buffer.
fn deliver(subscriber: &mut Subscriber, bytes: &[u8]) -> std::io::Result<()> {
    try_flush(subscriber)?;
    let mut offset = 0;
    if subscriber.pending.is_empty() {
        while offset < bytes.len() {
            match subscriber.stream.write(&bytes[offset..]) {
                Ok(0) => return Err(ErrorKind::WriteZero.into()),
                Ok(written) => offset += written,
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }
    let remaining = &bytes[offset..];
    if remaining.len() > MAX_BACKLOG_BYTES - subscriber.pending.len() {
        return Err(std::io::Error::new(
            ErrorKind::OutOfMemory,
            "event subscriber exceeded the backlog ceiling",
        ));
    }
    subscriber.pending.extend(remaining);
    Ok(())
}

impl SubscriberSet {
    fn retire(&mut self, index: usize) {
        let subscriber = self.subscribers.remove(index);
        // The read/write sources own duplicated descriptors. Shutdown closes
        // the connection immediately even if removal must await a main-thread ping.
        let _ = subscriber.stream.shutdown(Shutdown::Both);
        self.retired.extend(subscriber.watcher);
        self.retired.extend(subscriber.writer);
    }

    fn wake(&self) {
        if let Some(ping) = &self.ping {
            ping.ping();
        }
    }
}

fn publish_to(shared: &SharedSubscribers, bytes: &[u8]) {
    let mut set = shared.lock().unwrap();
    let mut changed = false;
    let mut index = 0;
    while index < set.subscribers.len() {
        let subscriber = &mut set.subscribers[index];
        match deliver(subscriber, bytes) {
            Ok(()) => {
                changed |= !subscriber.pending.is_empty() && subscriber.writer.is_none();
                index += 1;
            }
            Err(error) => {
                tracing::info!(component = "events", %error,
                    "event subscriber disconnected or evicted");
                set.retire(index);
                changed = true;
            }
        }
    }
    if changed {
        set.wake();
    }
}

/// Mirrors one serialized event, appending a newline. Slow readers have a
/// bounded backlog, drained by write readiness even after the last publish.
pub fn publish_line(line: &str) {
    if !has_subscribers() {
        return;
    }
    let mut bytes = Vec::with_capacity(line.len() + 1);
    bytes.extend_from_slice(line.as_bytes());
    bytes.push(b'\n');
    publish_to(subscribers(), &bytes);
}

fn register_writer<D: 'static>(
    handle: &LoopHandle<'static, D>,
    shared: SharedSubscribers,
    id: u64,
    stream: &UnixStream,
) -> Result<RegistrationToken, Box<dyn std::error::Error>> {
    let writer_handle = handle.clone();
    Ok(handle.insert_source(
        Generic::new(stream.try_clone()?, Interest::WRITE, Mode::Level),
        move |_, _, _| {
            let mut set = shared.lock().unwrap();
            let Some(index) = set.subscribers.iter().position(|s| s.id == id) else {
                return Ok(PostAction::Remove);
            };
            let subscriber = &mut set.subscribers[index];
            match try_flush(subscriber) {
                Ok(()) if !subscriber.pending.is_empty() => Ok(PostAction::Continue),
                Ok(()) => {
                    subscriber.writer = None;
                    Ok(PostAction::Remove)
                }
                Err(error) => {
                    tracing::info!(component = "events", %error,
                        "event subscriber disconnected while flushing");
                    // This callback removes its own source via PostAction.
                    subscriber.writer = None;
                    set.retire(index);
                    for token in set.retired.drain(..) {
                        writer_handle.remove(token);
                    }
                    Ok(PostAction::Remove)
                }
            }
        },
    )?)
}

fn register_flush<D: 'static>(
    handle: &LoopHandle<'static, D>,
    shared: SharedSubscribers,
) -> Result<(), Box<dyn std::error::Error>> {
    let (ping, source) = make_ping()?;
    let flush_handle = handle.clone();
    let flush_shared = shared.clone();
    handle.insert_source(source, move |(), _, _| {
        let mut set = flush_shared.lock().unwrap();
        for token in set.retired.drain(..) {
            flush_handle.remove(token);
        }
        let mut index = 0;
        while index < set.subscribers.len() {
            let subscriber = &mut set.subscribers[index];
            if subscriber.pending.is_empty() || subscriber.writer.is_some() {
                index += 1;
                continue;
            }
            match register_writer(
                &flush_handle,
                flush_shared.clone(),
                subscriber.id,
                &subscriber.stream,
            ) {
                Ok(token) => {
                    subscriber.writer = Some(token);
                    index += 1;
                }
                Err(error) => {
                    tracing::warn!(component = "events", %error,
                        "failed to register event subscriber writer");
                    set.retire(index);
                    for token in set.retired.drain(..) {
                        flush_handle.remove(token);
                    }
                }
            }
        }
    })?;
    shared.lock().unwrap().ping = Some(ping);
    Ok(())
}

fn register_subscriber<D: 'static>(
    handle: &LoopHandle<'static, D>,
    shared: &SharedSubscribers,
    stream: UnixStream,
) -> Result<(), Box<dyn std::error::Error>> {
    stream.set_nonblocking(true)?;
    let mut set = shared.lock().unwrap();
    if set.subscribers.len() >= MAX_SUBSCRIBERS {
        tracing::warn!(
            component = "events",
            max = MAX_SUBSCRIBERS,
            "rejecting event subscriber past the connection cap"
        );
        return Ok(());
    }
    let id = set.next_id;
    set.next_id += 1;
    let watcher_shared = shared.clone();
    let watcher_handle = handle.clone();
    let token = handle.insert_source(
        // Linux epoll always reports HUP/ERR, even with empty interest.
        // Watching only those events preserves write-half-closed readers and
        // cannot spin on input or EOF while no event bytes need flushing.
        Generic::new(stream.try_clone()?, Interest::EMPTY, Mode::Level),
        move |_, _, _| {
            let mut set = watcher_shared.lock().unwrap();
            if let Some(index) = set.subscribers.iter().position(|s| s.id == id) {
                set.subscribers[index].watcher = None;
                set.retire(index);
                for token in set.retired.drain(..) {
                    watcher_handle.remove(token);
                }
            }
            Ok(PostAction::Remove)
        },
    )?;
    set.subscribers.push(Subscriber {
        id,
        stream,
        pending: VecDeque::new(),
        watcher: Some(token),
        writer: None,
    });
    tracing::info!(
        component = "events",
        count = set.subscribers.len(),
        "event subscriber connected"
    );
    Ok(())
}

pub fn init(
    event_loop: &mut EventLoop<'static, MindeState>,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = socket_path()?;
    crate::runtime_dir::remove_stale_socket(&path)?;
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let handle = event_loop.handle();
    register_flush(&handle, subscribers().clone())?;
    let listener_handle = handle.clone();
    handle.insert_source(
        Generic::new(listener, Interest::READ, Mode::Level),
        move |_, listener, _state| {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if let Err(error) =
                            register_subscriber(&listener_handle, subscribers(), stream)
                        {
                            tracing::warn!(component = "events", %error,
                                "failed to register event subscriber");
                        }
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                    Err(error) => {
                        tracing::warn!(component = "events", action = "accept", %error,
                            "failed to accept event subscriber");
                        break;
                    }
                }
            }
            Ok(PostAction::Continue)
        },
    )?;
    tracing::info!(component = "events", path = %path.display(), mode = "0600",
        "event push socket listening");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::time::{Duration, Instant};

    fn fixture() -> (EventLoop<'static, ()>, SharedSubscribers, UnixStream) {
        let event_loop = EventLoop::try_new().unwrap();
        let shared = Arc::new(Mutex::new(SubscriberSet::default()));
        register_flush(&event_loop.handle(), shared.clone()).unwrap();
        let (stream, peer) = UnixStream::pair().unwrap();
        peer.set_nonblocking(true).unwrap();
        register_subscriber(&event_loop.handle(), &shared, stream).unwrap();
        (event_loop, shared, peer)
    }

    fn drain(peer: &mut UnixStream, received: &mut Vec<u8>) {
        match peer.read_to_end(received) {
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            Err(error) => panic!("failed to read events: {error}"),
        }
    }

    fn backpressure(shared: &SharedSubscribers) -> Vec<u8> {
        let mut expected = Vec::new();
        for index in 0..1024 {
            let bytes = format!("({index:04} {})\n", "x".repeat(8192));
            publish_to(shared, bytes.as_bytes());
            expected.extend_from_slice(bytes.as_bytes());
            let set = shared.lock().unwrap();
            assert_eq!(set.subscribers.len(), 1);
            if !set.subscribers[0].pending.is_empty() {
                return expected;
            }
        }
        panic!("socket never became backpressured");
    }

    #[test]
    fn resumes_final_backlog_without_a_new_publish_and_preserves_order() {
        let (mut event_loop, shared, mut peer) = fixture();
        let mut expected = backpressure(&shared);
        // Exercise the off-thread publish path, including its coalesced ping.
        let worker_shared = shared.clone();
        std::thread::spawn(move || publish_to(&worker_shared, b"(last)\n"))
            .join()
            .unwrap();
        expected.extend_from_slice(b"(last)\n");
        event_loop.dispatch(Duration::ZERO, &mut ()).unwrap();
        assert!(shared.lock().unwrap().subscribers[0].writer.is_some());

        let mut received = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        while received.len() < expected.len() && Instant::now() < deadline {
            drain(&mut peer, &mut received);
            event_loop
                .dispatch(Duration::from_millis(5), &mut ())
                .unwrap();
        }
        drain(&mut peer, &mut received);
        assert_eq!(received, expected);
        let set = shared.lock().unwrap();
        assert!(set.subscribers[0].pending.is_empty());
        assert!(set.subscribers[0].writer.is_none());
        drop(set);
        // No writable socket is left registered to make an idle loop spin.
        let started = Instant::now();
        event_loop
            .dispatch(Duration::from_millis(30), &mut ())
            .unwrap();
        assert!(started.elapsed() >= Duration::from_millis(25));
    }

    #[test]
    fn backlog_ceiling_evicts_and_removes_registered_sources() {
        let (mut event_loop, shared, mut peer) = fixture();
        backpressure(&shared);
        event_loop.dispatch(Duration::ZERO, &mut ()).unwrap();
        assert!(shared.lock().unwrap().subscribers[0].writer.is_some());
        publish_to(&shared, &vec![b'x'; MAX_BACKLOG_BYTES + 1]);
        assert!(shared.lock().unwrap().subscribers.is_empty());
        event_loop.dispatch(Duration::ZERO, &mut ()).unwrap();
        assert!(shared.lock().unwrap().retired.is_empty());
        let mut received = Vec::new();
        // Shutdown makes EOF visible even though sources previously owned dup fds.
        peer.read_to_end(&mut received).unwrap();
        let started = Instant::now();
        event_loop
            .dispatch(Duration::from_millis(30), &mut ())
            .unwrap();
        assert!(started.elapsed() >= Duration::from_millis(25));
    }

    #[test]
    fn cap_is_checked_before_growing_the_backlog() {
        let (_, shared, _peer) = fixture();
        backpressure(&shared);
        let mut set = shared.lock().unwrap();
        let subscriber = &mut set.subscribers[0];
        let before = subscriber.pending.len();
        let capacity = subscriber.pending.capacity();
        let error = deliver(subscriber, &vec![b'x'; MAX_BACKLOG_BYTES + 1]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::OutOfMemory);
        assert_eq!(subscriber.pending.len(), before);
        assert_eq!(subscriber.pending.capacity(), capacity);
    }

    #[test]
    fn disconnect_without_new_events_frees_the_subscription_slot() {
        let (mut event_loop, shared, peer) = fixture();
        drop(peer);
        event_loop
            .dispatch(Duration::from_millis(100), &mut ())
            .unwrap();
        assert!(shared.lock().unwrap().subscribers.is_empty());
        assert!(shared.lock().unwrap().retired.is_empty());
    }

    #[test]
    fn disconnect_during_backpressure_removes_both_sources() {
        let (mut event_loop, shared, peer) = fixture();
        backpressure(&shared);
        event_loop.dispatch(Duration::ZERO, &mut ()).unwrap();
        drop(peer);
        event_loop
            .dispatch(Duration::from_millis(100), &mut ())
            .unwrap();
        assert!(shared.lock().unwrap().subscribers.is_empty());
        assert!(shared.lock().unwrap().retired.is_empty());
    }

    #[test]
    fn write_half_close_keeps_delivery_and_full_close_reclaims_the_slot() {
        let (mut event_loop, shared, mut peer) = fixture();
        peer.shutdown(Shutdown::Write).unwrap();
        let started = Instant::now();
        event_loop
            .dispatch(Duration::from_millis(30), &mut ())
            .unwrap();
        assert!(started.elapsed() >= Duration::from_millis(25));
        assert_eq!(shared.lock().unwrap().subscribers.len(), 1);
        publish_to(&shared, b"(after-half-close)\n");
        let mut received = Vec::new();
        drain(&mut peer, &mut received);
        assert_eq!(received, b"(after-half-close)\n");
        drop(peer);
        event_loop
            .dispatch(Duration::from_millis(100), &mut ())
            .unwrap();
        assert!(shared.lock().unwrap().subscribers.is_empty());
    }

    #[test]
    fn subscriber_limit_rejects_an_extra_connection() {
        let (event_loop, shared, peer) = fixture();
        let mut peers = vec![peer];
        for _ in 1..MAX_SUBSCRIBERS {
            let (stream, peer) = UnixStream::pair().unwrap();
            register_subscriber(&event_loop.handle(), &shared, stream).unwrap();
            peers.push(peer);
        }
        let (stream, mut rejected) = UnixStream::pair().unwrap();
        register_subscriber(&event_loop.handle(), &shared, stream).unwrap();
        assert_eq!(shared.lock().unwrap().subscribers.len(), MAX_SUBSCRIBERS);
        assert_eq!(rejected.read(&mut [0]).unwrap(), 0);
    }

    #[test]
    fn socket_is_scoped_to_the_current_user() {
        assert_eq!(
            socket_path().unwrap().file_name().unwrap(),
            "minde-events.sock"
        );
    }
}
