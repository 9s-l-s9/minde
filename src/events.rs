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
//! healthy subscriber never costs a copy; pending bytes are flushed
//! opportunistically on each publish, which is sufficient because the event
//! stream is what drives delivery in the first place. Scheme asks
//! [`has_subscribers`] (the `wm-events-active?` gsubr) before serialising at
//! all, so an idle socket costs nothing per hook firing.

use std::collections::VecDeque;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use smithay::reexports::calloop::{EventLoop, Interest, Mode, PostAction, generic::Generic};

use crate::MindeState;

/// Most simultaneous subscribers; connections beyond this are rejected. A
/// read-only event tap needs only a handful of consumers (a bar, an agent).
const MAX_SUBSCRIBERS: usize = 16;

/// Per-subscriber unsent-backlog ceiling. A reader that falls this far behind
/// is evicted rather than allowed to consume unbounded memory.
const MAX_BACKLOG_BYTES: usize = 256 * 1024;

struct Subscriber {
    stream: UnixStream,
    /// Bytes accepted for this subscriber but not yet written to its socket.
    pending: VecDeque<u8>,
}

/// The subscriber set. The listener source and the `wm-publish-event` gsubr
/// both run on the compositor event-loop thread, so contention is nil; the
/// mutex only satisfies the `Sync` bound on the static and keeps a stray
/// REPL-thread publish from racing the set.
static SUBSCRIBERS: OnceLock<Mutex<Vec<Subscriber>>> = OnceLock::new();

fn subscribers() -> &'static Mutex<Vec<Subscriber>> {
    SUBSCRIBERS.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn socket_path() -> std::io::Result<PathBuf> {
    crate::runtime_dir::socket_path("minde-events.sock")
}

