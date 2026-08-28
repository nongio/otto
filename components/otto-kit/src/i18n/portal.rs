//! Where a component's language comes from.
//!
//! Otto has its own "Preferred languages" setting, and it is not the same
//! thing as `LANG`. A user who sets Italian in settings while the session was
//! started with `LANG=en_GB` should get an Italian desktop, not an Italian
//! compositor bolted to English components.
//!
//! So the locale is read from the compositor, over the portal, under
//! `org.otto.desktop locales` — the same route [`crate::color_scheme`] and
//! [`crate::accent`] take for their settings. It falls back to the
//! environment whenever the portal cannot answer, which covers a component
//! started before the portal is up, a component run outside an Otto session,
//! and the tests.
//!
//! There is deliberately no watcher here. The other portal-backed settings
//! repaint on change; language cannot, because strings are handed out as
//! `&'static str` that live as long as the process. The setting is marked
//! `Restart` in the compositor's schema for exactly that reason.

use zbus::zvariant::{OwnedValue, Value};

/// Read the preferred locales from the compositor, falling back to the
/// environment.
///
/// Blocking, and called once before the first string is looked up — a
/// component cannot draw its interface until it knows what language it is in,
/// so there is nothing useful to do concurrently. The timeout keeps a missing
/// or wedged portal from holding up startup.
pub fn locales_blocking() -> Vec<String> {
    match read_from_portal() {
        Some(locales) if !locales.is_empty() => locales,
        _ => super::env_locales(),
    }
}

fn read_from_portal() -> Option<Vec<String>> {
    // On a thread of its own, with a short-lived runtime.
    //
    // The thread is not an optimisation. This runs from `main` before anything
    // is drawn, and Otto's components do not agree on what `main` is: otto-bar
    // is `#[tokio::main]`, so its calling thread is already driving a runtime,
    // while otto-settings and otto-files are synchronous. Building a runtime
    // on a thread that is already inside one panics rather than failing, so
    // doing this on the caller's thread works in two components and kills the
    // third at startup. A fresh thread belongs to no runtime, which makes the
    // one call correct from either kind of `main`.
    std::thread::spawn(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()?;

        runtime.block_on(async {
            // A portal that is absent answers by not answering. Two seconds is
            // long enough for a live one and short enough not to be felt.
            tokio::time::timeout(std::time::Duration::from_secs(2), query())
                .await
                .ok()
                .flatten()
        })
    })
    .join()
    .ok()
    .flatten()
}

async fn query() -> Option<Vec<String>> {
    use zbus::{proxy, Connection};

    #[proxy(
        interface = "org.freedesktop.portal.Settings",
        default_service = "org.freedesktop.portal.Desktop",
        default_path = "/org/freedesktop/portal/desktop"
    )]
    trait Settings {
        fn read(&self, namespace: &str, key: &str) -> zbus::Result<OwnedValue>;
    }

    let conn = Connection::session().await.ok()?;
    let proxy = SettingsProxy::new(&conn).await.ok()?;
    match proxy.read("org.otto.desktop", "locales").await {
        Ok(owned) => extract_strings(owned.into()),
        Err(err) => {
            tracing::debug!("locales read failed (portal absent?): {err}");
            None
        }
    }
}

/// Unwrap the array of strings, through however many variants the portal
/// nested it in — `Read` returns `a{sv}`-style doubly-wrapped values.
fn extract_strings(value: Value<'_>) -> Option<Vec<String>> {
    match value {
        Value::Array(array) => {
            let out: Vec<String> = array
                .iter()
                .filter_map(|item| match item {
                    Value::Str(s) => Some(s.as_str().to_string()),
                    _ => None,
                })
                .collect();
            (!out.is_empty()).then_some(out)
        }
        Value::Value(inner) => extract_strings(*inner),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::locales_blocking;

    /// The same answer whether or not a runtime is already running.
    ///
    /// `init_from_desktop` is called from `main`, and Otto's components do not
    /// agree on what `main` is: otto-bar, otto-islands and otto-quickview are
    /// `#[tokio::main]`, while otto-settings, otto-files, otto-launcher,
    /// otto-greeter and otto-lock are synchronous. Building a runtime on a
    /// thread that is already driving one panics outright — "Cannot start a
    /// runtime from within a runtime" — so a portal read done on the caller's
    /// thread works in five components and kills three at startup, which is
    /// how otto-bar came to fail to launch at all.
    ///
    /// Comparing against a call made off-runtime rather than merely asserting
    /// "did not panic": the thread hop is an implementation detail, and the
    /// point is that it does not change the answer.
    fn agrees_with_an_off_runtime_read() {
        let inside = locales_blocking();
        let outside = std::thread::spawn(locales_blocking).join().unwrap();
        assert_eq!(inside, outside);
    }

    /// The flavour `#[tokio::main]` gives a component by default.
    #[tokio::test(flavor = "multi_thread")]
    async fn reads_from_inside_a_multi_thread_runtime() {
        agrees_with_an_off_runtime_read();
    }

    /// And the single-threaded flavour, which a component can ask for.
    #[tokio::test(flavor = "current_thread")]
    async fn reads_from_inside_a_current_thread_runtime() {
        agrees_with_an_off_runtime_read();
    }

    /// The synchronous components, for completeness.
    #[test]
    fn reads_with_no_runtime_at_all() {
        agrees_with_an_off_runtime_read();
    }
}
