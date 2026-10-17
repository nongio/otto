//! `org.otto.Island1`: activities on the island and the dock.
//!
//! Served by otto-islands (`components/otto-islands/src/dbus_service.rs`).

/// The well-known bus name.
pub const SERVICE: &str = "org.otto.Island";
/// The object path.
pub const PATH: &str = "/org/otto/Island";

#[zbus::proxy(
    interface = "org.otto.Island1",
    default_service = "org.otto.Island",
    default_path = "/org/otto/Island"
)]
pub trait Island {
    /// Create an activity; answers its id. `progress` is `0.0..=1.0`, or
    /// negative for none; `timeout_ms` `0` for none; `priority` one of `low`,
    /// `normal`, `high`, `critical`. A `quiet` activity fills the dock icon
    /// without taking the island.
    #[allow(clippy::too_many_arguments)]
    fn create_activity(
        &self,
        app_id: &str,
        title: &str,
        icon: &str,
        progress: f64,
        timeout_ms: u32,
        priority: &str,
        live: bool,
        quiet: bool,
    ) -> zbus::Result<u64>;

    /// Update an activity. An empty `title` leaves it; a negative `progress`
    /// clears it. Whether the activity exists.
    fn update_activity(&self, id: u64, title: &str, progress: f64) -> zbus::Result<bool>;

    /// Show or hide an activity's island without ending it.
    fn set_activity_quiet(&self, id: u64, quiet: bool) -> zbus::Result<bool>;

    /// End an activity.
    fn dismiss_activity(&self, id: u64) -> zbus::Result<bool>;
}
