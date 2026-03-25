//! D-Bus interface implementation for `org.freedesktop.impl.portal.Settings`.

use std::collections::HashMap;

use tracing::{debug, error};
use zbus::fdo;
use zbus::interface;
use zbus::zvariant::{OwnedValue, Value};

use crate::otto_client::settings::OttoSettingsProxy;
use crate::otto_client::OttoClient;

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

        let mut namespaces = HashMap::new();
        let mut appearance = HashMap::new();

        appearance.insert("color-scheme".to_string(), color_scheme.into());
        appearance.insert("icon-theme".to_string(), Value::from(icon_theme).try_into().unwrap());

        namespaces.insert("org.freedesktop.appearance".to_string(), appearance);
        Ok(namespaces)
    }

    /// Gets a single setting value.
    async fn get_setting(&self, namespace: &str, key: &str) -> fdo::Result<OwnedValue> {
        match (namespace, key) {
            ("org.freedesktop.appearance", "color-scheme") => {
                let color_scheme = self.read_color_scheme().await?;
                Ok(color_scheme.into())
            }
            ("org.freedesktop.appearance", "icon-theme") => {
                let icon_theme = self.read_icon_theme().await?;
                Ok(Value::from(icon_theme).try_into().unwrap())
            }
            _ => Err(fdo::Error::Failed(format!(
                "Unknown setting: {}.{}",
                namespace, key
            ))),
        }
    }

    /// Gets a proxy to the Otto Settings D-Bus interface.
    async fn get_settings_proxy(&self) -> fdo::Result<OttoSettingsProxy> {
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

    /// Reads the icon theme name from the compositor.
    async fn read_icon_theme(&self) -> fdo::Result<String> {
        let proxy = self.get_settings_proxy().await?;
        proxy.get_icon_theme().await.map_err(|err| {
            error!(?err, "Failed to read icon theme from compositor");
            fdo::Error::Failed(format!("Failed to read icon theme: {err}"))
        })
    }

    /// Helper to match namespace patterns (supports trailing wildcard).
    fn matches_namespace(namespace: &str, pattern: &str) -> bool {
        if pattern.ends_with(".*") {
            let prefix = &pattern[..pattern.len() - 2];
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

    #[zbus(property)]
    fn version(&self) -> u32 {
        1
    }
}
