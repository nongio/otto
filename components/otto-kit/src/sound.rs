//! Event sounds, from the XDG sound theme.
//!
//! One player for the whole desktop: the compositor plays its volume clicks
//! and its lock chime through this, and so does every otto-kit app, so a drop
//! in the file browser and a lock of the screen come from the same theme and
//! sound like the same system.
//!
//! Lookup follows the XDG Sound Theme spec — an extra search directory first
//! (the compositor points this at its own resources, so a bundled sound can
//! override a themed one), then the configured theme, then what is installed,
//! then `freedesktop`. Results are cached, misses included, because a miss
//! costs a walk of every theme directory and would otherwise be paid on every
//! keystroke of a volume key.
//!
//! Who sets the theme: the compositor from its own config, and an app from
//! whatever the compositor publishes over the settings portal. An app that
//! sets nothing still gets sound — the auto-detected theme — it just does not
//! follow the desktop's choice.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, Instant};

/// The same sound twice inside this window is one sound. A held volume key or
/// a drag that ends in a flurry of drops should tick, not buzz.
const MIN_INTERVAL: Duration = Duration::from_millis(100);

/// Extensions to try, in preference order.
const EXTENSIONS: [&str; 4] = ["oga", "ogg", "wav", "flac"];

static ENABLED: AtomicBool = AtomicBool::new(true);
static THEME: LazyLock<RwLock<Option<String>>> = LazyLock::new(|| RwLock::new(None));
static EXTRA_DIRS: LazyLock<RwLock<Vec<PathBuf>>> = LazyLock::new(|| RwLock::new(Vec::new()));
static CACHE: LazyLock<RwLock<HashMap<String, Option<PathBuf>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
static LAST_PLAY: LazyLock<RwLock<HashMap<String, Instant>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Turn event sounds on or off for this process.
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Choose the theme to play from. `None` auto-detects.
///
/// Clears the cache: the same event name resolves to a different file now.
pub fn set_theme(theme: Option<String>) {
    let mut current = THEME.write().unwrap();
    if *current == theme {
        return;
    }
    *current = theme;
    CACHE.write().unwrap().clear();
}

pub fn theme() -> Option<String> {
    THEME.read().unwrap().clone()
}

/// Directories searched before any theme, in order — for sounds shipped with
/// the application rather than installed as a theme.
pub fn set_extra_search_dirs(dirs: Vec<PathBuf>) {
    *EXTRA_DIRS.write().unwrap() = dirs;
    CACHE.write().unwrap().clear();
}

/// Play the theme's sound for `event`, by its
/// [sound naming spec](https://specifications.freedesktop.org/sound-naming-spec/)
/// name — `audio-volume-change`, `trash-empty`, `desktop-screen-lock`.
///
/// Returns immediately; the sound plays on a thread of its own. A name no
/// installed theme has is silence, not an error: theme coverage varies, and an
/// event without a sound is a normal outcome rather than something to report.
pub fn play_event(event: &str) {
    if !enabled() {
        return;
    }

    {
        let mut last = LAST_PLAY.write().unwrap();
        if let Some(at) = last.get(event) {
            if at.elapsed() < MIN_INTERVAL {
                return;
            }
        }
        last.insert(event.to_string(), Instant::now());
    }

    if let Some(cached) = CACHE.read().unwrap().get(event) {
        if let Some(path) = cached {
            play_file(path);
        }
        return;
    }

    let found = find_event(event);
    CACHE
        .write()
        .unwrap()
        .insert(event.to_string(), found.clone());
    if let Some(path) = found {
        play_file(&path);
    } else {
        tracing::debug!(event, "no sound in any installed theme");
    }
}

/// Play the first of `events` any installed theme has a sound for.
///
/// The naming spec is thinner than a desktop needs — there is no "paste", for
/// one — and coverage differs between themes, so most callers have a preferred
/// name and a more common one to fall back on.
pub fn play_first(events: &[&str]) {
    if !enabled() {
        return;
    }
    for event in events {
        if find_cached(event).is_some() {
            play_event(event);
            return;
        }
    }
}

/// Resolve `events` now and remember the answers, so the first play of one is
/// not also the first walk of every theme directory on disk.
pub fn prewarm(events: &[&str]) {
    let events: Vec<String> = events.iter().map(|e| e.to_string()).collect();
    std::thread::spawn(move || {
        for event in events {
            find_cached(&event);
        }
    });
}

/// Play one file directly, off the theme.
pub fn play_file(path: &Path) {
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        let status = std::process::Command::new("pw-cat")
            .arg("--playback")
            .arg(&path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        match status {
            Ok(status) if status.success() => {}
            Ok(_) => tracing::debug!(?path, "pw-cat could not play the sound"),
            Err(err) => tracing::debug!(?path, %err, "could not run pw-cat"),
        }
    });
}

/// Resolve `event` to a file, remembering the answer — including "nothing".
fn find_cached(event: &str) -> Option<PathBuf> {
    if let Some(cached) = CACHE.read().unwrap().get(event) {
        return cached.clone();
    }
    let found = find_event(event);
    CACHE
        .write()
        .unwrap()
        .insert(event.to_string(), found.clone());
    found
}

fn find_event(event: &str) -> Option<PathBuf> {
    for dir in EXTRA_DIRS.read().unwrap().iter() {
        for ext in EXTENSIONS {
            let path = dir.join(format!("{event}.{ext}"));
            if path.exists() {
                return Some(path);
            }
        }
    }

    if let Some(theme) = theme() {
        if let Some(path) = find_in_theme(event, &theme) {
            return Some(path);
        }
    }

    for theme in detect_themes() {
        if let Some(path) = find_in_theme(event, &theme) {
            return Some(path);
        }
    }

    // The one theme the spec requires every system to have.
    find_in_theme(event, "freedesktop")
}

