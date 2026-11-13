// SPDX-License-Identifier: MIT
//! Window frame eligibility and demand-driven pacing for parked windows.

use crate::MindeState;
use std::time::Duration;

use smithay::{
    desktop::{Space, Window, space::SpaceElement},
    output::Output,
    reexports::calloop::{
        LoopHandle, RegistrationToken,
        timer::{TimeoutAction, Timer},
    },
    utils::{Logical, Rectangle},
    wayland::compositor::SurfaceAttributes,
};

/// Keep powered-off outputs in the layout for visibility checks, but select
/// one rendering output to service windows parked outside every output.
pub(crate) struct FrameCallbacks<'a, E: SpaceElement> {
    space: &'a Space<E>,
    output_geometry: Option<Rectangle<i32, Logical>>,
    serves_parked: bool,
}

impl<'a, E: SpaceElement + PartialEq> FrameCallbacks<'a, E> {
    pub(crate) fn new(
        space: &'a Space<E>,
        output: &Output,
        can_render: impl Fn(&Output) -> bool,
    ) -> Self {
        Self {
            space,
            output_geometry: space.output_geometry(output).filter(|_| can_render(output)),
            serves_parked: space.outputs().find(|o| can_render(o)) == Some(output),
        }
    }

    /// The boolean distinguishes visible windows from parked windows.
    pub(crate) fn windows(&self) -> impl Iterator<Item = (&E, bool)> {
        self.space.elements().filter_map(|window| {
            let output_geometry = self.output_geometry?;
            let geo = self.space.element_geometry(window);
            let on_this = geo.is_some_and(|g| g.overlaps(output_geometry));
            // Visible windows already qualify. Only the selected rendering
            // output needs the more expensive parked-window check.
            let selected = on_this
                || (self.serves_parked
                    && geo
                        .map(|g| {
                            !self
                                .space
                                .outputs()
                                .filter_map(|o| self.space.output_geometry(o))
                                .any(|og| g.overlaps(og))
                        })
                        .unwrap_or(true));
            selected.then_some((window, on_this))
        })
    }
}

fn pending_frames(window: &Window) -> bool {
    let mut pending = false;
    window.with_surfaces(|_, states| {
        pending |= !states
            .cached_state
            .get::<SurfaceAttributes>()
            .current()
            .frame_callbacks
            .is_empty();
    });
    pending
}

impl FrameCallbacks<'_, Window> {
    /// Send visible frames immediately and report whether hidden requests need
    /// a timer. Traversing the whole window includes popups and subsurfaces.
    pub(crate) fn send_visible(&self, output: &Output, now: Duration) -> bool {
        let mut parked_pending = false;
        for (window, visible) in self.windows() {
            if visible {
                window.send_frame(output, now, Some(Duration::ZERO), |_, _| {
                    Some(output.clone())
                });
            } else {
                parked_pending |= pending_frames(window);
            }
        }
        parked_pending
    }
}

/// A single timer batches all pending hidden-window callbacks. Timers are
/// armed only for pending requests, and no GPU work is needed to wake
/// a hidden client on an otherwise idle desktop.
#[derive(Default)]
pub(crate) struct ParkedFrames {
    token: Option<RegistrationToken>,
    last_sent: Option<Duration>,
}

impl ParkedFrames {
    pub(crate) fn schedule(&mut self, handle: &LoopHandle<'static, MindeState>, now: Duration) {
        if self.token.is_some() {
            return;
        }
        let due = self
            .last_sent
            .unwrap_or(now)
            .saturating_add(Duration::from_secs(1));
        self.token = handle
            .insert_source(
                Timer::from_duration(due.saturating_sub(now)),
                |_, _, state| {
                    state.parked_frames.token = None;
                    state.send_parked_frames();
                    TimeoutAction::Drop
                },
            )
            .ok();
    }
}

impl MindeState {
    fn send_parked_frames(&mut self) {
        if self.locked {
            return;
        }
        let Some(output) = self
            .space
            .outputs()
            .find(|o| self.output_can_render(o))
            .cloned()
        else {
            return;
        };
        let frames = FrameCallbacks::new(&self.space, &output, |o| self.output_can_render(o));
        let now = self.start_time.elapsed();
        let mut sent = false;
        for (window, visible) in frames.windows() {
            if !visible && pending_frames(window) {
                window.send_frame(&output, now, Some(Duration::ZERO), |_, _| {
                    Some(output.clone())
                });
                sent = true;
            }
        }
        if sent {
            self.parked_frames.last_sent = Some(now);
        }
        let _ = self.display_handle.flush_clients();
    }
}
