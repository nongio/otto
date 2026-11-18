//! Confirming it's the person before a saved password is read.
//!
//! NetworkManager authorises `GetSecrets` with its own polkit action, which
//! an active session usually holds outright, so reading a password would
//! need no one at the keyboard. [`wifi_share`](crate::wifi_share) first asks
//! polkit for [`ACTION`], which is `auth_self_keep`: the session's
//! authentication agent asks the person for their password, and polkit
//! remembers the answer for a few minutes. Anything short of a yes refuses.

// Rust guideline compliant 2026-02-21

use std::collections::HashMap;

use otto_dbus::polkit::{self, AuthorityProxy, WireAuthorization};
use zbus::proxy::CacheProperties;
use zbus::Connection;

/// The polkit action asked for before a saved Wi-Fi password is read.
/// Declared in `resources/polkit/org.otto.network.policy`.
pub const ACTION: &str = "org.otto.network.reveal-password";

/// Why a saved password was not read.
#[derive(Debug)]
pub enum ShareError {
    /// The person cancelled the prompt, or did not prove who they are.
    NotConfirmed,
    /// polkit could not be asked, or does not know [`ACTION`] because its
    /// policy is not installed. Nothing is read without the check.
    Unavailable(String),
    /// NetworkManager refused or failed the call.
    Bus(zbus::Error),
}

impl ShareError {
    /// Whether this is a refusal to read the password, as opposed to a
    /// failure: the check said no or could not be made, or NetworkManager
    /// denied `GetSecrets`.
    #[must_use]
    pub fn is_refusal(&self) -> bool {
        match self {
            Self::NotConfirmed | Self::Unavailable(_) => true,
            Self::Bus(zbus::Error::MethodError(name, _, _)) => {
                name.contains("PermissionDenied") || name.ends_with("AccessDenied")
            }
            Self::Bus(_) => false,
        }
    }
}

impl std::fmt::Display for ShareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfirmed => f.write_str("reading the password was not confirmed"),
            Self::Unavailable(reason) => write!(f, "polkit could not confirm it ({reason})"),
            Self::Bus(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for ShareError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Bus(err) => Some(err),
            Self::NotConfirmed | Self::Unavailable(_) => None,
        }
    }
}

impl From<zbus::Error> for ShareError {
    fn from(err: zbus::Error) -> Self {
        Self::Bus(err)
    }
}

/// Ask polkit, on `conn`, whether this process may read a saved password,
/// letting the session's agent ask the person. Waits for as long as they
/// take to answer.
pub(crate) async fn confirm(conn: &Connection) -> Result<(), ShareError> {
    let reply = async {
        let authority = AuthorityProxy::builder(conn)
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        authority
            .check_authorization(
                &polkit::this_process(),
                ACTION,
                HashMap::new(),
                polkit::ALLOW_USER_INTERACTION,
                "",
            )
            .await
    }
    .await;
    confirmed(reply)
}

/// What polkit's answer comes to: only an authorization reads the password.
fn confirmed(reply: zbus::Result<WireAuthorization>) -> Result<(), ShareError> {
    match reply {
        Ok((true, _, _)) => Ok(()),
        Ok((false, _, _)) => Err(ShareError::NotConfirmed),
        Err(err) => Err(ShareError::Unavailable(err.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn method_error(name: &str) -> zbus::Error {
        zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from(name).unwrap(),
            None,
            zbus::message::Message::method_call("/", "Ping")
                .unwrap()
                .build(&())
                .unwrap(),
        )
    }

    #[test]
    fn an_authorization_reads_the_password() {
        assert!(confirmed(Ok((true, false, HashMap::new()))).is_ok());
    }

    #[test]
    fn anything_else_refuses() {
        let refused = confirmed(Ok((false, false, HashMap::new()))).unwrap_err();
        assert!(matches!(refused, ShareError::NotConfirmed));
        assert!(refused.is_refusal());

        // A challenge polkit could not put to anyone is still a no.
        assert!(matches!(
            confirmed(Ok((false, true, HashMap::new()))),
            Err(ShareError::NotConfirmed)
        ));
    }

    #[test]
    fn no_polkit_refuses() {
        for err in [
            method_error("org.freedesktop.DBus.Error.ServiceUnknown"),
            // What polkitd answers when the policy is not installed.
            method_error("org.freedesktop.PolicyKit1.Error.Failed"),
            method_error("org.freedesktop.PolicyKit1.Error.Cancelled"),
            zbus::Error::Failure("no system bus".into()),
        ] {
            let refused = confirmed(Err(err)).unwrap_err();
            assert!(matches!(refused, ShareError::Unavailable(_)), "{refused:?}");
            assert!(refused.is_refusal());
        }
    }

    #[test]
    fn network_manager_denying_is_a_refusal() {
        let denied = |name| ShareError::Bus(method_error(name));
        assert!(denied("org.freedesktop.NetworkManager.Settings.PermissionDenied").is_refusal());
        assert!(denied("org.freedesktop.DBus.Error.AccessDenied").is_refusal());
        assert!(!denied("org.freedesktop.NetworkManager.Settings.InvalidConnection").is_refusal());
        assert!(
            !ShareError::Bus(zbus::Error::Failure("not a Wi-Fi connection".into())).is_refusal()
        );
    }
}
