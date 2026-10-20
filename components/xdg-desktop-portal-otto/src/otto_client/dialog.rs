//! Clients for the Access-style dialog renderers this portal can broker to.
//!
//! The portal owns which renderer presents a dialog. The native one is
//! otto-islands (`org.otto.Dialog1`); when it isn't running we fall back to any
//! other desktop's `org.freedesktop.impl.portal.Access` implementation, which
//! is the standard interface `org.otto.Dialog1` was modelled on.
//! See `specs/portal-access-dialog.md`.

use std::collections::HashMap;

use zbus::zvariant::{ObjectPath, Value};
use zbus::Result;

use crate::otto_client::OttoClient;
pub use otto_dbus::dialog::{DialogProxy, WireChoice};
use otto_dbus::portal_access::PortalAccessProxy;

/// Withdraws the dialog it names when dropped while still armed: the call
/// that put the dialog up was abandoned before its answer.
struct Withdraw {
    connection: zbus::Connection,
    app_id: String,
    cookie: String,
    armed: bool,
}

impl Withdraw {
    /// The dialog was answered; there is nothing to take down.
    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for Withdraw {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // Dropped from inside the abandoned call: the withdrawal cannot be
        // awaited here, so it runs on its own.
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::warn!(cookie = %self.cookie, "no runtime to withdraw the dialog on");
            return;
        };
        let connection = self.connection.clone();
        let (app_id, cookie) = (
            std::mem::take(&mut self.app_id),
            std::mem::take(&mut self.cookie),
        );
        runtime.spawn(async move {
            let withdrawn = match DialogProxy::new(&connection).await {
                Ok(proxy) => proxy.withdraw(&app_id, &cookie).await,
                Err(err) => Err(err),
            };
            match withdrawn {
                Ok(found) => tracing::info!(%cookie, found, "dialog withdrawn"),
                Err(err) => tracing::warn!(?err, %cookie, "withdrawing the dialog failed"),
            }
        });
    }
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

