//! Applying Otto's `input.*` configuration to libinput devices.
//!
//! Split out of device set-up because the same code has to run in three
//! places: for the devices libinput already knows about at startup, for a
//! device plugged in later, and again whenever a setting changes on the bus.
//! All three read the live configuration, so a keyboard-and-touchpad combo
//! plugged in after a change gets the changed value rather than the one Otto
//! booted with.
//!
//! Every setter is best-effort per device: libinput refuses what the hardware
//! cannot do, and one mouse that has no tap support must not make a `Set` fail
//! for the touchpad next to it.

use smithay::reexports::input::{AccelProfile, ClickMethod, Device, DeviceCapability};

use crate::config::{InputConfig, PointerAccelProfile, TouchpadClickMethod};

/// libinput's click method for a configured one.
pub fn click_method(method: TouchpadClickMethod) -> ClickMethod {
    match method {
        TouchpadClickMethod::Clickfinger => ClickMethod::Clickfinger,
        TouchpadClickMethod::ButtonAreas => ClickMethod::ButtonAreas,
    }
}

/// libinput's acceleration profile for a configured one.
pub fn accel_profile(profile: PointerAccelProfile) -> AccelProfile {
    match profile {
        PointerAccelProfile::Flat => AccelProfile::Flat,
        PointerAccelProfile::Adaptive => AccelProfile::Adaptive,
    }
}

/// Whether `device` takes the touchpad half of the configuration.
///
/// libinput reports a tap finger count only for devices with touch support, so
/// it doubles as the touchpad test: a plain mouse answers 0 and keeps its own
/// button behaviour instead of being handed touchpad settings.
pub fn is_touchpad(device: &Device) -> bool {
    device.has_capability(DeviceCapability::Pointer) && device.config_tap_finger_count() > 0
}

/// Bring one device in line with `input`.
///
/// Silently skips whatever this device does not support — libinput's
/// `*_is_available` predicates first, then the setter's own error — so a
/// device is configured as far as it can be and no further.
pub fn apply_device_config(device: &mut Device, input: &InputConfig) {
    if !device.has_capability(DeviceCapability::Pointer) {
        return;
    }

    if is_touchpad(device) {
        let _ = device.config_tap_set_enabled(input.tap_enabled);
        let _ = device.config_tap_set_drag_enabled(input.tap_drag_enabled);
        let _ = device.config_tap_set_drag_lock_enabled(input.tap_drag_lock_enabled);

        if device
            .config_click_methods()
            .contains(&click_method(input.touchpad_click_method))
        {
            let _ = device.config_click_set_method(click_method(input.touchpad_click_method));
        }
        if device.config_dwt_is_available() {
            let _ = device.config_dwt_set_enabled(input.touchpad_dwt_enabled);
        }
        if device.config_scroll_has_natural_scroll() {
            let _ = device
                .config_scroll_set_natural_scroll_enabled(input.touchpad_natural_scroll_enabled);
        }
        if device.config_left_handed_is_available() {
            let _ = device.config_left_handed_set(input.touchpad_left_handed);
        }
        if device.config_middle_emulation_is_available() {
            let _ =
                device.config_middle_emulation_set_enabled(input.touchpad_middle_emulation_enabled);
        }

        tracing::debug!(
            device = device.name(),
            tap = input.tap_enabled,
            drag = input.tap_drag_enabled,
            natural_scroll = input.touchpad_natural_scroll_enabled,
            "Configured touchpad"
        );
    }

    // Acceleration is the one part that belongs to every pointer, mice
    // included.
    if device.config_accel_is_available() {
        let profile = accel_profile(input.pointer_accel_profile);
        if device.config_accel_profiles().contains(&profile) {
            let _ = device.config_accel_set_profile(profile);
        }
        let _ = device.config_accel_set_speed(input.pointer_accel_speed);
        tracing::debug!(
            device = device.name(),
            speed = input.pointer_accel_speed,
            profile = ?input.pointer_accel_profile,
            "Configured pointer acceleration"
        );
    }
}

/// Re-apply `input` to every device Otto currently holds.
pub fn apply_to_all(devices: &mut [Device], input: &InputConfig) {
    for device in devices.iter_mut() {
        apply_device_config(device, input);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_methods_map_onto_libinput() {
        assert_eq!(
            click_method(TouchpadClickMethod::Clickfinger),
            ClickMethod::Clickfinger
        );
        assert_eq!(
            click_method(TouchpadClickMethod::ButtonAreas),
            ClickMethod::ButtonAreas
        );
    }

    #[test]
    fn accel_profiles_map_onto_libinput() {
        assert_eq!(accel_profile(PointerAccelProfile::Flat), AccelProfile::Flat);
        assert_eq!(
            accel_profile(PointerAccelProfile::Adaptive),
            AccelProfile::Adaptive
        );
    }
}
