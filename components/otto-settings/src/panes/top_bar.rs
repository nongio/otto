//! The top bar pane: what otto-bar shows.
//!
//! Every row is bound to `org.otto.Settings`; otto-bar follows the `Changed`
//! signal itself, so each lands on the bar at once. The clock format pop-up is
//! filled here, with each format shown as it would render the current time.

use chrono::Local;

use crate::discovery::Choice;
use crate::model::{group, Control, Pane, Row};

/// The setting the clock format pop-up edits.
const CLOCK_FORMAT_ID: &str = "topbar.clock_format";

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
        name: otto_kit::t!("settings-pane-top-bar"),
        icon: "top_bar",
        intro: None,
        groups: vec![
            group(
                otto_kit::t!("settings-group-app-menu"),
                vec![Row::new(
                    otto_kit::t!("settings-show-app-menu"),
                    Control::Toggle(true),
                )
                .detail(otto_kit::t!("settings-show-app-menu-detail"))
                .id("topbar.show_app_menu")],
            ),
            group(
                otto_kit::t!("settings-group-clock"),
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
