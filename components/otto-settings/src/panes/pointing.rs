//! The pointing pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.

use crate::model::{group, Control, Pane, Row};

pub fn build() -> Pane {
    Pane {
        name: "Trackpad & Mouse",
        icon: "pointer",
        groups: vec![
            group(
                "Trackpad",
                vec![
                    Row::new("Tap to click", Control::Toggle(true)).id("input.tap_enabled"),
                    Row::new("Tap and drag", Control::Toggle(true)).id("input.tap_drag_enabled"),
                    Row::new("Drag lock", Control::Toggle(false)).id("input.tap_drag_lock_enabled"),
                    Row::new("Click method", Control::Select("Click with fingers".into()))
                        .id("input.touchpad_click_method"),
                    Row::new("Ignore while typing", Control::Toggle(true))
                        .id("input.touchpad_dwt_enabled"),
                    Row::new("Natural scrolling", Control::Toggle(true))
                        .id("input.touchpad_natural_scroll_enabled"),
                    Row::new("Left-handed", Control::Toggle(false))
                        .id("input.touchpad_left_handed"),
                    Row::new("Middle-click emulation", Control::Toggle(false))
                        .id("input.touchpad_middle_emulation_enabled"),
                ],
            ),
            group(
                "Pointer",
                vec![
                    Row::new(
                        "Tracking speed",
                        Control::Slider {
                            value: 0.0,
                            min: -1.0,
                            max: 1.0,
                            readout: "0.0".into(),
                        },
                    )
                    .id("input.pointer_accel_speed"),
                    Row::new("Acceleration", Control::Select("Adaptive".into()))
                        .id("input.pointer_accel_profile"),
                    Row::new(
                        "Scrolling speed",
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
