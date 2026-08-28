//! The lock and login pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.

use crate::model::{group, Control, Pane, Row};

pub fn build() -> Pane {
    Pane {
        name: otto_kit::t!("settings-pane-lock-and-login"),
        icon: "lock",
        groups: vec![
            group(
                otto_kit::t!("settings-group-lock"),
                vec![
                    // Seconds, matching the setting's own unit — a select of
                    // pretty intervals would have to invent choices the
                    // schema does not serve, and the type would not match.
                    Row::new(
                        otto_kit::t!("settings-lock-after"),
                        Control::Slider {
                            value: 600.0,
                            min: 0.0,
                            max: 86400.0,
                            readout: "600 s".into(),
                        },
                    )
                    .id("lock.auto_lock_timeout"),
                    Row::new(
                        otto_kit::t!("settings-lock-screen"),
                        Control::Select("otto-lock".into()),
                    )
                    .detail(otto_kit::t!("settings-lock-screen-detail"))
                    .id("lock.locker_command"),
                    Row::new(
                        otto_kit::t!("settings-lock-screen-arguments"),
                        Control::Text(String::new()),
                    )
                    .id("lock.locker_args"),
                ],
            ),
            group(
                otto_kit::t!("settings-group-login"),
                vec![
                    Row::new(
                        otto_kit::t!("settings-greeter"),
                        Control::Select("otto-greeter".into()),
                    )
                    .detail(otto_kit::t!("settings-greeter-detail"))
                    .id("login.greeter_command"),
                    Row::new(
                        otto_kit::t!("settings-greeter-arguments"),
                        Control::Text(String::new()),
                    )
                    .id("login.greeter_args"),
                ],
            ),
        ],
    }
}
