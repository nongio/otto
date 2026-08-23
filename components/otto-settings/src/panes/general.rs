//! The general pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.

use crate::model::{group, Control, Pane, Row};

pub fn build() -> Pane {
    Pane {
        name: "General",
        icon: "settings",
        groups: vec![
            group(
                "Appearance",
                vec![
                    Row::new("Appearance", Control::Select("Light".into())).id("theme_scheme"),
                    Row::new("Accent colour", Control::Color(0xFF0A84FF)).id("accent_color"),
                    Row::new("Font", Control::Text("Inter".into())).id("font_family"),
                    // Applies to GTK clients rather than to Otto's own
                    // interface, so it sits with the other appearance rows but
                    // is deliberately not called "Appearance".
                    Row::new("GTK theme", Control::Text(String::new())).id("gtk_theme"),
                ],
            ),
            group(
                "Desktop",
                vec![
                    Row::new("Background colour", Control::Color(0xFF2C2CA0))
                        .id("background_color"),
                    Row::new("Background image", Control::File("".into()))
                        .detail("Chosen through the desktop portal's file picker")
                        .id("background_image"),
                ],
            ),
            group(
                "Pointer & icons",
                vec![
                    Row::new("Cursor theme", Control::Select("Notwaita-Black".into()))
                        .id("cursor_theme"),
                    Row::new(
                        "Cursor size",
                        Control::Slider {
                            value: 24.0,
                            min: 16.0,
                            max: 96.0,
                            readout: "24 px".into(),
                        },
                    )
                    .id("cursor_size"),
                    Row::new("Icon theme", Control::Select("".into())).id("icon_theme"),
                ],
            ),
            // The app switcher has no pane of its own — the workspaces pane
            // was dropped — and this is the only setting it owns.
            group(
                "Window switcher",
                vec![
                    Row::new("Show on the pointer's display", Control::Toggle(false))
                        .id("appswitcher.follow_cursor"),
                ],
            ),
            group(
                "Language",
                vec![Row::new("Preferred languages", Control::Text("en".into())).id("locales")],
            ),
        ],
    }
}
