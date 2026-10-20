//! Otto's appearance in GNOME's settings store, for GTK 3 and libadwaita.
//!
//! Outside a sandbox, GTK 3 on Wayland reads how to set and rasterise text
//! from GSettings (`org.gnome.desktop.interface`), and libadwaita reads the
//! colour scheme from there, not from the Settings portal GTK 4 reads or the
//! XSETTINGS X11 clients read. Left alone, their windows follow whatever that
//! store last said, usually what another desktop left in it, beside Otto's
//! own. This writes the same values Otto publishes elsewhere: the font
//! settings (see `Otto::font_settings`, with `font-rendering` as `manual` so a
//! GTK 4 without the portal follows them too) and the colour scheme.
//!
//! Only a session of Otto's own writes: the udev backend says so with
//! [`sync_session`], and until it has, [`sync_color_scheme`] writes nothing,
//! so a nested Otto never overwrites the desktop it runs inside. Only keys
//! that differ are written, so a store that already agrees costs a read, and
//! everything runs on a thread of its own, since each step spawns a process.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::gsettings;
use crate::state::FontSettings;

const SCHEMA: &str = "org.gnome.desktop.interface";

/// Set once the udev backend has synced: this Otto is the desktop.
static OWNS_SESSION: AtomicBool = AtomicBool::new(false);

/// Write the font settings and the colour scheme, and from now on every
/// colour scheme change: this Otto is the session's desktop.
pub fn sync_session(fonts: &FontSettings, dark: bool) {
    OWNS_SESSION.store(true, Ordering::Relaxed);
    write(vec![
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
        ("color-scheme", color_scheme(dark)),
    ]);
}

/// Write the colour scheme after it changed, if this Otto is the desktop.
pub fn sync_color_scheme(dark: bool) {
    if OWNS_SESSION.load(Ordering::Relaxed) {
        write(vec![("color-scheme", color_scheme(dark))]);
    }
}

/// GNOME's `color-scheme` for Otto's: light is `default`, which every
/// schema has (`prefer-light` arrived with GNOME 47) and which libadwaita
/// draws light.
fn color_scheme(dark: bool) -> String {
    gsettings::quoted(if dark { "prefer-dark" } else { "default" })
}

fn write(keys: Vec<(&'static str, String)>) {
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
        tracing::warn!(?err, "Could not start writing to GSettings");
    }
}
