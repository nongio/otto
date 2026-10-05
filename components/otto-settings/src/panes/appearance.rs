//! The appearance pane: how the desktop looks.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; the desk's group
//! is [`super::desk`]'s, which also writes the desk's own file. The font, cursor
//! and icon theme pop-ups are filled by [`crate::discovery`], which scans what
//! is installed. The desktop widget pop-up is inactive, with a note, where
//! ewwii is not installed. The top bar's clock is [`super::top_bar`]'s.

// Rust guideline compliant 2026-02-21

use std::sync::OnceLock;

use crate::model::{group, Control, Pane, Row};

/// The setting the desktop widget pop-up edits.
const DESKTOP_WIDGET_ID: &str = "desktop.widget";

pub fn build() -> Pane {
    Pane {
        name: otto_kit::t!("settings-pane-appearance"),
        icon: "appearance",
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
                    desktop_widget_row(),
                ],
            ),
            super::desk::group_rows(),
            group(
                otto_kit::t!("settings-group-pointer-and-icons"),
                vec![
                    Row::new(
                        otto_kit::t!("settings-cursor-theme"),
                        Control::Select("Otto-MacTahoe".into()),
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
        ],
    }
}

/// The desktop widget pop-up. ewwii draws the widgets and is not a
/// dependency of Otto's, so without it the row says so and cannot be opened.
/// With it, the row names ewwii and the folder that takes a theme of one's
/// own.
fn desktop_widget_row() -> Row {
    let row = Row::new(
        otto_kit::t!("settings-desktop-widget"),
        Control::Select("none".into()),
    );
    if ewwii_installed() {
        row.detail(otto_kit::t!(
            "settings-desktop-widget-detail",
            folder = user_theme_dir()
        ))
        .id(DESKTOP_WIDGET_ID)
    } else {
        row.detail(otto_kit::t!("settings-desktop-widget-needs-ewwii"))
            .id(DESKTOP_WIDGET_ID)
            .inactive(true)
    }
}

/// Where Otto looks for the user's own widget theme, with the home folder
/// written as `~`.
fn user_theme_dir() -> String {
    static DIR: OnceLock<String> = OnceLock::new();
    DIR.get_or_init(|| {
        let home = std::env::var("HOME").unwrap_or_default();
        let config = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|dir| !dir.is_empty())
            .unwrap_or_else(|| format!("{home}/.config"));
        let dir = format!("{config}/otto/widgets/ewwii");
        match dir.strip_prefix(&home) {
            Some(rest) if !home.is_empty() => format!("~{rest}"),
            _ => dir,
        }
    })
    .clone()
}

/// Whether `ewwii` is an executable on `PATH`. Looked up once: the pane is
/// rebuilt every frame.
fn ewwii_installed() -> bool {
    use std::os::unix::fs::PermissionsExt;
    static INSTALLED: OnceLock<bool> = OnceLock::new();
    *INSTALLED.get_or_init(|| {
        std::env::var_os("PATH").is_some_and(|path| {
            std::env::split_paths(&path).any(|dir| {
                std::fs::metadata(dir.join("ewwii"))
                    .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            })
        })
    })
}
