//! Whether the desktop is light or dark, from two sources.
//!
//! The freedesktop Settings portal is the right channel and the first one
//! consulted: it is what third-party apps read, and it reports live changes.
//! But it is optional — a session without `xdg-desktop-portal-otto` running
//! answers nothing, and every otto-kit window used to fall back to light on a
//! dark desktop.
//!
//! So the compositor also publishes its configured `theme_scheme` in the
//! environment, the way [`crate::corners`] publishes corner rounding, and that
//! is what this module falls back to. The portal always wins when it has an
//! answer, so a later, more authoritative reply is never clobbered by the
//! value a process inherited when it started.
//!
//! The environment half is read once and never changes: it is the value this
//! process was started with. `theme_scheme` does apply live, and a change to it
//! reaches a running application over the portal alone.
//!
//! Any otto-kit app gets light/dark via `AppContext::current_theme()`.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::LazyLock;
use zbus::zvariant::{OwnedValue, Value};

use crate::theme::ColorScheme;

/// The variable the compositor publishes: `dark`, `light`, or absent.
pub const ENV: &str = "OTTO_COLOR_SCHEME";

/// Raw portal value stored atomically.  0 = no preference (or no answer yet),
/// 1 = dark, 2 = light.
static COLOR_SCHEME_VALUE: LazyLock<AtomicU32> = LazyLock::new(|| AtomicU32::new(0));

/// The compositor's configured scheme, in the same encoding, with `u32::MAX`
/// meaning "the environment has not been looked at yet".
static CONFIGURED: AtomicU32 = AtomicU32::new(u32::MAX);

/// Read the current color scheme.
///
/// The portal's answer where there is one; otherwise the scheme the
/// compositor configured.
pub fn current_color_scheme() -> ColorScheme {
    match COLOR_SCHEME_VALUE.load(Ordering::Relaxed) {
        0 => ColorScheme::from_portal_value(configured()),
        v => ColorScheme::from_portal_value(v),
    }
}

/// The compositor's scheme, read from the environment on first use.
fn configured() -> u32 {
    match CONFIGURED.load(Ordering::Relaxed) {
        u32::MAX => {
            let value = match std::env::var(ENV) {
                Ok(text) => match text.trim().to_ascii_lowercase().as_str() {
                    "dark" => 1,
                    "light" => 2,
                    _ => 0,
                },
                Err(_) => 0,
            };
            CONFIGURED.store(value, Ordering::Relaxed);
            value
        }
        value => value,
    }
}

/// Publish `scheme` to this process and everything it starts, and return the
/// assignment for the session's activation environments — a bus-activated
/// helper is not a child of the compositor and inherits nothing from it. See
/// [`crate::corners::export`].
///
/// Only the compositor calls this: it holds the configuration, and its own
/// drawing takes the palette straight from it.
pub fn export(scheme: ColorScheme) -> String {
    let (value, text) = match scheme {
        ColorScheme::Dark => (1, "dark"),
        _ => (2, "light"),
    };
    CONFIGURED.store(value, Ordering::Relaxed);
    std::env::set_var(ENV, text);
    format!("{ENV}={text}")
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

    crate::portal_runtime::spawn("color-scheme-watcher", async move {
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
                crate::portal_runtime::theme_changed();
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
                crate::portal_runtime::theme_changed();
            }
        }
    }

    Ok(())
}
