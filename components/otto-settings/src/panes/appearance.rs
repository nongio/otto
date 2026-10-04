//! The appearance pane: how the desktop looks.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; the desk's group
//! is [`super::desk`]'s, which also writes the desk's own file. The font, cursor
//! and icon theme pop-ups are filled by [`crate::discovery`], which scans what
//! is installed; the clock format pop-up is filled here, with each format
//! shown as it would render the current time. The desktop widget pop-up is
//! inactive, with a note, where ewwii is not installed.

// Rust guideline compliant 2026-02-21

use std::sync::OnceLock;

use chrono::Local;

use crate::discovery::Choice;
use crate::model::{group, Control, Pane, Row};

/// The setting the clock format pop-up edits.
const CLOCK_FORMAT_ID: &str = "topbar.clock_format";

/// The setting the desktop widget pop-up edits.
const DESKTOP_WIDGET_ID: &str = "desktop.widget";

/// The formats the clock format pop-up offers, as strftime strings.
///
/// The empty string leads: no format chosen, so the bar follows the language.
/// The rest run from the shortest to the fullest, 24-hour first and then the
/// same shapes on a 12-hour clock. The double space before the time is the
/// one the language formats in the catalogue use, which keeps the date and
/// the time from running together.
const CLOCK_FORMATS: &[&str] = &[
    "",
    "%H:%M",
    "%a %H:%M",
    "%a %-d %b  %H:%M",
    "%A %-d %B  %H:%M",
    "%-I:%M %p",
    "%a %-I:%M %p",
    "%a %-d %b  %-I:%M %p",
];

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
            group(
                otto_kit::t!("settings-group-bar-clock"),
                vec![
                    Row::new(otto_kit::t!("settings-show-clock"), Control::Toggle(true))
                        .detail(otto_kit::t!("settings-show-clock-detail"))
                        .id("topbar.show_clock"),
                    Row::new(
                        otto_kit::t!("settings-clock-format"),
                        Control::Select(String::new()),
                    )
                    .detail(otto_kit::t!("settings-clock-format-detail"))
                    .id(CLOCK_FORMAT_ID),
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

/// The clock format pop-up's entries, each labelled with the current time in
/// that format.
///
/// A format set by hand in the configuration file that the list does not
/// offer is listed too, so the pop-up always shows what is in force.
/// Returns `None` for any other setting.
pub fn menu_choices(id: &str, current: &str) -> Option<Vec<Choice>> {
    if id != CLOCK_FORMAT_ID {
        return None;
    }
    let mut formats: Vec<&str> = CLOCK_FORMATS.to_vec();
    if !formats.contains(&current) {
        formats.insert(1, current);
    }
    Some(
        formats
            .into_iter()
            .map(|format| Choice {
                label: clock_label(format),
                value: format.to_string(),
            })
            .collect(),
    )
}

/// What the clock format field shows for `value`: the current time in it.
/// Returns `None` for any other setting.
pub fn display(id: &str, value: &str) -> Option<String> {
    (id == CLOCK_FORMAT_ID).then(|| clock_label(value))
}

/// The label for one clock format. The empty format is the language's own,
/// named as such with a preview beside it.
fn clock_label(format: &str) -> String {
    if format.is_empty() {
        return otto_kit::t_owned!(
            "settings-clock-format-automatic",
            preview = preview(otto_kit::t!("bar-clock-format"))
        );
    }
    preview(format)
}

/// The current time rendered in `format`, in the interface's language, the
/// way otto-bar renders it. A format chrono cannot render is shown as the
/// format itself rather than as a time.
fn preview(format: &str) -> String {
    use chrono::format::{Item, StrftimeItems};
    if StrftimeItems::new(format).any(|item| matches!(item, Item::Error)) {
        return format.to_string();
    }
    let posix = otto_kit::i18n::posix_locale();
    let locale = chrono::Locale::try_from(posix.as_str()).unwrap_or(chrono::Locale::en_GB);
    Local::now().format_localized(format, locale).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_menu_leads_with_the_language_default() {
        let choices = menu_choices(CLOCK_FORMAT_ID, "").expect("ours");
        assert_eq!(choices[0].value, "");
        assert_eq!(choices.len(), CLOCK_FORMATS.len());
    }

    #[test]
    fn a_hand_written_format_is_listed_so_the_menu_shows_it() {
        let choices = menu_choices(CLOCK_FORMAT_ID, "%Y-%m-%d %H:%M").expect("ours");
        assert!(choices.iter().any(|c| c.value == "%Y-%m-%d %H:%M"));
        assert_eq!(choices.len(), CLOCK_FORMATS.len() + 1);
    }

    #[test]
    fn other_settings_are_not_answered() {
        assert!(menu_choices("font_family", "").is_none());
        assert!(display("font_family", "Inter").is_none());
    }

    #[test]
    fn a_malformed_format_previews_as_itself() {
        assert_eq!(preview("%Q"), "%Q");
    }
}
