//! The three appearance settings that are Otto's own, followed live.
//!
//! Corner rounding, which end of the titlebar the window controls sit at, and
//! whether the zoom dot is drawn are read from the environment on first use
//! (see [`crate::corners`]) because that is the one channel every child and
//! every bus-activated helper inherits. But an environment variable is a value
//! a process was *started* with: change the setting and every window already on
//! screen keeps drawing the old answer until it is restarted.
//!
//! So the compositor announces them, and this module follows that the way
//! [`crate::color_scheme`] follows the colour scheme: it updates the same
//! atomics the environment seeded, then bumps the theme generation so the run
//! loop hands the app an `on_theme_changed`.
//!
//! Two watchers, because there are two channels and they fail in different
//! ways. `org.otto.Settings` is the compositor's own interface, and an
//! otto-kit application is talking to Otto by definition — one hop, nothing
//! in between, and it keeps working in a session with no portal, or with a
//! portal older than the compositor. The Settings portal is the channel every
//! *other* toolkit reads, under Otto's own namespace since the freedesktop
//! `appearance` namespace has no key for any of these; following it too means
//! a portal that answers before the compositor does is not ignored.
//!
//! Both write the same atomics from the same source of truth, so whichever
//! arrives first wins and the other is a no-op. Where neither answers, the
//! environment is still there and nothing changes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;

use zbus::zvariant::{OwnedValue, Value};

use crate::controls_side::ControlsSide;

/// Otto's own portal namespace, where settings with no freedesktop key live.
const NAMESPACE: &str = "org.otto.desktop";

const ROUNDED_CORNERS: &str = "rounded-corners";
const WINDOW_CONTROLS_SIDE: &str = "window-controls-side";
const MAXIMIZE_BUTTON: &str = "maximize-button";

/// Spawn the watcher. Safe to call repeatedly — only one is ever active.
pub fn spawn_desktop_appearance_watcher() {
    static STARTED: LazyLock<AtomicBool> = LazyLock::new(|| AtomicBool::new(false));
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    crate::portal_runtime::spawn("desktop-appearance-watcher", async move {
        if let Err(e) = run_watcher().await {
            tracing::warn!("desktop-appearance watcher stopped: {e}");
        }
    });
    crate::portal_runtime::spawn("otto-settings-watcher", async move {
        if let Err(e) = run_compositor_watcher().await {
            tracing::warn!("otto-settings watcher stopped: {e}");
        }
    });
}

/// Otto's own identifier for each key.
///
/// The compositor names its settings; the portal keys are those names
/// translated into a namespace other toolkits read. Going straight to the
/// compositor means going back to the names.
const OTTO_IDS: &[(&str, &str)] = &[
    ("rounded_corners", ROUNDED_CORNERS),
    ("window_controls_side", WINDOW_CONTROLS_SIDE),
    ("show_maximize_button", MAXIMIZE_BUTTON),
];

/// Follow `org.otto.Settings` directly: read the three settings, then keep up
/// with the compositor's `Changed` signal.
///
/// The signal carries the identifiers that moved, not their values, so each
/// one that interests us is read back — the same shape the portal's own relay
/// uses.
async fn run_compositor_watcher() -> Result<(), zbus::Error> {
    use zbus::{proxy, Connection};

    #[proxy(
        interface = "org.otto.Settings",
        default_service = "org.otto.Settings",
        default_path = "/org/otto/Settings"
    )]
    trait OttoSettings {
        fn get(&self, id: &str) -> zbus::Result<OwnedValue>;
        #[zbus(signal)]
        fn changed(
            &self,
            values: std::collections::HashMap<String, OwnedValue>,
        ) -> zbus::Result<()>;
    }

    let conn = Connection::session().await?;
    let proxy = OttoSettingsProxy::new(&conn).await?;
    // Subscribed before the first read, so a change landing between the two is
    // seen rather than falling into the gap.
    let mut stream = proxy.receive_changed().await?;

    let mut changed = false;
    for (id, key) in OTTO_IDS {
        match proxy.get(id).await {
            Ok(owned) => changed |= apply(key, owned.into()),
            Err(e) => tracing::debug!("org.otto.Settings {id} read failed (no compositor?): {e}"),
        }
    }
    if changed {
        crate::portal_runtime::theme_changed();
    }

    loop {
        use futures_util::StreamExt as _;
        let Some(signal) = stream.next().await else {
            break;
        };
        let args = signal.args()?;
        let mut changed = false;
        for (id, key) in OTTO_IDS {
            if !args.values.contains_key(*id) {
                continue;
            }
            match proxy.get(id).await {
                Ok(owned) => changed |= apply(key, owned.into()),
                Err(e) => tracing::debug!("org.otto.Settings {id} read failed: {e}"),
            }
        }
        if changed {
            tracing::debug!("org.otto.Settings appearance changed");
            crate::portal_runtime::theme_changed();
        }
    }

    Ok(())
}

