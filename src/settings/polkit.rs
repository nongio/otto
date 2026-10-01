//! Asking polkit before a protected setting changes.
//!
//! The settings in [`super::schema::PROTECTED`] decide what runs in front of
//! the password, or whether the session locks at all. Any client on the
//! session bus can ask to change them, so the settings service first asks
//! polkit to authorize `org.otto.settings.lock` for Otto's own process. The
//! action is `auth_self`: polkitd asks the session's authentication agent,
//! which is Otto's own (`otto-authorize --polkit-agent`), and the agent shows
//! the auth panel naming the change from the details passed here. A script on
//! the bus can make the request; it cannot type the password.
//!
//! This proves someone at the session agreed. It does not stop a program
//! running as the user from editing `config.toml` directly.

use std::collections::HashMap;

use zbus::zvariant::Value;

/// The polkit action every protected setting asks for. Declared in
/// `resources/polkit/org.otto.settings.policy`.
pub const ACTION: &str = "org.otto.settings.lock";

/// Detail naming the setting, for the agent's reason line.
pub const DETAIL_SETTING: &str = "otto.setting";
/// Detail carrying the value asked for, as text; absent for a reset.
pub const DETAIL_VALUE: &str = "otto.value";
/// Detail with the setting's label, for a reason line that names it.
pub const DETAIL_LABEL: &str = "otto.label";

/// `CheckAuthorization`'s `AllowUserInteraction` flag.
const ALLOW_USER_INTERACTION: u32 = 1;

/// Why a protected change was not authorized.
#[derive(Debug)]
pub enum Refusal {
    /// The user cancelled, or did not prove who they are.
    NotAuthorized,
    /// polkit could not be asked: not installed, not running, or no agent.
    Unavailable(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::NotAuthorized => write!(f, "the change was not confirmed"),
            Refusal::Unavailable(reason) => write!(
                f,
                "polkit is not available ({reason}); change it in config.toml instead"
            ),
        }
    }
}

/// Ask polkit to authorize changing `setting` to `value` (`None` for a
/// reset). Waits for as long as the user takes to answer the panel.
pub async fn authorize(setting: &str, label: &str, value: Option<&str>) -> Result<(), Refusal> {
    let connection = zbus::Connection::system()
        .await
        .map_err(|err| Refusal::Unavailable(err.to_string()))?;

    let mut subject_fields: HashMap<&str, Value<'_>> = HashMap::new();
    subject_fields.insert("pid", Value::from(std::process::id()));
    subject_fields.insert("start-time", Value::from(start_time().unwrap_or(0)));
    let subject = ("unix-process", subject_fields);

    let mut details: HashMap<&str, &str> = HashMap::new();
    details.insert(DETAIL_SETTING, setting);
    details.insert(DETAIL_LABEL, label);
    if let Some(value) = value {
        details.insert(DETAIL_VALUE, value);
    }

    let reply = connection
        .call_method(
            Some("org.freedesktop.PolicyKit1"),
            "/org/freedesktop/PolicyKit1/Authority",
            Some("org.freedesktop.PolicyKit1.Authority"),
            "CheckAuthorization",
            &(subject, ACTION, details, ALLOW_USER_INTERACTION, ""),
        )
        .await
        .map_err(|err| Refusal::Unavailable(err.to_string()))?;
    let (authorized, _challenge, _details): (bool, bool, HashMap<String, String>) = reply
        .body()
        .deserialize()
        .map_err(|err| Refusal::Unavailable(err.to_string()))?;

    if authorized {
        tracing::info!(setting, "protected setting change confirmed");
        Ok(())
    } else {
        tracing::info!(setting, "protected setting change not confirmed");
        Err(Refusal::NotAuthorized)
    }
}

/// This process's start time in clock ticks since boot, the 22nd field of
/// `/proc/self/stat`, as polkit identifies a process by.
fn start_time() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // The command name is in parentheses and may hold spaces; count from
    // after the last one.
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(19)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_has_a_start_time() {
        assert!(start_time().is_some_and(|ticks| ticks > 0));
    }
}
