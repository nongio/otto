//! Clients for the Access-style dialog renderers this portal can broker to.
//!
//! The portal owns which renderer presents a dialog. The native one is
//! otto-islands (`org.otto.Dialog1`); when it isn't running we fall back to any
//! other desktop's `org.freedesktop.impl.portal.Access` implementation, which
//! is the standard interface `org.otto.Dialog1` was modelled on.
//! See `specs/portal-access-dialog.md`.

use std::collections::HashMap;

use zbus::zvariant::{ObjectPath, OwnedValue, Value};
use zbus::Result;

use crate::otto_client::OttoClient;

/// A choice group as sent to the renderer:
/// `(group_id, group_label, [(option_id, option_label, option_icon)], default_option_id)`.
pub type WireChoice = (String, String, Vec<(String, String, String)>, String);

/// D-Bus proxy for `org.otto.Dialog1` (served by otto-islands).
#[zbus::proxy(
    interface = "org.otto.Dialog1",
    default_service = "org.otto.Island",
    default_path = "/org/otto/Dialog"
)]
trait Dialog {
    /// Present a dialog and block until the user answers or the request is
    /// withdrawn. Returns `(response, results)`:
    /// - `response`: `0` granted, `1` cancelled/denied, `2` ended.
    /// - `results`: `[(group_id, selected_option_id)]`.
    #[allow(clippy::too_many_arguments)]
    async fn present_access(
        &self,
        app_id: &str,
        title: &str,
        subtitle: &str,
        body: &str,
        icon: &str,
        grant_label: &str,
        deny_label: &str,
        modal: bool,
        choices: Vec<WireChoice>,
    ) -> Result<(u32, Vec<(String, String)>)>;
}

/// A choice group in the **standard** `org.freedesktop.impl.portal.Access`
/// shape: `(group_id, group_label, [(option_id, option_label)], default)`.
///
/// Note the option tuple is `(ss)`, not `(sss)` — the standard interface has no
/// per-option icon, so icons are dropped on this path.
pub type AccessChoice = (String, String, Vec<(String, String)>, String);

/// Other desktops' Access implementations, tried in order when otto-islands
/// isn't running.
///
/// These are named explicitly rather than resolved through portals.conf on
/// purpose: **this portal implements Access itself**, so asking for "the
/// configured Access backend" could route straight back here and deadlock.
/// GTK leads because it's the one most likely present on a system that is
/// neither GNOME nor KDE.
pub const ACCESS_FALLBACK_BACKENDS: &[&str] = &[
    "org.freedesktop.impl.portal.desktop.gtk",
    "org.freedesktop.impl.portal.desktop.gnome",
    "org.freedesktop.impl.portal.desktop.kde",
];

/// D-Bus proxy for `org.freedesktop.impl.portal.Access`.
///
/// No `default_service`: the destination is chosen per call from
/// [`ACCESS_FALLBACK_BACKENDS`].
#[zbus::proxy(
    interface = "org.freedesktop.impl.portal.Access",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait Access {
    /// Present a dialog and block until the user answers.
    ///
    /// `options` carries `modal`, `grant_label`, `deny_label`, `icon` and
    /// `choices`; `results` carries the selected `choices` back.
    #[allow(clippy::too_many_arguments)]
    async fn access_dialog(
        &self,
        handle: &ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        title: &str,
        subtitle: &str,
        body: &str,
        options: HashMap<&str, Value<'_>>,
    ) -> Result<(u32, HashMap<String, OwnedValue>)>;
}

impl OttoClient {
    /// Build a proxy to the dialog renderer.
    pub async fn dialog_proxy(&self) -> Result<DialogProxy<'_>> {
        DialogProxy::new(&self.connection).await
    }

    /// Present a dialog through the first reachable standard Access backend.
    ///
    /// Returns `(response, [(group_id, option_id)])`, matching
    /// [`DialogProxy::present_access`] so callers can treat the two
    /// interchangeably. `Err` means no backend answered at all.
    #[allow(clippy::too_many_arguments)] // mirrors the Access interface 1:1
    pub async fn present_access_fallback(
        &self,
        app_id: &str,
        title: &str,
        subtitle: &str,
        body: &str,
        icon: &str,
        grant_label: &str,
        deny_label: &str,
        modal: bool,
        choices: Vec<AccessChoice>,
    ) -> Result<(u32, Vec<(String, String)>)> {
        let handle = ObjectPath::try_from("/org/freedesktop/portal/desktop/request/otto/1")
            .map_err(|e| zbus::Error::Failure(format!("bad request handle: {e}")))?;

        let mut last_err = None;
        for service in ACCESS_FALLBACK_BACKENDS {
            let proxy: AccessProxy<'_> =
                match AccessProxy::builder(&self.connection).destination(*service) {
                    Ok(builder) => match builder.build().await {
                        Ok(p) => p,
                        Err(err) => {
                            last_err = Some(err);
                            continue;
                        }
                    },
                    Err(err) => {
                        last_err = Some(err);
                        continue;
                    }
                };

            let mut options: HashMap<&str, Value<'_>> = HashMap::new();
            options.insert("modal", Value::Bool(modal));
            options.insert("grant_label", Value::from(grant_label));
            options.insert("deny_label", Value::from(deny_label));
            if !icon.is_empty() {
                options.insert("icon", Value::from(icon));
            }
            options.insert(
                "choices",
                Value::from(choices.clone())
                    .try_clone()
                    .map_err(|e| zbus::Error::Failure(format!("bad choices: {e}")))?,
            );

            match proxy
                .access_dialog(&handle, app_id, "", title, subtitle, body, options)
                .await
            {
                Ok((response, results)) => {
                    tracing::info!(service, response, "Access fallback answered");
                    let selected = results
                        .get("choices")
                        .and_then(|v| v.try_clone().ok())
                        .and_then(|v| Vec::<(String, String)>::try_from(v).ok())
                        .unwrap_or_default();
                    return Ok((response, selected));
                }
                Err(err) => {
                    tracing::debug!(service, ?err, "Access backend unavailable");
                    last_err = Some(err);
                }
            }
        }

        Err(last_err
            .unwrap_or_else(|| zbus::Error::Failure("no Access backend available".to_string())))
    }
}
