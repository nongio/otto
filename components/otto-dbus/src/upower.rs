//! `org.freedesktop.UPower.Device`: a power source as UPower reports it.
//!
//! Served by UPower on the system bus. The default path is the aggregate
//! `DisplayDevice`: one battery's worth of numbers however many cells the
//! machine has, and the only device present on a desktop.

/// The well-known bus name.
pub const SERVICE: &str = "org.freedesktop.UPower";
/// The aggregate device's object path.
pub const PATH: &str = "/org/freedesktop/UPower/devices/DisplayDevice";

#[zbus::proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower/devices/DisplayDevice"
)]
pub trait UPowerDevice {
    /// The charge, `0.0..=100.0`.
    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<f64>;

    /// `1` charging, `2` discharging, `3` empty, `4` fully charged, `5` and
    /// `6` pending charge and discharge; `0` unknown.
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;

    /// Seconds until empty while discharging; `0` when unknown.
    #[zbus(property)]
    fn time_to_empty(&self) -> zbus::Result<i64>;

    /// Seconds until full while charging; `0` when unknown.
    #[zbus(property)]
    fn time_to_full(&self) -> zbus::Result<i64>;

    /// Whether the device is there at all.
    #[zbus(property)]
    fn is_present(&self) -> zbus::Result<bool>;

    /// `2` is a battery; a desktop's `DisplayDevice` reports `0` (unknown).
    #[zbus(property, name = "Type")]
    fn device_type(&self) -> zbus::Result<u32>;
}
