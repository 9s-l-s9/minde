// SPDX-License-Identifier: MIT

//! Resize grab is the state of a composer during which the client window is being resized.
//!
//! eg. Usually whenever a user clicks on the app's border and starts dragging, the compositors
//! enters a ResizeSurfaceGrab state.

use crate::MindeState;
use smithay::{
    desktop::Window,
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
        GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
        GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData as PointerGrabStartData,
        MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Point, Rectangle, Serial, Size},
    wayland::{compositor, shell::xdg::SurfaceCachedState},
};
use std::{
    cell::RefCell,
    sync::{Arc, Mutex},
};

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct ResizeEdge: u32 {
        const TOP          = 0b0001;
        const BOTTOM       = 0b0010;
        const LEFT         = 0b0100;
        const RIGHT        = 0b1000;

        const TOP_LEFT     = Self::TOP.bits() | Self::LEFT.bits();
        const BOTTOM_LEFT  = Self::BOTTOM.bits() | Self::LEFT.bits();

        const TOP_RIGHT    = Self::TOP.bits() | Self::RIGHT.bits();
        const BOTTOM_RIGHT = Self::BOTTOM.bits() | Self::RIGHT.bits();
    }
}

impl From<xdg_toplevel::ResizeEdge> for ResizeEdge {
    #[inline]
    fn from(x: xdg_toplevel::ResizeEdge) -> Self {
        Self::from_bits(x as u32).unwrap()
    }
}

pub struct ResizeSurfaceGrab {
    start_data: PointerGrabStartData<MindeState>,
    pub(crate) window: Window,

    edges: ResizeEdge,

    initial_rect: Rectangle<i32, Logical>,
    last_window_size: Size<i32, Logical>,
    configures: Arc<Mutex<ResizeConfigureQueue>>,
    released: bool,
}

/// One latest size per event-loop dispatch. Clearing the pending size also
/// cancels the idle callback when the grab ends or is replaced.
struct ResizeConfigureQueue {
    pending: Option<Size<i32, Logical>>,
    sent: Size<i32, Logical>,
}

impl ResizeConfigureQueue {
    fn new(size: Size<i32, Logical>) -> Self {
        Self {
            pending: None,
            sent: size,
        }
    }

    /// Returns true only when a new idle callback is needed.
    fn request(&mut self, size: Size<i32, Logical>) -> bool {
        if self.pending.is_none() && size == self.sent {
            return false;
        }
        self.pending.replace(size).is_none()
    }

    fn take(&mut self) -> Option<Size<i32, Logical>> {
        let size = self.pending.take()?;
        if size == self.sent {
            None
        } else {
            self.sent = size;
            Some(size)
        }
    }

    fn cancel(&mut self) {
        self.pending = None;
    }
}

impl ResizeSurfaceGrab {
    pub fn start(
        start_data: PointerGrabStartData<MindeState>,
        window: Window,
        edges: ResizeEdge,
        initial_window_rect: Rectangle<i32, Logical>,
    ) -> Self {
        let initial_rect = initial_window_rect;

        // The commit-time reposition state only applies to xdg toplevels;
        // X11 windows (super+drag on a floated one) are resized by direct
        // configure and never enter the commit dance.
        if let Some(toplevel) = window.toplevel() {
            ResizeSurfaceState::with(toplevel.wl_surface(), |state| {
                *state = ResizeSurfaceState::Resizing {
                    edges,
                    initial_rect,
                };
            });
        }

        Self {
            start_data,
            window,
            edges,
            initial_rect,
            last_window_size: initial_rect.size,
            configures: Arc::new(Mutex::new(ResizeConfigureQueue::new(initial_rect.size))),
            released: false,
        }
    }

    fn report_geometry(
        &self,
        data: &mut MindeState,
        rect: Rectangle<i32, Logical>,
        final_serial: Option<Serial>,
    ) {
        let Some(id) = data.id_for_window(&self.window) else {
            return;
        };
        data.publish_window_geometry(id);
        let window = self.window.clone();
        // Pointer dispatch holds a non-reentrant lock. Policy may touch the
        // pointer, so notify it only once dispatch has returned.
        data.handle.insert_idle(move |data| {
            if data.windows.get(&id) != Some(&window) {
                return;
            }
            let awaiting_commit = final_serial.is_some_and(|serial| {
                window.toplevel().is_some_and(|xdg| {
                    ResizeSurfaceState::with(xdg.wl_surface(), |state| {
                        matches!(*state, ResizeSurfaceState::WaitingForLastCommit {
                            serial: pending, ..
                        } if pending == serial)
                    })
                })
            });
            // A final commit may already have supplied the client's chosen
            // size. Never overwrite that with the earlier requested size.
            let rect = if final_serial.is_some() && !awaiting_commit {
                data.space.element_geometry(&window).unwrap_or(rect)
            } else {
                rect
            };
            crate::guile::on_window_moved(id, rect.loc.x, rect.loc.y, rect.size.w, rect.size.h);
        });
    }
}

