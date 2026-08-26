//! D-Bus service implementation for `org.otto.Settings`.
//!
//! The wire contract lives in `docs/developer/settings-dbus-api.md`; the
//! behaviour it implements is specified in `specs/settings-app.md`. This module
//! is only the bus end of it — validation, apply, persistence and announcement
//! all live in [`crate::settings`], so an in-compositor interaction takes
//! exactly the same path as a call from a settings app.

use std::collections::HashMap;

use tokio::sync::oneshot;
use tracing::info;
use zbus::zvariant::{OwnedValue, Value};
use zbus::{interface, Connection, SignalContext};

use crate::config::Config;
use crate::screenshare::CompositorCommand;
use crate::settings::value::SettingValue;
use crate::settings::{self, SetError};
use crate::theme::ThemeScheme;

/// The `Set`/`Reset` errors, as the five names the contract fixes.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.otto.Settings.Error")]
pub enum SettingsFault {
    #[zbus(error)]
    ZBus(zbus::Error),
    /// No such identifier.
    UnknownSetting(String),
    /// The variant's type does not match the schema.
    InvalidType(String),
    /// Outside `min`/`max`, or not one of `choices`.
    OutOfRange(String),
    /// `apply` is `unsupported` on this system.
    Unsupported(String),
    /// Valid, but the compositor could not apply it. Nothing was persisted.
    ApplyFailed(String),
}

impl From<SetError> for SettingsFault {
    fn from(error: SetError) -> Self {
        match error {
            SetError::Unknown(message) => SettingsFault::UnknownSetting(message),
            SetError::InvalidType(message) => SettingsFault::InvalidType(message),
            SetError::OutOfRange(message) => SettingsFault::OutOfRange(message),
            SetError::Unsupported(message) => SettingsFault::Unsupported(message),
            SetError::ApplyFailed(message) => SettingsFault::ApplyFailed(message),
        }
    }
}

/// The main Settings D-Bus interface.
///
/// Implements `org.otto.Settings` at `/org/otto/Settings`.
pub struct SettingsInterface {
    /// Reading is served straight from the live configuration, but changing a
    /// setting has to happen on the compositor thread, where the running system
    /// can actually be reconciled with it.
    compositor_tx: smithay::reexports::calloop::channel::Sender<CompositorCommand>,
}

#[interface(name = "org.otto.Settings")]
impl SettingsInterface {
    /// Returns the color scheme preference.
    ///
    /// Returns:
    /// - 0: No preference
    /// - 1: Prefer dark appearance
    /// - 2: Prefer light appearance
    async fn get_color_scheme(&self) -> u32 {
        Config::with(|config| match config.theme_scheme {
            ThemeScheme::Dark => 1,
            ThemeScheme::Light => 2,
        })
    }

    /// Returns the accent colour as sRGB components in `0.0..=1.0`.
    ///
    /// The wire shape matches `org.freedesktop.appearance accent-color`, so the
    /// portal can hand it straight on to applications; Otto's own name for the
    /// colour is available through `Get("accent_color")`.
    async fn get_accent_color(&self) -> (f64, f64, f64) {
        // The theme stores colours in Oklab; `c4f` is the sRGB conversion, and
        // it already normalises to 0..1.
        let accent = crate::theme::accent_color().c4f();
        (accent.r as f64, accent.g as f64, accent.b as f64)
    }

    /// Returns the configured icon theme name (e.g. "Adwaita", "Papirus").
    ///
    /// Returns an empty string if no theme is configured (auto-detect).
    async fn get_icon_theme(&self) -> String {
        Config::with(|config| config.icon_theme.clone().unwrap_or_default())
    }

    /// Returns the configured sound theme name (e.g. "freedesktop", "ocean").
    ///
    /// Empty means no preference, which leaves the app to auto-detect — the
    /// same contract `GetIconTheme` has.
    async fn get_sound_theme(&self) -> String {
        Config::with(|config| config.audio.sound_theme.clone().unwrap_or_default())
    }

