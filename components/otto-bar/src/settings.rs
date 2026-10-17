//! Reading the compositor's settings over `org.otto.Settings`.
//!
//! Each part of the bar that follows a setting asks for its own ids and
//! listens to `Changed` itself, through `otto_kit::dbus`'s proxy; this holds
//! only the value handling they share.

use otto_kit::dbus::settings::SettingsProxy;
use zbus::zvariant::Value;

/// Unwrap the variant a `Get` answer comes in, however deep.
pub fn unwrap_variant(value: Value<'_>) -> Value<'_> {
    match value {
        Value::Value(inner) => unwrap_variant(*inner),
        other => other,
    }
}

/// Read a boolean setting, or `None` when it cannot be read (no compositor).
pub async fn get_bool(proxy: &SettingsProxy<'_>, id: &str) -> Option<bool> {
    match proxy.get(id).await {
        Ok(owned) => match unwrap_variant(owned.into()) {
            Value::Bool(value) => Some(value),
            _ => None,
        },
        Err(e) => {
            tracing::debug!("{id} read failed (no compositor?): {e}");
            None
        }
    }
}
