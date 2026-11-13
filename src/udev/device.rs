// SPDX-License-Identifier: GPL-3.0-or-later

//! Owners for resources acquired outside Rust's ordinary file ownership.

use std::{os::fd::OwnedFd, path::Path};

use smithay::{
    backend::session::Session,
    reexports::{
        calloop::{LoopHandle, RegistrationToken},
        rustix::fs::OFlags,
    },
    utils::DeviceFd,
};

/// Keep libseat's original descriptor for `Session::close`. Smithay gets
/// a duplicate, so its refcounted descriptor never competes for ownership
/// of the fd by which libseat identifies the device lease.
pub(super) struct SessionDevice<S: Session> {
    session: S,
    fd: Option<OwnedFd>,
}

impl<S: Session> SessionDevice<S> {
    pub(super) fn open(mut session: S, path: &Path) -> Result<Self, S::Error> {
        let fd = session.open(
            path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        )?;
        Ok(Self {
            session,
            fd: Some(fd),
        })
    }

    pub(super) fn duplicate(&self) -> std::io::Result<DeviceFd> {
        self.fd.as_ref().unwrap().try_clone().map(DeviceFd::from)
    }
}

impl<S: Session> Drop for SessionDevice<S> {
    fn drop(&mut self) {
        if let Err(err) = self.session.close(self.fd.take().unwrap()) {
            tracing::warn!(?err, "failed to release session device");
        }
    }
}

/// Removing a calloop source also drops the notifier and its DRM fd.
pub(super) struct EventRegistration<D: 'static> {
    handle: LoopHandle<'static, D>,
    token: RegistrationToken,
}

impl<D: 'static> EventRegistration<D> {
    pub(super) fn new(handle: LoopHandle<'static, D>, token: RegistrationToken) -> Self {
        Self { handle, token }
    }
}

impl<D: 'static> Drop for EventRegistration<D> {
    fn drop(&mut self) {
        self.handle.remove(self.token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::reexports::calloop::{EventLoop, channel};
    use std::{cell::RefCell, fs::File, os::fd::AsRawFd, rc::Rc, time::Duration};

    #[derive(Clone, Default)]
    struct TestSession(Rc<RefCell<Vec<(bool, i32)>>>);

    impl Session for TestSession {
        type Error = ();
        fn open(&mut self, _: &Path, _: OFlags) -> Result<OwnedFd, ()> {
            let fd: OwnedFd = File::open("/dev/null").unwrap().into();
            self.0.borrow_mut().push((true, fd.as_raw_fd()));
            Ok(fd)
        }
        fn close(&mut self, fd: OwnedFd) -> Result<(), ()> {
            self.0.borrow_mut().push((false, fd.as_raw_fd()));
            Ok(())
        }
        fn change_vt(&mut self, _: i32) -> Result<(), ()> {
            unreachable!()
        }
        fn is_active(&self) -> bool {
            true
        }
        fn seat(&self) -> String {
            "test".into()
        }
    }

    #[test]
    fn device_lease_releases_the_original_fd_on_success_error_and_unwind() {
        for failure in [0, 1, 2] {
            let session = TestSession::default();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let device = SessionDevice::open(session.clone(), Path::new("/unused")).unwrap();
                if failure == 1 {
                    return Err(());
                }
                let duplicate = device.duplicate().unwrap();
                assert_ne!(duplicate.as_raw_fd(), session.0.borrow()[0].1);
                if failure == 2 {
                    panic!("injected setup failure");
                }
                drop(duplicate);
                Ok(())
            }));
            assert_eq!(result.is_err(), failure == 2);
            let events = session.0.borrow();
            assert_eq!(events.len(), 2);
            assert_eq!(events[0], (true, events[1].1));
            assert!(!events[1].0);
        }
    }

    #[test]
    fn failed_drm_initialization_releases_the_session_lease() {
        use smithay::backend::drm::{DrmDevice, DrmDeviceFd};

        let session = TestSession::default();
        for _ in 0..32 {
            let device = SessionDevice::open(session.clone(), Path::new("/unused")).unwrap();
            let fd = DrmDeviceFd::new(device.duplicate().unwrap());
            let weak = fd.downgrade();
            assert!(DrmDevice::new(fd, true).is_err());
            assert!(weak.upgrade().is_none(), "failed DRM setup retained its fd");
            drop(device);
        }
        let events = session.0.borrow();
        assert_eq!(events.len(), 64);
        for pair in events.chunks_exact(2) {
            assert_eq!(pair[0], (true, pair[1].1));
            assert!(!pair[1].0);
        }
    }

    #[test]
    fn removing_a_registration_drops_its_source_and_retained_descriptor() {
        let mut event_loop = EventLoop::<usize>::try_new().unwrap();
        for dispatch in [false, true].into_iter().cycle().take(32) {
            let session = TestSession::default();
            let device = SessionDevice::open(session.clone(), Path::new("/unused")).unwrap();
            let fd = device.duplicate().unwrap();
            let retained_fd = fd.clone();
            let (sender, source) = channel::channel::<()>();
            let token = event_loop
                .handle()
                .insert_source(source, move |_, _, count| {
                    assert!(retained_fd.as_raw_fd() >= 0);
                    *count += 1;
                })
                .unwrap();
            let registration = EventRegistration::new(event_loop.handle(), token);
            let mut calls = 0;
            if dispatch {
                sender.send(()).unwrap();
                event_loop.dispatch(Duration::ZERO, &mut calls).unwrap();
                assert_eq!(calls, 1);
            }
            drop(registration);
            assert!(sender.send(()).is_err(), "event source survived removal");
            let owned: OwnedFd = fd.try_into().expect("notifier retained a descriptor clone");
            drop(owned);
            drop(device);
            assert_eq!(session.0.borrow().len(), 2);
            event_loop.dispatch(Duration::ZERO, &mut calls).unwrap();
            assert_eq!(calls, usize::from(dispatch));
        }
    }
}
