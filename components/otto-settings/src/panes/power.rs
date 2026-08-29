//! The power pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.

use crate::model::{untitled, Control, Pane, Row};

pub fn build() -> Pane {
    Pane {
        name: otto_kit::t!("settings-pane-power"),
        icon: "battery",
        groups: vec![untitled(vec![
            Row::new(
                otto_kit::t!("settings-manage-lid-switch"),
                Control::Toggle(true),
            )
            .detail(otto_kit::t!("settings-manage-lid-switch-detail"))
            .id("power_management.manage_lid_switch"),
            Row::new(
                otto_kit::t!("settings-on-lid-close"),
                Control::Select("Automatic".into()),
            )
            .id("power_management.on_lid_close"),
            Row::new(
                otto_kit::t!("settings-on-power-button"),
                Control::Select("Lock".into()),
            )
            .id("power_management.on_power_button"),
        ])],
    }
}
