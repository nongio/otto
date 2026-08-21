//! The displays pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.
//!
//! Resolution, refresh rate and "use as primary" are per-output settings —
//! in the compositor's config they live under `displays.named.<connector>`
//! (`DisplaysConfig`/`DisplayProfile` in `src/config/mod.rs`), keyed by
//! connector name. The wire contract
//! (`docs/developer/settings-dbus-api.md`, "Open" section) explicitly leaves
//! that identity unresolved: settings are meant to follow the panel, not the
//! port, and connector names do not survive a display moving to a different
//! port or a docking-station reshuffle. Inventing an identifier now (e.g.
//! `displays.named.HDMI-A-1.resolution`) would bake a wire contract we would
//! have to support forever even after a real display-identity scheme lands,
//! so these three rows stay unbound.
//!
//! Scale is different: `DisplayProfile` has no per-output scale field at
//! all today, only the single top-level `screen_scale` (see
//! `src/settings/schema.rs`). So although this row is drawn under a
//! per-display header, it is bound to that global identifier — it is the
//! only scale setting Otto actually has.

use crate::model::{group, Control, Pane, Row};

pub fn build() -> Pane {
    Pane {
        name: "Displays",
        icon: "monitor",
        // The arrangement canvas is drawn by the pane itself, not as a row.
        // Below it sit the settings for whichever display is selected there.
        groups: vec![group(
            Some("HDMI-A-1 — Dell U2720Q"),
            vec![
                Row::new("Resolution", Control::Select("3840 × 2160".into())).overridden(),
                Row::new("Refresh rate", Control::Select("60.00 Hz".into())),
                Row::new(
                    "Scale",
                    Control::Slider {
                        value: 2.0,
                        min: 0.5,
                        max: 4.0,
                        readout: "200%".into(),
                    },
                )
                .id("screen_scale"),
                Row::new("Use as primary", Control::Toggle(false))
                    .detail("The dock and the bar live on the primary display"),
            ],
        )],
    }
}
