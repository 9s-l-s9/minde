// SPDX-License-Identifier: MIT

//! libinput device configuration for the udev backend: the capability
//! names reported to Scheme, the `(wm-input-devices)` registry, and the
//! application of `define-input-rules!` settings to each device. Split
//! from `udev.rs`, which keeps the DRM device lifecycle and frame
//! scheduling.

use crate::{MindeState, guile};
use smithay::reexports::input::{
    AccelProfile, ClickMethod, Device as LibinputDevice, DeviceCapability, DeviceConfigError,
};
use tracing::{info, warn};

/// The libinput capabilities a device exposes, as the names reported to
/// Scheme through `(wm-input-devices)`.
fn device_capabilities(device: &LibinputDevice) -> Vec<String> {
    [
        (DeviceCapability::Keyboard, "keyboard"),
        (DeviceCapability::Pointer, "pointer"),
        (DeviceCapability::Touch, "touch"),
        (DeviceCapability::TabletTool, "tablet-tool"),
        (DeviceCapability::TabletPad, "tablet-pad"),
        (DeviceCapability::Gesture, "gesture"),
        (DeviceCapability::Switch, "switch"),
    ]
    .into_iter()
    .filter(|(cap, _)| device.has_capability(*cap))
    .map(|(_, name)| name.to_string())
    .collect()
}

/// Logs the outcome of one libinput config setter. Devices that do not
/// support a setting return an error status; that is expected and only
/// logged, never fatal (the roadmap's "ignored gracefully").
fn log_config(device: &str, setting: &str, result: Result<(), DeviceConfigError>) {
    match result {
        Ok(()) => info!(device, setting, "applied libinput setting"),
        Err(err) => info!(
            device,
            setting,
            ?err,
            "libinput setting not applied (unsupported by device)"
        ),
    }
}

/// Applies every rule matching `device` (in registration order; later rules
/// win). A rule with an empty match string applies to all devices.
fn apply_input_rules(device: &mut LibinputDevice, rules: &[guile::InputRule]) {
    let name = device.name().to_string();
    for rule in rules {
        if rule.match_name.is_empty() || name.contains(&rule.match_name) {
            apply_one_rule(device, &name, rule);
        }
    }
}

fn apply_one_rule(device: &mut LibinputDevice, name: &str, rule: &guile::InputRule) {
    if let Some(on) = rule.tap {
        log_config(name, "tap-to-click", device.config_tap_set_enabled(on));
    }
    if let Some(on) = rule.natural_scroll {
        log_config(
            name,
            "natural-scroll",
            device.config_scroll_set_natural_scroll_enabled(on),
        );
    }
    if let Some(profile) = &rule.accel_profile {
        match profile.as_str() {
            "flat" => log_config(
                name,
                "accel-profile",
                device.config_accel_set_profile(AccelProfile::Flat),
            ),
            "adaptive" => log_config(
                name,
                "accel-profile",
                device.config_accel_set_profile(AccelProfile::Adaptive),
            ),
            other => warn!(other, "wm-configure-input!: unknown accel-profile"),
        }
    }
    if let Some(method) = &rule.click_method {
        match method.as_str() {
            "button-areas" => log_config(
                name,
                "click-method",
                device.config_click_set_method(ClickMethod::ButtonAreas),
            ),
            "clickfinger" => log_config(
                name,
                "click-method",
                device.config_click_set_method(ClickMethod::Clickfinger),
            ),
            other => warn!(other, "wm-configure-input!: unknown click-method"),
        }
    }
}

impl MindeState {
    /// A libinput device arrived (udev backend). Register it for
    /// `(wm-input-devices)`, apply any stored `wm-configure-input!` rules,
    /// retain it for later re-configuration, and fire the optional
    /// `(handle-input-device-added!)` Scheme hook.
    pub(crate) fn libinput_device_added(&mut self, mut device: LibinputDevice) {
        let name = device.name().to_string();
        let capabilities = device_capabilities(&device);
        info!(device = %name, ?capabilities, "input device added");
        apply_input_rules(&mut device, &guile::input_rules());
        guile::register_input_device(name.clone(), capabilities.clone());
        // Tablet devices are advertised via zwp_tablet_manager_v2 so a stylus
        // (e.g. a laptop's built-in pencil) reaches tablet-aware clients. The
        // tool devices themselves are added lazily on first proximity (see
        // `process_input_event`). `add_tablet` is idempotent per descriptor.
        if device.has_capability(DeviceCapability::TabletTool) {
            use smithay::wayland::tablet_manager::TabletSeatTrait;
            let desc = smithay::wayland::tablet_manager::TabletDescriptor::from(&device);
            self.seat
                .tablet_seat()
                .add_tablet::<Self>(&self.display_handle, &desc);
        }
        if let Some(udev) = self.udev_data.as_mut() {
            udev.input_devices.push(device);
        }
        guile::on_input_device_added(&name, &capabilities);
    }

    /// A libinput device was removed (udev backend): drop it from the
    /// `(wm-input-devices)` registry and the retained set.
    pub(crate) fn libinput_device_removed(&mut self, device: &LibinputDevice) {
        let name = device.name().to_string();
        info!(device = %name, "input device removed");
        guile::unregister_input_device(&name);
        // Drop the tablet from the seat so clients stop seeing it. Tools added
        // on proximity are left registered (a tool may outlive one tablet and
        // is cheap to keep; Smithay documents tool-removal as compositor policy).
        if device.has_capability(DeviceCapability::TabletTool) {
            use smithay::wayland::tablet_manager::TabletSeatTrait;
            let desc = smithay::wayland::tablet_manager::TabletDescriptor::from(device);
            self.seat.tablet_seat().remove_tablet(&desc);
        }
        if let Some(udev) = self.udev_data.as_mut() {
            udev.input_devices.retain(|d| d != device);
        }
    }

    /// Re-applies the stored input rules to every device already present.
    /// Driven by `WmCommand::ReapplyInputConfig` when `wm-configure-input!`
    /// is called at runtime. No-op under winit (no `udev_data`).
    pub(crate) fn reapply_input_config(&mut self) {
        let rules = guile::input_rules();
        let Some(udev) = self.udev_data.as_mut() else {
            return;
        };
        for device in &mut udev.input_devices {
            apply_input_rules(device, &rules);
        }
    }
}
