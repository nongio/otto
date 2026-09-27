//! What the window remembers between runs, in
//! `$XDG_STATE_HOME/otto/files.toml` (`~/.local/state/otto/files.toml`).
//!
//! State, not configuration: nothing here is something a person writes by
//! hand, so it lives apart from `~/.config/otto/files.toml` — which is
//! theirs, comments and all, and which this module never rewrites. The whole
//! file is optional and losing it costs nothing but a remembered position.
//!
//! ```toml
//! [palette]
//! # How far the command palette was last dragged from where it opens, in
//! # window points.
//! offset = [-320.0, 210.0]
//!
//! [photos]
//! # The Photos view's row height and how it groups its pictures.
//! row_height = 190.0
//! group = "day"
//!
//! [icons]
//! # The icon view's icon size.
//! size = 64.0
//!
//! # The view chosen for each folder, least recently chosen first.
//! [[views]]
//! path = "/home/me/Pictures"
//! view = "photos"
//! ```

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Remembered {
    #[serde(default)]
    pub palette: Palette,
    #[serde(default)]
    pub photos: Photos,
    #[serde(default)]
    pub icons: Icons,
    /// The view chosen for each folder, least recently chosen first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<FolderView>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Icons {
    /// The icon view's icon size, in points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
}

/// One folder's chosen view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FolderView {
    pub path: String,
    /// `list`, `grid`, `columns` or `photos`.
    pub view: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Photos {
    /// The row height the size slider was left at, in points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_height: Option<f32>,
    /// How the pictures were grouped: `day`, `month` or `none`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Palette {
    /// Where the palette was last left, as an offset from where it opens.
    /// Relative rather than absolute because the window moves and resizes
    /// between runs, and "the same distance from its resting place" survives
    /// that where "the same corner of the screen" would not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<(f32, f32)>,
}

impl Remembered {
    /// Read the state file, if it is there. Anything wrong with it is a
    /// warning and the defaults.
    pub fn load() -> Self {
        let Some(path) = state_path() else {
            return Self::default();
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(err) => {
                tracing::warn!("could not read {}: {err}", path.display());
                return Self::default();
            }
        };
        match toml::from_str(&text) {
            Ok(remembered) => remembered,
            Err(err) => {
                tracing::warn!("could not parse {}: {err}", path.display());
                Self::default()
            }
        }
    }

    /// Write the state file, creating the directory on the way. Quiet on
    /// failure: a position that could not be remembered is not worth an
    /// error in the user's face.
    pub fn save(&self) {
        let Some(path) = state_path() else {
            return;
        };
        let text = match toml::to_string(self) {
            Ok(text) => text,
            Err(err) => {
                tracing::warn!("could not serialise state: {err}");
                return;
            }
        };
        if let Some(dir) = path.parent() {
            if let Err(err) = std::fs::create_dir_all(dir) {
                tracing::warn!("could not create {}: {err}", dir.display());
                return;
            }
        }
        if let Err(err) = std::fs::write(&path, text) {
            tracing::warn!("could not write {}: {err}", path.display());
        }
    }
}

/// Where the state file is, honouring `XDG_STATE_HOME`. `OTTO_FILES_STATE`
/// overrides the whole path, for tests and for running two copies side by
/// side.
fn state_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("OTTO_FILES_STATE") {
        return Some(PathBuf::from(path));
    }
    let base = std::env::var("XDG_STATE_HOME")
        .ok()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".local").join("state"))
        })?;
    Some(base.join("otto").join("files.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offset_survives_a_round_trip_and_nothing_else_is_written() {
        let remembered = Remembered {
            palette: Palette {
                offset: Some((-320.0, 210.5)),
            },
            photos: Photos {
                row_height: Some(240.0),
                group: Some("month".into()),
            },
            icons: Icons { size: Some(96.0) },
            views: vec![FolderView {
                path: "/home/me/Pictures".into(),
                view: "photos".into(),
            }],
        };
        let text = toml::to_string(&remembered).unwrap();
        assert!(text.contains("offset"));
        let back: Remembered = toml::from_str(&text).unwrap();
        assert_eq!(back, remembered);

        let empty = toml::to_string(&Remembered::default()).unwrap();
        let back: Remembered = toml::from_str(&empty).unwrap();
        assert_eq!(back.palette.offset, None);
    }

    /// The info panel used to be a toggle remembered as `info`; it follows
    /// the selection now, and a file that still says so reads as before.
    #[test]
    fn an_old_info_key_is_ignored() {
        let back: Remembered =
            toml::from_str("[photos]\nrow_height = 240.0\ninfo = true\n").unwrap();
        assert_eq!(back.photos.row_height, Some(240.0));
    }

    #[test]
    fn a_file_from_the_future_still_reads() {
        let back: Remembered = toml::from_str(
            "[palette]\noffset = [1.0, 2.0]\nsomething_new = true\n[other]\nx = 1\n",
        )
        .unwrap();
        assert_eq!(back.palette.offset, Some((1.0, 2.0)));
    }
}
