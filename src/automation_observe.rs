// SPDX-License-Identifier: GPL-3.0-or-later

//! Thread-safe snapshots used by the Scheme automation inspection primitives.
//!
//! IPC evaluation normally runs on the compositor thread, but the optional
//! Guile REPL may call gsubrs from a Guile-owned thread.  Keep the values
//! exposed by `wm-pointer-position` and `wm-window-geometry` independent of
//! `MindeState` rather than lending compositor objects across threads.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

static POINTER_X: AtomicI32 = AtomicI32::new(0);
static POINTER_Y: AtomicI32 = AtomicI32::new(0);
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

pub fn set_window_geometry(id: u64, rect: Option<[i32; 4]>) {
    let mut geometries = window_geometries().lock().unwrap();
    if let Some(rect) = rect {
        geometries.insert(id, rect);
    } else {
        geometries.remove(&id);
    }
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
        set_window_geometry(991, Some([-10, 20, 640, 480]));
        assert_eq!(window_geometry(991), Some([-10, 20, 640, 480]));
        set_window_geometry(991, None);
        assert_eq!(window_geometry(991), None);
    }
}
