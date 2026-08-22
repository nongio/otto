//! The lock and login pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.

use crate::model::{group, Control, Pane, Row};

pub fn build() -> Pane {
    Pane {
        name: "Lock & Login",
        icon: "lock",
        groups: vec![
            group(
                Some("Lock"),
                vec![
                    // Seconds, matching the setting's own unit — a select of
                    // pretty intervals would have to invent choices the
                    // schema does not serve, and the type would not match.
                    Row::new(
                        "Lock after",
                        Control::Slider {
                            value: 600.0,
                            min: 0.0,
                            max: 86400.0,
                            readout: "600 s".into(),
                        },
                    )
                    .id("lock.auto_lock_timeout"),
                    Row::new("Lock screen", Control::Select("otto-lock".into()))
                        .detail("Applies the next time the screen locks")
                        .id("lock.locker_command"),
                    Row::new("Lock screen arguments", Control::Value(String::new()))
                        .id("lock.locker_args"),
                ],
            ),
            group(
                Some("Login"),
                vec![
                    Row::new("Greeter", Control::Select("otto-greeter".into()))
                        .detail("Applies at the next login")
                        .id("login.greeter_command"),
                    Row::new("Greeter arguments", Control::Value(String::new()))
                        .id("login.greeter_args"),
                ],
            ),
        ],
    }
}
