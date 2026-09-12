//! The tiling pane: the `[tiling]` defaults every tiled workspace starts from.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet. Every row here is bound — the pane is
//! exactly the `tiling.*` half of the schema, and the test below says so, so
//! a setting added to one and not the other is caught rather than quietly
//! unreachable.

use crate::model::{group, untitled, Control, Pane, Row};

pub fn build() -> Pane {
    Pane {
        name: otto_kit::t!("settings-pane-tiling"),
        icon: "tiling",
        intro: Some(otto_kit::t!("settings-tiling-intro")),
        groups: vec![
            untitled(vec![Row::new(
                otto_kit::t!("settings-tiling-decoration"),
                Control::Select("minimal".into()),
            )
            .id("tiling.decoration")]),
            group(
                otto_kit::t!("settings-group-tiling-gaps"),
                vec![
                    Row::new(
                        otto_kit::t!("settings-tiling-inner-gap"),
                        Control::Slider {
                            value: 8.0,
                            min: 0.0,
                            max: 64.0,
                            readout: "8 px".into(),
                        },
                    )
                    .id("tiling.inner_gap"),
                    Row::new(
                        otto_kit::t!("settings-tiling-outer-gap"),
                        Control::Slider {
                            value: 8.0,
                            min: 0.0,
                            max: 64.0,
                            readout: "8 px".into(),
                        },
                    )
                    .id("tiling.outer_gap"),
                    Row::new(
                        otto_kit::t!("settings-tiling-smart-gaps"),
                        Control::Toggle(true),
                    )
                    .id("tiling.smart_gaps"),
                ],
            ),
            group(
                otto_kit::t!("settings-group-tiling-keyboard"),
                vec![Row::new(
                    otto_kit::t!("settings-tiling-resize-step"),
                    Control::Slider {
                        value: 0.05,
                        min: 0.01,
                        max: 0.5,
                        readout: "0.05".into(),
                    },
                )
                .id("tiling.resize_step")],
            ),
            // Durations in seconds, matching the setting's own unit, and each
            // paired with the bounce of the same spring — the two numbers only
            // mean anything beside each other. Zero seconds is the documented
            // way to turn an animation off, so the sliders start at zero
            // rather than at some smallest visible duration.
            group(
                otto_kit::t!("settings-group-tiling-animation"),
                vec![
                    Row::new(
                        otto_kit::t!("settings-tiling-layout-duration"),
                        Control::Slider {
                            value: 0.3,
                            min: 0.0,
                            max: 2.0,
                            readout: "0.30 s".into(),
                        },
                    )
                    .id("tiling.layout_duration"),
                    Row::new(
                        otto_kit::t!("settings-tiling-layout-bounce"),
                        Control::Slider {
                            value: 0.0,
                            min: 0.0,
                            max: 1.0,
                            readout: "0.00".into(),
                        },
                    )
                    .id("tiling.layout_bounce"),
                    Row::new(
                        otto_kit::t!("settings-tiling-mode-duration"),
                        Control::Slider {
                            value: 0.4,
                            min: 0.0,
                            max: 2.0,
                            readout: "0.40 s".into(),
                        },
                    )
                    .id("tiling.mode_duration"),
                    Row::new(
                        otto_kit::t!("settings-tiling-mode-bounce"),
                        Control::Slider {
                            value: 0.1,
                            min: 0.0,
                            max: 1.0,
                            readout: "0.10".into(),
                        },
                    )
                    .id("tiling.mode_bounce"),
                ],
            ),
        ],
    }
}

#[cfg(test)]
mod tests {
    /// The pane is the `tiling.*` section of the schema, no more and no less.
    ///
    /// Written as an exact list rather than "every row is bound": a `tiling.*`
    /// setting the compositor grows and this pane never gains is a setting
    /// with no way to reach it, and that failure is invisible — the pane
    /// simply does not mention it.
    #[test]
    fn the_pane_lists_exactly_the_tiling_settings() {
        let listed: Vec<&str> = super::build()
            .groups
            .iter()
            .flat_map(|group| group.rows.iter())
            .filter_map(|row| row.id)
            .collect();

        assert_eq!(
            listed,
            vec![
                "tiling.decoration",
                "tiling.inner_gap",
                "tiling.outer_gap",
                "tiling.smart_gaps",
                "tiling.resize_step",
                "tiling.layout_duration",
                "tiling.layout_bounce",
                "tiling.mode_duration",
                "tiling.mode_bounce",
            ]
        );

        // And nothing unbound slipped in beside them: a display-only row in
        // this pane would be a control that changes nothing.
        let rows = super::build()
            .groups
            .iter()
            .map(|group| group.rows.len())
            .sum::<usize>();
        assert_eq!(rows, listed.len());
    }
}
