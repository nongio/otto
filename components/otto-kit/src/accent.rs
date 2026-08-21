//! Accent colour detection via the freedesktop Settings portal.
//!
//! Queries `org.freedesktop.appearance accent-color` on startup and watches for
//! `SettingChanged`, keeping a global up to date. `Theme::for_scheme` folds the
//! result in, so any otto-kit app follows the user's accent without asking.
//!
//! The sibling of `color_scheme`, and started by the same call.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::LazyLock;

use skia_safe::Color;
use zbus::zvariant::{OwnedValue, Value};

/// The accent as ARGB, or 0 when the portal has not answered (yet or ever) —
/// a fully transparent accent is not a colour any palette contains, so it
/// doubles as "no value" without a second atomic.
static ACCENT_ARGB: LazyLock<AtomicU32> = LazyLock::new(|| AtomicU32::new(0));

/// The user's accent colour, or `None` when the portal did not supply one.
pub fn current_accent() -> Option<Color> {
    match ACCENT_ARGB.load(Ordering::Relaxed) {
        0 => None,
        argb => Some(Color::from(argb)),
    }
}

/// Set the accent directly, for a process that already knows it.
///
/// The compositor resolves the accent from its own configuration and *serves*
/// `org.freedesktop.appearance accent-color`; asking the portal for it would
/// be Otto querying itself. It writes the value here instead, so the store
/// this module keeps is the single accent every otto-kit drawing routine
/// reads — window decorations included — whether it was filled by the
/// compositor or by the watcher below.
pub fn set_accent(color: Color) {
    store(color);
}

/// Start the background task that reads `accent-color` and then follows it.
///
/// Safe to call multiple times — only one watcher is ever active.
pub fn spawn_accent_watcher() {
    use std::sync::atomic::AtomicBool;
    static STARTED: LazyLock<AtomicBool> = LazyLock::new(|| AtomicBool::new(false));
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    crate::portal_runtime::spawn("accent-watcher", async move {
        if let Err(e) = run_watcher().await {
            tracing::warn!("accent-color watcher stopped: {e}");
        }
    });
}

/// Extract `(ddd)` from a possibly variant-wrapped `Value`.
///
/// The portal wraps its return in a variant, so the struct may be a level or
/// two deep — the same shape `color_scheme` has to unwrap.
fn extract_accent(val: Value<'_>) -> Option<Color> {
    match val {
        Value::Value(inner) => extract_accent(*inner),
        Value::Structure(fields) => {
            let mut components = fields.fields().iter().filter_map(|f| match f {
                Value::F64(v) => Some(*v),
                _ => None,
            });
            let r = components.next()?;
            let g = components.next()?;
            let b = components.next()?;
            let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            Some(Color::from_argb(0xFF, byte(r), byte(g), byte(b)))
        }
        _ => None,
    }
}

/// Store the accent and wake the run loop, so a change lands on the next
/// frame rather than whenever the user next happens to touch the window.
fn store(color: Color) {
    let argb = (u32::from(color.a()) << 24)
        | (u32::from(color.r()) << 16)
        | (u32::from(color.g()) << 8)
        | u32::from(color.b());
    ACCENT_ARGB.store(argb, Ordering::Relaxed);
    crate::portal_runtime::theme_changed();
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

    match proxy
        .read("org.freedesktop.appearance", "accent-color")
        .await
    {
        Ok(owned) => {
            let val: Value<'_> = owned.into();
            if let Some(color) = extract_accent(val) {
                tracing::debug!("accent-color initial value: {color:?}");
                store(color);
            }
        }
        Err(e) => tracing::debug!("accent-color read failed (portal absent?): {e}"),
    }

    let mut stream = proxy.receive_setting_changed().await?;
    loop {
        use futures_util::StreamExt as _;
        let Some(signal) = stream.next().await else {
            break;
        };
        let args = signal.args()?;
        if args.namespace == "org.freedesktop.appearance" && args.key == "accent-color" {
            if let Some(color) = extract_accent(args.value) {
                tracing::debug!("accent-color changed to: {color:?}");
                store(color);
            }
        }
    }

    Ok(())
}