impl PointerGrab<MindeState> for ResizeSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut MindeState,
        handle: &mut PointerInnerHandle<'_, MindeState>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        // While the grab is active, no client has pointer focus
        handle.motion(data, None, event);

        let mut delta = event.location - self.start_data.location;

        let mut new_window_width = self.initial_rect.size.w;
        let mut new_window_height = self.initial_rect.size.h;

        if self.edges.intersects(ResizeEdge::LEFT | ResizeEdge::RIGHT) {
            if self.edges.intersects(ResizeEdge::LEFT) {
                delta.x = -delta.x;
            }

            new_window_width = (self.initial_rect.size.w as f64 + delta.x) as i32;
        }

        if self.edges.intersects(ResizeEdge::TOP | ResizeEdge::BOTTOM) {
            if self.edges.intersects(ResizeEdge::TOP) {
                delta.y = -delta.y;
            }

            new_window_height = (self.initial_rect.size.h as f64 + delta.y) as i32;
        }

        let (min_size, max_size) = match self.window.toplevel() {
            Some(toplevel) => compositor::with_states(toplevel.wl_surface(), |states| {
                let mut guard = states.cached_state.get::<SurfaceCachedState>();
                let data = guard.current();
                (data.min_size, data.max_size)
            }),
            // X11 windows: no cached xdg min/max; clamp to >= 1 below.
            None => (Size::default(), Size::default()),
        };

        let min_width = min_size.w.max(1);
        let min_height = min_size.h.max(1);

        let max_width = if max_size.w == 0 {
            i32::MAX
        } else {
            max_size.w
        };
        let max_height = if max_size.h == 0 {
            i32::MAX
        } else {
            max_size.h
        };

        self.last_window_size = Size::from((
            new_window_width.max(min_width).min(max_width),
            new_window_height.max(min_height).min(max_height),
        ));

        if self.window.toplevel().is_some() {
            if self
                .configures
                .lock()
                .unwrap()
                .request(self.last_window_size)
            {
                let configures = self.configures.clone();
                let window = self.window.clone();
                data.handle.insert_idle(move |data| {
                    let Some(size) = configures.lock().unwrap().take() else {
                        return;
                    };
                    let Some(xdg) = window.toplevel() else { return };
                    xdg.with_pending_state(|state| {
                        state.states.set(xdg_toplevel::State::Resizing);
                        state.size = Some(size);
                    });
                    xdg.send_pending_configure();
                    data.schedule_redraw();
                });
            }
        } else if let Some(x11) = self.window.x11_surface() {
            let loc = data
                .space
                .element_location(&self.window)
                .unwrap_or(self.initial_rect.loc);
            let _ = x11.configure(Rectangle::new(loc, self.last_window_size));
        }
    }

    fn button(
        &mut self,
        data: &mut MindeState,
        handle: &mut PointerInnerHandle<'_, MindeState>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);

        // Release once no buttons are held (client-side resizes start on
        // left, super+drag resize on right).
        if handle.current_pressed().is_empty() {
            self.released = true;
            handle.unset_grab(self, data, event.serial, event.time, true);

            let final_serial = self.window.toplevel().map(|xdg| {
                xdg.with_pending_state(|state| {
                    state.states.unset(xdg_toplevel::State::Resizing);
                    state.size = Some(self.last_window_size);
                });

                // Always send a final configure, even without motion, so the
                // commit router has an unambiguous end-of-resize serial.
                let serial = xdg.send_configure();

                ResizeSurfaceState::with(xdg.wl_surface(), |state| {
                    *state = ResizeSurfaceState::WaitingForLastCommit {
                        edges: self.edges,
                        initial_rect: self.initial_rect,
                        serial,
                    };
                });
                serial
            });

            let loc = data
                .space
                .element_location(&self.window)
                .unwrap_or(self.initial_rect.loc);
            let loc = resize_location(self.edges, self.initial_rect, loc, self.last_window_size);
            self.report_geometry(
                data,
                Rectangle::new(loc, self.last_window_size),
                final_serial,
            );
        }
    }

    crate::grabs::forward_pointer_events!();

    fn start_data(&self) -> &PointerGrabStartData<MindeState> {
        &self.start_data
    }

    fn unset(&mut self, data: &mut MindeState) {
        self.configures.lock().unwrap().cancel();
        if self.released {
            return;
        }
        if let Some(xdg) = self.window.toplevel() {
            // Cancellation keeps the last committed geometry and drops the
            // old anchor, including when the live surface is being unmapped.
            ResizeSurfaceState::with(xdg.wl_surface(), |state| {
                *state = ResizeSurfaceState::Idle;
            });
            xdg.with_pending_state(|state| state.states.unset(xdg_toplevel::State::Resizing));
            if xdg.alive()
                && xdg.is_initial_configure_sent()
                && let Some(rect) = data.space.element_geometry(&self.window)
            {
                xdg.with_pending_state(|state| state.size = Some(rect.size));
                xdg.send_pending_configure();
                self.report_geometry(data, rect, None);
            }
        }
    }
}

