// SPDX-License-Identifier: GPL-3.0-or-later

//! Local control socket evaluated on the compositor event-loop thread.
//!
//! The wire protocol is one request per connection: the client writes a single
//! expression, half-closes its write side (SHUT_WR), and reads back exactly one
//! reply datum -- `(ok VALUE)` or `(error ...)` -- terminated by a newline,
//! after which the server closes the connection.
//!
//! Like the event push socket (src/events.rs), nothing here may block the
//! compositor event loop. Each accepted connection becomes its own nonblocking
//! calloop source: request bytes accumulate across readiness callbacks until
//! EOF, evaluation runs synchronously on the main thread (that serialization is
//! the contract), and the reply is written nonblocking with a pending buffer
//! for back-pressure. A per-connection deadline timer drops any client -- slow
//! sender or non-reading receiver -- that outlives its budget, so a stalled
//! peer costs one fd for at most [`CLIENT_DEADLINE`] instead of freezing the
//! compositor.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::rc::Rc;

use smithay::reexports::calloop::{
    EventLoop, Interest, LoopHandle, Mode, PostAction, RegistrationToken,
    generic::Generic,
    timer::{TimeoutAction, Timer},
};

use crate::MindeState;

const MAX_REQUEST_BYTES: usize = 64 * 1024;

/// Total lifetime budget for one connection, request through reply. A client
/// that has neither finished sending nor drained its reply by then is dropped.
const CLIENT_DEADLINE: std::time::Duration = std::time::Duration::from_millis(250);

pub fn socket_path() -> std::io::Result<PathBuf> {
    crate::runtime_dir::socket_path("minde-ipc.sock")
}

/// Shared between a client's I/O source and its deadline timer: whether the
/// exchange completed, the calloop token of whichever source (read, then
/// write) currently owns the connection so the timer can evict it, and the
/// timer's own token so a finished exchange can disarm it instead of leaving
/// a dead timer to wake the loop later.
struct ClientState {
    finished: bool,
    token: Option<RegistrationToken>,
    timer: Option<RegistrationToken>,
}

impl ClientState {
    /// Marks the exchange complete and cancels the deadline timer, if it is
    /// still armed.
    fn finish(&mut self, handle: &LoopHandle<'static, MindeState>) {
        self.finished = true;
        if let Some(timer) = self.timer.take() {
            handle.remove(timer);
        }
    }
}

type SharedClient = Rc<RefCell<ClientState>>;

