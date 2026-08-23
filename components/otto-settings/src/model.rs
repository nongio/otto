//! Static stand-in for the settings schema.
//!
//! The real app fetches this from the compositor over `org.otto.Settings`
//! (see `specs/settings-app.md`). Nothing here is wired: the values are
//! plausible constants chosen to exercise every control the panes need.

use std::borrow::Cow;

use crate::panes;
use crate::settings_client::{self, Value};

/// One row in a pane.
pub enum Control {
    /// On/off switch.
    Toggle(bool),
    /// Continuous value: current, min, max, and the formatted readout.
    Slider {
        value: f32,
        min: f32,
        max: f32,
        readout: String,
    },
    /// Pop-up button showing the current choice.
    Select(String),
    /// Colour swatch plus its hex value.
    Color(u32),
    /// Free text (paths, command lines, font names). No pane uses it since
    /// the background image became a [`Control::File`]; kept because it is
    /// the natural control for the next free-text setting that appears.
    #[allow(dead_code)]
    Text(String),
    /// A file the user picks through the desktop portal. Holds the chosen
    /// path, empty until one is chosen.
    File(String),
    /// Static informational value, not editable here.
    Value(String),
    /// One editable shortcut line: the action pop-up, the key combination
    /// field, and the button that deletes it.
    ///
    /// `index` is the line's position in [`crate::panes::keyboard`]'s list,
    /// which is what every hit test and edit is addressed by — the row itself
    /// holds no state, since the model is rebuilt on every frame.
    Shortcut { index: usize },
    /// The trailing line that appends a shortcut.
    AddShortcut,
}

pub struct Row {
    pub label: &'static str,
    /// Secondary line under the label. `None` keeps the row single-height.
    ///
    /// Usually the served schema's description — help text belongs to the
    /// compositor, which owns the setting — but a pane can write its own for
    /// something the schema cannot know, such as a caveat about where a
    /// setting takes effect.
    pub detail: Option<Cow<'static, str>>,
    pub control: Control,
    /// Row differs from the inherited value, so it offers a reset.
    pub overridden: bool,
    /// Change is persisted but needs a restart to take effect.
    pub restart_required: bool,
    /// The `org.otto.Settings` identifier this row edits. `None` means the row
    /// is not wired to the compositor yet and is display-only.
    pub id: Option<&'static str>,
}

impl Row {
    pub(crate) fn new(label: &'static str, control: Control) -> Self {
        Self {
            label,
            detail: None,
            control,
            overridden: false,
            restart_required: false,
            id: None,
        }
    }

    /// Bind the row to a settings identifier, and take its current value and
    /// override state from the compositor when it is serving them.
    pub(crate) fn id(mut self, id: &'static str) -> Self {
        self.id = Some(id);
        self.overridden = settings_client::is_overridden(id);

        let desc = settings_client::describe(id);
        let step = desc.as_ref().and_then(|d| d.step);

        // The schema owns the range. A pane's min/max is a placeholder written
        // before the contract existed, and where the two disagree the pane is
        // simply wrong — a slider that stops at 1000 ms cannot reach a value
        // the compositor will happily accept up to 2000.
        if let Some(desc) = desc.as_ref() {
            if let Control::Slider {
                value,
                min,
                max,
                readout,
            } = self.control
            {
                self.control = Control::Slider {
                    value,
                    min: desc.min.map(|m| m as f32).unwrap_or(min),
                    max: desc.max.map(|m| m as f32).unwrap_or(max),
                    readout,
                };
            }
        }

        if let Some(value) = settings_client::value(id) {
            self.control = self.control.with_value(&value, step);
        }
        // Only badge a setting that has actually been changed and is waiting
        // on a restart. Almost everything is `Apply::Restart` today, so
        // badging from the schema alone would put a pill on every row and say
        // nothing.
        self.restart_required = settings_client::is_pending_restart(id);

        // Help text comes from the schema so there is one source of truth: the
        // compositor owns the setting, so it owns the sentence explaining it.
        // A pane that already wrote its own detail keeps it.
        if self.detail.is_none() {
            if let Some(desc) = desc {
                if !desc.description.is_empty() {
                    self.detail = Some(Cow::Owned(desc.description));
                }
            }
        }
        self
    }

    pub(crate) fn detail(mut self, detail: impl Into<Cow<'static, str>>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub(crate) fn overridden(mut self) -> Self {
        self.overridden = true;
        self
    }

    /// Mark a row as restart-required by hand.
    ///
    /// Bound rows take this from the served schema instead, so this is only
    /// for rows that have no identifier yet and are known to need a restart.
    #[allow(dead_code)]
    pub(crate) fn restart(mut self) -> Self {
        self.restart_required = true;
        self
    }
}