/// State of the resize operation.
///
/// It is stored inside of WlSurface,
/// and can be accessed using [`ResizeSurfaceState::with`]
#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
enum ResizeSurfaceState {
    #[default]
    Idle,
    Resizing {
        edges: ResizeEdge,
        /// The initial window size and location.
        initial_rect: Rectangle<i32, Logical>,
    },
    /// Resize is done, we are now waiting for last commit, to do the final move
    WaitingForLastCommit {
        edges: ResizeEdge,
        /// The initial window size and location.
        initial_rect: Rectangle<i32, Logical>,
        /// Final configure; older acknowledgements cannot finish the resize.
        serial: Serial,
    },
}

impl ResizeSurfaceState {
    fn with<F, T>(surface: &WlSurface, cb: F) -> T
    where
        F: FnOnce(&mut Self) -> T,
    {
        compositor::with_states(surface, |states| {
            states.data_map.insert_if_missing(RefCell::<Self>::default);
            let state = states.data_map.get::<RefCell<Self>>().unwrap();

            cb(&mut state.borrow_mut())
        })
    }

    fn commit(
        &mut self,
        committed: Option<Serial>,
    ) -> Option<(ResizeEdge, Rectangle<i32, Logical>, bool)> {
        let Some(committed) = committed else {
            *self = Self::Idle;
            return None;
        };
        match *self {
            Self::Resizing {
                edges,
                initial_rect,
            } => Some((edges, initial_rect, false)),
            Self::WaitingForLastCommit {
                edges,
                initial_rect,
                serial,
            } => {
                let finished = committed.is_no_older_than(&serial);
                if finished {
                    *self = Self::Idle;
                }
                Some((edges, initial_rect, finished))
            }
            Self::Idle => None,
        }
    }
}

/// Should be called on `WlSurface::commit`
pub fn handle_commit(data: &mut MindeState, surface: &WlSurface) -> Option<()> {
    // Cheap per-surface state check (no `Space` scan) first: almost every
    // commit is not part of an active resize, so bail before paying for
    // `window_for_surface`'s linear walk over `space.elements()` (3.9).
    if ResizeSurfaceState::with(surface, |state| *state == ResizeSurfaceState::Idle) {
        return None;
    }
    let Some(window) = crate::state::window_for_surface(&data.space, surface) else {
        ResizeSurfaceState::with(surface, |state| *state = ResizeSurfaceState::Idle);
        return None;
    };
    let committed = window
        .toplevel()?
        .with_cached_state(|state| state.last_acked.as_ref().map(|configure| configure.serial));
    let (edges, initial_rect, finished) =
        ResizeSurfaceState::with(surface, |state| state.commit(committed))?;
    let loc = data.space.element_location(&window)?;
    let size = window.geometry().size;
    let new_loc = resize_location(edges, initial_rect, loc, size);
    if new_loc != loc {
        data.space.map_element(window.clone(), new_loc, false);
    }
    if finished && let Some(id) = data.id_for_window(&window) {
        // The client may choose a different size (e.g. terminal increments).
        // The acknowledged final commit, rather than a size comparison, ends
        // the resize and supplies the authoritative floating rectangle.
        crate::guile::on_window_moved(id, new_loc.x, new_loc.y, size.w, size.h);
    }

    Some(())
}

