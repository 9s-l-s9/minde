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
//! ioctl, or queues them while the output is inaccessible. Output
//! reinitialization reapplies them; destroy/client-death restores the snapshot.
//! Any condition we can't honour (no CRTC, gamma-less CRTC, a second control
//! on one output, a failed ioctl) degrades to the protocol's `failed` event
//! rather than a hard error, matching wlroots.

use smithay::output::Output;
use smithay::reexports::{
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

use crate::MindeState;

mod state;
pub use state::GammaControls;

/// The resource's output key is not proof of ownership. Failed and retired
/// resources keep their data, but every mutation checks the current owner.
pub struct GammaControlUserData {
    output: Option<Output>,
}

impl MindeState {
    pub fn gamma_output_removed(&mut self, output: &Output) {
        self.gamma_controls.output_removed(output);
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

        if self.gamma_controls.has_owner(&output) {
            control.failed();
            return;
        }
        let Some((fd, crtc, size)) = self.gamma_info_for_output(&output) else {
            control.failed();
            return;
        };
        self.gamma_controls
            .create_drm(output, control, fd, crtc, size);
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
                let writable = state.gamma_output_writable(output);
                if let Err(message) = state
                    .gamma_controls
                    .set(output, control, fd.into(), writable)
                {
                    control.post_error(zwlr_gamma_control_v1::Error::InvalidGamma, message);
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
        control: &ZwlrGammaControlV1,
        data: &GammaControlUserData,
    ) {
        if let Some(output) = data.output.as_ref() {
            let writable = state.gamma_output_writable(output);
            state.gamma_controls.finish(output, control, writable);
        }
    }
}

/// Creates the manager global and returns its id. Called only by the udev
/// backend; the winit backend deliberately never advertises gamma control.
pub fn init_gamma_control_manager(dh: &DisplayHandle) -> GlobalId {
    dh.create_global::<MindeState, ZwlrGammaControlManagerV1, ()>(1, ())
}
