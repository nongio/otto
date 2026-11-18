//! Asking polkit before a protected setting changes.
//!
//! The settings in [`super::schema::PROTECTED`] decide what runs in front of
//! the password, or whether the session locks at all. Any client on the
//! session bus can ask to change them, so the settings service first asks
//! polkit to authorize `org.otto.settings.lock` for Otto's own process. The
//! action is `auth_self`: polkitd asks the session's authentication agent,
//! which is Otto's own (`otto-authorize --polkit-agent`), and the agent shows
//! the auth panel with the action's message. A script on the bus can make the
//! request; it cannot type the password.
//!
//! No details go with the request: polkit accepts them only from root or the
//! action's owner, and refuses the whole check otherwise.
//!
//! This proves someone at the session agreed. It does not stop a program
//! running as the user from editing `config.toml` directly.

use std::collections::HashMap;

use otto_kit::dbus::polkit::{self, AuthorityProxy};

/// The polkit action every protected setting asks for. Declared in
/// `resources/polkit/org.otto.settings.policy`.
pub const ACTION: &str = "org.otto.settings.lock";

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

/// Ask polkit to authorize changing `setting`. Waits for as long as the user
/// takes to answer the panel.
pub async fn authorize(setting: &str) -> Result<(), Refusal> {
    let connection = zbus::Connection::system()
        .await
        .map_err(|err| Refusal::Unavailable(err.to_string()))?;

    let authority = AuthorityProxy::new(&connection)
        .await
        .map_err(|err| Refusal::Unavailable(err.to_string()))?;
    let (authorized, _challenge, _details) = authority
        .check_authorization(
            &polkit::this_process(),
            ACTION,
            HashMap::new(),
            polkit::ALLOW_USER_INTERACTION,
            "",
        )
        .await
        .map_err(|err| Refusal::Unavailable(err.to_string()))?;

    if authorized {
        tracing::info!(setting, "protected setting change confirmed");
        Ok(())
    } else {
        tracing::info!(setting, "protected setting change not confirmed");
        Err(Refusal::NotAuthorized)
    }
}
