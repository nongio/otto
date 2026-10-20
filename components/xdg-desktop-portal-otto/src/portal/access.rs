//! `org.freedesktop.impl.portal.Access` backend.
//!
//! The portal is the broker for permission/choice dialogs: it receives the
//! request, decides which renderer presents it (today: otto-islands), and
//! relays the decision. If no renderer is available the request is denied —
//! a screenshare that cannot prompt must fail closed.
//!
//! See `specs/portal-access-dialog.md`.

use std::collections::HashMap;

use tracing::{info, warn};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Str, Value};
use zbus::{interface, ObjectServer};

use crate::otto_client::dialog::WireChoice;
use crate::otto_client::OttoClient;
use crate::portal::Request;

/// A choice group as it arrives in the portal `options` map:
/// `(id, label, [(option_id, option_label)], default_option_id)`.
/// Note: freedesktop options carry no per-option icon.
type PortalChoice = (String, String, Vec<(String, String)>, String);

/// Extract an owned `String` option value (OwnedValue is not `Clone`, so we
/// go through `try_clone`).
fn string_opt(options: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    options
        .get(key)
        .and_then(|v| v.try_clone().ok())
        .and_then(|owned| TryInto::<String>::try_into(owned).ok())
}

pub struct AccessPortal {
    client: OttoClient,
}

impl AccessPortal {
    pub fn new(client: OttoClient) -> Self {
        Self { client }
    }
}

#[interface(name = "org.freedesktop.impl.portal.Access")]
impl AccessPortal {
    /// Present an access/permission dialog. Mirrors the freedesktop contract:
    /// returns `(response, results)` with `response` `0` allow, `1` cancel,
    /// `2` other, and `results` mapping each choice id to the selected option.
    ///
    /// The request is exported at `handle` while the dialog is up: when the
    /// frontend closes it, the dialog is withdrawn and the call answers `2`.
    #[allow(clippy::too_many_arguments)]
    async fn access_dialog(
        &self,
        #[zbus(object_server)] object_server: &ObjectServer,
        handle: OwnedObjectPath,
        app_id: String,
        _parent_window: String,
        title: String,
        subtitle: String,
        body: String,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        info!(?app_id, %title, "AccessDialog called");
        let asked = self.ask(&handle, &app_id, &title, &subtitle, &body, &options);
        Request::serve(object_server, &handle, asked)
            .await
            .unwrap_or_else(|| {
                info!(?app_id, "AccessDialog closed by the frontend");
                (2, HashMap::new())
            })
    }
}

impl AccessPortal {
    async fn ask(
        &self,
        handle: &OwnedObjectPath,
        app_id: &str,
        title: &str,
        subtitle: &str,
        body: &str,
        options: &HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let modal = options
            .get("modal")
            .and_then(|v| bool::try_from(v).ok())
            .unwrap_or(true);
        let icon = string_opt(options, "icon").unwrap_or_default();
        let grant_label = string_opt(options, "grant_label").unwrap_or_default();
        let deny_label = string_opt(options, "deny_label").unwrap_or_default();

        // Translate freedesktop choices `(id, label, [(id, label)], default)`
        // into the renderer wire format (adds an empty per-option icon).
        let portal_choices: Vec<PortalChoice> = options
            .get("choices")
            .and_then(|v| v.try_clone().ok())
            .and_then(|owned| TryInto::<Vec<PortalChoice>>::try_into(owned).ok())
            .unwrap_or_default();
        let wire: Vec<WireChoice> = portal_choices
            .into_iter()
            .map(|(id, label, opts, default)| {
                let opts = opts
                    .into_iter()
                    .map(|(oid, olabel)| (oid, olabel, String::new()))
                    .collect();
                (id, label, opts, default)
            })
            .collect();

        match self
            .client
            .present_access_for(
                handle.as_str(),
                app_id,
                title,
                subtitle,
                body,
                &icon,
                &grant_label,
                &deny_label,
                modal,
                wire,
            )
            .await
        {
            Ok((response, results)) => {
                let mut map = HashMap::new();
                for (group_id, option_id) in results {
                    if let Ok(v) = OwnedValue::try_from(Value::from(Str::from(option_id))) {
                        map.insert(group_id, v);
                    }
                }
                info!(?app_id, response, "AccessDialog resolved");
                (response, map)
            }
            Err(err) => {
                warn!(?err, "no dialog renderer answered; denying request");
                (1, HashMap::new())
            }
        }
    }
}
