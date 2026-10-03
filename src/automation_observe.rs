// SPDX-License-Identifier: GPL-3.0-or-later

//! Thread-safe snapshots used by the Scheme automation inspection primitives.
//!
//! IPC evaluation normally runs on the compositor thread, but the optional
//! Guile REPL may call gsubrs from a Guile-owned thread. `wm-window-geometry`
//! reads live committed Space bounds when safe, using this last-published
//! committed snapshot only off-thread or during reentrant command application.
//! No compositor objects are lent across threads. Pointer position is always
//! read from the thread-safe snapshot.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

static POINTER_X: AtomicI32 = AtomicI32::new(0);
static POINTER_Y: AtomicI32 = AtomicI32::new(0);
static GEOMETRY_REVISION: AtomicU64 = AtomicU64::new(0);
static WINDOW_GEOMETRIES: OnceLock<Mutex<HashMap<u64, [i32; 4]>>> = OnceLock::new();

fn window_geometries() -> &'static Mutex<HashMap<u64, [i32; 4]>> {
    WINDOW_GEOMETRIES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn set_pointer_position(x: f64, y: f64) {
    POINTER_X.store(x.round() as i32, Ordering::SeqCst);
    POINTER_Y.store(y.round() as i32, Ordering::SeqCst);
}

pub fn pointer_position() -> (i32, i32) {
    (
        POINTER_X.load(Ordering::SeqCst),
        POINTER_Y.load(Ordering::SeqCst),
    )
}

/// Publish committed bounds and report whether the observed fact changed.
/// The native epoch detects change-and-revert between desktop snapshots,
/// without entering Scheme from pointer dispatch or surface commit paths.
pub fn set_window_geometry(id: u64, rect: Option<[i32; 4]>) -> bool {
    let mut geometries = window_geometries().lock().unwrap();
    if geometries.get(&id).copied() == rect {
        return false;
    }
    if let Some(rect) = rect {
        geometries.insert(id, rect);
    } else {
        geometries.remove(&id);
    }
    GEOMETRY_REVISION.fetch_add(1, Ordering::SeqCst);
    true
}

pub fn geometry_revision() -> u64 {
    GEOMETRY_REVISION.load(Ordering::SeqCst)
}

pub fn window_geometry(id: u64) -> Option<[i32; 4]> {
    window_geometries().lock().unwrap().get(&id).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_coordinates_are_rounded_consistently() {
        set_pointer_position(12.49, -8.51);
        assert_eq!(pointer_position(), (12, -9));
    }

    #[test]
    fn window_geometry_can_be_published_and_removed() {
        let before = geometry_revision();
        assert!(set_window_geometry(991, Some([-10, 20, 640, 480])));
        assert_eq!(window_geometry(991), Some([-10, 20, 640, 480]));
        assert!(!set_window_geometry(991, Some([-10, 20, 640, 480])));
        assert!(set_window_geometry(991, Some([-10, 20, 639, 480])));
        assert!(set_window_geometry(991, Some([-10, 20, 640, 480])));
        assert_eq!(geometry_revision(), before + 3);
        assert!(set_window_geometry(991, None));
        assert_eq!(window_geometry(991), None);
        assert!(!set_window_geometry(991, None));
        assert_eq!(geometry_revision(), before + 4);
    }
}
