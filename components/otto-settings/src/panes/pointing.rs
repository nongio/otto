//! The pointing pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.

use crate::model::{group, Control, Pane, Row};

pub fn build() -> Pane {
    Pane {
        name: otto_kit::t!("settings-pane-pointing"),
        icon: "pointer",
        groups: vec![
            group(
                otto_kit::t!("settings-group-trackpad"),
                vec![
                    Row::new(otto_kit::t!("settings-tap-to-click"), Control::Toggle(true))
                        .id("input.tap_enabled"),
                    Row::new(otto_kit::t!("settings-tap-and-drag"), Control::Toggle(true))
                        .id("input.tap_drag_enabled"),
                    Row::new(otto_kit::t!("settings-drag-lock"), Control::Toggle(false))
                        .id("input.tap_drag_lock_enabled"),
                    Row::new(
                        otto_kit::t!("settings-click-method"),
                        Control::Select("Click with fingers".into()),
                    )
                    .id("input.touchpad_click_method"),
                    Row::new(
                        otto_kit::t!("settings-ignore-while-typing"),
                        Control::Toggle(true),
                    )
                    .id("input.touchpad_dwt_enabled"),
                    Row::new(
                        otto_kit::t!("settings-natural-scrolling"),
                        Control::Toggle(true),
                    )
                    .id("input.touchpad_natural_scroll_enabled"),
                    Row::new(otto_kit::t!("settings-left-handed"), Control::Toggle(false))
                        .id("input.touchpad_left_handed"),
                    Row::new(
                        otto_kit::t!("settings-middle-click-emulation"),
                        Control::Toggle(false),
                    )
                    .id("input.touchpad_middle_emulation_enabled"),
                ],
            ),
            group(
                otto_kit::t!("settings-group-pointer"),
                vec![
                    Row::new(
                        otto_kit::t!("settings-tracking-speed"),
                        Control::Slider {
                            value: 0.0,
                            min: -1.0,
                            max: 1.0,
                            readout: "0.0".into(),
                        },
                    )
                    .id("input.pointer_accel_speed"),
                    Row::new(
                        otto_kit::t!("settings-pointer-acceleration"),
                        Control::Select("Adaptive".into()),
                    )
                    .id("input.pointer_accel_profile"),
                    Row::new(
                        otto_kit::t!("settings-scrolling-speed"),
                        Control::Slider {
                            value: 1.0,
                            min: 0.1,
                            max: 2.0,
                            readout: "1.0".into(),
                        },
                    )
                    .id("input.scroll_speed"),
                ],
            ),
        ],
    }
}