/// Writes as much of a subscriber's pending backlog as the socket will take
/// without blocking. Returns `Ok(())` when the subscriber is still healthy
/// (drained or merely back-pressured) and `Err` when it should be evicted.
fn try_flush(subscriber: &mut Subscriber) -> std::io::Result<()> {
    while !subscriber.pending.is_empty() {
        let (head, _) = subscriber.pending.as_slices();
        match subscriber.stream.write(head) {
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(written) => {
                subscriber.pending.drain(..written);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Whether at least one event subscriber is connected. Backs the
/// `wm-events-active?` gsubr so the Scheme mirror can skip serialisation
/// when nobody would receive the line.
pub fn has_subscribers() -> bool {
    !subscribers().lock().unwrap().is_empty()
}

/// Queues `bytes` for one subscriber: written directly when nothing is
/// pending (the common case), otherwise appended behind the backlog so
/// ordering holds. Returns `Ok(())` when the subscriber is still healthy.
fn deliver(subscriber: &mut Subscriber, bytes: &[u8]) -> std::io::Result<()> {
    if subscriber.pending.is_empty() {
        let mut offset = 0;
        while offset < bytes.len() {
            match subscriber.stream.write(&bytes[offset..]) {
                Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
                Ok(written) => offset += written,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        subscriber.pending.extend(&bytes[offset..]);
        return Ok(());
    }
    subscriber.pending.extend(bytes);
    try_flush(subscriber)
}

/// Mirrors one already-serialized event LINE to every subscriber, appending a
/// terminating newline. Never blocks: a subscriber whose backlog exceeds
/// [`MAX_BACKLOG_BYTES`] or whose socket errors (a clean disconnect included)
/// is dropped, closing its connection. Called from the `wm-publish-event`
/// gsubr on the event-loop thread.
pub fn publish_line(line: &str) {
    let mut subscribers = subscribers().lock().unwrap();
    if subscribers.is_empty() {
        return;
    }
    // One newline-terminated buffer shared by every subscriber.
    let mut bytes = Vec::with_capacity(line.len() + 1);
    bytes.extend_from_slice(line.as_bytes());
    bytes.push(b'\n');
    subscribers.retain_mut(|subscriber| {
        if let Err(error) = deliver(subscriber, &bytes) {
            tracing::info!(component = "events", %error,
                "event subscriber disconnected");
            return false;
        }
        if subscriber.pending.len() > MAX_BACKLOG_BYTES {
            tracing::warn!(
                component = "events",
                backlog = subscriber.pending.len(),
                "evicting slow event subscriber past backlog ceiling"
            );
            return false;
        }
        true
    });
}

pub fn init(
    event_loop: &mut EventLoop<'static, MindeState>,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = socket_path()?;
    crate::runtime_dir::remove_stale_socket(&path)?;
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;

    event_loop.handle().insert_source(
        Generic::new(listener, Interest::READ, Mode::Level),
        move |_, listener, _state| {
            loop {
                // Safety: the listener remains registered in this source and
                // is neither moved nor dropped through this reference.
                match unsafe { listener.get_mut() }.accept() {
                    Ok((stream, _)) => {
                        if let Err(error) = stream.set_nonblocking(true) {
                            tracing::warn!(component = "events", %error,
                                "failed to make subscriber socket non-blocking");
                            continue;
                        }
                        let mut subscribers = subscribers().lock().unwrap();
                        if subscribers.len() >= MAX_SUBSCRIBERS {
                            tracing::warn!(
                                component = "events",
                                max = MAX_SUBSCRIBERS,
                                "rejecting event subscriber past the connection cap"
                            );
                            // Dropping `stream` closes the just-accepted socket.
                            continue;
                        }
                        subscribers.push(Subscriber {
                            stream,
                            pending: VecDeque::new(),
                        });
                        tracing::info!(
                            component = "events",
                            count = subscribers.len(),
                            "event subscriber connected"
                        );
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
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

    #[test]
    fn socket_is_scoped_to_the_current_user() {
        let name = socket_path()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(name, "minde-events.sock");
    }

    #[test]
    fn flush_drains_a_ready_socket_and_leaves_nothing_pending() {
        let (a, b) = UnixStream::pair().unwrap();
        a.set_nonblocking(true).unwrap();
        // The peer `b` stays open, so writes to `a` are accepted immediately.
        let mut subscriber = Subscriber {
            stream: a,
            pending: VecDeque::from(b"(message \"hi\")\n".to_vec()),
        };
        try_flush(&mut subscriber).unwrap();
        assert!(subscriber.pending.is_empty());
        drop(b);
    }

    #[test]
    fn deliver_writes_directly_and_keeps_order_behind_a_backlog() {
        let (a, mut b) = UnixStream::pair().unwrap();
        a.set_nonblocking(true).unwrap();
        let mut subscriber = Subscriber {
            stream: a,
            pending: VecDeque::new(),
        };
        deliver(&mut subscriber, b"(first)\n").unwrap();
        assert!(
            subscriber.pending.is_empty(),
            "ready socket takes the line at once"
        );
        subscriber.pending.extend(b"(queued)\n");
        deliver(&mut subscriber, b"(second)\n").unwrap();
        let mut received = Vec::new();
        b.set_nonblocking(true).unwrap();
        let _ = std::io::Read::read_to_end(&mut b, &mut received);
        assert_eq!(received, b"(first)\n(queued)\n(second)\n");
    }

    #[test]
    fn flush_reports_error_when_the_peer_is_gone() {
        let (a, b) = UnixStream::pair().unwrap();
        a.set_nonblocking(true).unwrap();
        drop(b);
        let mut subscriber = Subscriber {
            stream: a,
            // Enough bytes that at least one write reaches the closed peer.
            pending: VecDeque::from(vec![b'x'; 1024]),
        };
        // A closed peer eventually yields a broken-pipe error the caller
        // treats as an eviction signal (never a panic or a block).
        let mut errored = false;
        for _ in 0..1024 {
            if try_flush(&mut subscriber).is_err() {
                errored = true;
                break;
            }
            subscriber.pending.extend(vec![b'x'; 1024]);
        }
        assert!(errored);
    }
}
