//! The dock pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.

use crate::model::{group, untitled, Control, Pane, Row};

pub fn build() -> Pane {
    Pane {
        name: "Dock",
        icon: "dock",
        groups: vec![
            untitled(vec![
                Row::new(
                    "Size",
                    Control::Slider {
                        value: 1.0,
                        min: 0.5,
                        max: 2.0,
                        readout: "100%".into(),
                    },
                )
                .id("dock.size"),
                Row::new("Position on screen", Control::Select("Bottom".into()))
                    .id("dock.position"),
                Row::new("Automatically hide", Control::Toggle(false)).id("dock.autohide"),
                Row::new("Magnification", Control::Toggle(true)).id("dock.magnification"),
            ]),
            group(
                "Magnification & icons",
                vec![
                    Row::new(
                        "Magnification amount",
                        Control::Slider {
                            value: 0.5,
                            min: 0.0,
                            max: 1.0,
                            readout: "0.50".into(),
                        },
                    )
                    .id("dock.genie_scale"),
                    Row::new("Tint icons", Control::Toggle(false)).id("dock.colorize_icons"),
                    Row::new("Icon tint", Control::Color(0xFF3B82F6)).id("dock.colorize_color"),
                    Row::new(
                        "Icon tint strength",
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
