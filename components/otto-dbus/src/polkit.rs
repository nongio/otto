//! `org.freedesktop.PolicyKit1.Authority`: asking polkit whether an action
//! is allowed.
//!
//! Served by polkitd on the system bus. With [`ALLOW_USER_INTERACTION`] the
//! call waits while the session's authentication agent (Otto's
//! `otto-authorize --polkit-agent`) asks the person, for as long as they
//! take. Otto's actions are declared in `resources/polkit/`.

// Rust guideline compliant 2026-02-21

use std::collections::HashMap;

use zbus::zvariant::Value;

/// The well-known bus name.
pub const SERVICE: &str = "org.freedesktop.PolicyKit1";
/// The object path.
pub const PATH: &str = "/org/freedesktop/PolicyKit1/Authority";

/// `CheckAuthorization`'s flag letting polkit ask the person through the
/// session's authentication agent.
pub const ALLOW_USER_INTERACTION: u32 = 1;

/// Who an authorization is for: a kind (`unix-process`) and its fields.
pub type Subject = (&'static str, HashMap<&'static str, Value<'static>>);

/// `CheckAuthorization`'s answer: whether the action is authorized, whether
/// it would be after a challenge, and polkit's details.
pub type WireAuthorization = (bool, bool, HashMap<String, String>);

/// This process as a polkit `unix-process` subject: its pid and its start
/// time, which together name it even after the pid is reused.
///
/// A start time that cannot be read is sent as 0, which polkitd fills in
/// itself.
#[must_use]
pub fn this_process() -> Subject {
    let fields = HashMap::from([
        ("pid", Value::from(std::process::id())),
        ("start-time", Value::from(start_time().unwrap_or(0))),
    ]);
    ("unix-process", fields)
}

/// This process's start time in clock ticks since boot, the 22nd field of
/// `/proc/self/stat`.
fn start_time() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // The command name is in parentheses and may hold spaces; count from
    // after the last one.
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(19)?.parse().ok()
}

#[zbus::proxy(
    interface = "org.freedesktop.PolicyKit1.Authority",
    default_service = "org.freedesktop.PolicyKit1",
    default_path = "/org/freedesktop/PolicyKit1/Authority"
)]
pub trait Authority {
    /// Whether `subject` may perform `action_id`. `details` go to the agent
    /// and are accepted only from root or the action's owner; `flags` is
    /// [`ALLOW_USER_INTERACTION`] or 0. An action polkit does not know is an
    /// error, not a refusal.
    fn check_authorization(
        &self,
        subject: &Subject,
        action_id: &str,
        details: HashMap<&str, &str>,
        flags: u32,
        cancellation_id: &str,
    ) -> zbus::Result<WireAuthorization>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_has_a_start_time() {
        assert!(start_time().is_some_and(|ticks| ticks > 0));
    }
}
