// SPDX-License-Identifier: MIT

//! `wp_tearing_control_manager_v1` / `wp_tearing_control_v1` (v1): lets a
//! client hint that a surface tolerates tearing (async / immediate page
//! flips) instead of vsync, which games use to minimise latency.
//!
//! The vendored Smithay revision ships no server module for this staging
//! protocol, so the `GlobalDispatch`/`Dispatch` impls are hand-written here on
//! the `wayland-protocols` staging bindings, following the same pattern as
//! [`super::virtual_pointer`], [`super::gamma_control`] and
//! [`super::wlr_screencopy`]. The userdata types deliberately do not implement
//! `Dispatch2`, so the crate-wide `delegate_dispatch2!` blanket does not
//! collide with the explicit impls below.
//!
//! # Honesty: advisory only
//!
//! The global is advertised on **both** backends so tearing-aware clients (and
//! `wayland-info`) find the protocol and render identically whether or not the
//! compositor acts on the hint. It is **not** honoured: the `DrmCompositor` in
//! this vendored Smithay revision exposes no async/immediate page-flip path
//! (`FrameFlags` has no tearing bit and `DrmSurface` only ever flips with
//! `PageFlipFlags::EVENT`), so there is no realistic way to turn an `async`
//! hint into a real tearing flip. We record the latest hint per control object
//! (for logging/inspection) and stop there, rather than claim behaviour we do
//! not implement. The capability matrix marks this row accordingly.

use std::sync::Mutex;

use smithay::reexports::{
    wayland_protocols::wp::tearing_control::v1::server::{
        wp_tearing_control_manager_v1::{self, WpTearingControlManagerV1},
        wp_tearing_control_v1::{self, WpTearingControlV1},
    },
    wayland_server::{
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, WEnum, backend::GlobalId,
    },
};

use crate::state::MindeState;

/// Registers the `wp_tearing_control_manager_v1` global (v1). The returned id
/// is kept alive by the display; callers discard it like the other hand-rolled
/// globals.
pub fn init_tearing_control_manager(display: &DisplayHandle) -> GlobalId {
    display.create_global::<MindeState, WpTearingControlManagerV1, _>(1, ())
}

/// Userdata for a `wp_tearing_control_v1` object: the last presentation hint
/// the client requested. Kept purely for inspection/logging (see module docs);
/// nothing consumes it, because the DRM backend cannot honour it.
#[derive(Default)]
pub struct TearingControlData {
    async_requested: Mutex<bool>,
}

impl GlobalDispatch<WpTearingControlManagerV1, ()> for MindeState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<WpTearingControlManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<WpTearingControlManagerV1, ()> for MindeState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _manager: &WpTearingControlManagerV1,
        request: wp_tearing_control_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wp_tearing_control_manager_v1::Request::GetTearingControl { id, surface: _ } => {
                // Advisory only: we do not track per-surface controls or raise
                // the `tearing_control_exists` error, since the hint is never
                // acted upon. Well-behaved clients create exactly one per
                // surface.
                data_init.init(id, TearingControlData::default());
            }
            wp_tearing_control_manager_v1::Request::Destroy => {}
            _ => unreachable!(),
        }
    }
}

impl Dispatch<WpTearingControlV1, TearingControlData> for MindeState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _control: &WpTearingControlV1,
        request: wp_tearing_control_v1::Request,
        data: &TearingControlData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wp_tearing_control_v1::Request::SetPresentationHint { hint } => {
                let wants_async = matches!(
                    hint,
                    WEnum::Value(wp_tearing_control_v1::PresentationHint::Async)
                );
                *data.async_requested.lock().unwrap() = wants_async;
                tracing::debug!(
                    wants_async,
                    "wp_tearing_control: presentation hint recorded (advisory only; \
                     the DRM backend cannot perform async/tearing page flips)"
                );
            }
            wp_tearing_control_v1::Request::Destroy => {}
            _ => unreachable!(),
        }
    }
}
