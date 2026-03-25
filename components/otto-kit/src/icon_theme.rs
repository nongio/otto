//! Icon theme detection via the freedesktop Settings portal.
//!
//! Queries `org.freedesktop.appearance icon-theme` on startup and watches for
//! `SettingChanged` signals, keeping a global string up to date.
//!
//! Any otto-kit app gets the compositor's configured icon theme via
//! `current_icon_theme()`.

use std::sync::{LazyLock, RwLock};
use zbus::zvariant::{OwnedValue, Value};

/// The current icon theme name. Empty string means auto-detect / no preference.
static ICON_THEME: LazyLock<RwLock<String>> = LazyLock::new(|| RwLock::new(String::new()));

/// Read the current icon theme name.
///
/// Returns `None` if no theme has been configured (empty string from portal).
pub fn current_icon_theme() -> Option<String> {
    let theme = ICON_THEME.read().unwrap();
    if theme.is_empty() {
        None
    } else {
        Some(theme.clone())
    }
}

/// Spawn a background tokio task that:
/// 1. Reads the initial `icon-theme` from the XDG Settings portal.
/// 2. Subscribes to `SettingChanged` and updates the value on every change.
///
/// Safe to call multiple times — only one watcher is ever active.
pub fn spawn_icon_theme_watcher() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static STARTED: LazyLock<AtomicBool> = LazyLock::new(|| AtomicBool::new(false));
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    tokio::spawn(async move {
        if let Err(e) = run_watcher().await {
            tracing::warn!("icon-theme watcher stopped: {e}");
        }
    });
}

/// Extract a string from a possibly variant-wrapped `Value`.
fn extract_string(val: Value<'_>) -> Option<String> {
    match val {
        Value::Str(s) => Some(s.to_string()),
        Value::Value(inner) => extract_string(*inner),
        _ => None,
    }
}

async fn run_watcher() -> Result<(), zbus::Error> {
    use zbus::{proxy, Connection};

    #[proxy(
        interface = "org.freedesktop.portal.Settings",
        default_service = "org.freedesktop.portal.Desktop",
        default_path = "/org/freedesktop/portal/desktop"
    )]
    trait Settings {
        fn read(&self, namespace: &str, key: &str) -> zbus::Result<OwnedValue>;
        #[zbus(signal)]
        fn setting_changed(&self, namespace: &str, key: &str, value: Value<'_>)
            -> zbus::Result<()>;
    }

    let conn = Connection::session().await?;
    let proxy = SettingsProxy::new(&conn).await?;

    // Read initial value.
    match proxy.read("org.freedesktop.appearance", "icon-theme").await {
        Ok(owned) => {
            let val: Value<'_> = owned.into();
            if let Some(theme) = extract_string(val) {
                tracing::debug!("icon-theme initial value: {theme}");
                *ICON_THEME.write().unwrap() = theme;
            }
        }
        Err(e) => tracing::debug!("icon-theme read failed (portal absent?): {e}"),
    }

    // Watch for changes via zbus signal stream.
    let mut stream = proxy.receive_setting_changed().await?;
    loop {
        use futures_util::StreamExt as _;
        let Some(signal) = stream.next().await else {
            break;
        };
        let args = signal.args()?;
        if args.namespace == "org.freedesktop.appearance" && args.key == "icon-theme" {
            if let Some(theme) = extract_string(args.value) {
                tracing::debug!("icon-theme changed to: {theme}");
                *ICON_THEME.write().unwrap() = theme;
            }
        }
    }

    Ok(())
}
