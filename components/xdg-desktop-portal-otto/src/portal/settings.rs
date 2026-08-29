//! D-Bus interface implementation for `org.freedesktop.impl.portal.Settings`.

use std::collections::HashMap;

use tracing::{debug, error};
use zbus::export::futures_util::StreamExt;
use zbus::fdo;
use zbus::interface;
use zbus::zvariant::{OwnedValue, Value};
use zbus::{Connection, SignalContext};

use crate::otto_client::settings::OttoSettingsProxy;
use crate::otto_client::OttoClient;
use crate::portal::desktop_path;

/// Settings portal implementing org.freedesktop.impl.portal.Settings.
#[derive(Clone)]
pub struct SettingsPortal {
    client: OttoClient,
}

impl SettingsPortal {
    pub fn new(client: OttoClient) -> Self {
        Self { client }
    }

    /// Returns all settings as a nested HashMap.
    async fn get_all_settings(&self) -> fdo::Result<HashMap<String, HashMap<String, OwnedValue>>> {
        let color_scheme = self.read_color_scheme().await?;
        let icon_theme = self.read_icon_theme().await?;
        let accent_color = self.read_accent_color().await?;

        let mut namespaces = HashMap::new();
        let mut appearance = HashMap::new();

        appearance.insert("color-scheme".to_string(), color_scheme.into());
        appearance.insert("accent-color".to_string(), accent_color);
        appearance.insert(
            "icon-theme".to_string(),
            Value::from(icon_theme).try_into().unwrap(),
        );

        namespaces.insert("org.freedesktop.appearance".to_string(), appearance);

        // The appearance namespace has no sound key, so the theme goes out
        // under the one GTK and libcanberra already read.
        let sound_theme = self.read_sound_theme().await?;
        let mut sound = HashMap::new();
        sound.insert(
            "theme-name".to_string(),
            Value::from(sound_theme).try_into().unwrap(),
        );
        namespaces.insert("org.gnome.desktop.sound".to_string(), sound);

        // Language has no portal key at all: the appearance namespace does not
        // carry one, and sandboxed apps take their locale from the environment
        // instead. This is here for Otto's own components, which need to agree
        // with the compositor rather than with LANG — so it goes out under
        // Otto's namespace rather than borrowing someone else's.
        let locales = self.read_locales().await?;
        let mut desktop = HashMap::new();
        desktop.insert(
            "locales".to_string(),
            Value::from(locales).try_into().unwrap(),
        );
        namespaces.insert("org.otto.desktop".to_string(), desktop);

        Ok(namespaces)
    }

    /// Gets a single setting value.
    async fn get_setting(&self, namespace: &str, key: &str) -> fdo::Result<OwnedValue> {
        match (namespace, key) {
            ("org.freedesktop.appearance", "color-scheme") => {
                let color_scheme = self.read_color_scheme().await?;
                Ok(color_scheme.into())
            }
            ("org.freedesktop.appearance", "accent-color") => self.read_accent_color().await,
            ("org.freedesktop.appearance", "icon-theme") => {
                let icon_theme = self.read_icon_theme().await?;
                Ok(Value::from(icon_theme).try_into().unwrap())
            }
            ("org.otto.desktop", "locales") => {
                let locales = self.read_locales().await?;
                Ok(Value::from(locales).try_into().unwrap())
            }
            ("org.gnome.desktop.sound", "theme-name") => {
                let sound_theme = self.read_sound_theme().await?;
                Ok(Value::from(sound_theme).try_into().unwrap())
            }
            _ => Err(fdo::Error::Failed(format!(
                "Unknown setting: {}.{}",
                namespace, key
            ))),
        }
    }

