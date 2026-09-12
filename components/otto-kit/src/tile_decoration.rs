//! How much chrome a tiled window keeps.
//!
//! Travels exactly the way corner rounding does — see [`crate::corners`]: the
//! compositor reads `[tiling] decoration` from its configuration, publishes the
//! answer in the environment, and everything that draws a titlebar reads it
//! back from here. A change while a client is running arrives over
//! `org.otto.Settings` and the Settings portal, and
//! [`crate::desktop_appearance`] stores it here with [`set`].
//!
//! The setting only ever matters for a window the compositor has told is
//! tiled: a floating window keeps its full decoration under every value.

use std::sync::atomic::{AtomicU8, Ordering};

/// The variable the compositor publishes.
pub const ENV: &str = "OTTO_TILING_DECORATION";

/// What a tile's decoration reduces to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TileDecoration {
    /// The same bar a floating window wears — title, all three controls — on
    /// a tile, which still squares its corners and drops its shadow. For a
    /// desktop that tiles occasionally and wants its windows to look the same
    /// either way.
    Normal,
    /// A bar one text line high with the title and a close control, squared
    /// corners, no shadow. Otto's default, and i3's.
    #[default]
    Minimal,
    /// No bar at all; the compositor marks the focused tile with a hairline
    /// border instead. Sway's `default_border pixel`.
    None,
}

impl TileDecoration {
    /// The configuration token, and what goes on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            TileDecoration::Normal => "normal",
            TileDecoration::Minimal => "minimal",
            TileDecoration::None => "none",
        }
    }

    /// Parse a configuration token, case-insensitively. Anything else is
    /// `None`, and the caller keeps the default rather than guessing.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "normal" => Some(TileDecoration::Normal),
            "minimal" => Some(TileDecoration::Minimal),
            "none" => Some(TileDecoration::None),
            _ => None,
        }
    }
}

/// 0 = not resolved yet, 1 = minimal, 2 = none, 3 = normal.
static DECORATION: AtomicU8 = AtomicU8::new(0);

/// What the desktop reduces a tile's decoration to.
pub fn decoration() -> TileDecoration {
    match DECORATION.load(Ordering::Relaxed) {
        0 => {
            let value = std::env::var(ENV)
                .ok()
                .and_then(|text| TileDecoration::parse(&text))
                .unwrap_or_default();
            set(value);
            value
        }
        2 => TileDecoration::None,
        3 => TileDecoration::Normal,
        _ => TileDecoration::Minimal,
    }
}

/// Update the answer without touching the environment — see
/// [`crate::corners::set`].
pub fn set(value: TileDecoration) {
    DECORATION.store(
        match value {
            TileDecoration::Minimal => 1,
            TileDecoration::None => 2,
            TileDecoration::Normal => 3,
        },
        Ordering::Relaxed,
    );
}

/// Publish `value` to this process and everything it starts, and return the
/// assignment for the session's activation environments. See
/// [`crate::corners::export`].
pub fn export(value: TileDecoration) -> String {
    set(value);
    std::env::set_var(ENV, value.as_str());
    format!("{ENV}={}", value.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_round_trip() {
        for value in [
            TileDecoration::Normal,
            TileDecoration::Minimal,
            TileDecoration::None,
        ] {
            assert_eq!(TileDecoration::parse(value.as_str()), Some(value));
        }
        assert_eq!(TileDecoration::parse("  NONE "), Some(TileDecoration::None));
        // An unknown token is not a decoration; the caller keeps its default
        // rather than dropping a window's bar on a typo.
        assert_eq!(TileDecoration::parse("pixel 2"), None);
        assert_eq!(TileDecoration::default(), TileDecoration::Minimal);
    }
}
