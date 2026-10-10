//! Icon theme detection.
//!
//! Otto publishes its icon theme as `org.freedesktop.appearance icon-theme` on
//! the settings portal. That key is Otto's own: neither the KDE nor the GNOME
//! portal sets it, and an empty theme name makes every lookup search `hicolor`
//! alone — which ships no `folder` and no mimetype icons, so a file listing
//! draws without any. Under another desktop the theme is taken from where that
//! desktop keeps it instead:
//!
//! 1. the portal's `org.freedesktop.appearance icon-theme` (Otto);
//! 2. the portal's desktop namespaces — `org.gnome.desktop.interface
//!    icon-theme` (GNOME), `org.kde.kdeglobals.Icons Theme` (Plasma);
//! 3. the files those desktops write — `kdeglobals`, GTK's `settings.ini` —
//!    read synchronously at startup, so the first frame already has icons;
//! 4. the first of Breeze or Adwaita that is installed.
//!
//! A theme is only taken if it is actually installed. Any otto-kit app gets the
//! result through `current_icon_theme()`.

use std::path::PathBuf;
use std::sync::{LazyLock, RwLock};
use zbus::zvariant::Value;

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

/// Replace the theme, dropping every icon resolved against the old one.
fn set_theme(theme: String) {
    {
        let mut current = ICON_THEME.write().unwrap();
        if *current == theme {
            return;
        }
        *current = theme;
    }
    // Every cached icon — and every cached miss — answers for the theme that
    // has just been replaced.
    crate::icons::clear_cache();
    crate::portal_runtime::theme_changed();
}

/// Settle on a theme from the desktop's own files, and spawn a background task
/// that:
/// 1. Reads the theme from the XDG Settings portal.
/// 2. Subscribes to `SettingChanged` and updates the value on every change.
///
/// Safe to call multiple times — only one watcher is ever active.
pub fn spawn_icon_theme_watcher() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static STARTED: LazyLock<AtomicBool> = LazyLock::new(|| AtomicBool::new(false));
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    // Before the portal answers — and in place of it, where there is none.
    if current_icon_theme().is_none() {
        if let Some(theme) = desktop_theme() {
            tracing::debug!("icon-theme from the desktop's files: {theme}");
            set_theme(theme);
        }
    }

    // The rank of the key the current theme came from; a value under a less
    // specific key does not override a more specific one. The keys are read in
    // rank order, so the first installed theme found is also the best.
    let mut source = PORTAL_KEYS.len();
    crate::portal_settings::watch(
        "icon-theme-watcher",
        &PORTAL_KEYS,
        move |namespace, key, value| {
            let Some(rank) = PORTAL_KEYS
                .iter()
                .position(|(ns, k)| namespace == *ns && key == *k)
            else {
                return;
            };
            if rank > source {
                return;
            }
            if let Some(theme) = extract_string(value).filter(|t| is_installed(t)) {
                tracing::debug!("icon-theme from {namespace}: {theme}");
                source = rank;
                set_theme(theme);
            }
        },
    );
}

/// Extract a string from a possibly variant-wrapped `Value`.
fn extract_string(val: Value<'_>) -> Option<String> {
    match val {
        Value::Str(s) => Some(s.to_string()),
        Value::Value(inner) => extract_string(*inner),
        _ => None,
    }
}

/// Where the portal carries an icon theme, most specific first. Otto's key
/// wins over a desktop's own wherever both are answered.
const PORTAL_KEYS: [(&str, &str); 3] = [
    ("org.freedesktop.appearance", "icon-theme"),
    ("org.gnome.desktop.interface", "icon-theme"),
    ("org.kde.kdeglobals.Icons", "Theme"),
];

/// The theme the running desktop wrote down for itself, if it did.
fn desktop_file_theme() -> Option<String> {
    let config = crate::xdg::config_home()?;
    let kde = std::env::var("XDG_CURRENT_DESKTOP")
        .map(|desktops| desktops.split(':').any(|d| d.eq_ignore_ascii_case("KDE")))
        .unwrap_or(false);
    let kdeglobals = || {
        std::fs::read_to_string(config.join("kdeglobals"))
            .ok()
            .and_then(|text| ini_value(&text, "Icons", "Theme"))
    };
    let gtk = || {
        ["gtk-4.0", "gtk-3.0"].iter().find_map(|dir| {
            std::fs::read_to_string(config.join(dir).join("settings.ini"))
                .ok()
                .and_then(|text| ini_value(&text, "Settings", "gtk-icon-theme-name"))
        })
    };
    let found = if kde {
        // Plasma's own default when kdeglobals names none.
        kdeglobals().or_else(|| Some("breeze".to_string()))
    } else {
        gtk()
    };
    found.filter(|theme| is_installed(theme))
}

/// The theme the desktop Otto runs in would use: the one its own files name,
/// or else Breeze or Adwaita — for a process that has no portal to ask.
pub fn desktop_theme() -> Option<String> {
    desktop_file_theme().or_else(installed_default_theme)
}

/// A complete theme to fall back on, where the desktop names none that is
/// installed. Anything is better than `hicolor` alone.
fn installed_default_theme() -> Option<String> {
    ["breeze", "Adwaita"]
        .into_iter()
        .find(|theme| is_installed(theme))
        .map(str::to_string)
}

/// Whether a theme called `name` has an `index.theme` anywhere icons are
/// looked for.
pub fn is_installed(name: &str) -> bool {
    if name.is_empty() || name.contains('/') {
        return false;
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(home) = crate::xdg::home() {
        roots.push(home.join(".icons"));
    }
    roots.extend(crate::xdg::data_home().map(|d| d.join("icons")));
    roots.extend(crate::xdg::data_dirs().iter().map(|d| d.join("icons")));
    roots
        .iter()
        .any(|root| root.join(name).join("index.theme").is_file())
}

/// `key` in `[group]` of an INI-style file — the format of both `kdeglobals`
/// and GTK's `settings.ini`.
fn ini_value(text: &str, group: &str, key: &str) -> Option<String> {
    let mut in_group = false;
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_group = name == group;
            continue;
        }
        if !in_group {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if k.trim() == key {
            let v = v.trim().trim_matches('"');
            return (!v.is_empty()).then(|| v.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::ini_value;

    #[test]
    fn reads_a_key_from_its_own_group_only() {
        let kdeglobals =
            "[General]\nTheme=wrong\n\n[Icons]\nTheme=WhiteSur\n\n[KDE]\nTheme=also-wrong\n";
        assert_eq!(
            ini_value(kdeglobals, "Icons", "Theme").as_deref(),
            Some("WhiteSur")
        );
        let gtk = "[Settings]\ngtk-theme-name=Adwaita\ngtk-icon-theme-name = \"Newaita\"\n";
        assert_eq!(
            ini_value(gtk, "Settings", "gtk-icon-theme-name").as_deref(),
            Some("Newaita")
        );
        assert_eq!(ini_value(gtk, "Icons", "Theme"), None);
    }
}
