//! Where the person put each card, remembered by node identity.
//!
//! Kept in `$XDG_STATE_HOME/otto/audio/layout.toml`: it is the window's own
//! memory, not something the service needs. A card is keyed by its node's
//! identity and its place among nodes of the same identity, so two streams of
//! one app keep two places.

// Rust guideline compliant 2026-02-21

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Every remembered card position, in canvas points.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    #[serde(default)]
    cards: BTreeMap<String, [f32; 2]>,
}

impl Layout {
    /// Where the layout is kept.
    #[must_use]
    pub fn path() -> Option<PathBuf> {
        otto_kit::xdg::state_home().map(|dir| dir.join("otto/audio/layout.toml"))
    }

    /// Read the layout at `path`; missing or unreadable is empty.
    #[must_use]
    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Write the layout to `path`.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let text = toml::to_string(self).map_err(std::io::Error::other)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        otto_kit::fs::write_atomically(path, text.as_bytes())
    }

    /// The remembered place of the card keyed `key`.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<(f32, f32)> {
        self.cards.get(key).map(|[x, y]| (*x, *y))
    }

    /// Remember `key` at `(x, y)`.
    pub fn set(&mut self, key: String, (x, y): (f32, f32)) {
        self.cards.insert(key, [x, y]);
    }
}

/// The key a card is remembered by: its identity, and which of the nodes
/// sharing that identity it is (0 for the first by id).
#[must_use]
pub fn card_key(identity: &str, index: usize) -> String {
    if index == 0 {
        identity.to_string()
    } else {
        format!("{identity}#{index}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio/layout.toml");
        let mut layout = Layout::default();
        layout.set(card_key("node:headset", 0), (12.0, 340.5));
        layout.set(card_key("stream:mpv:playback", 1), (-4.0, 8.0));
        layout.save_to(&path).unwrap();
        let back = Layout::load_from(&path);
        assert_eq!(back, layout);
        assert_eq!(back.get("stream:mpv:playback#1"), Some((-4.0, 8.0)));
    }

    #[test]
    fn a_broken_file_is_an_empty_layout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layout.toml");
        std::fs::write(&path, "cards = 3").unwrap();
        assert_eq!(Layout::load_from(&path), Layout::default());
    }
}