/// Writes as much of the pending reply as the socket will take without
/// blocking. `Ok(true)` means fully drained, `Ok(false)` back-pressured;
/// `Err` means the client should be dropped.
fn try_flush(stream: &mut UnixStream, pending: &mut VecDeque<u8>) -> std::io::Result<bool> {
    while !pending.is_empty() {
        let (head, _) = pending.as_slices();
        match stream.write(head) {
            Ok(0) => return Err(ErrorKind::WriteZero.into()),
            Ok(written) => {
                pending.drain(..written);
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}

fn evaluate(request: &[u8]) -> String {
    if request.len() > MAX_REQUEST_BYTES {
        return "(error request-too-large ())".to_owned();
    }
    match std::str::from_utf8(request) {
        Ok(source) => crate::guile::eval_ipc(source)
            .unwrap_or_else(|| "(error evaluation-failed ())".to_owned()),
        Err(_) => "(error invalid-utf8 ())".to_owned(),
    }
}

/// Hands a back-pressured reply off to a dedicated write-interest source. The
/// read source's stream is cloned (dup'd) so the read source can be removed
/// while the connection stays open until the reply drains.
fn spawn_writer(
    handle: &LoopHandle<'static, MindeState>,
    stream: &UnixStream,
    mut pending: VecDeque<u8>,
    shared: SharedClient,
) {
    let clone = match stream.try_clone() {
        Ok(clone) => clone,
        Err(error) => {
            tracing::warn!(component = "ipc", action = "write", %error,
                "failed to clone IPC stream for reply flush");
            shared.borrow_mut().finish(handle);
            return;
        }
    };
    let writer_shared = shared.clone();
    let writer_handle = handle.clone();
    let token = handle.insert_source(
        Generic::new(clone, Interest::WRITE, Mode::Level),
        move |_, stream, _state| {
            // Safety: the stream remains registered in this source and is
            // neither moved nor dropped through this reference.
            match try_flush(unsafe { stream.get_mut() }, &mut pending) {
                Ok(false) => Ok(PostAction::Continue),
                Ok(true) => {
                    writer_shared.borrow_mut().finish(&writer_handle);
                    Ok(PostAction::Remove)
                }
                Err(error) => {
                    tracing::warn!(component = "ipc", action = "write", %error,
                        "failed to write IPC response");
                    writer_shared.borrow_mut().finish(&writer_handle);
                    Ok(PostAction::Remove)
                }
            }
        },
    );
    match token {
        Ok(token) => shared.borrow_mut().token = Some(token),
        Err(error) => {
            tracing::warn!(component = "ipc", action = "write", %error,
                "failed to register IPC reply source");
            shared.borrow_mut().finish(handle);
        }
    }
}

/// Registers one accepted connection: a read source that accumulates the
/// request until EOF then evaluates and replies, plus a one-shot deadline
/// timer that evicts the client if the exchange has not finished in time
/// (and is disarmed as soon as it does).
fn register_client(handle: &LoopHandle<'static, MindeState>, stream: UnixStream) {
    let shared: SharedClient = Rc::new(RefCell::new(ClientState {
        finished: false,
        token: None,
        timer: None,
    }));
    let reader_shared = shared.clone();
    let reader_handle = handle.clone();
    let mut request: Vec<u8> = Vec::new();
    let token = handle.insert_source(
        Generic::new(stream, Interest::READ, Mode::Level),
        move |_, stream, _state| {
            // Safety: the stream remains registered in this source and is
            // neither moved nor dropped through this reference.
            let stream = unsafe { stream.get_mut() };
            let mut buffer = [0u8; 4096];
            let complete = loop {
                if request.len() > MAX_REQUEST_BYTES {
                    // Oversized: stop reading and reply with the error datum.
                    break true;
                }
                match stream.read(&mut buffer) {
                    Ok(0) => break true, // client half-closed: request complete
                    Ok(count) => request.extend_from_slice(&buffer[..count]),
                    Err(error) if error.kind() == ErrorKind::WouldBlock => break false,
                    Err(error) if error.kind() == ErrorKind::Interrupted => {}
                    Err(error) => {
                        tracing::warn!(component = "ipc", action = "read", %error,
                            "failed to read IPC request");
                        reader_shared.borrow_mut().finish(&reader_handle);
                        return Ok(PostAction::Remove);
                    }
                }
            };
            if !complete {
                return Ok(PostAction::Continue);
            }
            let mut pending: VecDeque<u8> = evaluate(&request).into_bytes().into();
            pending.push_back(b'\n');
            match try_flush(stream, &mut pending) {
                Ok(true) => reader_shared.borrow_mut().finish(&reader_handle),
                Ok(false) => {
                    spawn_writer(&reader_handle, stream, pending, reader_shared.clone());
                }
                Err(error) => {
                    tracing::warn!(component = "ipc", action = "write", %error,
                        "failed to write IPC response");
                    reader_shared.borrow_mut().finish(&reader_handle);
                }
            }
            Ok(PostAction::Remove)
        },
    );
    match token {
        Ok(token) => shared.borrow_mut().token = Some(token),
        Err(error) => {
            tracing::warn!(component = "ipc", action = "register", %error,
                "failed to register IPC client source");
            return;
        }
    }

    let timer_handle = handle.clone();
    let timer_shared = shared.clone();
    let timer = handle.insert_source(
        Timer::from_duration(CLIENT_DEADLINE),
        move |_, _, _state| {
            let mut client = timer_shared.borrow_mut();
            // Firing consumes the registration; forget the token so a
            // later `finish` does not try to remove it again.
            client.timer = None;
            if !client.finished {
                client.finished = true;
                if let Some(token) = client.token.take() {
                    // Removing the source drops its stream, closing the fd.
                    timer_handle.remove(token);
                }
                tracing::warn!(
                    component = "ipc",
                    deadline_ms = CLIENT_DEADLINE.as_millis() as u64,
                    "dropping stalled IPC client past deadline"
                );
            }
            TimeoutAction::Drop
        },
    );
    match timer {
        Ok(token) => shared.borrow_mut().timer = Some(token),
        Err(error) => {
            tracing::warn!(component = "ipc", action = "deadline", %error,
                "failed to arm IPC client deadline timer");
        }
    }
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
    event_loop.handle().insert_source(
        Generic::new(listener, Interest::READ, Mode::Level),
        move |_, listener, _state| {
            loop {
                // Safety: the listener remains registered in this source and
                // is neither moved nor dropped through this reference.
                match unsafe { listener.get_mut() }.accept() {
                    Ok((stream, _)) => {
                        if let Err(error) = stream.set_nonblocking(true) {
                            tracing::warn!(component = "ipc", %error,
                                "failed to make IPC client socket non-blocking");
                            continue;
                        }
                        register_client(&handle, stream);
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                    Err(error) => {
                        tracing::warn!(component = "ipc", action = "accept", %error,
                            "failed to accept IPC connection");
                        break;
                    }
                }
            }
            Ok(PostAction::Continue)
        },
    )?;

    tracing::info!(component = "ipc", path = %path.display(), mode = "0600",
        "main-thread IPC listening");
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
        assert_eq!(name, "minde-ipc.sock");
    }

    #[test]
    fn flush_drains_a_ready_socket() {
        let (mut a, b) = UnixStream::pair().unwrap();
        a.set_nonblocking(true).unwrap();
        let mut pending = VecDeque::from(b"(ok 42)\n".to_vec());
        assert!(try_flush(&mut a, &mut pending).unwrap());
        assert!(pending.is_empty());
        drop(b);
    }

    #[test]
    fn flush_reports_error_when_the_peer_is_gone() {
        let (mut a, b) = UnixStream::pair().unwrap();
        a.set_nonblocking(true).unwrap();
        drop(b);
        let mut pending = VecDeque::from(vec![b'x'; 1024]);
        // A closed peer eventually yields a broken-pipe error the caller
        // treats as a drop signal (never a panic or a block).
        let mut errored = false;
        for _ in 0..1024 {
            if try_flush(&mut a, &mut pending).is_err() {
                errored = true;
                break;
            }
            pending.extend(vec![b'x'; 1024]);
        }
        assert!(errored);
    }
}
