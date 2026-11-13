// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded observation traffic during interactive move and resize grabs.
//! Committed geometry and its revision remain current; only notifications
//! are delayed. No Scheme callbacks run while pointer dispatch holds a lock.

use smithay::reexports::calloop::{
    RegistrationToken,
    timer::{TimeoutAction, Timer},
};
use std::{
    collections::{BTreeMap, HashSet},
    time::Duration,
};

use crate::MindeState;

#[derive(Default)]
pub(crate) struct GeometryEvents {
    active: HashSet<u64>,
    pending: BTreeMap<u64, Option<[i32; 4]>>,
    timer: Option<RegistrationToken>,
}

fn publish(name: &str, id: u64, geometry: Option<[i32; 4]>) {
    let line = match geometry {
        Some([x, y, width, height]) => format!("({name} {id} ({x} {y} {width} {height}))"),
        None => format!("({name} {id} #f)"),
    };
    crate::events::publish_line(&line);
}

impl MindeState {
    pub(crate) fn begin_geometry_grab(&mut self, id: u64) {
        // A client may start another grab before acknowledging an earlier
        // resize's final configure. Finish that observation before replacing it.
        self.finish_geometry_grab(id);
        self.geometry_events.active.insert(id);
    }

    /// Update inspection and output associations now; coalesce grab events.
    pub(crate) fn publish_window_geometry(&mut self, id: u64) {
        if let Some(window) = self.windows.get(&id).cloned() {
            self.publish_geometry_for_window(&window);
        }
    }

    pub(crate) fn publish_geometry_for_window(&mut self, window: &smithay::desktop::Window) {
        let Some(id) = self.id_for_window(window) else {
            return;
        };
        let bounds = self.space.element_geometry(window);
        self.refresh_foreign_toplevel_geometry(id, bounds);
        let geometry = self.visible_window_geometry(bounds);
        if !crate::automation_observe::set_window_geometry(id, geometry) {
            return;
        }
        if !self.geometry_events.active.contains(&id) {
            publish("window-geometry", id, geometry);
            return;
        }
        self.geometry_events.pending.insert(id, geometry);
        if self.geometry_events.timer.is_none() {
            let timer = Timer::from_duration(Duration::from_millis(50));
            match self.handle.insert_source(timer, |_, _, state| {
                state.geometry_events.timer = None;
                state.flush_geometry_events();
                TimeoutAction::Drop
            }) {
                Ok(token) => self.geometry_events.timer = Some(token),
                Err(error) => {
                    tracing::warn!(%error, "cannot schedule geometry notifications");
                    self.flush_geometry_events();
                }
            }
        }
    }

    fn flush_geometry_events(&mut self) {
        for (id, geometry) in std::mem::take(&mut self.geometry_events.pending) {
            publish("window-geometry", id, geometry);
        }
    }

    /// Release/cancellation uses committed bounds. Native resize calls this
    /// after its final configure is committed, not on the requested size.
    pub(crate) fn finish_geometry_grab(&mut self, id: u64) {
        if !self.geometry_events.active.contains(&id) {
            return;
        }
        self.flush_geometry_grab(id);
        self.forget_geometry_events(id);
        if self.windows.contains_key(&id) {
            publish("window-geometry-settled", id, self.window_geometry(id));
        }
    }

    /// A native resize release reports the current bounds immediately while
    /// its eventual final commit remains eligible for a settled notification.
    pub(crate) fn flush_geometry_grab(&mut self, id: u64) {
        if !self.geometry_events.active.contains(&id) {
            return;
        }
        self.geometry_events.pending.remove(&id);
        self.cancel_empty_geometry_timer();
        if !self.windows.contains_key(&id) {
            return;
        }
        self.refresh_foreign_toplevel_output(id);
        let geometry = self.window_geometry(id);
        crate::automation_observe::set_window_geometry(id, geometry);
        // Always send the final fact, even if the periodic timer just sent
        // the same bounds. Existing consumers retain the ordinary event.
        publish("window-geometry", id, geometry);
    }

    /// Do not let a delayed notification follow a window's destruction.
    pub(crate) fn forget_geometry_events(&mut self, id: u64) {
        self.geometry_events.active.remove(&id);
        self.geometry_events.pending.remove(&id);
        self.cancel_empty_geometry_timer();
    }

    fn cancel_empty_geometry_timer(&mut self) {
        if self.geometry_events.pending.is_empty()
            && let Some(token) = self.geometry_events.timer.take()
        {
            self.handle.remove(token);
        }
    }
}