impl Control {
    /// Replace the control's value with one served by the compositor, keeping
    /// everything the schema does not own — the readout format the app chose
    /// for it. `step` sets how many decimals that readout needs.
    fn with_value(self, value: &Value, step: Option<f64>) -> Self {
        match (self, value) {
            (Control::Toggle(_), Value::Bool(on)) => Control::Toggle(*on),
            (
                Control::Slider {
                    min, max, readout, ..
                },
                value,
            ) => {
                let new = value.as_f32().unwrap_or(min);
                Control::Slider {
                    // The readout is a format, not a value: re-render it
                    // around the new number rather than keeping stale text.
                    readout: reformat_readout(&readout, new, step),
                    value: new,
                    min,
                    max,
                }
            }
            (Control::Select(_), Value::Text(text)) => Control::Select(text.clone()),
            (Control::Text(_), Value::Text(text)) => Control::Text(text.clone()),
            // A list setting edited as text — preferred languages, xkb options
            // — reads and writes the comma-separated form.
            (Control::Text(_), Value::List(items)) => Control::Text(items.join(", ")),
            (Control::File(_), Value::Text(text)) => Control::File(text.clone()),
            // Enumerated colour settings (the accent) carry a name rather than
            // hex, so a well that only understood hex drew them as black.
            (Control::Color(_), Value::Text(text)) => Control::Color(
                parse_hex(text)
                    .or_else(|| named_argb(text))
                    .unwrap_or(0xFF000000),
            ),
            (Control::Value(_), Value::List(items)) => Control::Value(items.join(", ")),
            (Control::Value(_), Value::Text(text)) => Control::Value(text.clone()),
            // A value whose type does not fit the control it was bound to:
            // keep drawing the placeholder rather than inventing something.
            (control, _) => control,
        }
    }
}

/// Rebuild a readout string around a new number, preserving whatever unit or
/// formatting the pane chose ("24 px", "150%", "0.50").
fn reformat_readout(previous: &str, value: f32, step: Option<f64>) -> String {
    if previous.ends_with('%') {
        format!("{:.0}%", value * 100.0)
    } else if let Some(unit) = previous.split_once(char::is_whitespace).map(|(_, u)| u) {
        format!("{value:.0} {unit}")
    } else if previous.contains('.') {
        format!("{value:.precision$}", precision = decimals(step))
    } else {
        format!("{value:.0}")
    }
}

/// Decimals a readout needs to show every position its slider can stop on.
/// Showing more is how `0.0` became `-0.01`; showing fewer would make two
/// distinct steps read identically.
fn decimals(step: Option<f64>) -> usize {
    match step {
        Some(step) if step >= 1.0 => 0,
        Some(step) if step >= 0.1 => 1,
        _ => 2,
    }
}

/// The compositor's accent palette, keyed by the name it stores.
///
/// These are the values `src/theme/colors_dark.rs` paints with, so the swatch
/// the user picks is the colour they get.
pub fn named_argb(name: &str) -> Option<u32> {
    Some(match name.to_ascii_lowercase().as_str() {
        "red" => 0xFFFF453A,
        "orange" => 0xFFFF9F0A,
        "yellow" => 0xFFFFD60A,
        "green" => 0xFF32D74B,
        "mint" => 0xFF66D4CF,
        "teal" => 0xFF6AC4DC,
        "cyan" => 0xFF5AC8F5,
        "blue" => 0xFF0A84FF,
        "indigo" => 0xFF5E5CE6,
        "purple" => 0xFFBF5AF2,
        "pink" => 0xFFFF375F,
        "gray" | "grey" | "graphite" => 0xFF98989D,
        "brown" => 0xFFAC8E68,
        _ => return None,
    })
}

fn parse_hex(text: &str) -> Option<u32> {
    let digits = text.strip_prefix('#')?;
    u32::from_str_radix(digits, 16)
        .ok()
        .map(|rgb| 0xFF00_0000 | rgb)
}

/// A titled run of rows inside a pane.
pub struct Group {
    pub title: Option<&'static str>,
    pub rows: Vec<Row>,
}

pub struct Pane {
    pub name: &'static str,
    /// Sidebar glyph name, drawn by `glyphs::draw`.
    pub icon: &'static str,
    pub groups: Vec<Group>,
}

/// The eight panes from the spec, in sidebar order.
pub fn panes() -> Vec<Pane> {
    vec![
        panes::general::build(),
        panes::displays::build(),
        panes::dock::build(),
        panes::keyboard::build(),
        panes::pointing::build(),
        panes::sound::build(),
        panes::power::build(),
        panes::lock_and_login::build(),
    ]
}

pub(crate) fn group(title: Option<&'static str>, rows: Vec<Row>) -> Group {
    Group { title, rows }
}

/// Outputs shown in the Displays arrangement canvas. Positions and sizes are
/// in compositor logical pixels; the canvas scales them to fit.
pub struct Output {
    pub name: &'static str,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub primary: bool,
    /// Whose settings the pane is showing below the canvas.
    pub selected: bool,
}

pub fn outputs() -> Vec<Output> {
    vec![
        Output {
            name: "eDP-1",
            x: 0.0,
            y: 560.0,
            width: 1128.0,
            height: 752.0,
            primary: true,
            selected: false,
        },
        Output {
            name: "HDMI-A-1",
            x: 1128.0,
            y: 0.0,
            width: 2560.0,
            height: 1440.0,
            primary: false,
            selected: true,
        },
    ]
}
