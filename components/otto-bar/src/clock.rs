//! The clock at the bar's right edge.
//!
//! Whether it is shown and how it is written are the compositor's settings
//! `topbar.show_clock` and `topbar.clock_format`, read over `org.otto.Settings`
//! and followed through its `Changed` signal, so a change in Settings lands at
//! once. `clock_format` in `otto-bar.toml` is the fallback for an empty
//! setting, and the language's own format the fallback for that.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use chrono::format::{Item, StrftimeItems};
use chrono::Local;
use futures_util::StreamExt;
use otto_kit::dbus::settings::SettingsProxy;
use otto_kit::prelude::AppContext;
use zbus::zvariant::Value;
use zbus::Connection;

use crate::config::file_clock_format;

/// The setting that shows or hides the clock.
const SHOW_ID: &str = "topbar.show_clock";
/// The setting that holds the chosen format. Empty means none is chosen.
const FORMAT_ID: &str = "topbar.clock_format";

/// The format used when neither the setting, the file nor the language gives
/// one chrono can render: hours and minutes is the least a clock can say.
const LAST_RESORT_FORMAT: &str = "%H:%M";

/// Whether the clock is drawn. Shown until the compositor says otherwise, so a
/// bar running without Otto still has a clock.
static SHOWN: AtomicBool = AtomicBool::new(true);
/// The format chosen in Settings, empty when none is.
static CHOSEN_FORMAT: Mutex<String> = Mutex::new(String::new());

/// Whether the clock is drawn at all.
pub fn shown() -> bool {
    SHOWN.load(Ordering::Relaxed)
}

/// Whether chrono can render `format`. A malformed format makes chrono's
/// `Display` fail, which `to_string` turns into a panic, so one is never used.
fn is_valid_format(format: &str) -> bool {
    !format.is_empty() && !StrftimeItems::new(format).any(|item| matches!(item, Item::Error))
}

/// The format the clock is written in: the one chosen in Settings, else the
/// one in `otto-bar.toml`, else the language's own.
pub fn clock_format() -> String {
    let chosen = CHOSEN_FORMAT.lock().unwrap().clone();
    pick_format(
        &chosen,
        file_clock_format(),
        otto_kit::t!("bar-clock-format"),
    )
}

/// The first of the three formats chrono can render.
fn pick_format(chosen: &str, file: &str, language: &str) -> String {
    [chosen, file, language]
        .into_iter()
        .find(|format| is_valid_format(format))
        .unwrap_or(LAST_RESORT_FORMAT)
        .to_string()
}

/// Minimal clock state — just the current formatted time string.
pub struct Clock {
    pub text: String,
}

impl Clock {
    pub fn new() -> Self {
        Self {
            text: Self::formatted_now(),
        }
    }

    /// Update the stored text. Returns `true` if the string changed.
    pub fn tick(&mut self) -> bool {
        let new = Self::formatted_now();
        if new != self.text {
            self.text = new;
            true
        } else {
            false
        }
    }

    fn formatted_now() -> String {
        if !shown() {
            return String::new();
        }
        // `format_localized` is what makes %A and %B come out in the user's
        // language; plain `format` renders English names whatever the locale.
        Local::now()
            .format_localized(&clock_format(), otto_kit::i18n::chrono_locale())
            .to_string()
    }

    /// Time until the clock text next changes: the next second boundary when
    /// the format shows seconds, otherwise the next minute boundary.
    pub fn until_next_change() -> std::time::Duration {
        use chrono::Timelike;
        let now = Local::now();
        let fmt = clock_format();
        let has_seconds = ["%S", "%T", "%X", "%r", "%s"]
            .iter()
            .any(|s| fmt.contains(s));
        // nanosecond() can exceed 1e9 during a leap second — clamp via modulo.
        let to_next_second = 1_000_000_000 - (now.nanosecond() % 1_000_000_000) as u64;
        let ns = if has_seconds {
            to_next_second
        } else {
            (59 - now.second().min(59)) as u64 * 1_000_000_000 + to_next_second
        };
        // +25ms so the wake lands safely past the boundary.
        std::time::Duration::from_nanos(ns) + std::time::Duration::from_millis(25)
    }
}

/// Follow the clock settings. Safe to call repeatedly: only one watcher runs.
pub fn spawn_watcher() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async move {
        if let Err(e) = run_watcher().await {
            tracing::warn!("clock settings: {e}");
        }
    });
}

async fn run_watcher() -> zbus::Result<()> {
    let conn = Connection::session().await?;
    let proxy = SettingsProxy::new(&conn).await?;
    // Subscribed before the first read, so a change landing between the two
    // is not lost.
    let mut changes = proxy.receive_changed().await?;
    // A compositor that starts after the bar, or restarts under it, is asked
    // again.
    let mut owners = proxy.inner().receive_owner_changed().await?;

    read_settings(&proxy).await;
    loop {
        tokio::select! {
            Some(signal) = changes.next() => {
                let ours = signal.args().is_ok_and(|args| {
                    args.values.contains_key(SHOW_ID) || args.values.contains_key(FORMAT_ID)
                });
                if ours {
                    read_settings(&proxy).await;
                }
            }
            Some(owner) = owners.next() => {
                if owner.is_some() {
                    read_settings(&proxy).await;
                }
            }
            else => break,
        }
    }
    Ok(())
}

/// Read both settings back and wake the bar if either moved. The signal
/// carries identifiers rather than values, so the values are asked for.
async fn read_settings(proxy: &SettingsProxy<'_>) {
    let mut moved = false;
    match proxy.get(SHOW_ID).await {
        Ok(owned) => {
            if let Value::Bool(show) = unwrap_variant(owned.into()) {
                moved |= SHOWN.swap(show, Ordering::Relaxed) != show;
            }
        }
        Err(e) => tracing::debug!("{SHOW_ID} read failed (no compositor?): {e}"),
    }
    match proxy.get(FORMAT_ID).await {
        Ok(owned) => {
            if let Value::Str(format) = unwrap_variant(owned.into()) {
                let mut chosen = CHOSEN_FORMAT.lock().unwrap();
                if chosen.as_str() != format.as_str() {
                    *chosen = format.to_string();
                    moved = true;
                }
            }
        }
        Err(e) => tracing::debug!("{FORMAT_ID} read failed (no compositor?): {e}"),
    }
    if moved {
        AppContext::request_wakeup();
    }
}

/// Unwrap the variant a `Get` answer comes in, however deep.
fn unwrap_variant(value: Value<'_>) -> Value<'_> {
    match value {
        Value::Value(inner) => unwrap_variant(*inner),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_malformed_format_is_never_used() {
        assert!(is_valid_format("%a %H:%M"));
        assert!(!is_valid_format("%Q %H"));
        assert!(!is_valid_format(""));
    }

    #[test]
    fn the_chosen_format_wins_then_the_file_then_the_language() {
        assert_eq!(pick_format("%H:%M", "%A", "%B"), "%H:%M");
        assert_eq!(pick_format("", "%A", "%B"), "%A");
        assert_eq!(pick_format("%Q", "", "%B"), "%B");
        assert_eq!(pick_format("", "", ""), LAST_RESORT_FORMAT);
    }
}
