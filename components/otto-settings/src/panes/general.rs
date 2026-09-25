//! The general pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.
//!
//! The renderer only reaches a login session: the windowed backends always
//! draw with OpenGL, and Vulkan exists only in a build that includes it. Where
//! there is nothing to choose, the row states what Otto draws with and why
//! instead of offering a menu.

use crate::model::{group, Control, Pane, Row};
use crate::settings_client;

/// The setting the renderer row edits.
const RENDERER_ID: &str = "rendering.renderer";

pub fn build() -> Pane {
    Pane {
        name: otto_kit::t!("settings-pane-general"),
        icon: "settings",
        intro: None,
        groups: vec![
            group(
                otto_kit::t!("settings-group-appearance"),
                vec![
                    Row::new(
                        otto_kit::t!("settings-colour-scheme"),
                        Control::Select("Light".into()),
                    )
                    .id("theme_scheme"),
                    Row::new(
                        otto_kit::t!("settings-accent-colour"),
                        Control::Color(0xFF0A84FF),
                    )
                    .id("accent_color"),
                    Row::new(
                        otto_kit::t!("settings-rounded-corners"),
                        Control::Toggle(true),
                    )
                    .id("rounded_corners"),
                    Row::new(otto_kit::t!("settings-frosting"), Control::Toggle(true))
                        .detail(otto_kit::t!("settings-frosting-detail"))
                        .id("frosting"),
                    Row::new(
                        otto_kit::t!("settings-window-controls"),
                        Control::Select("left".into()),
                    )
                    .id("window_controls_side"),
                    Row::new(
                        otto_kit::t!("settings-maximize-button"),
                        Control::Toggle(false),
                    )
                    .detail(otto_kit::t!("settings-maximize-button-detail"))
                    .id("show_maximize_button"),
                    Row::new(
                        otto_kit::t!("settings-font"),
                        Control::Select("Inter".into()),
                    )
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
            // was dropped — so its settings live here. Its tint toggle does
            // not: it says whether the switcher joins in the *dock's* tint,
            // and it is only legible beside the toggle it depends on, so it
            // sits in the dock pane's tint group.
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
                    otto_kit::t!("settings-display-language"),
                    Control::Select(String::new()),
                )
                .id("locales")],
            ),
            group(otto_kit::t!("settings-renderer"), vec![renderer_row()]),
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

/// The renderer row: a pop-up where there is a choice to make, and a plain
/// statement of what Otto draws with where there is not.
///
/// There is none under a windowed session, which always draws with OpenGL,
/// nor in a build without Vulkan, where OpenGL is the only renderer left.
fn renderer_row() -> Row {
    let mut row = Row::new(
        otto_kit::t!("settings-renderer"),
        Control::Select("gl".into()),
    )
    .detail(otto_kit::t!("settings-renderer-detail"))
    .id(RENDERER_ID);
    let Some(desc) = settings_client::describe(RENDERER_ID) else {
        return row;
    };
    let offered = desc
        .choices
        .iter()
        .filter(|choice| !desc.unavailable_choices.contains(choice))
        .count();
    let note = if !desc.applies_here {
        otto_kit::t!("settings-renderer-windowed-detail")
    } else if offered < 2 {
        otto_kit::t!("settings-renderer-no-vulkan-detail")
    } else {
        return row;
    };
    if let Control::Select(current) = &row.control {
        row.control = Control::Value(settings_client::display_choice(RENDERER_ID, current));
    }
    row.detail(note)
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