/// Look `event` up inside one theme.
///
/// The spec's layout is `<base>/<theme>/<profile>/<event>.<ext>`, but themes
/// take liberties with the profile directory — Pop files its events under
/// `stereo/action`, `stereo/alert` and `stereo/notification` — so the likely
/// ones are tried rather than only `stereo`.
fn find_in_theme(event: &str, theme: &str) -> Option<PathBuf> {
    const PROFILES: [&str; 5] = [
        "stereo",
        "stereo/action",
        "stereo/alert",
        "stereo/notification",
        "",
    ];

    for base in base_dirs() {
        let theme_dir = base.join(theme);
        for profile in PROFILES {
            let dir = if profile.is_empty() {
                theme_dir.clone()
            } else {
                theme_dir.join(profile)
            };
            for ext in EXTENSIONS {
                let path = dir.join(format!("{event}.{ext}"));
                if path.exists() {
                    return Some(path);
                }
            }
        }
    }
    None
}

/// Where themes are installed, user directory first so it can override.
fn base_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/sounds"));
    }
    dirs.push(PathBuf::from("/usr/local/share/sounds"));
    dirs.push(PathBuf::from("/usr/share/sounds"));
    dirs
}

/// Themes worth trying when none is configured, best guess first.
fn detect_themes() -> Vec<String> {
    let mut themes = Vec::new();
    if let Ok(desktop) = std::env::var("XDG_CURRENT_DESKTOP") {
        match desktop.to_lowercase().as_str() {
            "gnome" => themes.push("Yaru".to_string()),
            "kde" | "plasma" => themes.push("ocean".to_string()),
            "pop" => themes.push("Pop".to_string()),
            _ => {}
        }
    }
    themes.extend(["Pop", "Yaru", "ocean"].map(String::from));
    themes
}

/// Follow the compositor's sound theme, over the settings portal.
///
/// `org.gnome.desktop.sound theme-name` rather than something under
/// `org.freedesktop.appearance`: the appearance namespace has no sound key,
/// and this is the one GTK and libcanberra already read, so Otto publishing it
/// serves every app on the desktop rather than only ours.
///
/// Safe to call more than once — only one watcher ever runs. An app that never
/// calls it still has sound, just not the desktop's chosen theme.
pub fn spawn_theme_watcher() {
    use std::sync::atomic::AtomicBool;
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    crate::portal_runtime::spawn("sound-theme-watcher", async move {
        if let Err(err) = watch_theme().await {
            tracing::debug!("sound-theme watcher stopped: {err}");
        }
    });
}

async fn watch_theme() -> Result<(), zbus::Error> {
    use zbus::zvariant::{OwnedValue, Value};
    use zbus::{proxy, Connection};

    const NAMESPACE: &str = "org.gnome.desktop.sound";
    const KEY: &str = "theme-name";

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

    fn as_string(value: Value<'_>) -> Option<String> {
        match value {
            Value::Str(s) => Some(s.to_string()),
            Value::Value(inner) => as_string(*inner),
            _ => None,
        }
    }

    // An empty name is "no preference", which is `None` here, not a theme
    // called "".
    fn apply(name: String) {
        set_theme(Some(name).filter(|n| !n.is_empty()));
    }

    let conn = Connection::session().await?;
    let proxy = SettingsProxy::new(&conn).await?;

    match proxy.read(NAMESPACE, KEY).await {
        Ok(owned) => {
            if let Some(name) = as_string(owned.into()) {
                tracing::debug!(name, "sound theme");
                apply(name);
            }
        }
        Err(err) => tracing::debug!("sound-theme read failed (portal absent?): {err}"),
    }

    let mut stream = proxy.receive_setting_changed().await?;
    loop {
        use futures_util::StreamExt as _;
        let Some(signal) = stream.next().await else {
            break;
        };
        let args = signal.args()?;
        if args.namespace == NAMESPACE && args.key == KEY {
            if let Some(name) = as_string(args.value) {
                tracing::debug!(name, "sound theme changed");
                apply(name);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lookup has to survive a name nothing has, and say so by returning
    /// nothing rather than by picking something arbitrary.
    #[test]
    fn an_unknown_event_resolves_to_nothing() {
        assert_eq!(
            find_in_theme("otto-no-such-event-9f3a", "freedesktop"),
            None
        );
    }

    /// A custom directory wins over every installed theme — that is what makes
    /// it useful for a bundled sound that replaces a themed one.
    #[test]
    fn an_extra_directory_is_searched_first() {
        let dir = std::env::temp_dir().join(format!("otto-kit-sound-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let bundled = dir.join("bell.oga");
        std::fs::write(&bundled, b"not really an ogg").expect("temp sound");

        set_extra_search_dirs(vec![dir.clone()]);
        let found = find_event("bell");
        set_extra_search_dirs(Vec::new());
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(found.as_deref(), Some(bundled.as_path()));
    }

    /// Setting the same theme twice must not be treated as a change: it clears
    /// the cache, and a caller that re-publishes the theme on every portal
    /// signal would otherwise throw the lookups away over and over.
    #[test]
    fn setting_the_theme_it_already_has_is_not_a_change() {
        set_theme(Some("freedesktop".to_string()));
        CACHE
            .write()
            .unwrap()
            .insert("probe".to_string(), Some(PathBuf::from("/probe.oga")));

        set_theme(Some("freedesktop".to_string()));
        assert!(CACHE.read().unwrap().contains_key("probe"), "cache dropped");

        set_theme(None);
        assert!(!CACHE.read().unwrap().contains_key("probe"), "cache kept");
    }
}
