// SPDX-License-Identifier: MIT
// Real Wayland resource identities/events, with only the CRTC I/O replaced.
use super::*;
use crate::{MindeState, handlers::gamma_control::GammaControlUserData};
use smithay::{
    output::{PhysicalProperties, Subpixel},
    reexports::wayland_server::{Client, Display, DisplayHandle, Resource},
};
use std::{
    cell::RefCell,
    io::{Read, Seek, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::net::UnixStream,
    },
    rc::Rc,
    sync::Arc,
};

struct Hardware {
    ramps: Ramps,
    writes: Vec<Ramps>,
    reads: usize,
    fail_writes: usize,
    fail_read: bool,
}

#[derive(Clone)]
struct MockDevice(Rc<RefCell<Hardware>>);

impl GammaDevice for MockDevice {
    fn read(&self, _size: usize) -> io::Result<Ramps> {
        let mut hardware = self.0.borrow_mut();
        hardware.reads += 1;
        if hardware.fail_read {
            return Err(io::Error::other("injected read failure"));
        }
        Ok(hardware.ramps.clone())
    }

    fn write(&self, ramps: &Ramps) -> io::Result<()> {
        let mut hardware = self.0.borrow_mut();
        hardware.writes.push(ramps.clone());
        if hardware.fail_writes > 0 {
            hardware.fail_writes -= 1;
            return Err(io::Error::other("injected write failure"));
        }
        hardware.ramps = ramps.clone();
        Ok(())
    }
}

struct Fixture {
    _display: Display<MindeState>,
    handle: DisplayHandle,
    client: Client,
    stream: UnixStream,
    output: Output,
    controls: GammaControls,
    device: MockDevice,
}

fn ramps(first: u16) -> Ramps {
    (
        vec![first, first + 1],
        vec![first + 2, first + 3],
        vec![first + 4, first + 5],
    )
}

fn file(ramps: &Ramps) -> File {
    let fd = unsafe { libc::memfd_create(c"gamma-test".as_ptr(), libc::MFD_CLOEXEC) };
    assert!(fd >= 0);
    let mut file = unsafe { File::from_raw_fd(fd) };
    for value in ramps.0.iter().chain(&ramps.1).chain(&ramps.2) {
        file.write_all(&value.to_ne_bytes()).unwrap();
    }
    file
}

impl Fixture {
    fn new() -> Self {
        let display = Display::<MindeState>::new().unwrap();
        let mut handle = display.handle();
        let (server, stream) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        let client = handle
            .insert_client(server, Arc::new(crate::state::ClientState::default()))
            .unwrap();
        let output = Output::new(
            "test".into(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "test".into(),
                model: "test".into(),
                serial_number: "test".into(),
            },
        );
        Self {
            _display: display,
            handle,
            client,
            stream,
            output,
            controls: GammaControls::default(),
            device: MockDevice(Rc::new(RefCell::new(Hardware {
                ramps: ramps(1),
                writes: Vec::new(),
                reads: 0,
                fail_writes: 0,
                fail_read: false,
            }))),
        }
    }

    fn resource(&self) -> ZwlrGammaControlV1 {
        self.client
            .create_resource::<_, _, MindeState>(
                &self.handle,
                1,
                GammaControlUserData {
                    output: Some(self.output.clone()),
                },
            )
            .unwrap()
    }

    fn claim(&mut self) -> ZwlrGammaControlV1 {
        let control = self.resource();
        self.controls.create(
            self.output.clone(),
            control.clone(),
            Box::new(self.device.clone()),
            2,
        );
        control
    }

    fn set(&mut self, control: &ZwlrGammaControlV1, first: u16, writable: bool) {
        self.controls
            .set(&self.output, control, file(&ramps(first)), writable)
            .unwrap();
    }

    fn events(&mut self) -> Vec<(u32, u16)> {
        self.handle.flush_clients().unwrap();
        let mut bytes = Vec::new();
        match self.stream.read_to_end(&mut bytes) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("reading protocol events: {error}"),
        }
        let mut events = Vec::new();
        while !bytes.is_empty() {
            let id = u32::from_ne_bytes(bytes[..4].try_into().unwrap());
            let header = u32::from_ne_bytes(bytes[4..8].try_into().unwrap());
            events.push((id, header as u16));
            bytes.drain(..(header >> 16) as usize);
        }
        events
    }
}

