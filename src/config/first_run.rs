//! The first session's configuration.
//!
//! When Otto starts and the user has no configuration of their own, it writes
//! one that fits the system it finds: the icon theme the user's other desktops
//! point at, and a dock of the applications that are installed. It happens
//! once — from then on the file is the user's, and nothing here runs again.

use std::collections::HashMap;

use freedesktop_desktop_entry::{self as desktop_entry, DesktopEntry};

use super::{default_apps, file, user_config_file};

/// Write the user's first configuration, if they have none yet.
///
/// Must run before the configuration is first loaded, so the session starts
/// with what is written here.
pub fn prepare() {
    // A test session writes to a throwaway directory and must not depend on
    // what the host has installed.
    if super::isolated_config_file().is_some() {
        return;
    }
    let Some(path) = user_config_file() else {
        return;
    };
    if path.exists() {
        return;
    }

    let mut doc = toml_edit::DocumentMut::new();
    let icon_theme = otto_kit::icon_theme::detect_icon_theme();
    if let Some(theme) = &icon_theme {
        set(&mut doc, "icon_theme", toml::Value::String(theme.clone()));
    }
    let apps = installed_apps();
    let dock = pick_dock(&apps, default_apps::mime_defaults);
    if !dock.is_empty() {
        let ids = dock.iter().cloned().map(toml::Value::String).collect();
        set(&mut doc, "dock.bookmarks", toml::Value::Array(ids));
    }

    match file::store_document(&path, &doc) {
        Ok(()) => tracing::info!(
            path = %path.display(),
            ?icon_theme,
            ?dock,
            "wrote the first configuration"
        ),
        Err(err) => tracing::warn!("could not write the first configuration: {err}"),
    }
}

fn set(doc: &mut toml_edit::DocumentMut, key: &str, value: toml::Value) {
    if let Some(value) = file::to_edit_value(&value) {
        if let Err(err) = file::set_key(doc, key, value) {
            tracing::warn!("first configuration: cannot set `{key}`: {err}");
        }
    }
}

/// One thing the dock should offer, and how to find the app for it.
struct Role {
    /// Otto's own app for it, taken first when installed.
    own: Option<&'static str>,
    /// MIME types whose default handler is the user's choice for it.
    mime: &'static [&'static str],
    /// Well-known apps, most familiar first, for when no default is set.
    known: &'static [&'static str],
    /// The freedesktop menu category that marks any other app for it.
    category: Option<&'static str>,
}

/// Files, settings, a terminal, a browser, a text editor and a calculator —
/// in the order they sit in the dock.
const ROLES: [Role; 6] = [
    Role {
        own: Some("otto-files.desktop"),
        mime: &["inode/directory"],
        known: &[
            "org.gnome.Nautilus.desktop",
            "org.kde.dolphin.desktop",
            "nemo.desktop",
            "thunar.desktop",
            "pcmanfm-qt.desktop",
            "pcmanfm.desktop",
        ],
        category: Some("FileManager"),
    },
    Role {
        own: Some("otto-settings.desktop"),
        mime: &[],
        known: &[],
        category: None,
    },
    Role {
        own: None,
        mime: &["x-scheme-handler/terminal"],
        known: &[
            "org.gnome.Ptyxis.desktop",
            "org.gnome.Console.desktop",
            "org.gnome.Terminal.desktop",
            "org.kde.konsole.desktop",
            "com.mitchellh.ghostty.desktop",
            "kitty.desktop",
            "Alacritty.desktop",
            "org.wezfurlong.wezterm.desktop",
            "foot.desktop",
            "xfce4-terminal.desktop",
            "com.system76.CosmicTerm.desktop",
        ],
        category: Some("TerminalEmulator"),
    },
    Role {
        own: None,
        mime: &[
            "x-scheme-handler/https",
            "x-scheme-handler/http",
            "text/html",
        ],
        known: &[
            "firefox.desktop",
            "org.mozilla.firefox.desktop",
            "google-chrome.desktop",
            "chromium.desktop",
            "org.chromium.Chromium.desktop",
            "brave-browser.desktop",
            "com.brave.Browser.desktop",
            "vivaldi-stable.desktop",
            "org.gnome.Epiphany.desktop",
        ],
        category: Some("WebBrowser"),
    },
    Role {
        own: None,
        mime: &["text/plain"],
        known: &[
            "org.gnome.TextEditor.desktop",
            "org.gnome.gedit.desktop",
            "org.kde.kate.desktop",
            "org.kde.kwrite.desktop",
            "org.xfce.mousepad.desktop",
            "xed.desktop",
            "pluma.desktop",
        ],
        category: Some("TextEditor"),
    },
    Role {
        own: None,
        mime: &[],
        known: &[
            "org.gnome.Calculator.desktop",
            "org.kde.kalk.desktop",
            "org.kde.kcalc.desktop",
            "qalculate-gtk.desktop",
            "galculator.desktop",
            "mate-calc.desktop",
        ],
        category: Some("Calculator"),
    },
];

/// What the dock needs to know about an installed app.
#[derive(Debug, Default, Clone)]
struct App {
    categories: Vec<String>,
    /// Shown in menus, and opens a window of its own rather than running in a
    /// terminal.
    launchable: bool,
}

