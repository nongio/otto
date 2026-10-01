//! Screen shares remembered for an app, in xdg-permission-store's
//! `screencast` table.
//!
//! The frontend keeps one entry there per restore token it hands an app that
//! asked to be remembered (`persist_mode` 2): the resource id is the token,
//! the app holds `yes`, and the data is this backend's `(suv)` restore
//! payload. Some apps never ask (Chrome), so when the user ticks "Remember for
//! <app>" in the picker this backend writes an entry of the same shape
//! itself, under an id of its own (`otto-…`). On a later request that brings
//! no restore data, any entry the app holds names a source to reuse instead
//! of asking again. Settings › Privacy lists these entries and Forget deletes
//! them.
//!
//! Only apps with a real app id: unsandboxed apps all arrive with the empty
//! one, and remembering for it would let every program on the host share the
//! screen without asking.

use std::collections::HashMap;

use tracing::{info, warn};
use zbus::zvariant::{OwnedValue, Value};

use crate::portal::{decode_restore_data, encode_restore_data, RestoredSource};

const STORE_NAME: &str = "org.freedesktop.impl.portal.PermissionStore";
const STORE_PATH: &str = "/org/freedesktop/impl/portal/PermissionStore";
const STORE_INTERFACE: &str = "org.freedesktop.impl.portal.PermissionStore";

/// The frontend's table for remembered screen shares.
pub const TABLE: &str = "screencast";

/// The prefix of the entries this backend writes itself.
const OWN_PREFIX: &str = "otto-";

/// Whether a share can be remembered for `app_id`.
pub fn can_remember(app_id: &str) -> bool {
    !app_id.is_empty()
}

/// Peel variant wrappers off a value, as the store nests the payload.
fn unwrapped(value: &OwnedValue) -> Option<OwnedValue> {
    let mut current: &Value<'_> = value;
    while let Value::Value(inner) = current {
        current = inner.as_ref();
    }
    current
        .try_clone()
        .ok()
        .and_then(|v| OwnedValue::try_from(v).ok())
}

/// The entries of the table `app_id` holds `yes` on: their ids and payloads.
async fn entries_for(
    connection: &zbus::Connection,
    app_id: &str,
) -> zbus::Result<Vec<(String, OwnedValue)>> {
    let reply = connection
        .call_method(
            Some(STORE_NAME),
            STORE_PATH,
            Some(STORE_INTERFACE),
            "List",
            &(TABLE,),
        )
        .await?;
    let ids: Vec<String> = reply.body().deserialize()?;
    let mut found = Vec::new();
    for id in ids {
        let Ok(reply) = connection
            .call_method(
                Some(STORE_NAME),
                STORE_PATH,
                Some(STORE_INTERFACE),
                "Lookup",
                &(TABLE, &id),
            )
            .await
        else {
            continue;
        };
        let Ok((apps, data)) = reply
            .body()
            .deserialize::<(HashMap<String, Vec<String>>, OwnedValue)>()
        else {
            continue;
        };
        if apps
            .get(app_id)
            .is_some_and(|perms| perms.iter().any(|p| p == "yes"))
        {
            found.push((id, data));
        }
    }
    Ok(found)
}

/// The sources remembered for `app_id`, newest entries first is not
/// guaranteed: the caller takes the first one that still exists.
pub async fn sources(connection: &zbus::Connection, app_id: &str) -> Vec<RestoredSource> {
    if !can_remember(app_id) {
        return Vec::new();
    }
    match entries_for(connection, app_id).await {
        Ok(entries) => entries
            .iter()
            .filter_map(|(_, data)| unwrapped(data).as_ref().and_then(decode_restore_data))
            .collect(),
        Err(err) => {
            warn!(%err, "cannot read remembered screen shares");
            Vec::new()
        }
    }
}

/// Remember `source` for `app_id`, replacing what this backend remembered for
/// it before. Entries the frontend made for the app's own tokens stay.
pub async fn remember(connection: &zbus::Connection, app_id: &str, source: &RestoredSource) {
    if !can_remember(app_id) {
        return;
    }
    let data = match encode_restore_data(source, None) {
        Ok(data) => data,
        Err(err) => {
            warn!(?err, "cannot encode a share to remember");
            return;
        }
    };
    if let Ok(entries) = entries_for(connection, app_id).await {
        for (id, _) in entries.iter().filter(|(id, _)| id.starts_with(OWN_PREFIX)) {
            let _ = connection
                .call_method(
                    Some(STORE_NAME),
                    STORE_PATH,
                    Some(STORE_INTERFACE),
                    "Delete",
                    &(TABLE, id),
                )
                .await;
        }
    }
    // Unique enough: one per remembered share, and replaced on the next.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let id = format!("{OWN_PREFIX}{nanos:x}");
    let written = async {
        connection
            .call_method(
                Some(STORE_NAME),
                STORE_PATH,
                Some(STORE_INTERFACE),
                "SetPermission",
                &(TABLE, true, &id, app_id, vec!["yes"]),
            )
            .await?;
        connection
            .call_method(
                Some(STORE_NAME),
                STORE_PATH,
                Some(STORE_INTERFACE),
                "SetValue",
                &(TABLE, true, &id, Value::from(data)),
            )
            .await?;
        zbus::Result::Ok(())
    }
    .await;
    match written {
        Ok(()) => info!(app_id, ?source, "remembered the screen share"),
        Err(err) => warn!(app_id, %err, "cannot remember the screen share"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_apps_with_an_id_are_remembered() {
        assert!(can_remember("com.obsproject.Studio"));
        assert!(!can_remember(""));
    }

    #[test]
    fn a_payload_nested_in_variants_still_decodes() {
        let source = RestoredSource::Monitor("eDP-1".into());
        let data = encode_restore_data(&source, None).unwrap();
        let nested = OwnedValue::try_from(Value::Value(Box::new(Value::from(data)))).unwrap();
        assert_eq!(
            unwrapped(&nested).as_ref().and_then(decode_restore_data),
            Some(source)
        );
    }
}