fn resize_location(
    edges: ResizeEdge,
    initial: Rectangle<i32, Logical>,
    mut current: Point<i32, Logical>,
    size: Size<i32, Logical>,
) -> Point<i32, Logical> {
    let anchored = |origin: i32, old: i32, new: i32| {
        (i64::from(origin) + i64::from(old) - i64::from(new))
            .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    };
    if edges.contains(ResizeEdge::LEFT) {
        current.x = anchored(initial.loc.x, initial.size.w, size.w);
    }
    if edges.contains(ResizeEdge::TOP) {
        current.y = anchored(initial.loc.y, initial.size.h, size.h);
    }
    current
}

#[cfg(test)]
mod tests {
    use super::{ResizeConfigureQueue, ResizeEdge, ResizeSurfaceState, resize_location};
    use smithay::utils::Rectangle;

    #[test]
    fn final_resize_commit_tracks_configure_serial_including_wrap() {
        let initial_rect = Rectangle::new((100, 100).into(), (300, 250).into());
        for (final_serial, older, accepted) in [(20, 19, 20), (20, 19, 21), (1, u32::MAX, 2)] {
            let mut state = ResizeSurfaceState::WaitingForLastCommit {
                edges: ResizeEdge::TOP_LEFT,
                initial_rect,
                serial: final_serial.into(),
            };
            for _ in 0..2 {
                assert_eq!(
                    state.commit(Some(older.into())),
                    Some((ResizeEdge::TOP_LEFT, initial_rect, false))
                );
            }
            assert_eq!(
                state.commit(Some(accepted.into())),
                Some((ResizeEdge::TOP_LEFT, initial_rect, true))
            );
            assert_eq!(state, ResizeSurfaceState::Idle);
            assert_eq!(state.commit(Some(accepted.into())), None);
        }
    }

    #[test]
    fn unmapping_discards_a_pending_resize_commit() {
        let mut state = ResizeSurfaceState::WaitingForLastCommit {
            edges: ResizeEdge::LEFT,
            initial_rect: Rectangle::new((100, 100).into(), (300, 250).into()),
            serial: 20.into(),
        };
        assert_eq!(state.commit(None), None);
        assert_eq!(state, ResizeSurfaceState::Idle);
    }

    #[test]
    fn resize_anchor_preserves_opposite_edges_and_bounds_large_coordinates() {
        let initial = Rectangle::new((100, 100).into(), (300, 250).into());
        let current = (160, 120).into();
        let size = (260, 220).into();
        assert_eq!(
            resize_location(ResizeEdge::LEFT, initial, current, size),
            (140, 120).into()
        );
        assert_eq!(
            resize_location(ResizeEdge::TOP, initial, current, size),
            (160, 130).into()
        );
        assert_eq!(
            resize_location(ResizeEdge::TOP_LEFT, initial, current, size),
            (140, 130).into()
        );
        assert_eq!(
            resize_location(ResizeEdge::BOTTOM_RIGHT, initial, current, size),
            current
        );
        let huge = Rectangle::new((i32::MAX, i32::MIN).into(), (i32::MAX, 1).into());
        assert_eq!(
            resize_location(ResizeEdge::TOP_LEFT, huge, current, (1, i32::MAX).into()),
            (i32::MAX, i32::MIN).into()
        );
    }

    #[test]
    fn resize_burst_sends_only_latest_size() {
        let mut queue = ResizeConfigureQueue::new((640, 480).into());
        let mut callbacks = 0;
        for width in 641..=1640 {
            callbacks += usize::from(queue.request((width, 480).into()));
        }
        assert_eq!(callbacks, 1);
        assert_eq!(queue.take(), Some((1640, 480).into()));
        assert_eq!(queue.take(), None);
        assert!(!queue.request((1640, 480).into()));
        assert!(queue.request((1641, 480).into()));
        assert_eq!(queue.take(), Some((1641, 480).into()));
    }

    #[test]
    fn resize_return_to_sent_size_needs_no_configure() {
        let mut queue = ResizeConfigureQueue::new((640, 480).into());
        assert!(queue.request((641, 480).into()));
        assert!(!queue.request((640, 480).into()));
        assert_eq!(queue.take(), None);
    }

    #[test]
    fn resize_release_or_replacement_cancels_stale_callback() {
        let mut queue = ResizeConfigureQueue::new((640, 480).into());
        assert!(queue.request((800, 600).into()));
        queue.cancel();
        assert_eq!(queue.take(), None);
    }
}