    /// The schema: one dictionary per setting. Clients ignore keys they do not
    /// know, so the schema can grow without breaking them.
    async fn describe(&self) -> Vec<HashMap<String, OwnedValue>> {
        settings::describe()
    }

    /// The current effective value of every setting.
    async fn get_all(&self) -> HashMap<String, OwnedValue> {
        settings::all_values()
            .into_iter()
            .map(|(id, value)| (id.to_string(), value.to_variant()))
            .collect()
    }

    /// The current effective value of one setting.
    async fn get(&self, id: &str) -> Result<OwnedValue, SettingsFault> {
        settings::value_of(id)
            .map(|value| value.to_variant())
            .ok_or_else(|| SettingsFault::UnknownSetting(format!("no such setting `{id}`")))
    }

    /// The identifiers currently set in the writable configuration file.
    async fn get_overridden(&self) -> Vec<String> {
        settings::overridden()
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    /// Set one setting: validate, apply, persist, announce.
    async fn set(&self, id: &str, value: Value<'_>) -> Result<String, SettingsFault> {
        let value = SettingValue::from_variant(&value).ok_or_else(|| {
            SettingsFault::InvalidType(format!("`{id}` was given a value of an unusable type"))
        })?;

        let (response_tx, response_rx) = oneshot::channel();
        self.compositor_tx
            .send(CompositorCommand::SetSetting {
                id: id.to_string(),
                value,
                response_tx,
            })
            .map_err(|err| {
                SettingsFault::ApplyFailed(format!("compositor is not listening: {err}"))
            })?;

        let status = response_rx
            .await
            .map_err(|_| SettingsFault::ApplyFailed("compositor did not answer".to_string()))??;
        Ok(status.wire_name().to_string())
    }

    /// Remove one setting from the writable configuration file.
    async fn reset(&self, id: &str) -> Result<String, SettingsFault> {
        let (response_tx, response_rx) = oneshot::channel();
        self.compositor_tx
            .send(CompositorCommand::ResetSetting {
                id: id.to_string(),
                response_tx,
            })
            .map_err(|err| {
                SettingsFault::ApplyFailed(format!("compositor is not listening: {err}"))
            })?;

        let status = response_rx
            .await
            .map_err(|_| SettingsFault::ApplyFailed("compositor did not answer".to_string()))??;
        Ok(status.wire_name().to_string())
    }

    /// Every output the compositor currently has, physical and virtual.
    ///
    /// Not part of the settings schema: outputs come and go with the hardware,
    /// so they cannot be identifiers in a table fixed at compile time. A
    /// client that wants to draw a display arrangement reads it from here.
    async fn list_outputs(&self) -> Result<Vec<HashMap<String, OwnedValue>>, SettingsFault> {
        let (response_tx, response_rx) = oneshot::channel();
        self.compositor_tx
            .send(CompositorCommand::ListOutputs { response_tx })
            .map_err(|err| {
                SettingsFault::ApplyFailed(format!("compositor is not listening: {err}"))
            })?;

        let outputs = response_rx
            .await
            .map_err(|_| SettingsFault::ApplyFailed("compositor did not answer".to_string()))?;

        Ok(outputs
            .into_iter()
            .map(|output| {
                let mut entry = HashMap::new();
                let mut put = |key: &str, value: SettingValue| {
                    entry.insert(key.to_string(), value.to_variant());
                };
                put("name", SettingValue::Str(output.name.clone()));
                put("connector", SettingValue::Str(output.connector));
                put("width", SettingValue::Int(output.width as i64));
                put("height", SettingValue::Int(output.height as i64));
                // Millihertz, as the mode reports it.
                put("refresh", SettingValue::Int(output.refresh_rate as i64));
                put("virtual", SettingValue::Bool(output.is_virtual));
                put("x", SettingValue::Int(output.x as i64));
                put("y", SettingValue::Int(output.y as i64));
                put("scale", SettingValue::Double(output.scale));
                entry
            })
            .collect())
    }

    /// The file a changed setting is written to.
    ///
    /// Configuration is layered, and which layer is writable depends on what
    /// exists on this system — a local `otto_config.toml` next to the running
    /// binary quietly outranks `~/.config/otto/config.toml`. A client cannot
    /// work that out for itself, so it asks.
    async fn config_path(&self) -> String {
        crate::config::writable_config_path()
            .to_string_lossy()
            .into_owned()
    }

    /// Persist how one display should be driven, keyed by its connector.
    ///
    /// Not part of the settings schema, for the same reason virtual outputs
    /// are not: the identifier is a connector name the hardware supplies at
    /// runtime, and the schema is a table fixed at compile time. This writes
    /// `displays.named.<connector>` — the same profile
    /// `DisplaysConfig::resolve` reads when an output is brought up.
    ///
    /// **Takes effect at the next start.** Nothing here re-drives the running
    /// output: a mode change is a modeset, and one made from under a session
    /// that cannot be undone if the display does not come back is worse than
    /// one you restart for. The answer says so, in the same words `Set` uses.
    ///
    /// A zero width or height leaves the resolution unset, and a zero refresh
    /// leaves the rate unset, so a caller that only wants to move a display
    /// does not have to invent a mode for it.
    #[allow(clippy::too_many_arguments)]
    async fn set_output_profile(
        &self,
        connector: &str,
        width: u32,
        height: u32,
        refresh_hz: f64,
        x: i32,
        y: i32,
        primary: bool,
    ) -> Result<String, SettingsFault> {
        if connector.trim().is_empty() {
            return Err(SettingsFault::OutOfRange(
                "a display profile needs a connector".to_string(),
            ));
        }

        let key = |leaf: &str| format!("displays.named.{connector}.{leaf}");
        let write = |leaf: &str, value: toml::Value| {
            crate::config::persist_key(&key(leaf), &value).map_err(|reason| {
                SettingsFault::ApplyFailed(format!("could not persist: {reason}"))
            })
        };

        write("primary", toml::Value::Boolean(primary))?;
        write(
            "position",
            toml::Value::Table(toml::map::Map::from_iter([
                ("x".to_string(), toml::Value::Integer(x as i64)),
                ("y".to_string(), toml::Value::Integer(y as i64)),
            ])),
        )?;
        if width > 0 && height > 0 {
            write(
                "resolution",
                toml::Value::Table(toml::map::Map::from_iter([
                    ("width".to_string(), toml::Value::Integer(width as i64)),
                    ("height".to_string(), toml::Value::Integer(height as i64)),
                ])),
            )?;
        }
        if refresh_hz > 0.0 {
            write("refresh_hz", toml::Value::Float(refresh_hz))?;
        }

        info!("display profile for {connector} saved; it applies at the next start");
        Ok(settings::Status::PendingRestart.wire_name().to_string())
    }

    /// Create a virtual output on the running compositor.
    ///
    /// Takes effect immediately — a virtual screen you have to restart for is
    /// no use for the thing it is mostly wanted for. Answers with the PipeWire
    /// node id the output streams to, which is what a capture client needs.
    /// `persist` also writes it to the configuration so it comes back next
    /// session.
    #[allow(clippy::too_many_arguments)]
    async fn add_virtual_output(
        &self,
        name: &str,
        width: u32,
        height: u32,
        refresh_hz: f64,
        interactive: bool,
        persist: bool,
    ) -> Result<u32, SettingsFault> {
        if name.trim().is_empty() {
            return Err(SettingsFault::OutOfRange(
                "a virtual output needs a name".to_string(),
            ));
        }
        if width == 0 || height == 0 {
            return Err(SettingsFault::OutOfRange(
                "a virtual output needs a non-zero size".to_string(),
            ));
        }

        let config = crate::config::VirtualOutputConfig {
            name: name.to_string(),
            resolution: crate::config::DisplayResolution { width, height },
            refresh_hz,
            position: None,
            interactive,
            // Runtime-created outputs never steal primary from the session
            // that is already running; that is a config-time decision.
            primary: false,
        };

        let (response_tx, response_rx) = oneshot::channel();
        self.compositor_tx
            .send(CompositorCommand::AddVirtualOutput {
                config: config.clone(),
                response_tx,
            })
            .map_err(|err| {
                SettingsFault::ApplyFailed(format!("compositor is not listening: {err}"))
            })?;

        let node_id = response_rx
            .await
            .map_err(|_| SettingsFault::ApplyFailed("compositor did not answer".to_string()))?
            .map_err(SettingsFault::ApplyFailed)?;

        // Persist only after it came up, for the same reason `Set` does:
        // a configuration that describes a failure is worse than none.
        if persist {
            if let Err(err) = crate::config::persist_virtual_output(&config) {
                info!("virtual output '{name}' is running but was not persisted: {err}");
            }
        }

        Ok(node_id)
    }

    /// Remove a virtual output, and drop it from the configuration too.
    async fn remove_virtual_output(&self, name: &str) -> Result<(), SettingsFault> {
        let (response_tx, response_rx) = oneshot::channel();
        self.compositor_tx
            .send(CompositorCommand::RemoveVirtualOutput {
                name: name.to_string(),
                response_tx,
            })
            .map_err(|err| {
                SettingsFault::ApplyFailed(format!("compositor is not listening: {err}"))
            })?;

        response_rx
            .await
            .map_err(|_| SettingsFault::ApplyFailed("compositor did not answer".to_string()))?
            .map_err(SettingsFault::ApplyFailed)?;

        if let Err(err) = crate::config::forget_virtual_output(name) {
            info!("virtual output '{name}' is gone but the config still lists it: {err}");
        }
        Ok(())
    }

    /// Emitted after any effective value changes, from any source. A client
    /// that called `Set` receives this too, and must not suppress its own echo.
    #[zbus(signal)]
    async fn changed(
        context: &SignalContext<'_>,
        values: HashMap<String, OwnedValue>,
    ) -> zbus::Result<()>;
}

/// Registers the Settings interface on the existing D-Bus connection.
pub async fn register_settings_interface(
    connection: &Connection,
    compositor_tx: smithay::reexports::calloop::channel::Sender<CompositorCommand>,
) -> zbus::Result<()> {
    connection
        .object_server()
        .at("/org/otto/Settings", SettingsInterface { compositor_tx })
        .await?;

    connection.request_name("org.otto.Settings").await?;

    // Announcements originate on the compositor thread, which cannot await, so
    // they are handed to a task on this connection's runtime instead.
    let (announce_tx, mut announce_rx) =
        tokio::sync::mpsc::unbounded_channel::<Vec<(String, SettingValue)>>();
    settings::set_announcer(Box::new(move |changes| {
        // A closed channel means the bus went away; the compositor carries on.
        let _ = announce_tx.send(changes);
    }));

    let connection = connection.clone();
    tokio::spawn(async move {
        while let Some(changes) = announce_rx.recv().await {
            let values: HashMap<String, OwnedValue> = changes
                .into_iter()
                .map(|(id, value)| (id, value.to_variant()))
                .collect();
            let iface = connection
                .object_server()
                .interface::<_, SettingsInterface>("/org/otto/Settings")
                .await;
            match iface {
                Ok(iface) => {
                    if let Err(err) =
                        SettingsInterface::changed(iface.signal_context(), values).await
                    {
                        tracing::warn!("Could not emit the settings Changed signal: {err}");
                    }
                }
                Err(err) => tracing::warn!("Settings interface went away: {err}"),
            }
        }
    });

    info!("Settings D-Bus interface registered at org.otto.Settings");

    Ok(())
}
