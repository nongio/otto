//! The general pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.

use crate::model::{group, Control, Pane, Row};

pub fn build() -> Pane {
    Pane {
        name: "General",
        icon: "settings",
        groups: vec![
            group(
                "Appearance",
                vec![
                    Row::new("Appearance", Control::Select("Light".into())).id("theme_scheme"),
                    Row::new("Accent colour", Control::Color(0xFF0A84FF)).id("accent_color"),
                    Row::new("Font", Control::Text("Inter".into())).id("font_family"),
                    // Applies to GTK clients rather than to Otto's own
                    // interface, so it sits with the other appearance rows but
                    // is deliberately not called "Appearance".
                    Row::new("GTK theme", Control::Text(String::new())).id("gtk_theme"),
                ],
            ),
            group(
                "Desktop",
                vec![
                    Row::new("Background colour", Control::Color(0xFF2C2CA0))
                        .id("background_color"),
                    Row::new("Background image", Control::File("".into()))
                        .detail("Chosen through the desktop portal's file picker")
                        .id("background_image"),
                ],
            ),
            group(
                "Pointer & icons",
                vec![
                    Row::new("Cursor theme", Control::Select("Notwaita-Black".into()))
                        .id("cursor_theme"),
                    Row::new(
                        "Cursor size",
                        Control::Slider {
                            value: 24.0,
                            min: 16.0,
                            max: 96.0,
                            readout: "24 px".into(),
                        },
                    )
                    .id("cursor_size"),
                    Row::new("Icon theme", Control::Select("".into())).id("icon_theme"),
                ],
            ),
            // The app switcher has no pane of its own — the workspaces pane
            // was dropped — and this is the only setting it owns.
            group(
                "Window switcher",
                vec![
                    Row::new("Show on the pointer's display", Control::Toggle(false))
                        .id("appswitcher.follow_cursor"),
                ],
            ),
            group(
                "Language",
                vec![Row::new("Preferred languages", Control::Text("en".into())).id("locales")],
            ),
            // Not a setting: where the settings go. Otto's configuration is
            // layered, and which file is the writable one depends on what
            // exists on this machine — so it is worth being able to read it
            // off the pane rather than work it out. Everything this app
            // changes lands there, and anything it does not offer can be
            // edited by hand.
            group(
                "Configuration",
                vec![Row::new(CONFIG_FILE, Control::Button(&[OPEN])).detail(
                    crate::settings_client::config_path()
                        .unwrap_or_else(|| "not known — the compositor is not answering".into()),
                )],
            ),
        ],
    }
}

/// The row that shows where settings are written, and the button that opens
/// it. Matched by label, the way every unbound row in this app is.
const CONFIG_FILE: &str = "Configuration file";
const OPEN: &str = "Open";

/// A press on this pane's push buttons.
pub fn press(row: &str, button: &str) {
    if row != CONFIG_FILE || button != OPEN {
        return;
    }
    let Some(path) = crate::settings_client::config_path() else {
        return;
    };
    // Handed to the desktop rather than to a named editor: which application
    // opens a TOML file is the user's choice, and `xdg-open` is where that
    // choice is recorded. Spawned and forgotten — the editor outlives this
    // app, and waiting on it would freeze the pane.
    match std::process::Command::new("xdg-open").arg(&path).spawn() {
        Ok(_) => {}
        Err(err) => eprintln!("could not open {path}: {err}"),
    }
}
