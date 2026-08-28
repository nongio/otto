//! The general pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.

use crate::model::{group, Control, Pane, Row};

pub fn build() -> Pane {
    Pane {
        name: otto_kit::t!("settings-pane-general"),
        icon: "settings",
        groups: vec![
            group(
                otto_kit::t!("settings-group-appearance"),
                vec![
                    Row::new(
                        otto_kit::t!("settings-group-appearance"),
                        Control::Select("Light".into()),
                    )
                    .id("theme_scheme"),
                    Row::new(
                        otto_kit::t!("settings-accent-colour"),
                        Control::Color(0xFF0A84FF),
                    )
                    .id("accent_color"),
                    Row::new(otto_kit::t!("settings-font"), Control::Text("Inter".into()))
                        .id("font_family"),
                    // Applies to GTK clients rather than to Otto's own
                    // interface, so it sits with the other appearance rows but
                    // is deliberately not called "Appearance".
                    Row::new(
                        otto_kit::t!("settings-gtk-theme"),
                        Control::Text(String::new()),
                    )
                    .id("gtk_theme"),
                ],
            ),
            group(
                otto_kit::t!("settings-group-desktop"),
                vec![
                    Row::new(
                        otto_kit::t!("settings-background-colour"),
                        Control::Color(0xFF2C2CA0),
                    )
                    .id("background_color"),
                    Row::new(
                        otto_kit::t!("settings-background-image"),
                        Control::File("".into()),
                    )
                    .detail(otto_kit::t!("settings-background-image-detail"))
                    .id("background_image"),
                ],
            ),
            group(
                otto_kit::t!("settings-group-pointer-and-icons"),
                vec![
                    Row::new(
                        otto_kit::t!("settings-cursor-theme"),
                        Control::Select("Notwaita-Black".into()),
                    )
                    .id("cursor_theme"),
                    Row::new(
                        otto_kit::t!("settings-cursor-size"),
                        Control::Slider {
                            value: 24.0,
                            min: 16.0,
                            max: 96.0,
                            readout: "24 px".into(),
                        },
                    )
                    .id("cursor_size"),
                    Row::new(
                        otto_kit::t!("settings-icon-theme"),
                        Control::Select("".into()),
                    )
                    .id("icon_theme"),
                ],
            ),
            // The app switcher has no pane of its own — the workspaces pane
            // was dropped — and this is the only setting it owns.
            group(
                otto_kit::t!("settings-group-window-switcher"),
                vec![Row::new(
                    otto_kit::t!("settings-follow-cursor"),
                    Control::Toggle(false),
                )
                .id("appswitcher.follow_cursor")],
            ),
            group(
                otto_kit::t!("settings-group-language"),
                vec![Row::new(
                    otto_kit::t!("settings-preferred-languages"),
                    Control::Text("en".into()),
                )
                .id("locales")],
            ),
            // Not a setting: where the settings go. Otto's configuration is
            // layered, and which file is the writable one depends on what
            // exists on this machine — so it is worth being able to read it
            // off the pane rather than work it out. Everything this app
            // changes lands there, and anything it does not offer can be
            // edited by hand.
            group(
                otto_kit::t!("settings-group-configuration"),
                vec![
                    Row::new(config_file(), Control::Button(open_button())).detail(
                        crate::settings_client::config_path().unwrap_or_else(|| {
                            otto_kit::t_owned!("settings-configuration-file-unknown")
                        }),
                    ),
                ],
            ),
        ],
    }
}

/// The row that shows where settings are written, and the button that opens
/// it. Matched by label, the way every unbound row in this app is.
fn config_file() -> &'static str {
    otto_kit::t!("settings-configuration-file")
}
fn open() -> &'static str {
    otto_kit::t!("common-open")
}

/// The row's single button, as the `'static` slice a button row wants.
fn open_button() -> &'static [&'static str] {
    static BUTTONS: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    BUTTONS.get_or_init(|| vec![open()])
}

/// A press on this pane's push buttons.
pub fn press(row: &str, button: &str) {
    if row != config_file() || button != open() {
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
