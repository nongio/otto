//! The sound pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.

use crate::model::{untitled, Control, Pane, Row};

pub fn build() -> Pane {
    Pane {
        name: otto_kit::t!("settings-pane-sound"),
        icon: "sound",
        groups: vec![untitled(vec![
            Row::new(
                otto_kit::t!("settings-interface-sounds"),
                Control::Toggle(true),
            )
            .id("audio.sound_enabled"),
            Row::new(
                otto_kit::t!("settings-sound-theme"),
                Control::Select("Auto".into()),
            )
            .id("audio.sound_theme"),
        ])],
    }
}
