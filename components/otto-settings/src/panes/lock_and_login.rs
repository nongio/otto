//! The lock and login pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.
//!
//! Every setting here names a program the compositor runs in front of the
//! password, or decides whether the screen locks at all, so the compositor
//! asks for the user's password before it applies a change to one
//! (`confirm = "password"` in the schema; specs/permissions.md §8.7). Each row
//! says so under its label, and `main::apply` sends the change off the main
//! thread, since the answer waits on the password panel.

use std::borrow::Cow;

use crate::model::{group, Control, Pane, Row};
use crate::settings_client;

pub fn build() -> Pane {
    Pane {
        name: otto_kit::t!("settings-pane-lock-and-login"),
        icon: "lock",
        intro: None,
        groups: vec![
            group(
                otto_kit::t!("settings-group-lock"),
                vec![
                    // A pop-up of intervals rather than a slider: a slider
                    // sends a change per pointer motion, and each change here
                    // asks for the password. The menu keeps the current value
                    // even when it is not one of the offered intervals.
                    asks_for_password(
                        Row::new(
                            otto_kit::t!("settings-lock-after"),
                            Control::Select("600".into()),
                        )
                        .id("lock.auto_lock_timeout"),
                    ),
                    // On by default: a suspend from anywhere wakes to the lock
                    // screen. Turning it off weakens the lock, so it asks.
                    asks_for_password(
                        Row::new(
                            otto_kit::t!("settings-lock-on-suspend"),
                            Control::Toggle(true),
                        )
                        .id("lock.on_suspend"),
                    ),
                    asks_for_password(
                        Row::new(
                            otto_kit::t!("settings-lock-screen"),
                            Control::Select("otto-lock".into()),
                        )
                        .detail(otto_kit::t!("settings-lock-screen-detail"))
                        .id("lock.locker_command"),
                    ),
                    asks_for_password(
                        Row::new(
                            otto_kit::t!("settings-lock-screen-arguments"),
                            Control::Text(String::new()),
                        )
                        .id("lock.locker_args"),
                    ),
                ],
            ),
            group(
                otto_kit::t!("settings-group-login"),
                vec![
                    asks_for_password(
                        Row::new(
                            otto_kit::t!("settings-greeter"),
                            Control::Select("otto-greeter".into()),
                        )
                        .detail(otto_kit::t!("settings-greeter-detail"))
                        .id("login.greeter_command"),
                    ),
                    asks_for_password(
                        Row::new(
                            otto_kit::t!("settings-greeter-arguments"),
                            Control::Text(String::new()),
                        )
                        .id("login.greeter_args"),
                    ),
                ],
            ),
        ],
    }
}

/// Add "Changing this asks for your password" to a bound row's detail, when
/// the compositor says the setting asks — a compositor that does not ask is
/// not described as one that does.
fn asks_for_password(mut row: Row) -> Row {
    let asks = row
        .id
        .and_then(settings_client::describe)
        .is_some_and(|desc| desc.sensitive);
    if !asks {
        return row;
    }
    let note = otto_kit::t!("settings-asks-for-password");
    row.detail = Some(match row.detail.take() {
        Some(detail) if !detail.is_empty() => Cow::Owned(format!("{detail} · {note}")),
        _ => Cow::Borrowed(note),
    });
    row
}
