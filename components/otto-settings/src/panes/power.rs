//! The power pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.

use crate::model::{untitled, Control, Pane, Row};

pub fn build() -> Pane {
    Pane {
        name: "Power",
        icon: "battery",
        groups: vec![untitled(vec![
            Row::new("Handle the lid switch", Control::Toggle(true))
                .detail("Otto suspends on lid close instead of logind")
                .id("power_management.manage_lid_switch"),
            Row::new("When the lid closes", Control::Select("Automatic".into()))
                .id("power_management.on_lid_close"),
            Row::new(
                "When the power button is pressed",
                Control::Select("Lock".into()),
            )
            .id("power_management.on_power_button"),
        ])],
    }
}
