//! `org.freedesktop.login1.Manager`: logind's power and session actions.
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

    /// Hibernate: write memory to swap and power off. Answers once the
    /// machine is on its way down.
    fn hibernate(&self, interactive: bool) -> zbus::Result<()>;

    /// Whether hibernating would work for this caller: `"yes"`,
    /// `"challenge"` (polkit would ask), `"no"` or `"na"` (no swap or resume
    /// device to hibernate to).
    fn can_hibernate(&self) -> zbus::Result<String>;

    /// Ask the session `session_id` to lock: logind emits `Lock` on it, and
    /// whoever runs the session answers. `"auto"` is the caller's own
    /// session, or the user's graphical one when the caller is in none.
    fn lock_session(&self, session_id: &str) -> zbus::Result<()>;

    /// Sent with `true` as the machine is about to sleep, and with `false`
    /// once it has woken.
    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}
