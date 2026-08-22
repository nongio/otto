//! The sound pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.

use crate::model::{group, Control, Pane, Row};

pub fn build() -> Pane {
    Pane {
        name: "Sound",
        icon: "sound",
        groups: vec![group(
            None,
            vec![
                Row::new("Interface sounds", Control::Toggle(true)).id("audio.sound_enabled"),
                Row::new("Sound theme", Control::Select("Auto".into())).id("audio.sound_theme"),
            ],
        )],
    }
}
