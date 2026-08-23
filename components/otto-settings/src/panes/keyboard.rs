//! The keyboard pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.

use crate::model::{group, Control, Pane, Row};

pub fn build() -> Pane {
    Pane {
        name: "Keyboard",
        icon: "keyboard",
        groups: vec![
            group(
                None,
                vec![
                    Row::new(
                        "Key repeat delay",
                        Control::Slider {
                            value: 300.0,
                            min: 100.0,
                            max: 1000.0,
                            readout: "300 ms".into(),
                        },
                    )
                    .id("keyboard_repeat_delay"),
                    Row::new(
                        "Key repeat rate",
                        Control::Slider {
                            value: 30.0,
                            min: 5.0,
                            max: 60.0,
                            readout: "30 / s".into(),
                        },
                    )
                    .id("keyboard_repeat_rate"),
                ],
            ),
            group(
                Some("Input source"),
                vec![
                    // Shown, not editable: these are free text with no
                    // discoverable choice list, and the app has no text entry
                    // yet. Binding them at least stops the pane from hiding
                    // what the session is actually using.
                    Row::new("Layout", Control::Text(String::new())).id("input.xkb_layout"),
                    Row::new("Variant", Control::Text(String::new())).id("input.xkb_variant"),
                    Row::new("Options", Control::Text(String::new())).id("input.xkb_options"),
                ],
            ),
            group(
                Some("Shortcuts"),
                vec![
                    Row::new("Open terminal", Control::Value("Ctrl + Return".into())),
                    Row::new("Show all windows", Control::Value("Page Up".into())),
                    Row::new("Show desktop", Control::Value("Page Down".into())),
                    Row::new("Switch application", Control::Value("Ctrl + Tab".into())),
                    Row::new("Maximise window", Control::Value("Ctrl + ↑".into())).overridden(),
                    Row::new("Tile window left", Control::Value("Ctrl + ←".into())),
                    Row::new("Quit Otto", Control::Value("Ctrl + Esc".into())),
                ],
            ),
        ],
    }
}
