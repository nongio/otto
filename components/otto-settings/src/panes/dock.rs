//! The dock pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.

use crate::model::{group, untitled, Control, Pane, Row};

pub fn build() -> Pane {
    Pane {
        name: otto_kit::t!("settings-pane-dock"),
        icon: "dock",
        groups: vec![
            untitled(vec![
                Row::new(
                    otto_kit::t!("settings-dock-size"),
                    Control::Slider {
                        value: 1.0,
                        min: 0.5,
                        max: 2.0,
                        readout: "100%".into(),
                    },
                )
                .id("dock.size"),
                Row::new(
                    otto_kit::t!("settings-dock-position"),
                    Control::Select("Bottom".into()),
                )
                .id("dock.position"),
                Row::new(
                    otto_kit::t!("settings-dock-autohide"),
                    Control::Toggle(false),
                )
                .id("dock.autohide"),
                Row::new(
                    otto_kit::t!("settings-dock-magnification"),
                    Control::Toggle(true),
                )
                .id("dock.magnification"),
            ]),
            group(
                otto_kit::t!("settings-group-magnification-and-icons"),
                vec![
                    Row::new(
                        otto_kit::t!("settings-dock-magnification-amount"),
                        Control::Slider {
                            value: 0.5,
                            min: 0.0,
                            max: 1.0,
                            readout: "0.50".into(),
                        },
                    )
                    .id("dock.genie_scale"),
                    Row::new(
                        otto_kit::t!("settings-dock-tint-icons"),
                        Control::Toggle(false),
                    )
                    .id("dock.colorize_icons"),
                    Row::new(
                        otto_kit::t!("settings-dock-icon-tint"),
                        Control::Color(0xFF3B82F6),
                    )
                    .id("dock.colorize_color"),
                    Row::new(
                        otto_kit::t!("settings-dock-icon-tint-strength"),
                        Control::Slider {
                            value: 0.5,
                            min: 0.0,
                            max: 1.0,
                            readout: "0.50".into(),
                        },
                    )
                    .id("dock.colorize_intensity"),
                ],
            ),
        ],
    }
}
