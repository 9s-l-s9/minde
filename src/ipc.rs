// SPDX-License-Identifier: GPL-3.0-or-later

//! Local control socket evaluated on the compositor event-loop thread.

use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;

use smithay::reexports::calloop::{EventLoop, Interest, Mode, PostAction, generic::Generic};

use crate::MindeState;

const MAX_REQUEST_BYTES: usize = 64 * 1024;

pub fn socket_path() -> PathBuf {
    let directory = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    directory.join("minde-ipc.sock")
}

pub fn init(
    event_loop: &mut EventLoop<'static, MindeState>,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = socket_path();
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
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
                    Ok((mut stream, _)) => {
                        let _ =
                            stream.set_read_timeout(Some(std::time::Duration::from_millis(250)));
                        let mut request = Vec::new();
                        if let Err(error) = Read::by_ref(&mut stream)
                            .take((MAX_REQUEST_BYTES + 1) as u64)
                            .read_to_end(&mut request)
                        {
                            tracing::warn!(%error, "failed to read IPC request");
                            continue;
                        }
                        let response = if request.len() > MAX_REQUEST_BYTES {
                            "(error request-too-large ())".to_owned()
                        } else {
                            match std::str::from_utf8(&request) {
                                Ok(source) => crate::guile::eval_ipc(source)
                                    .unwrap_or_else(|| "(error evaluation-failed ())".to_owned()),
                                Err(_) => "(error invalid-utf8 ())".to_owned(),
                            }
                        };
                        if let Err(error) = writeln!(stream, "{response}") {
                            tracing::warn!(%error, "failed to write IPC response");
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) => {
                        tracing::warn!(%error, "failed to accept IPC connection");
                        break;
                    }
                }
            }
            Ok(PostAction::Continue)
        },
    )?;

    tracing::info!(path = %path.display(), "main-thread IPC listening");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_is_scoped_to_the_current_user() {
        let name = socket_path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(name, "minde-ipc.sock");
    }
}