#[test]
fn failed_duplicate_cannot_write_fail_or_destroy_owner() {
    let mut f = Fixture::new();
    let owner = f.claim();
    f.set(&owner, 10, true);
    let duplicate = f.claim();
    assert_eq!(
        f.events(),
        vec![
            (owner.id().protocol_id(), 0),
            (duplicate.id().protocol_id(), 1)
        ]
    );
    f.set(&duplicate, 20, true);
    let wrong_length = file(&ramps(30));
    wrong_length.set_len(1).unwrap();
    assert!(
        f.controls
            .set(&f.output, &duplicate, wrong_length, true)
            .is_ok()
    );
    // Keep the underlying memfd alive while reopening a write-only descriptor.
    let payload = file(&ramps(40));
    let write_only = File::options()
        .write(true)
        .open(format!("/proc/self/fd/{}", payload.as_raw_fd()))
        .unwrap();
    assert!(
        f.controls
            .set(&f.output, &duplicate, write_only, true)
            .is_ok()
    );
    f.controls.finish(&f.output, &duplicate, true);
    assert!(f.controls.owns(&f.output, &owner));
    assert_eq!(f.device.0.borrow().writes, vec![ramps(10)]);
    assert!(f.events().is_empty());
    f.set(&owner, 50, true);
    f.controls.finish(&f.output, &owner, true);
    assert_eq!(f.device.0.borrow().ramps, ramps(1));
    assert!(!f.controls.has_owner(&f.output));
}

#[test]
fn controls_on_different_clients_do_not_share_identity() {
    let mut f = Fixture::new();
    let owner = f.claim();
    let (server, _stream) = UnixStream::pair().unwrap();
    let other = f
        .handle
        .insert_client(server, Arc::new(crate::state::ClientState::default()))
        .unwrap();
    let duplicate = other
        .create_resource::<ZwlrGammaControlV1, _, MindeState>(
            &f.handle,
            1,
            GammaControlUserData {
                output: Some(f.output.clone()),
            },
        )
        .unwrap();
    // The wire number alone is not sufficient: it belongs to a client.
    assert_eq!(owner.id().protocol_id(), duplicate.id().protocol_id());
    assert_ne!(owner, duplicate);
    f.controls.create(
        f.output.clone(),
        duplicate.clone(),
        Box::new(f.device.clone()),
        2,
    );
    f.set(&duplicate, 20, true);
    f.controls.finish(&f.output, &duplicate, true);
    assert!(f.controls.owns(&f.output, &owner));
    assert!(f.device.0.borrow().writes.is_empty());
    f.set(&owner, 10, true);
    assert_eq!(f.device.0.borrow().ramps, ramps(10));
}

#[test]
fn removed_output_and_late_destroy_do_not_revoke_new_owner() {
    let mut f = Fixture::new();
    let old = f.claim();
    f.set(&old, 10, true);
    f.controls.output_removed(&f.output);
    assert_eq!(f.device.0.borrow().ramps, ramps(1));
    let new = f.claim();
    f.set(&new, 20, true);
    f.set(&old, 30, true);
    f.controls.finish(&f.output, &old, true);
    assert!(f.controls.owns(&f.output, &new));
    assert_eq!(f.device.0.borrow().ramps, ramps(20));
    assert_eq!(
        f.events(),
        vec![
            (old.id().protocol_id(), 0),
            (old.id().protocol_id(), 1),
            (new.id().protocol_id(), 0)
        ]
    );
}

#[test]
fn inaccessible_updates_and_crtc_resets_replay_the_latest_ramps() {
    let mut f = Fixture::new();
    let owner = f.claim();
    f.set(&owner, 10, false);
    f.set(&owner, 20, false);
    assert!(f.device.0.borrow().writes.is_empty());
    for _ in 0..3 {
        f.device.0.borrow_mut().ramps = ramps(0); // power, modeset or VT reset
        f.controls.reapply(&f.output);
        assert_eq!(f.device.0.borrow().ramps, ramps(20));
    }
    assert!(f.controls.owns(&f.output, &owner));
    assert_eq!(f.events(), vec![(owner.id().protocol_id(), 0)]);
}