impl OttoClient {
    /// Build a proxy to the dialog renderer.
    pub async fn dialog_proxy(&self) -> Result<DialogProxy<'_>> {
        DialogProxy::new(&self.connection).await
    }

    /// Present an access dialog through otto-islands on behalf of the portal
    /// request at `request`, which names the dialog.
    ///
    /// Dropped before the answer, which is what happens to the pending method
    /// when the frontend closes the request, the dialog is withdrawn rather
    /// than left on screen for an app that no longer waits. An otto-islands
    /// too old to take a cookie is asked through `PresentAccess`, without
    /// that.
    #[allow(clippy::too_many_arguments)] // mirrors PresentAccess 1:1
    pub async fn present_access_for(
        &self,
        request: &str,
        app_id: &str,
        title: &str,
        subtitle: &str,
        body: &str,
        icon: &str,
        grant_label: &str,
        deny_label: &str,
        modal: bool,
        choices: Vec<WireChoice>,
    ) -> Result<(u32, Vec<(String, String)>)> {
        let proxy = self.dialog_proxy().await?;
        let pending = Withdraw {
            connection: self.connection.clone(),
            app_id: app_id.to_owned(),
            cookie: request.to_owned(),
            armed: true,
        };
        let answer = proxy
            .present_access_with_cookie(
                app_id,
                request,
                title,
                subtitle,
                body,
                icon,
                grant_label,
                deny_label,
                modal,
                choices.clone(),
            )
            .await;
        pending.disarm();
        match answer {
            Err(zbus::Error::MethodError(name, _, _))
                if name.as_str() == "org.freedesktop.DBus.Error.UnknownMethod" =>
            {
                tracing::debug!("otto-islands takes no cookie; the dialog cannot be withdrawn");
                proxy
                    .present_access(
                        app_id,
                        title,
                        subtitle,
                        body,
                        icon,
                        grant_label,
                        deny_label,
                        modal,
                        choices,
                    )
                    .await
            }
            answer => answer,
        }
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
            let proxy: PortalAccessProxy<'_> =
                match PortalAccessProxy::builder(&self.connection).destination(*service) {
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

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};
    use std::time::Duration;

    use tokio::sync::mpsc;

    use super::*;

    /// A private bus, so the test needs no session and touches nobody's.
    fn private_bus() -> Option<(Child, String)> {
        let mut daemon = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .ok()?;
        let mut address = String::new();
        BufReader::new(daemon.stdout.take()?)
            .read_line(&mut address)
            .ok()?;
        Some((daemon, address.trim().to_owned()))
    }

    async fn connect(address: &str) -> zbus::Connection {
        zbus::connection::Builder::address(address)
            .unwrap()
            .build()
            .await
            .unwrap()
    }

    /// otto-islands, as far as the dialog goes: it never answers, and
    /// reports each withdrawal.
    struct Islands {
        withdrawn: mpsc::UnboundedSender<(String, String)>,
    }

    #[zbus::interface(name = "org.otto.Dialog1")]
    impl Islands {
        #[allow(clippy::too_many_arguments)]
        async fn present_access_with_cookie(
            &self,
            _app_id: &str,
            _cookie: &str,
            _title: &str,
            _subtitle: &str,
            _body: &str,
            _icon: &str,
            _grant_label: &str,
            _deny_label: &str,
            _modal: bool,
            _choices: Vec<WireChoice>,
        ) -> (u32, Vec<(String, String)>) {
            std::future::pending().await
        }

        async fn withdraw(&self, app_id: &str, cookie: &str) -> bool {
            self.withdrawn
                .send((app_id.to_owned(), cookie.to_owned()))
                .unwrap();
            true
        }
    }

    /// An otto-islands from before cookies: `PresentAccess` only.
    struct OldIslands;

    #[zbus::interface(name = "org.otto.Dialog1")]
    impl OldIslands {
        #[allow(clippy::too_many_arguments)]
        async fn present_access(
            &self,
            _app_id: &str,
            _title: &str,
            _subtitle: &str,
            _body: &str,
            _icon: &str,
            _grant_label: &str,
            _deny_label: &str,
            _modal: bool,
            _choices: Vec<WireChoice>,
        ) -> (u32, Vec<(String, String)>) {
            (0, Vec::new())
        }
    }

    async fn ask(client: &OttoClient) -> Result<(u32, Vec<(String, String)>)> {
        client
            .present_access_for(
                "/org/freedesktop/portal/desktop/request/1_2/t",
                "org.example.App",
                "Title",
                "",
                "",
                "",
                "",
                "",
                true,
                Vec::new(),
            )
            .await
    }

    #[tokio::test]
    async fn an_abandoned_dialog_is_withdrawn_by_its_request() {
        let Some((mut daemon, address)) = private_bus() else {
            eprintln!("no dbus-daemon; skipping");
            return;
        };
        let (withdrawn, mut withdrawals) = mpsc::unbounded_channel();
        let _islands = zbus::connection::Builder::address(address.as_str())
            .unwrap()
            .name(otto_dbus::dialog::SERVICE)
            .unwrap()
            .serve_at(otto_dbus::dialog::PATH, Islands { withdrawn })
            .unwrap()
            .build()
            .await
            .unwrap();
        let client = OttoClient {
            connection: connect(&address).await,
        };

        // The frontend closing the request drops the pending call.
        let abandoned = tokio::time::timeout(Duration::from_millis(300), ask(&client)).await;
        assert!(abandoned.is_err(), "the dialog never answers");

        let (app_id, cookie) = tokio::time::timeout(Duration::from_secs(5), withdrawals.recv())
            .await
            .expect("the dialog was withdrawn")
            .unwrap();
        assert_eq!(app_id, "org.example.App");
        assert_eq!(cookie, "/org/freedesktop/portal/desktop/request/1_2/t");
        let _ = daemon.kill();
    }

    #[tokio::test]
    async fn an_islands_without_cookies_is_asked_the_old_way() {
        let Some((mut daemon, address)) = private_bus() else {
            eprintln!("no dbus-daemon; skipping");
            return;
        };
        let _islands = zbus::connection::Builder::address(address.as_str())
            .unwrap()
            .name(otto_dbus::dialog::SERVICE)
            .unwrap()
            .serve_at(otto_dbus::dialog::PATH, OldIslands)
            .unwrap()
            .build()
            .await
            .unwrap();
        let client = OttoClient {
            connection: connect(&address).await,
        };
        assert_eq!(ask(&client).await.unwrap(), (0, Vec::new()));
        let _ = daemon.kill();
    }
}
