//! The settings schema: the curated, permanent surface of `org.otto.Settings`.
//!
//! Identifiers are a contract. Once a row here has shipped, its `id` is never
//! renamed or repurposed — settings apps hardcode these strings, and an app
//! built against an older compositor has to keep working. Adding a row is
//! cheap; changing one is not.
//!
//! The table is written by hand rather than derived from `Config`. A derive
//! would have to invent labels, could not express ranges, and — worse — would
//! expose every field the compositor happens to have, including session
//! plumbing that is deliberately out of scope (see `specs/settings-app.md`).

use super::value::{SettingType, SettingValue};

/// What happens when a setting is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Apply {
    /// Takes effect immediately.
    Live,
    /// Persisted, but only read at startup.
    Restart,
    /// Cannot be changed on this system or in this build. `Set` rejects it
    /// rather than pretending; a `Set` that silently does nothing is worse
    /// than one that refuses.
    Unsupported,
}

impl Apply {
    pub fn wire_name(self) -> &'static str {
        match self {
            Apply::Live => "live",
            Apply::Restart => "restart",
            Apply::Unsupported => "unsupported",
        }
    }
}

/// One row of the schema.
pub struct SettingSpec {
    /// Dotted path, matching the configuration structure.
    pub id: &'static str,
    pub ty: SettingType,
    pub label: &'static str,
    pub description: &'static str,
    pub apply: Apply,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Granularity a client should snap a continuous control to. Without one,
    /// dragging a slider produces whatever float the pixel happened to land
    /// on — `-0.01` where the user meant nothing at all. `None` means the
    /// value really is continuous.
    pub step: Option<f64>,
    pub choices: &'static [&'static str],
    /// Human names for `choices`, in the same order. The values in `choices`
    /// are configuration tokens and part of the permanent contract, so they
    /// cannot be prettied up — `clickfinger` has to stay `clickfinger` on the
    /// wire and in the file. This is what a client shows in its place.
    /// Empty means the tokens are already presentable.
    pub choice_labels: &'static [&'static str],
}

impl SettingSpec {
    /// The configuration section the setting lives in — everything before the
    /// last dot, empty for a top-level key.
    pub fn section(&self) -> &'static str {
        match self.id.rsplit_once('.') {
            Some((section, _)) => section,
            None => "",
        }
    }

    /// Check `value` against this row. The type must match exactly: the
    /// compositor never coerces, so a client cannot accidentally turn a double
    /// setting into an integer one and have it stick.
    pub fn validate(&self, value: &SettingValue) -> Result<(), Invalid> {
        if value.ty() != self.ty.wire_repr() {
            return Err(Invalid::Type(format!(
                "`{}` is {}, got {}",
                self.id,
                self.ty.wire_name(),
                value.ty().wire_name()
            )));
        }

        if self.ty == SettingType::Enum {
            let text = value.as_str().unwrap_or_default();
            if !self.choices.contains(&text) {
                return Err(Invalid::Range(format!(
                    "`{}` must be one of {}, got `{text}`",
                    self.id,
                    self.choices.join(", ")
                )));
            }
        }

        // A colour's `choices` are the palette names, offered as swatches; a
        // literal the palette has no name for is just as valid, so the check
        // is that the value resolves at all rather than that it is listed.
        if self.ty == SettingType::Color {
            let text = value.as_str().unwrap_or_default();
            if crate::theme::accent_from(text).is_none() {
                return Err(Invalid::Range(format!(
                    "`{}` must be a palette name ({}) or a #RGB or #RRGGBB \
                     colour, got `{text}`",
                    self.id,
                    self.choices.join(", ")
                )));
            }
        }

        if let Some(number) = value.as_number() {
            if let Some(min) = self.min {
                if number < min {
                    return Err(Invalid::Range(format!("`{}` must be >= {min}", self.id)));
                }
            }
            if let Some(max) = self.max {
                if number > max {
                    return Err(Invalid::Range(format!("`{}` must be <= {max}", self.id)));
                }
            }
        }

        Ok(())
    }
}

/// Why a value was rejected. The two cases are distinguishable on the bus.
#[derive(Debug)]
pub enum Invalid {
    Type(String),
    Range(String),
}

/// The row for `id`, if there is one.
pub fn lookup(id: &str) -> Option<&'static SettingSpec> {
    SETTINGS.iter().find(|spec| spec.id == id)
}

