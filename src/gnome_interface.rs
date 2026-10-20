//! Otto's font settings in GNOME's settings store, for GTK 3.
//!
//! GTK 3 on Wayland reads how to set and rasterise text from GSettings
//! (`org.gnome.desktop.interface`), not from the Settings portal GTK 4 reads
//! or the XSETTINGS X11 clients read. Left alone, its windows render text
//! however that store last said, usually what another desktop left in it,
//! beside Otto's own. This writes the same values Otto publishes elsewhere
//! (see `Otto::font_settings`), and `font-rendering` as `manual` so a GTK 4
//! without the portal follows them too.
//!
//! Only keys that differ are written, so a store that already agrees costs a
//! read. Everything runs on a thread of its own, since each step spawns a
//! process.

use crate::gsettings;
use crate::state::FontSettings;

const SCHEMA: &str = "org.gnome.desktop.interface";

/// Write `fonts` to GNOME's settings store.
pub fn sync(fonts: &FontSettings) {
    let keys = [
        ("font-name", gsettings::quoted(&fonts.name)),
        (
            "font-antialiasing",
            gsettings::quoted(fonts.rendering.antialiasing),
        ),
        ("font-hinting", gsettings::quoted(fonts.rendering.hinting)),
        (
            "font-rgba-order",
            gsettings::quoted(fonts.rendering.rgba_order),
        ),
        ("font-rendering", gsettings::quoted("manual")),
    ];
    let spawned = std::thread::Builder::new()
        .name("gnome-interface".into())
        .spawn(move || {
            for (key, value) in keys {
                // A schema older than the key (`font-rendering` arrived with
                // GNOME 47) has nothing to write it to.
                let Some(current) = gsettings::run(&["get", SCHEMA, key]) else {
                    tracing::debug!("{SCHEMA} has no {key}; not written");
                    continue;
                };
                if current != value && gsettings::run(&["set", SCHEMA, key, &value]).is_none() {
                    tracing::warn!("Could not set {SCHEMA} {key} to {value}");
                }
            }
        });
    if let Err(err) = spawned {
        tracing::warn!(
            ?err,
            "Could not start writing the font settings to GSettings"
        );
    }
}
