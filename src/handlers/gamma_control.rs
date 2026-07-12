// SPDX-License-Identifier: MIT

//! `wlr-gamma-control-unstable-v1`: lets a privileged client (gammastep,
//! wlsunset) set per-output gamma ramps for blue-light reduction.
//!
//! This is the compositor's only hand-written Wayland protocol: everything
//! else rides Smithay built-ins, and the blanket `delegate_dispatch2!`
//! (handlers/mod.rs) covers only those, so the `GlobalDispatch`/`Dispatch`
//! impls live here in full.
//!
//! Backend-bound by design: the manager global is created only by the
//! udev/DRM backend (see `udev::init_udev`), since gamma goes through the
//! legacy DRM `SETGAMMA` ioctl on a real CRTC. Winit never advertises it.
//!
//! Lifecycle: on `get_gamma_control` we snapshot the CRTC's current ramps
//! and hand the client `gamma_size`. `set_gamma` writes new ramps via the
//! ioctl; destroy/client-death restores the snapshot. Any condition we
//! can't honour (no CRTC, gamma-less CRTC, a second control on one output,
//! a failed ioctl) degrades to the protocol's `failed` event rather than a
//! hard error, matching wlroots.

use std::io::Read;

use smithay::backend::drm::DrmDeviceFd;
use smithay::output::Output;
use smithay::reexports::{
    drm::control::{Device as ControlDevice, crtc},
    wayland_protocols_wlr::gamma_control::v1::server::{
        zwlr_gamma_control_manager_v1::{self, ZwlrGammaControlManagerV1},
        zwlr_gamma_control_v1::{self, ZwlrGammaControlV1},
    },
    wayland_server::{
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        backend::{ClientId, GlobalId},
        protocol::wl_output::WlOutput,
    },
};
use tracing::warn;

use crate::MindeState;

/// Per-output gamma control state held in `MindeState::gamma_controls`,
/// keyed by the compositor `Output`. Owns the client's control resource,
/// the CRTC handle its gamma is driven through, and the original ramps to
/// restore on destroy/death.
pub struct GammaControlEntry {
    control: ZwlrGammaControlV1,
    fd: DrmDeviceFd,
    crtc: crtc::Handle,
    size: usize,
    /// The ramps as they were before this client touched them (r, g, b),
    /// each `size` u16s. Written back on restore.
    original: (Vec<u16>, Vec<u16>, Vec<u16>),
}

/// User data on a `ZwlrGammaControlV1`: which output it drives. The entry
/// itself lives in `gamma_controls` (needs `&mut MindeState` to touch);
/// this is only the key to find it. `None` for a control that never bound
/// to a resolvable output (immediately failed) -- still needs valid user
/// data since the resource must be initialized.
pub struct GammaControlUserData {
    output: Option<Output>,
}

/// Splits a flat `set_gamma` payload into three `size`-length u16 ramps.
///
/// The protocol payload is three successive ramps (red, green, blue), each
/// `size` native-endian u16s, so exactly `3 * size * 2` bytes. Any other
/// length is a client error (`invalid_gamma`); returns `None` so the caller
/// can raise it.
fn parse_ramps(bytes: &[u8], size: usize) -> Option<(Vec<u16>, Vec<u16>, Vec<u16>)> {
    if bytes.len() != size * 3 * 2 {
        return None;
    }
    let ramp = |channel: usize| -> Vec<u16> {
        bytes[channel * size * 2..(channel + 1) * size * 2]
            .chunks_exact(2)
            .map(|c| u16::from_ne_bytes([c[0], c[1]]))
            .collect()
    };
    Some((ramp(0), ramp(1), ramp(2)))
}

impl MindeState {
    /// Writes an entry's saved original ramps back to its CRTC. Best-effort:
    /// a failure here (e.g. the CRTC vanished) is logged, not surfaced.
    fn restore_gamma(entry: &GammaControlEntry) {
        let (r, g, b) = &entry.original;
        if let Err(err) = entry.fd.set_gamma(entry.crtc, r, g, b) {
            warn!(?err, "failed to restore original gamma ramps");
        }
    }

    /// Drops the control for `output` after restoring its original ramps.
    /// Shared by explicit destroy and client death.
    fn finish_gamma_control(&mut self, output: &Output) {
        if let Some(entry) = self.gamma_controls.remove(output) {
            Self::restore_gamma(&entry);
        }
    }

    /// Called by the udev backend when an output is going away: the client's
    /// control is now void, so send `failed`. No restore -- the CRTC is gone.
    pub fn gamma_output_removed(&mut self, output: &Output) {
        if let Some(entry) = self.gamma_controls.remove(output) {
            entry.control.failed();
        }
    }
}

// zwlr_gamma_control_manager_v1: advertised only on udev (see module docs).

impl GlobalDispatch<ZwlrGammaControlManagerV1, ()> for MindeState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrGammaControlManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwlrGammaControlManagerV1, ()> for MindeState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _manager: &ZwlrGammaControlManagerV1,
        request: zwlr_gamma_control_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_gamma_control_manager_v1::Request;
        match request {
            Request::GetGammaControl { id, output } => {
                state.create_gamma_control(id, &output, data_init);
            }
            Request::Destroy => {}
            _ => unreachable!(),
        }
    }
}