/// Terse constructors, so the table below reads as a table.
const fn spec(
    id: &'static str,
    ty: SettingType,
    label: &'static str,
    description: &'static str,
    apply: Apply,
) -> SettingSpec {
    SettingSpec {
        id,
        ty,
        label,
        description,
        apply,
        min: None,
        max: None,
        step: None,
        choices: &[],
        choice_labels: &[],
    }
}

#[allow(clippy::too_many_arguments)]
const fn ranged(
    id: &'static str,
    ty: SettingType,
    label: &'static str,
    description: &'static str,
    apply: Apply,
    min: f64,
    max: f64,
    step: f64,
) -> SettingSpec {
    SettingSpec {
        min: Some(min),
        max: Some(max),
        step: Some(step),
        ..spec(id, ty, label, description, apply)
    }
}

const fn choice(
    id: &'static str,
    label: &'static str,
    description: &'static str,
    apply: Apply,
    choices: &'static [&'static str],
) -> SettingSpec {
    SettingSpec {
        choices,
        ..spec(id, SettingType::Enum, label, description, apply)
    }
}

/// A choice whose configuration tokens are not fit to show the user.
const fn labelled_choice(
    id: &'static str,
    label: &'static str,
    description: &'static str,
    apply: Apply,
    choices: &'static [&'static str],
    choice_labels: &'static [&'static str],
) -> SettingSpec {
    SettingSpec {
        choice_labels,
        ..choice(id, label, description, apply, choices)
    }
}

/// A colour: one of `choices`, resolved against the current palette, or a hex
/// literal. `choice_labels` names the swatches, as [`labelled_choice`] does.
const fn color(
    id: &'static str,
    label: &'static str,
    description: &'static str,
    apply: Apply,
    choices: &'static [&'static str],
    choice_labels: &'static [&'static str],
) -> SettingSpec {
    SettingSpec {
        choices,
        choice_labels,
        ..spec(id, SettingType::Color, label, description, apply)
    }
}

use Apply::{Live, Restart};
use SettingType::{Bool, Double, Int, Str, StrList};

/// The languages the language picker offers, as the `locales` setting spells
/// them. The empty string is the absence of a preference — the environment
/// answers — and is a real choice rather than a blank row, so it leads.
const LOCALE_CHOICES: &[&str] = &[
    "", "en-GB", "en-US", "de", "es", "fr", "it", "pl", "pt-BR", "ru", "uk", "zh-CN",
];

/// Presentation for `LOCALE_CHOICES`, in the same order.
///
/// Endonyms, passed through untranslated — `choice_label` only looks up the
/// `settings-choice-` keys, which is what the first entry is. A language list
/// has to stay readable to someone who cannot read the interface it is drawn
/// in, which is precisely the person about to change it.
const LOCALE_LABELS: &[&str] = &[
    "settings-choice-system-language",
    "English (United Kingdom)",
    "English (United States)",
    "Deutsch",
    "Español",
    "Français",
    "Italiano",
    "Polski",
    "Português (Brasil)",
    "Русский",
    "Українська",
    "简体中文",
];

/// The accent names the compositor's palette can resolve, offered as the
/// swatches a colour well shows. The theme owns the list, so a name offered
/// here always resolves; a hex literal outside it is accepted too.
const ACCENT_COLORS: &[&str] = crate::theme::ACCENT_NAMES;
/// Presentation for `ACCENT_COLORS`, in the same order.
const ACCENT_COLOR_LABELS: &[&str] = &[
    "settings-choice-accent-blue",
    "settings-choice-accent-purple",
    "settings-choice-accent-pink",
    "settings-choice-accent-red",
    "settings-choice-accent-orange",
    "settings-choice-accent-yellow",
    "settings-choice-accent-green",
    "settings-choice-accent-mint",
    "settings-choice-accent-teal",
    "settings-choice-accent-cyan",
    "settings-choice-accent-indigo",
    "settings-choice-accent-brown",
    "settings-choice-accent-graphite",
];