    /// Gets a proxy to the Otto Settings D-Bus interface.
    async fn get_settings_proxy(&self) -> fdo::Result<OttoSettingsProxy<'_>> {
        OttoSettingsProxy::new(&self.client.connection)
            .await
            .map_err(|err| {
                error!(?err, "Failed to create Settings proxy");
                fdo::Error::Failed(format!("Failed to connect to compositor settings: {err}"))
            })
    }

    /// Reads the color scheme from the compositor.
    async fn read_color_scheme(&self) -> fdo::Result<u32> {
        let proxy = self.get_settings_proxy().await?;
        proxy.get_color_scheme().await.map_err(|err| {
            error!(?err, "Failed to read color scheme from compositor");
            fdo::Error::Failed(format!("Failed to read color scheme: {err}"))
        })
    }

    /// Reads the accent colour from the compositor as `(ddd)`.
    ///
    /// The spec is explicit that this is a plain sRGB triple in `0.0..=1.0`
    /// with no alpha, so it is passed through exactly as the compositor gives
    /// it rather than being re-derived here.
    async fn read_accent_color(&self) -> fdo::Result<OwnedValue> {
        let proxy = self.get_settings_proxy().await?;
        let (r, g, b) = proxy.get_accent_color().await.map_err(|err| {
            error!(?err, "Failed to read accent colour from compositor");
            fdo::Error::Failed(format!("Failed to read accent colour: {err}"))
        })?;
        Value::from((r, g, b))
            .try_into()
            .map_err(|err| fdo::Error::Failed(format!("Failed to encode accent colour: {err}")))
    }

    /// Reads the icon theme name from the compositor.
    async fn read_icon_theme(&self) -> fdo::Result<String> {
        let proxy = self.get_settings_proxy().await?;
        proxy.get_icon_theme().await.map_err(|err| {
            error!(?err, "Failed to read icon theme from compositor");
            fdo::Error::Failed(format!("Failed to read icon theme: {err}"))
        })
    }

    /// Reads the sound theme name from the compositor.
    async fn read_sound_theme(&self) -> fdo::Result<String> {
        let proxy = self.get_settings_proxy().await?;
        proxy.get_sound_theme().await.map_err(|err| {
            error!(?err, "Failed to read sound theme from compositor");
            fdo::Error::Failed(format!("Failed to read sound theme: {err}"))
        })
    }

    /// The user's preferred locales, most preferred first.
    async fn read_locales(&self) -> fdo::Result<Vec<String>> {
        let proxy = self.get_settings_proxy().await?;
        proxy.get_locales().await.map_err(|err| {
            error!(?err, "Failed to read locales from compositor");
            fdo::Error::Failed(format!("Failed to read locales: {err}"))
        })
    }

    /// Helper to match namespace patterns (supports trailing wildcard).
    fn matches_namespace(namespace: &str, pattern: &str) -> bool {
        if let Some(prefix) = pattern.strip_suffix(".*") {
            namespace.starts_with(prefix)
        } else {
            namespace == pattern
        }
    }
}

#[interface(name = "org.freedesktop.impl.portal.Settings")]
impl SettingsPortal {
    /// Reads all settings, optionally filtered by namespace.
    async fn read_all(
        &self,
        namespaces: Vec<String>,
    ) -> fdo::Result<HashMap<String, HashMap<String, OwnedValue>>> {
        debug!(?namespaces, "ReadAll called");

        let all_settings = self.get_all_settings().await?;

        // If namespaces is empty or contains empty string, return all
        if namespaces.is_empty() || namespaces.iter().any(|s| s.is_empty()) {
            return Ok(all_settings);
        }

        // Filter by requested namespaces (supporting simple globbing)
        let filtered = all_settings
            .into_iter()
            .filter(|(ns, _)| {
                namespaces
                    .iter()
                    .any(|requested| Self::matches_namespace(ns, requested))
            })
            .collect();

        Ok(filtered)
    }

    /// Reads a single setting (deprecated, but required by spec).
    async fn read(&self, namespace: String, key: String) -> fdo::Result<OwnedValue> {
        debug!(namespace, key, "Read called (deprecated)");
        self.get_setting(&namespace, &key).await
    }

    /// Emitted when a setting this portal serves changes, so applications do
    /// not have to poll. `xdg-desktop-portal` relays it to its own clients.
    #[zbus(signal)]
    async fn setting_changed(
        context: &SignalContext<'_>,
        namespace: &str,
        key: &str,
        value: Value<'_>,
    ) -> zbus::Result<()>;

    /// Lowercase per the impl portal spec — see the ScreenCast interface.
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

/// Watches `org.otto.Settings` and re-emits the settings this portal serves as
/// `SettingChanged`.
///
/// Without this the portal is read-only in practice: a client reads the accent
/// once at startup and never learns that the user changed it.
pub async fn spawn_change_relay(connection: Connection, client: OttoClient) -> zbus::Result<()> {
    let proxy = OttoSettingsProxy::new(&client.connection).await?;
    let mut changes = proxy.receive_changed().await?;

    tokio::spawn(async move {
        let portal = SettingsPortal::new(client);
        while let Some(change) = changes.next().await {
            let Ok(args) = change.args() else { continue };
            // Otto's identifiers are not the portal's keys, so only the ones
            // with a counterpart here are forwarded.
            for id in args.values.keys() {
                let (namespace, key) = match id.as_str() {
                    "accent_color" => ("org.freedesktop.appearance", "accent-color"),
                    "theme_scheme" => ("org.freedesktop.appearance", "color-scheme"),
                    "icon_theme" => ("org.freedesktop.appearance", "icon-theme"),
                    "audio.sound_theme" => ("org.gnome.desktop.sound", "theme-name"),
                    _ => continue,
                };
                let Ok(value) = portal.get_setting(namespace, key).await else {
                    continue;
                };
                let iface = connection
                    .object_server()
                    .interface::<_, SettingsPortal>(desktop_path())
                    .await;
                match iface {
                    Ok(iface) => {
                        if let Err(err) = SettingsPortal::setting_changed(
                            iface.signal_context(),
                            namespace,
                            key,
                            Value::from(value),
                        )
                        .await
                        {
                            error!(?err, key, "Failed to emit SettingChanged");
                        }
                    }
                    Err(err) => error!(?err, "Settings portal interface went away"),
                }
            }
        }
    });

    Ok(())
}
