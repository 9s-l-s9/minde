// SPDX-License-Identifier: MIT
//! Exclusive gamma ownership and deferred hardware updates. Keep this state
//! independent of the render surface: DPMS and VT changes can remove access
//! to the CRTC while the client and its desired ramps remain valid.

use std::{collections::HashMap, fs::File, io, os::unix::fs::FileExt};

use smithay::reexports::drm::control::{Device as _, crtc};
use smithay::reexports::wayland_protocols_wlr::gamma_control::v1::server::zwlr_gamma_control_v1::ZwlrGammaControlV1;
use smithay::{backend::drm::DrmDeviceFd, output::Output};

type Ramps = (Vec<u16>, Vec<u16>, Vec<u16>);

trait GammaDevice {
    fn read(&self, size: usize) -> io::Result<Ramps>;
    fn write(&self, ramps: &Ramps) -> io::Result<()>;
}

struct DrmGammaDevice {
    fd: DrmDeviceFd,
    crtc: crtc::Handle,
}

impl GammaDevice for DrmGammaDevice {
    fn read(&self, size: usize) -> io::Result<Ramps> {
        let mut ramps = (vec![0; size], vec![0; size], vec![0; size]);
        self.fd
            .get_gamma(self.crtc, &mut ramps.0, &mut ramps.1, &mut ramps.2)?;
        Ok(ramps)
    }

    fn write(&self, ramps: &Ramps) -> io::Result<()> {
        self.fd.set_gamma(self.crtc, &ramps.0, &ramps.1, &ramps.2)
    }
}

struct Entry {
    /// None while restoring a departed owner's ramps is deferred.
    control: Option<ZwlrGammaControlV1>,
    device: Box<dyn GammaDevice>,
    original: Ramps,
    current: Ramps,
}

#[derive(Default)]
pub struct GammaControls {
    entries: HashMap<Output, Entry>,
}

enum ReadError {
    Invalid(&'static str),
    Io(io::Error),
}

impl From<io::Error> for ReadError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn parse_ramps(bytes: &[u8], size: usize) -> Option<Ramps> {
    if bytes.len() != size.checked_mul(6)? {
        return None;
    }
    let ramp = |channel: usize| {
        bytes[channel * size * 2..(channel + 1) * size * 2]
            .chunks_exact(2)
            .map(|value| u16::from_ne_bytes([value[0], value[1]]))
            .collect()
    };
    Some((ramp(0), ramp(1), ramp(2)))
}

fn read_ramps(file: &File, size: usize) -> Result<Ramps, ReadError> {
    let metadata = file.metadata()?;
    let length = size
        .checked_mul(6)
        .ok_or(ReadError::Invalid("gamma table is too large"))?;
    if !metadata.is_file() {
        return Err(ReadError::Invalid(
            "gamma table fd must be a regular file or shared memory",
        ));
    }
    if metadata.len() != length as u64 {
        return Err(ReadError::Invalid("gamma table has the wrong length"));
    }
    // Reject pipes/devices before reading and leave the client's shared file
    // offset untouched. A concurrent truncate is an I/O failure, not a panic.
    let mut bytes = vec![0; length];
    file.read_exact_at(&mut bytes, 0)?;
    parse_ramps(&bytes, size).ok_or(ReadError::Invalid("invalid gamma table"))
}

impl GammaControls {
    pub(super) fn has_owner(&self, output: &Output) -> bool {
        self.entries
            .get(output)
            .is_some_and(|entry| entry.control.is_some())
    }

    pub(super) fn create_drm(
        &mut self,
        output: Output,
        control: ZwlrGammaControlV1,
        fd: DrmDeviceFd,
        crtc: crtc::Handle,
        size: u32,
    ) {
        self.create(
            output,
            control,
            Box::new(DrmGammaDevice { fd, crtc }),
            size as usize,
        );
    }

    fn create(
        &mut self,
        output: Output,
        control: ZwlrGammaControlV1,
        device: Box<dyn GammaDevice>,
        size: usize,
    ) {
        if size == 0 || size > u32::MAX as usize || self.has_owner(&output) {
            control.failed();
            return;
        }
        if let Some(entry) = self.entries.get_mut(&output) {
            // A previous owner died while the head was inaccessible. Retain
            // its original baseline, rather than snapshotting its stale tint.
            if entry.original.0.len() != size {
                control.failed();
                return;
            }
            entry.control = Some(control.clone());
            entry.device = device;
        } else {
            let original = match device.read(size) {
                Ok(ramps) => ramps,
                Err(error) => {
                    tracing::warn!(%error, "failed to read original gamma ramps");
                    control.failed();
                    return;
                }
            };
            self.entries.insert(
                output,
                Entry {
                    control: Some(control.clone()),
                    device,
                    current: original.clone(),
                    original,
                },
            );
        }
        control.gamma_size(size as u32);
    }

    fn owns(&self, output: &Output, control: &ZwlrGammaControlV1) -> bool {
        self.entries
            .get(output)
            .is_some_and(|entry| entry.control.as_ref() == Some(control))
    }

    /// Only the current resource may read a supplied fd or change state.
    pub(super) fn set(
        &mut self,
        output: &Output,
        control: &ZwlrGammaControlV1,
        file: File,
        writable: bool,
    ) -> Result<(), &'static str> {
        if !self.owns(output, control) {
            return Ok(());
        }
        let entry = self.entries.get(output).unwrap();
        let ramps = match read_ramps(&file, entry.original.0.len()) {
            Ok(ramps) => ramps,
            Err(ReadError::Invalid(message)) => return Err(message),
            Err(ReadError::Io(error)) => {
                tracing::warn!(%error, "failed to read gamma table fd");
                control.failed();
                self.finish(output, control, writable);
                return Ok(());
            }
        };
        self.entries.get_mut(output).unwrap().current = ramps;
        if writable {
            self.reapply(output);
        }
        Ok(())
    }

    pub(super) fn finish(&mut self, output: &Output, control: &ZwlrGammaControlV1, writable: bool) {
        if !self.owns(output, control) {
            return;
        }
        let entry = self.entries.get_mut(output).unwrap();
        entry.control = None;
        entry.current = entry.original.clone();
        if writable {
            self.reapply(output);
        }
    }

    /// Called only once the output is accessible again, after reinitializing
    /// its CRTC. Also finishes a deferred restore if its owner disconnected.
    pub fn reapply(&mut self, output: &Output) {
        let Some(entry) = self.entries.get_mut(output) else {
            return;
        };
        if let Err(error) = entry.device.write(&entry.current) {
            tracing::warn!(%error, output = %output.name(), "failed to apply gamma ramps");
            if let Some(control) = entry.control.take() {
                control.failed();
                entry.current = entry.original.clone();
                // Best-effort restoration. Keep it pending if the CRTC is
                // temporarily unavailable, so a later resume can restore it.
                if entry.device.write(&entry.current).is_ok() {
                    self.entries.remove(output);
                }
            }
        } else if entry.control.is_none() {
            self.entries.remove(output);
        }
    }

    pub fn output_removed(&mut self, output: &Output) {
        if let Some(entry) = self.entries.remove(output) {
            // Called before dropping a disabled head's DRM surface. On actual
            // disconnect this can fail; no pending state may outlive the head.
            if let Err(error) = entry.device.write(&entry.original) {
                tracing::warn!(%error, "failed to restore gamma on removed output");
            }
            if let Some(control) = entry.control {
                control.failed();
            }
        }
    }
}

#[cfg(test)]
mod tests;