/// Everything `org.otto.Settings` describes.
///
/// `apply` is truthful in both directions. A setting is `live` when the running
/// session genuinely follows it — either because [`crate::settings::apply`]
/// reconciles something, or because the value is read from the live
/// configuration at the moment it is used — and `restart` only where a restart
/// is what it actually takes. Marking a setting `restart` that already applies
/// itself is the same lie as the other way round: it puts a "takes effect after
/// a restart" badge on a change the user can already see.
pub static SETTINGS: &[SettingSpec] = &[
    // ---- General ---------------------------------------------------------
    ranged(
        "screen_scale",
        Double,
        "Display scale",
        "Global scale factor applied to the desktop.",
        Restart,
        0.5,
        4.0,
        0.25,
    ),
    labelled_choice(
        "theme_scheme",
        "Colour scheme",
        "Light or dark colour scheme.",
        Live,
        &["Light", "Dark"],
        &["settings-choice-light", "settings-choice-dark"],
    ),
    color(
        "accent_color",
        "Accent colour",
        "A palette name, which follows the light and dark schemes, or a \
         #RRGGBB colour.",
        Live,
        ACCENT_COLORS,
        ACCENT_COLOR_LABELS,
    ),
    spec(
        "rounded_corners",
        Bool,
        "Rounded corners",
        "The dock, the top bar, window decorations and the desktop's own \
         panels.",
        Live,
    ),
    labelled_choice(
        "window_controls_side",
        "Window controls",
        "Which end of a window's titlebar the close, minimize and zoom \
         controls sit at.",
        Live,
        &["left", "right"],
        &[
            "settings-choice-controls-left",
            "settings-choice-controls-right",
        ],
    ),
    spec(
        "show_maximize_button",
        Bool,
        "Maximize button",
        "Show the zoom control in a window's titlebar. Off by default: a \
         double click on the titlebar zooms a window either way.",
        Live,
    ),
    spec(
        "font_family",
        Str,
        "Interface font",
        "Font family used by Otto's own interface.",
        Restart,
    ),
    spec(
        "background_color",
        Str,
        "Background colour",
        "Desktop background colour, as a hex string.",
        Live,
    ),
    spec(
        "background_image",
        Str,
        "Background image",
        "Path to the desktop background image. Empty for none.",
        Live,
    ),
    spec(
        "cursor_theme",
        Str,
        "Cursor theme",
        "Name of the XCursor theme.",
        Live,
    ),
    // XCursor themes ship a fixed set of sizes (24, 32, 48, 64 are the usual
    // ones) and scale between them, so a slider that stops on every second
    // pixel offers 60 values where only a handful look right. The step lands
    // on those sizes; the range stops at 96 because a cursor larger than that
    // is an accessibility setting no theme actually draws for.
    ranged(
        "cursor_size",
        Int,
        "Cursor size",
        "Cursor size in logical pixels.",
        Live,
        16.0,
        96.0,
        8.0,
    ),
    spec(
        "icon_theme",
        Str,
        "Icon theme",
        "Name of the icon theme. Empty auto-detects.",
        Live,
    ),
    spec(
        "gtk_theme",
        Str,
        "GTK theme",
        "GTK theme name handed to clients. Empty auto-detects.",
        Restart,
    ),
    SettingSpec {
        choices: LOCALE_CHOICES,
        choice_labels: LOCALE_LABELS,
        ..spec(
            "locales",
            StrList,
            "Language",
            "The language Otto and the applications it starts are drawn in. \
             Empty follows the environment.",
            Restart,
        )
    },
    // ---- Dock ------------------------------------------------------------
    ranged(
        "dock.size",
        Double,
        "Size",
        "Dock size multiplier.",
        Live,
        0.5,
        2.0,
        0.05,
    ),
    labelled_choice(
        "dock.position",
        "Position on screen",
        "Screen edge the dock lives on.",
        Live,
        &["bottom", "left", "right"],
        &[
            "settings-choice-position-bottom",
            "settings-choice-position-left",
            "settings-choice-position-right",
        ],
    ),
    spec(
        "dock.autohide",
        Bool,
        "Automatically hide",
        "Hide the dock until the pointer reaches its screen edge.",
        Live,
    ),
    spec(
        "dock.magnification",
        Bool,
        "Magnification",
        "Grow the icons under the pointer.",
        Live,
    ),
    ranged(
        "dock.genie_scale",
        Double,
        "Magnification amount",
        "How much the icons under the pointer grow.",
        Live,
        0.0,
        1.0,
        0.05,
    ),
    ranged(
        "dock.genie_span",
        Double,
        "Magnification falloff",
        "How sharply the magnification falls off with distance: higher keeps the bump tight around the pointer.",
        Live,
        0.0,
        100.0,
        5.0,
    ),
    spec(
        "dock.colorize_icons",
        Bool,
        "Tint icons",
        "Tint dock icons with a single colour.",
        Live,
    ),
    spec(
        "dock.colorize_color",
        Str,
        "Icon tint",
        "Colour used to tint dock icons, as a hex string.",
        Live,
    ),
    ranged(
        "dock.colorize_intensity",
        Double,
        "Icon tint strength",
        "How strongly the tint is applied.",
        Live,
        0.0,
        1.0,
        0.05,
    ),
    // ---- Keyboard --------------------------------------------------------
    ranged(
        "keyboard_repeat_delay",
        Int,
        "Repeat delay",
        "Milliseconds a key is held before it starts repeating.",
        Live,
        100.0,
        2000.0,
        25.0,
    ),
    ranged(
        "keyboard_repeat_rate",
        Int,
        "Repeat rate",
        "Repeats per second while a key is held.",
        Live,
        1.0,
        100.0,
        1.0,
    ),
    spec(
        "input.xkb_layout",
        Str,
        "Keyboard layout",
        "XKB layout name. Empty uses the system default.",
        Live,
    ),
    spec(
        "input.xkb_variant",
        Str,
        "Keyboard variant",
        "XKB variant name. Empty uses the system default.",
        Live,
    ),
    spec(
        "input.xkb_options",
        StrList,
        "Keyboard options",
        "XKB option strings.",
        Live,
    ),
    // ---- Trackpad & Mouse ------------------------------------------------
    spec(
        "input.tap_enabled",
        Bool,
        "Tap to click",
        "Treat a tap on the touchpad as a click.",
        Live,
    ),
    spec(
        "input.tap_drag_enabled",
        Bool,
        "Tap and drag",
        "Start a drag from a tap followed by a held touch.",
        Live,
    ),
    spec(
        "input.tap_drag_lock_enabled",
        Bool,
        "Drag lock",
        "Keep a tap-drag going through a brief lift of the finger.",
        Live,
    ),
    labelled_choice(
        "input.touchpad_click_method",
        "Click method",
        "Whether a click means finger count or button areas.",
        Live,
        &["clickfinger", "buttonareas"],
        &["settings-choice-clickfinger", "settings-choice-buttonareas"],
    ),
    spec(
        "input.touchpad_dwt_enabled",
        Bool,
        "Disable while typing",
        "Ignore the touchpad while the keyboard is in use.",
        Live,
    ),
    spec(
        "input.touchpad_natural_scroll_enabled",
        Bool,
        "Natural scrolling",
        "Content follows the fingers.",
        Live,
    ),
    spec(
        "input.touchpad_left_handed",
        Bool,
        "Left-handed",
        "Swap the primary and secondary buttons.",
        Live,
    ),
    spec(
        "input.touchpad_middle_emulation_enabled",
        Bool,
        "Middle-click emulation",
        "Pressing both buttons together is a middle click.",
        Live,
    ),
    ranged(
        "input.scroll_speed",
        Double,
        "Scroll speed",
        "Software multiplier applied to scroll events.",
        Live,
        0.1,
        2.0,
        0.05,
    ),
    ranged(
        "input.pointer_accel_speed",
        Double,
        "Pointer speed",
        "Pointer acceleration, from -1 (slowest) to 1 (fastest).",
        Live,
        -1.0,
        1.0,
        0.1,
    ),
    labelled_choice(
        "input.pointer_accel_profile",
        "Pointer acceleration",
        "Flat is raw speed; adaptive follows libinput's curve.",
        Live,
        &["flat", "adaptive"],
        &[
            "settings-choice-accel-flat",
            "settings-choice-accel-adaptive",
        ],
    ),
    // ---- Sound -----------------------------------------------------------
    spec(
        "audio.sound_enabled",
        Bool,
        "Interface sounds",
        "Play sound feedback for interface events.",
        Live,
    ),
    spec(
        "audio.sound_theme",
        Str,
        "Sound theme",
        "XDG sound theme name. Empty auto-detects.",
        Live,
    ),
    // ---- Power -----------------------------------------------------------
    spec(
        "power_management.manage_lid_switch",
        Bool,
        "Handle the lid switch",
        "Let Otto act on the lid rather than leaving it to logind.",
        Live,
    ),
    labelled_choice(
        "power_management.on_lid_close",
        "When the lid closes",
        "What happens when the laptop lid is closed.",
        Live,
        &["auto", "lock", "disable_internal_screen"],
        &[
            "settings-choice-lid-auto",
            "settings-choice-lid-lock",
            "settings-choice-lid-disable-internal",
        ],
    ),
    labelled_choice(
        "power_management.on_power_button",
        "When the power button is pressed",
        "What happens when the hardware power button is pressed.",
        Live,
        &["ignore", "lock", "suspend", "shutdown"],
        &[
            "settings-choice-power-ignore",
            "settings-choice-power-lock",
            "settings-choice-power-suspend",
            "settings-choice-power-shutdown",
        ],
    ),
    // ---- Lock & Login ----------------------------------------------------
    spec(
        "lock.locker_command",
        Str,
        "Lock screen command",
        "The locker launched to lock the session.",
        Live,
    ),
    spec(
        "lock.locker_args",
        StrList,
        "Lock screen arguments",
        "Arguments passed to the locker.",
        Live,
    ),
    ranged(
        "lock.auto_lock_timeout",
        Int,
        "Lock after",
        "Seconds of inactivity before locking. 0 never locks.",
        Live,
        0.0,
        86400.0,
        60.0,
    ),
    spec(
        "login.greeter_command",
        Str,
        "Greeter command",
        "The greeter launched in login mode.",
        Restart,
    ),
    spec(
        "login.greeter_args",
        StrList,
        "Greeter arguments",
        "Arguments passed to the greeter.",
        Restart,
    ),
    // ---- Window management ----------------------------------------------
    spec(
        "appswitcher.follow_cursor",
        Bool,
        "Switcher follows the pointer",
        "Show the app switcher on the output the pointer is on.",
        Live,
    ),
    spec(
        "appswitcher.colorize_icons",
        Bool,
        "Tint switcher icons",
        "Let the dock's icon tint reach the app switcher. Does nothing while the dock's tint is off.",
        Live,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn every_range_has_a_step_that_divides_it() {
        for spec in SETTINGS {
            let (Some(min), Some(max)) = (spec.min, spec.max) else {
                continue;
            };
            let step = spec
                .step
                .unwrap_or_else(|| panic!("`{}` has a range but no step", spec.id));
            assert!(step > 0.0, "`{}` has a non-positive step", spec.id);

            // A step that does not divide the range leaves the top of the
            // slider unreachable: the last position before `max` snaps short
            // and the user can never set the maximum the schema advertises.
            let steps = (max - min) / step;
            assert!(
                (steps - steps.round()).abs() < 1e-9,
                "`{}` step {step} does not divide its {min}..{max} range",
                spec.id
            );
        }
    }

    fn accent() -> &'static SettingSpec {
        lookup("accent_color").expect("the accent is in the schema")
    }

    /// The swatches the settings app offers stay settable.
    #[test]
    fn every_accent_name_is_accepted() {
        for name in crate::theme::ACCENT_NAMES {
            accent()
                .validate(&SettingValue::Str((*name).to_string()))
                .unwrap_or_else(|_| panic!("`{name}` is offered but refused"));
        }
    }

    /// The point of the exercise: a colour the palette has no name for.
    #[test]
    fn a_hex_accent_is_accepted() {
        for text in ["#ff00aa", "#FF00AA", "#f0a"] {
            accent()
                .validate(&SettingValue::Str(text.to_string()))
                .unwrap_or_else(|_| panic!("`{text}` is a colour but was refused"));
        }
    }

    /// Accepting hex must not turn the setting into free text: a typo still
    /// comes back as an error rather than silently painting the fallback.
    #[test]
    fn nonsense_is_still_refused() {
        for text in ["", "ff00aa", "#ff00a", "#gggggg", "chartreuse"] {
            assert!(
                accent()
                    .validate(&SettingValue::Str(text.to_string()))
                    .is_err(),
                "`{text}` is not a colour but was accepted"
            );
        }
    }

    #[test]
    fn choice_labels_when_present_cover_every_choice() {
        for spec in SETTINGS {
            if spec.choice_labels.is_empty() {
                continue;
            }
            assert_eq!(
                spec.choices.len(),
                spec.choice_labels.len(),
                "`{}` labels do not line up with its choices",
                spec.id
            );
        }
    }

    /// The language picker offers exactly the catalogues that are compiled
    /// in, named as they name themselves. Two lists that have to agree, in
    /// two crates: a catalogue nobody can pick, or a language offered that
    /// falls straight back to English, fails here rather than in front of
    /// the user.
    #[test]
    fn every_catalogue_is_offered_as_a_language() {
        let available = otto_kit::i18n::available();

        assert_eq!(LOCALE_CHOICES[0], "", "the system entry leads the list");
        assert_eq!(
            LOCALE_CHOICES[1..].to_vec(),
            available.iter().map(|(tag, _)| *tag).collect::<Vec<_>>(),
            "the offered languages have drifted from the compiled-in catalogues"
        );
        assert_eq!(
            LOCALE_LABELS[1..].to_vec(),
            available.iter().map(|(_, name)| *name).collect::<Vec<_>>(),
            "the language names have drifted from the catalogues' own"
        );
    }

    #[test]
    fn identifiers_are_unique_and_dotted_sensibly() {
        let mut seen = std::collections::HashSet::new();
        for setting in SETTINGS {
            assert!(seen.insert(setting.id), "duplicate id `{}`", setting.id);
            assert!(!setting.id.is_empty());
            assert!(!setting.id.starts_with('.') && !setting.id.ends_with('.'));
            assert!(!setting.label.is_empty(), "`{}` has no label", setting.id);
            if setting.ty == SettingType::Enum {
                assert!(
                    !setting.choices.is_empty(),
                    "enum `{}` has no choices",
                    setting.id
                );
            }
        }
    }

    #[test]
    fn every_identifier_resolves_in_the_config() {
        // The identifier scheme is "the dotted path into the config structure",
        // so a typo here would be a setting nobody can ever read or write.
        let config =
            toml::Value::try_from(Config::default()).expect("default config is valid toml");
        for setting in SETTINGS {
            let mut value = Some(&config);
            for segment in setting.id.split('.') {
                value = value.and_then(|value| value.get(segment));
            }
            // `Option<T>` fields are absent when unset, which is legitimate —
            // but their parent table must still exist.
            if value.is_none() {
                let parent = setting.section();
                assert!(
                    parent.is_empty() || config.get(parent).is_some(),
                    "`{}` has no home in the config",
                    setting.id
                );
            }
        }
    }

    #[test]
    fn lookup_finds_and_misses() {
        assert!(lookup("dock.size").is_some());
        assert!(lookup("dock.magnification").is_some());
        assert!(lookup("dock.siz").is_none());
        assert!(lookup("").is_none());
    }

    #[test]
    fn validation_rejects_wrong_types() {
        let size = lookup("dock.size").expect("dock.size exists");
        assert!(matches!(
            size.validate(&SettingValue::Str("big".into())),
            Err(Invalid::Type(_))
        ));
        // No coercion: an integer is not a double.
        assert!(matches!(
            size.validate(&SettingValue::Int(1)),
            Err(Invalid::Type(_))
        ));
        assert!(size.validate(&SettingValue::Double(1.25)).is_ok());
    }

    #[test]
    fn validation_rejects_out_of_range_and_unknown_choices() {
        let size = lookup("dock.size").expect("dock.size exists");
        assert!(matches!(
            size.validate(&SettingValue::Double(9.0)),
            Err(Invalid::Range(_))
        ));
        assert!(matches!(
            size.validate(&SettingValue::Double(0.1)),
            Err(Invalid::Range(_))
        ));
        assert!(size.validate(&SettingValue::Double(0.5)).is_ok());
        assert!(size.validate(&SettingValue::Double(2.0)).is_ok());

        let position = lookup("dock.position").expect("dock.position exists");
        assert!(position.validate(&SettingValue::Str("left".into())).is_ok());
        assert!(matches!(
            position.validate(&SettingValue::Str("top".into())),
            Err(Invalid::Range(_))
        ));
    }

    #[test]
    fn sections_come_from_the_identifier() {
        assert_eq!(
            lookup("dock.size").expect("dock.size exists").section(),
            "dock"
        );
        assert_eq!(
            lookup("screen_scale")
                .expect("screen_scale exists")
                .section(),
            ""
        );
    }
}