/// Every installed app by desktop id; the first entry found for an id wins, as
/// the user's own directory comes first.
fn installed_apps() -> HashMap<String, App> {
    let mut apps = HashMap::new();
    for path in desktop_entry::Iter::new(desktop_entry::default_paths()) {
        let Some(id) = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        if apps.contains_key(&id) {
            continue;
        }
        let Ok(entry) = DesktopEntry::from_path(&path, None::<&[&str]>) else {
            continue;
        };
        // An entry restricted to other desktops does not work outside them.
        let elsewhere_only = entry
            .only_show_in()
            .is_some_and(|desktops| !desktops.iter().any(|d| d.eq_ignore_ascii_case("otto")));
        let launchable = !entry.no_display()
            && !entry.hidden()
            && !entry.terminal()
            && !elsewhere_only
            && entry.exec().is_some();
        let categories = entry
            .categories()
            .unwrap_or_default()
            .into_iter()
            .map(str::to_string)
            .collect();
        apps.insert(
            id,
            App {
                categories,
                launchable,
            },
        );
    }
    apps
}

/// The dock's apps, one per role that something installed fills: Otto's own,
/// else the user's default for the role, else a well-known app, else any app
/// in the role's category.
fn pick_dock(
    apps: &HashMap<String, App>,
    mime_defaults: impl Fn(&str) -> Vec<String>,
) -> Vec<String> {
    let usable = |id: &str| apps.get(id).is_some_and(|app| app.launchable);
    let mut dock: Vec<String> = Vec::new();
    for role in &ROLES {
        let own = role.own.filter(|id| usable(id)).map(str::to_string);
        let default = || {
            role.mime
                .iter()
                .flat_map(|mime| mime_defaults(mime))
                .find(|id| usable(id))
        };
        let known = || {
            role.known
                .iter()
                .find(|id| usable(id))
                .map(|id| id.to_string())
        };
        let categorised = || {
            let category = role.category?;
            let mut matching: Vec<&String> = apps
                .iter()
                .filter(|(_, app)| app.launchable && app.categories.iter().any(|c| c == category))
                .map(|(id, _)| id)
                .collect();
            matching.sort();
            matching.first().map(|id| id.to_string())
        };
        if let Some(id) = own.or_else(default).or_else(known).or_else(categorised) {
            if !dock.contains(&id) {
                dock.push(id);
            }
        }
    }
    dock
}

#[cfg(test)]
mod tests {
    use super::{pick_dock, App, HashMap};

    fn apps(entries: &[(&str, &[&str])]) -> HashMap<String, App> {
        entries
            .iter()
            .map(|(id, categories)| {
                (
                    id.to_string(),
                    App {
                        categories: categories.iter().map(|c| c.to_string()).collect(),
                        launchable: true,
                    },
                )
            })
            .collect()
    }

    fn no_defaults(_: &str) -> Vec<String> {
        Vec::new()
    }

    #[test]
    fn a_gnome_system_gets_one_app_per_role() {
        let installed = apps(&[
            ("otto-files.desktop", &["FileManager"]),
            ("otto-settings.desktop", &["Settings"]),
            ("org.gnome.Nautilus.desktop", &["FileManager"]),
            ("org.gnome.Console.desktop", &["TerminalEmulator"]),
            ("firefox.desktop", &["WebBrowser"]),
            ("org.gnome.TextEditor.desktop", &["TextEditor"]),
            ("org.gnome.Calculator.desktop", &["Calculator"]),
        ]);
        assert_eq!(
            pick_dock(&installed, no_defaults),
            [
                "otto-files.desktop",
                "otto-settings.desktop",
                "org.gnome.Console.desktop",
                "firefox.desktop",
                "org.gnome.TextEditor.desktop",
                "org.gnome.Calculator.desktop",
            ]
        );
    }

    #[test]
    fn the_users_default_beats_a_well_known_app() {
        let installed = apps(&[
            ("firefox.desktop", &["WebBrowser"]),
            ("com.brave.Browser.desktop", &["WebBrowser"]),
        ]);
        let defaults = |mime: &str| {
            if mime == "x-scheme-handler/https" {
                vec!["com.brave.Browser.desktop".to_string()]
            } else {
                Vec::new()
            }
        };
        assert_eq!(
            pick_dock(&installed, defaults),
            ["com.brave.Browser.desktop"]
        );
    }

    #[test]
    fn an_unknown_app_fills_its_category() {
        let installed = apps(&[("io.example.Term.desktop", &["System", "TerminalEmulator"])]);
        assert_eq!(
            pick_dock(&installed, no_defaults),
            ["io.example.Term.desktop"]
        );
    }

    #[test]
    fn a_terminal_editor_is_not_pinned() {
        let mut installed = apps(&[("nvim.desktop", &["TextEditor"])]);
        installed.get_mut("nvim.desktop").unwrap().launchable = false;
        let defaults = |mime: &str| {
            if mime == "text/plain" {
                vec!["nvim.desktop".to_string()]
            } else {
                Vec::new()
            }
        };
        assert!(pick_dock(&installed, defaults).is_empty());
    }
}
