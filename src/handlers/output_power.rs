// SPDX-License-Identifier: MIT

//! `wlr-output-power-management-unstable-v1`: lets `wlopm`, `swayidle`
//! (`timeout N 'wlopm --off *' resume 'wlopm --on *'`) and lock screens
//! switch outputs to DPMS off and back without touching the layout.
//!
//! Hand-written on the raw `wayland-protocols-wlr` bindings like
//! `output_management` (Smithay's vendored revision has no server module
//! for it, and the blanket `delegate_dispatch2!` in handlers/mod.rs covers
//! only Smithay's own user-data types).
//!
//! ## Semantics
//!
//! Power is orthogonal to *enabled*: a powered-off head keeps its space
//! mapping, its `wl_output` global and its windows, and
//! wlr-output-management keeps reporting it as enabled -- only the
//! backend stops driving it (udev: the CRTC is switched off by dropping
//! the head's render surface; winit: the window paints black and no frame
//! callbacks are sent). Powering on re-initialises the backend surface and
//! repaints. Turning off a *disabled* head fails (`failed()`), as does a
//! second controller for an output that already has a live one (wlroots
//! semantics: one controller per output).
//!
//! Any keyboard press, pointer button or pointer motion powers every head
//! back on while [`MindeState::wake_on_input`] is set (the default) and
//! tells every controller `mode(on)`, so a screen blanked by an idle
//! daemon wakes even if the daemon's `resume` command is slow or missing.

use smithay::output::Output;
use smithay::reexports::{
    wayland_protocols_wlr::output_power_management::v1::server::{
        zwlr_output_power_manager_v1::{self, ZwlrOutputPowerManagerV1},
        zwlr_output_power_v1::{self, Mode as PowerMode, ZwlrOutputPowerV1},
    },
    wayland_server::{
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        backend::{ClientId, GlobalId},
    },
};

use crate::MindeState;

/// `wlr-output-power-management` global state, held in
/// `MindeState::output_power`.
pub struct OutputPowerState {
    global: GlobalId,
    /// Live controls and the output each one drives.
    controls: Vec<(ZwlrOutputPowerV1, Output)>,
}

impl OutputPowerState {
    /// [`GlobalId`] getter (parity with the Smithay-provided states).
    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }
}

/// Creates the manager global (version 1) and returns the state.
pub fn init_output_power(dh: &DisplayHandle) -> OutputPowerState {
    let global = dh.create_global::<MindeState, ZwlrOutputPowerManagerV1, ()>(1, ());
    OutputPowerState {
        global,
        controls: Vec::new(),
    }
}

/// User data on a `ZwlrOutputPowerV1`: the output it controls (`None`
/// when the request named an already-gone `wl_output`; such a control
/// only ever receives `failed`).
pub struct PowerControlData {
    output: Option<Output>,
}

fn power_mode(on: bool) -> PowerMode {
    if on { PowerMode::On } else { PowerMode::Off }
}

impl MindeState {
    /// Sends `mode` to every live control of `output`.
    pub fn output_power_notify(&self, output: &Output, on: bool) {
        for (control, o) in &self.output_power.controls {
            if o == output {
                control.mode(power_mode(on));
            }
        }
    }

    /// Sends `failed` to every control of `output` and forgets them. Call
    /// when the output goes away (connector unplugged / window closed).
    pub fn output_power_output_removed(&mut self, output: &Output) {
        let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.output_power.controls)
            .into_iter()
            .partition(|(_, o)| o == output);
        self.output_power.controls = kept;
        for (control, _) in gone {
            control.failed();
        }
    }

    /// Powers every head back on and tells the controllers. Used by the
    /// wake-on-input path; harmless when nothing is off.
    pub(crate) fn output_power_wake(&mut self) {
        let outputs: Vec<Output> = self.powered_off_outputs();
        if outputs.is_empty() {
            return;
        }
        tracing::info!("input activity: powering outputs back on");
        self.power_all_outputs_on();
        for output in outputs {
            if self.output_power_is_on(&output) {
                self.output_power_notify(&output, true);
            }
        }
    }
}

// zwlr_output_power_manager_v1

impl GlobalDispatch<ZwlrOutputPowerManagerV1, ()> for MindeState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrOutputPowerManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwlrOutputPowerManagerV1, ()> for MindeState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _manager: &ZwlrOutputPowerManagerV1,
        request: zwlr_output_power_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_output_power_manager_v1::Request;
        match request {
            Request::GetOutputPower { id, output } => {
                let output = Output::from_resource(&output);
                let known = output
                    .as_ref()
                    .is_some_and(|o| state.output_power_is_known(o));
                let taken = output.as_ref().is_some_and(|o| {
                    state
                        .output_power
                        .controls
                        .iter()
                        .any(|(c, existing)| existing == o && c.is_alive())
                });
                let control = data_init.init(
                    id,
                    PowerControlData {
                        output: output.clone(),
                    },
                );
                let Some(output) = output.filter(|_| known && !taken) else {
                    // Output gone, unknown to a backend, or already
                    // controlled by another client: this control is dead
                    // on arrival.
                    control.failed();
                    return;
                };
                let on = state.output_power_is_on(&output);
                control.mode(power_mode(on));
                state.output_power.controls.push((control, output));
            }
            Request::Destroy => {}
            _ => {}
        }
    }
}

// zwlr_output_power_v1

impl Dispatch<ZwlrOutputPowerV1, PowerControlData> for MindeState {
    fn request(
        state: &mut Self,
        _client: &Client,
        control: &ZwlrOutputPowerV1,
        request: zwlr_output_power_v1::Request,
        data: &PowerControlData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_output_power_v1::Request;
        match request {
            Request::SetMode { mode } => {
                let on = match mode.into_result() {
                    Ok(PowerMode::On) => true,
                    Ok(PowerMode::Off) => false,
                    _ => {
                        control.post_error(
                            zwlr_output_power_v1::Error::InvalidMode,
                            "nonexistent power save mode",
                        );
                        return;
                    }
                };
                // Only a registered (live, non-failed) control may drive
                // its output.
                let registered = state
                    .output_power
                    .controls
                    .iter()
                    .any(|(c, _)| c == control);
                let Some(output) = data.output.clone().filter(|_| registered) else {
                    control.failed();
                    return;
                };
                match state.set_output_power(&output, on) {
                    Ok(()) => state.output_power_notify(&output, on),
                    Err(err) => {
                        tracing::warn!(output = %output.name(), %err, "output power change refused");
                        control.failed();
                        state.output_power.controls.retain(|(c, _)| c != control);
                    }
                }
            }
            Request::Destroy => {
                state.output_power.controls.retain(|(c, _)| c != control);
            }
            _ => {}
        }
    }

    fn destroyed(
        state: &mut Self,
        _client: ClientId,
        control: &ZwlrOutputPowerV1,
        _data: &PowerControlData,
    ) {
        state.output_power.controls.retain(|(c, _)| c != control);
    }
}
