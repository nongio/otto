//! What the window renders. Filled in from MPRIS once the D-Bus client lands.

use skia_safe::Image;

#[derive(Clone)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Playback position and track length, in microseconds (MPRIS units).
    pub position: u64,
    pub length: u64,
    pub playing: bool,
    pub cover: Option<Image>,
    /// Scan of the record label itself (Cover Art Archive image type `Medium`).
    pub label: Option<Image>,
}

impl Track {
    /// A stand-in track so the layout can be developed without a player.
    pub fn example() -> Self {
        Self {
            title: "Disorder".to_string(),
            artist: "Joy Division".to_string(),
            album: "Unknown Pleasures".to_string(),
            position: 96_000_000,
            length: 209_000_000,
            playing: true,
            cover: crate::cover::bundled_cover().or_else(|| Some(crate::cover::example_cover(600))),
            label: crate::cover::bundled_label(),
        }
    }

    pub fn progress(&self) -> f32 {
        if self.length == 0 {
            0.0
        } else {
            self.position as f32 / self.length as f32
        }
    }
}