#[test]
fn disconnected_owner_is_restored_after_power_on() {
    let mut f = Fixture::new();
    let owner = f.claim();
    f.set(&owner, 10, true);
    f.controls.finish(&f.output, &owner, false);
    assert!(!f.controls.has_owner(&f.output));
    assert_eq!(f.device.0.borrow().ramps, ramps(10));
    f.set(&owner, 20, false);
    f.controls.reapply(&f.output);
    assert_eq!(f.device.0.borrow().ramps, ramps(1));
    assert!(f.controls.entries.is_empty());
}

#[test]
fn replacement_before_deferred_restore_keeps_original_baseline() {
    let mut f = Fixture::new();
    let first = f.claim();
    f.set(&first, 10, true);
    f.controls.finish(&f.output, &first, false);
    let second = f.claim();
    assert_eq!(f.device.0.borrow().reads, 1);
    f.set(&second, 20, false);
    f.controls.finish(&f.output, &first, true);
    f.controls.reapply(&f.output);
    assert_eq!(f.device.0.borrow().ramps, ramps(20));
    f.controls.finish(&f.output, &second, true);
    assert_eq!(f.device.0.borrow().ramps, ramps(1));
}

#[test]
fn failed_reapply_retires_owner_and_retains_failed_restore() {
    let mut f = Fixture::new();
    let owner = f.claim();
    f.set(&owner, 10, false);
    f.device.0.borrow_mut().fail_writes = 3;
    f.controls.reapply(&f.output); // desired write and first restore fail
    assert!(!f.controls.has_owner(&f.output));
    f.controls.reapply(&f.output); // restore remains pending
    assert!(!f.controls.entries.is_empty());
    f.controls.reapply(&f.output);
    assert!(f.controls.entries.is_empty());
    assert_eq!(f.device.0.borrow().ramps, ramps(1));
    assert_eq!(
        f.events(),
        vec![(owner.id().protocol_id(), 0), (owner.id().protocol_id(), 1)]
    );
}

#[test]
fn payload_validation_preserves_owner_and_shared_file_offset() {
    let mut f = Fixture::new();
    let owner = f.claim();
    let mut payload = file(&ramps(10));
    payload.seek(io::SeekFrom::Start(4)).unwrap();
    f.controls
        .set(&f.output, &owner, payload.try_clone().unwrap(), true)
        .unwrap();
    assert_eq!(payload.stream_position().unwrap(), 4);
    payload.set_len(1).unwrap();
    assert!(f.controls.set(&f.output, &owner, payload, true).is_err());
    let mut pipe = [0; 2];
    assert_eq!(
        unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) },
        0
    );
    let reader = unsafe { File::from_raw_fd(pipe[0]) };
    let _writer = unsafe { File::from_raw_fd(pipe[1]) };
    assert!(f.controls.set(&f.output, &owner, reader, true).is_err());
    assert!(f.controls.owns(&f.output, &owner));
    assert_eq!(f.device.0.borrow().writes, vec![ramps(10)]);
    assert!(parse_ramps(&[], usize::MAX).is_none());
    assert_eq!(
        parse_ramps(&[1, 0, 2, 0, 3, 0], 1),
        Some((
            vec![u16::from_ne_bytes([1, 0])],
            vec![u16::from_ne_bytes([2, 0])],
            vec![u16::from_ne_bytes([3, 0])]
        ))
    );
}

#[test]
fn snapshot_and_payload_io_failures_send_failed() {
    let mut f = Fixture::new();
    f.device.0.borrow_mut().fail_read = true;
    let failed = f.claim();
    assert!(!f.controls.has_owner(&f.output));
    f.device.0.borrow_mut().fail_read = false;
    let owner = f.claim();
    let payload = file(&ramps(10));
    let write_only = File::options()
        .write(true)
        .open(format!("/proc/self/fd/{}", payload.as_raw_fd()))
        .unwrap();
    assert!(f.controls.set(&f.output, &owner, write_only, true).is_ok());
    assert!(!f.controls.has_owner(&f.output));
    assert_eq!(f.device.0.borrow().ramps, ramps(1));
    assert_eq!(
        f.events(),
        vec![
            (failed.id().protocol_id(), 1),
            (owner.id().protocol_id(), 0),
            (owner.id().protocol_id(), 1)
        ]
    );
}
