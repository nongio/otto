//! XDG color scheme detection via the freedesktop Settings portal.
//!
//! Queries `org.freedesktop.appearance color-scheme` on startup and watches for
//! `SettingChanged` signals, keeping a global atomic up to date.
//!
//! Any otto-kit app gets automatic light/dark switching via `AppContext::current_theme()`.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::LazyLock;
use zbus::zvariant::{OwnedValue, Value};

use crate::theme::ColorScheme;

/// Raw portal value stored atomically.  0 = no preference, 1 = dark, 2 = light.
static COLOR_SCHEME_VALUE: LazyLock<AtomicU32> = LazyLock::new(|| AtomicU32::new(0));

/// Read the current color scheme.
pub fn current_color_scheme() -> ColorScheme {
    ColorScheme::from_portal_value(COLOR_SCHEME_VALUE.load(Ordering::Relaxed))
}

/// Spawn a background tokio task that:
/// 1. Reads the initial `color-scheme` from the XDG Settings portal.
/// 2. Subscribes to `SettingChanged` and updates the atomic on every change.
///
/// Safe to call multiple times — only one watcher is ever active.
pub fn spawn_color_scheme_watcher() {
    use std::sync::atomic::AtomicBool;
    static STARTED: LazyLock<AtomicBool> = LazyLock::new(|| AtomicBool::new(false));
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    tokio::spawn(async move {
        if let Err(e) = run_watcher().await {
            tracing::warn!("color-scheme watcher stopped: {e}");
        }
    });
}

/// Extract u32 from a possibly variant-wrapped `Value`.
///
/// The XDG Settings portal wraps its return in `v` (variant), so the real u32
/// may be one or two levels deep.
fn extract_u32(val: Value<'_>) -> Option<u32> {
    match val {
        Value::U32(n) => Some(n),
        Value::Value(inner) => extract_u32(*inner),
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

    // Read initial value. The portal returns the value wrapped in a variant.
    match proxy
        .read("org.freedesktop.appearance", "color-scheme")
        .await
    {
        Ok(owned) => {
            // OwnedValue is Value<'static>; convert into Value then extract.
            let val: Value<'_> = owned.into();
            if let Some(v) = extract_u32(val) {
                COLOR_SCHEME_VALUE.store(v, Ordering::Relaxed);
                tracing::debug!("color-scheme initial value: {v}");
            }
        }
        Err(e) => tracing::debug!("color-scheme read failed (portal absent?): {e}"),
    }

    // Watch for changes via zbus signal stream.
    let mut stream = proxy.receive_setting_changed().await?;
    loop {
        use futures_util::StreamExt as _;
        let Some(signal) = stream.next().await else {
            break;
        };
        let args = signal.args()?;
        if args.namespace == "org.freedesktop.appearance" && args.key == "color-scheme" {
            if let Some(v) = extract_u32(args.value) {
                tracing::debug!("color-scheme changed to: {v}");
                COLOR_SCHEME_VALUE.store(v, Ordering::Relaxed);
            }
        }
    }

    Ok(())
}
