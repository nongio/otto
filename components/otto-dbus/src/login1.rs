//! `org.freedesktop.login1.Manager`: logind's power actions.
//!
//! Served by systemd-logind on the system bus. Whether the caller may act is
//! polkit's call; `interactive` lets polkit ask for a password where its
//! policy says to.

/// The well-known bus name.
pub const SERVICE: &str = "org.freedesktop.login1";
/// The object path.
pub const PATH: &str = "/org/freedesktop/login1";

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
pub trait Login1Manager {
    /// Shut the machine down.
    fn power_off(&self, interactive: bool) -> zbus::Result<()>;

    /// Restart the machine.
    fn reboot(&self, interactive: bool) -> zbus::Result<()>;

    /// Suspend to RAM. Answers once the machine is on its way down.
    fn suspend(&self, interactive: bool) -> zbus::Result<()>;
}