impl MindeState {
    fn create_gamma_control(
        &mut self,
        id: New<ZwlrGammaControlV1>,
        wl_output: &WlOutput,
        data_init: &mut DataInit<'_, MindeState>,
    ) {
        let Some(output) = Output::from_resource(wl_output) else {
            // No compositor output for this wl_output: init so the resource
            // is live (mandatory), then fail it.
            let control = data_init.init(id, GammaControlUserData { output: None });
            control.failed();
            return;
        };

        let control = data_init.init(
            id,
            GammaControlUserData {
                output: Some(output.clone()),
            },
        );

        // Exclusive access: a second control on the same output fails.
        if self.gamma_controls.contains_key(&output) {
            control.failed();
            return;
        }

        // The CRTC and its ramp length come from the udev backend; a
        // gamma-less CRTC (size 0) or an output with no CRTC can't be driven.
        let Some((fd, crtc, size)) = self.gamma_info_for_output(&output) else {
            control.failed();
            return;
        };
        if size == 0 {
            control.failed();
            return;
        }
        let size = size as usize;

        // Snapshot the current ramps to restore on destroy/death.
        let (mut r, mut g, mut b) = (vec![0u16; size], vec![0u16; size], vec![0u16; size]);
        if let Err(err) = fd.get_gamma(crtc, &mut r, &mut g, &mut b) {
            warn!(?err, "failed to read current gamma ramps");
            control.failed();
            return;
        }

        control.gamma_size(size as u32);
        self.gamma_controls.insert(
            output,
            GammaControlEntry {
                control,
                fd,
                crtc,
                size,
                original: (r, g, b),
            },
        );
    }
}

// zwlr_gamma_control_v1: one per output, owns exclusive gamma access.

impl Dispatch<ZwlrGammaControlV1, GammaControlUserData> for MindeState {
    fn request(
        state: &mut Self,
        _client: &Client,
        control: &ZwlrGammaControlV1,
        request: zwlr_gamma_control_v1::Request,
        data: &GammaControlUserData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_gamma_control_v1::Request;
        match request {
            Request::SetGamma { fd } => {
                let Some(output) = data.output.as_ref() else {
                    return;
                };
                let Some(entry) = state.gamma_controls.get(output) else {
                    // Already failed/void; ignore late writes.
                    return;
                };
                let size = entry.size;

                // Validate the payload length by seeking (wlroots does the
                // same): a pipe can't seek, so this also rejects fds that
                // could block the event loop indefinitely on read.
                let mut file = std::fs::File::from(fd);
                let expected = (size * 3 * 2) as u64;
                let bytes = (|| -> std::io::Result<Vec<u8>> {
                    use std::io::{Seek, SeekFrom};
                    let len = file.seek(SeekFrom::End(0))?;
                    file.seek(SeekFrom::Start(0))?;
                    if len != expected {
                        // Wrong size is the client's error; report it as an
                        // empty read so the parse below raises invalid_gamma.
                        return Ok(Vec::new());
                    }
                    let mut bytes = vec![0u8; expected as usize];
                    file.read_exact(&mut bytes)?;
                    Ok(bytes)
                })();
                let bytes = match bytes {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        warn!(?err, "failed to read gamma table fd");
                        state.finish_gamma_control(output);
                        control.failed();
                        return;
                    }
                };
                let Some((r, g, b)) = parse_ramps(&bytes, size) else {
                    control.post_error(
                        zwlr_gamma_control_v1::Error::InvalidGamma,
                        "gamma table has the wrong length",
                    );
                    return;
                };

                let entry = state.gamma_controls.get(output).unwrap();
                if let Err(err) = entry.fd.set_gamma(entry.crtc, &r, &g, &b) {
                    warn!(?err, "failed to set gamma ramps");
                    state.finish_gamma_control(output);
                    control.failed();
                }
            }
            Request::Destroy => {
                // `destroyed` runs the restore; nothing to do here.
            }
            _ => unreachable!(),
        }
    }

    fn destroyed(
        state: &mut Self,
        _client: ClientId,
        _control: &ZwlrGammaControlV1,
        data: &GammaControlUserData,
    ) {
        if let Some(output) = data.output.as_ref() {
            state.finish_gamma_control(output);
        }
    }
}

/// Creates the manager global and returns its id. Called only by the udev
/// backend; the winit backend deliberately never advertises gamma control.
pub fn init_gamma_control_manager(dh: &DisplayHandle) -> GlobalId {
    dh.create_global::<MindeState, ZwlrGammaControlManagerV1, ()>(1, ())
}

#[cfg(test)]
mod tests {
    use super::parse_ramps;

    #[test]
    fn parse_ramps_splits_three_channels() {
        let size = 2;
        // r = [1, 2], g = [3, 4], b = [5, 6], native-endian u16.
        let mut bytes = Vec::new();
        for v in [1u16, 2, 3, 4, 5, 6] {
            bytes.extend_from_slice(&v.to_ne_bytes());
        }
        let (r, g, b) = parse_ramps(&bytes, size).expect("well-formed table");
        assert_eq!(r, vec![1, 2]);
        assert_eq!(g, vec![3, 4]);
        assert_eq!(b, vec![5, 6]);
    }

    #[test]
    fn parse_ramps_rejects_wrong_length() {
        let size = 4; // expects 4 * 3 * 2 = 24 bytes
        assert!(parse_ramps(&[0u8; 23], size).is_none());
        assert!(parse_ramps(&[0u8; 25], size).is_none());
        assert!(parse_ramps(&[], size).is_none());
    }
}