/// Unwrap the variant the portal wraps its answers in, however deep.
fn unwrap_variant(value: Value<'_>) -> Value<'_> {
    match value {
        Value::Value(inner) => unwrap_variant(*inner),
        other => other,
    }
}

/// Store one key's value, returning whether it was one we follow.
fn apply(key: &str, value: Value<'_>) -> bool {
    match (key, unwrap_variant(value)) {
        (ROUNDED_CORNERS, Value::Bool(rounded)) => {
            crate::corners::set(rounded);
            true
        }
        (MAXIMIZE_BUTTON, Value::Bool(shown)) => {
            crate::maximize_button::set(shown);
            true
        }
        (WINDOW_CONTROLS_SIDE, Value::Str(side)) => match ControlsSide::parse(&side) {
            Some(side) => {
                crate::controls_side::set(side);
                true
            }
            // A token this build does not know is not a reason to move the
            // controls somewhere the compositor did not ask for.
            None => false,
        },
        _ => false,
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

    let mut changed = false;
    for key in [ROUNDED_CORNERS, WINDOW_CONTROLS_SIDE, MAXIMIZE_BUTTON] {
        match proxy.read(NAMESPACE, key).await {
            Ok(owned) => changed |= apply(key, owned.into()),
            Err(e) => tracing::debug!("{NAMESPACE} {key} read failed (portal absent?): {e}"),
        }
    }
    if changed {
        crate::portal_runtime::theme_changed();
    }

    let mut stream = proxy.receive_setting_changed().await?;
    loop {
        use futures_util::StreamExt as _;
        let Some(signal) = stream.next().await else {
            break;
        };
        let args = signal.args()?;
        if args.namespace != NAMESPACE {
            continue;
        }
        if apply(args.key, args.value) {
            tracing::debug!("{NAMESPACE} {} changed", args.key);
            crate::portal_runtime::theme_changed();
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The portal wraps its answers in a variant, sometimes more than once,
    /// and a value that arrives still wrapped matches none of the arms in
    /// [`apply`] — which reads as the setting silently not applying.
    #[test]
    fn a_wrapped_value_is_unwrapped_to_the_value_itself() {
        let wrapped = |value| Value::Value(Box::new(value));
        assert_eq!(
            unwrap_variant(wrapped(Value::Bool(true))),
            Value::Bool(true)
        );
        assert_eq!(
            unwrap_variant(wrapped(wrapped(Value::Bool(true)))),
            Value::Bool(true)
        );
        // A value that was never wrapped comes back as it went in.
        assert_eq!(unwrap_variant(Value::Bool(false)), Value::Bool(false));
    }

    /// The compositor's channel and the portal's carry the same three
    /// settings. A key followed on one and not the other is a setting that
    /// applies live in some sessions and not others, which is worse than not
    /// applying live at all.
    #[test]
    fn both_channels_follow_the_same_settings() {
        let mut direct: Vec<_> = OTTO_IDS.iter().map(|(_, key)| *key).collect();
        direct.sort_unstable();
        let mut portal = vec![ROUNDED_CORNERS, WINDOW_CONTROLS_SIDE, MAXIMIZE_BUTTON];
        portal.sort_unstable();
        assert_eq!(direct, portal);
    }

    /// A key this build does not follow, and a side token it does not know,
    /// both leave the stored answer alone rather than guessing at one.
    #[test]
    fn an_unknown_key_or_token_changes_nothing() {
        assert!(!apply("something-else", Value::from(true)));
        assert!(!apply(WINDOW_CONTROLS_SIDE, Value::from("sideways")));
        // Right type, wrong shape: a side is a string, not a boolean.
        assert!(!apply(WINDOW_CONTROLS_SIDE, Value::from(true)));
        assert!(!apply(ROUNDED_CORNERS, Value::from("yes")));
    }
}
