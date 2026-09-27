//! The desk: one folder shown on the desktop itself, between the wallpaper
//! and the windows — see `specs/desk.md`.
//!
//! The desk is another shell over the browser's view layer, like the Trash
//! window (see [`crate::view::Shell`]). What lives here is only what the
//! browser has no use for: the `[desk]` section of `~/.config/otto/files.toml`,
//! where the panel sits inside its surface, and the lock that keeps one desk
//! per session.
//!
//! ```toml
//! [desk]
//! folder = "~/Desktop"    # default: XDG_DESKTOP_DIR, then ~/Desktop
//! arrange = "grid"        # grid | free
//! sort = "name"           # name | kind | modified
//! anchor = "fill"         # fill | top-left | top | top-right | left | center
//!                         # | right | bottom-left | bottom | bottom-right
//! size = [800, 600]       # ignored with fill; points, or strings like "60%"
//! padding = 24            # points, inside the panel
//! icon_size = 64          # points
//! ```
//!
//! Every key is optional, and a value that cannot be read is a warning in the
//! log that leaves that one key at its default. Whether the desk runs at all
//! is not in here: that is a session setting, and the compositor starts and
//! stops `otto-files --desk` to follow it.

// Rust guideline compliant 2026-02-21

use std::path::{Path, PathBuf};

use serde::Deserialize;
use skia_safe::Rect;

use crate::model::{self, SortKey};
use crate::places_config;

/// The padding the desk leaves around its icons when the config says
/// nothing, in points. Enough to keep the first column clear of the edge of
/// the screen and of the dock's or the bar's reserved zone.
pub const DEFAULT_PADDING: f32 = 24.0;

/// The most padding the desk accepts, in points. Past this a small panel is
/// all padding and no icons.
const MAX_PADDING: f32 = 400.0;

/// The icon sizes the desk accepts, in points. Below the smaller one a
/// caption is wider than its icon by so much that the grid reads as a list
/// of names; above the larger one a single icon is most of a panel.
const ICON_SIZE_RANGE: (f32, f32) = (32.0, 256.0);

/// How the desk places its icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrange {
    /// A sorted grid, filling rows from the top-left corner.
    Grid,
    /// Each icon where it was dropped, on an invisible grid of slots.
    Free,
}

/// Where the panel sits inside the usable area of its output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// The whole usable area. `size` is ignored.
    Fill,
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    fn parse(value: &str) -> Option<Self> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "fill" => Self::Fill,
            "top-left" => Self::TopLeft,
            "top" => Self::Top,
            "top-right" => Self::TopRight,
            "left" => Self::Left,
            "center" | "centre" => Self::Center,
            "right" => Self::Right,
            "bottom-left" => Self::BottomLeft,
            "bottom" => Self::Bottom,
            "bottom-right" => Self::BottomRight,
            _ => return None,
        })
    }

    /// Where along each axis the panel sits: 0 at the leading edge, 0.5
    /// centred, 1 at the trailing edge.
    fn alignment(self) -> (f32, f32) {
        match self {
            Self::Fill | Self::Center => (0.5, 0.5),
            Self::TopLeft => (0.0, 0.0),
            Self::Top => (0.5, 0.0),
            Self::TopRight => (1.0, 0.0),
            Self::Left => (0.0, 0.5),
            Self::Right => (1.0, 0.5),
            Self::BottomLeft => (0.0, 1.0),
            Self::Bottom => (0.5, 1.0),
            Self::BottomRight => (1.0, 1.0),
        }
    }
}

/// One side of the panel's size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Extent {
    /// Logical points.
    Points(f32),
    /// A percentage of the usable area along the same axis.
    Percent(f32),
}

impl Extent {
    /// This extent against `available` points, never more than that.
    fn resolve(self, available: f32) -> f32 {
        let wanted = match self {
            Self::Points(points) => points,
            Self::Percent(percent) => available * percent / 100.0,
        };
        wanted.clamp(0.0, available.max(0.0))
    }

    fn parse(value: &toml::Value) -> Option<Self> {
        match value {
            toml::Value::Integer(points) if *points > 0 => Some(Self::Points(*points as f32)),
            toml::Value::Float(points) if *points > 0.0 => Some(Self::Points(*points as f32)),
            toml::Value::String(text) => {
                let text = text.trim();
                match text.strip_suffix('%') {
                    Some(percent) => percent
                        .trim()
                        .parse::<f32>()
                        .ok()
                        .filter(|p| *p > 0.0 && *p <= 100.0)
                        .map(Self::Percent),
                    None => text
                        .parse::<f32>()
                        .ok()
                        .filter(|p| *p > 0.0)
                        .map(Self::Points),
                }
            }
            _ => None,
        }
    }
}

/// The panel's size when `anchor` is not `fill` and the config gives none:
/// half the usable area each way, which is a panel rather than a second way
/// of spelling `fill`.
const DEFAULT_SIZE: [Extent; 2] = [Extent::Percent(50.0), Extent::Percent(50.0)];

/// The `[desk]` section, read and defaulted.
#[derive(Debug, Clone, PartialEq)]
pub struct DeskConfig {
    /// The folder the desk shows. It may not exist yet; the desk shows an
    /// empty panel until it does and never creates it.
    pub folder: PathBuf,
    pub arrange: Arrange,
    /// The grid's order. Size is not offered: it is not something anybody
    /// arranges a desktop by.
    pub sort: SortKey,
    pub anchor: Anchor,
    /// Width and height, ignored under [`Anchor::Fill`].
    pub size: [Extent; 2],
    /// Room between the panel's edges and the icons, in points.
    pub padding: f32,
    /// The grid icon's size, in points.
    pub icon_size: f32,
}

impl DeskConfig {
    /// The defaults, around `folder`.
    pub fn with_folder(folder: PathBuf) -> Self {
        Self {
            folder,
            arrange: Arrange::Grid,
            sort: SortKey::Name,
            anchor: Anchor::Fill,
            size: DEFAULT_SIZE,
            padding: DEFAULT_PADDING,
            icon_size: crate::view::DEFAULT_GRID_ICON,
        }
    }

    /// Read `~/.config/otto/files.toml`, or the defaults where it has no
    /// `[desk]` section, cannot be read, or cannot be parsed.
    pub fn load() -> Self {
        let home = places_config::home_dir();
        let default_folder = default_folder(home.as_deref());
        let Some(path) = places_config::config_path() else {
            return Self::with_folder(default_folder);
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Self::with_folder(default_folder);
            }
            Err(err) => {
                tracing::warn!(path = %path.display(), %err, "cannot read the desk config");
                return Self::with_folder(default_folder);
            }
        };
        Self::parse(&text, home.as_deref(), default_folder)
    }

    /// Read a whole `files.toml`, taking `default_folder` where it names no
    /// folder of its own.
    fn parse(text: &str, home: Option<&Path>, default_folder: PathBuf) -> Self {
        #[derive(Deserialize, Default)]
        struct File {
            #[serde(default)]
            desk: Raw,
        }
        // Loose on purpose: each value is checked on its own below, so one
        // bad value costs that key rather than the whole section.
        #[derive(Deserialize, Default)]
        struct Raw {
            folder: Option<String>,
            arrange: Option<String>,
            sort: Option<String>,
            anchor: Option<String>,
            size: Option<toml::Value>,
            padding: Option<toml::Value>,
            icon_size: Option<toml::Value>,
        }

        let raw = match toml::from_str::<File>(text) {
            Ok(file) => file.desk,
            Err(err) => {
                tracing::warn!(%err, "ignoring the desk config");
                return Self::with_folder(default_folder);
            }
        };

        let mut config = Self::with_folder(default_folder);
        if let Some(folder) = raw.folder.as_deref() {
            match places_config::expand_home(folder, home) {
                Some(path) => config.folder = path,
                None => warn_key("folder", folder),
            }
        }
        if let Some(arrange) = raw.arrange.as_deref() {
            match arrange.trim().to_ascii_lowercase().as_str() {
                "grid" => config.arrange = Arrange::Grid,
                "free" => config.arrange = Arrange::Free,
                _ => warn_key("arrange", arrange),
            }
        }
        if let Some(sort) = raw.sort.as_deref() {
            match sort.trim().to_ascii_lowercase().as_str() {
                "name" => config.sort = SortKey::Name,
                "kind" => config.sort = SortKey::Kind,
                "modified" => config.sort = SortKey::Modified,
                _ => warn_key("sort", sort),
            }
        }
        if let Some(anchor) = raw.anchor.as_deref() {
            match Anchor::parse(anchor) {
                Some(anchor) => config.anchor = anchor,
                None => warn_key("anchor", anchor),
            }
        }
        if let Some(size) = raw.size.as_ref() {
            let parsed = size.as_array().and_then(|pair| match pair.as_slice() {
                [width, height] => Some([Extent::parse(width)?, Extent::parse(height)?]),
                _ => None,
            });
            match parsed {
                Some(size) => config.size = size,
                None => warn_key("size", &size.to_string()),
            }
        }
        if let Some(padding) = raw.padding.as_ref() {
            match number(padding).filter(|p| *p >= 0.0) {
                Some(padding) => config.padding = padding.min(MAX_PADDING),
                None => warn_key("padding", &padding.to_string()),
            }
        }
        if let Some(icon_size) = raw.icon_size.as_ref() {
            match number(icon_size).filter(|s| *s > 0.0) {
                Some(size) => config.icon_size = size.clamp(ICON_SIZE_RANGE.0, ICON_SIZE_RANGE.1),
                None => warn_key("icon_size", &icon_size.to_string()),
            }
        }
        config
    }
}

/// `XDG_DESKTOP_DIR` from `user-dirs.dirs`, then `~/Desktop`.
fn default_folder(home: Option<&Path>) -> PathBuf {
    let Some(home) = home else {
        return PathBuf::from("/");
    };
    model::user_dir(home, "XDG_DESKTOP_DIR").unwrap_or_else(|| home.join("Desktop"))
}

fn number(value: &toml::Value) -> Option<f32> {
    match value {
        toml::Value::Integer(n) => Some(*n as f32),
        toml::Value::Float(n) => Some(*n as f32),
        _ => None,
    }
}

fn warn_key(key: &str, value: &str) {
    tracing::warn!(
        name: "desk.config.invalid",
        key,
        value,
        "ignoring [desk] {{key}} = {{value}}; the default stands"
    );
}

/// The panel's rect inside a surface of `surface` points.
///
/// The surface always covers its output's whole usable area — the
/// compositor already leaves out every reserved zone — so an anchor and a
/// size are a rect inside it rather than a placement of the surface itself.
/// That keeps percentages and the clamp to the usable area exact, because
/// the usable area is simply what the configure said.
pub fn panel_rect(anchor: Anchor, size: [Extent; 2], surface: (f32, f32)) -> Rect {
    let (width, height) = (surface.0.max(0.0), surface.1.max(0.0));
    if anchor == Anchor::Fill {
        return Rect::from_wh(width, height);
    }
    let panel_w = size[0].resolve(width);
    let panel_h = size[1].resolve(height);
    let (ax, ay) = anchor.alignment();
    Rect::from_xywh(
        ((width - panel_w) * ax).round(),
        ((height - panel_h) * ay).round(),
        panel_w,
        panel_h,
    )
}

/// Holds the session's desk lock for as long as it lives.
///
/// Dropping it — or the process ending, however it ends — releases the lock,
/// so a crashed desk never keeps the next one from starting.
#[derive(Debug)]
pub struct InstanceLock {
    _file: std::fs::File,
}

/// Claim the one desk this session may have.
///
/// `Ok(None)` means another desk already holds it. The lock is an advisory
/// `flock` on a file in `XDG_RUNTIME_DIR`, which is per session and per user
/// and is cleaned up at logout.
///
/// # Errors
///
/// Fails when the lock file cannot be opened or locked for a reason other
/// than another desk holding it.
pub fn claim_instance() -> std::io::Result<Option<InstanceLock>> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    claim_instance_at(&dir.join("otto-desk.lock"))
}

fn claim_instance_at(path: &Path) -> std::io::Result<Option<InstanceLock>> {
    use std::os::fd::AsRawFd;

    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    // SAFETY: `flock` takes a descriptor this function owns through `file`,
    // which stays open for the whole call, and no pointers.
    let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if locked == 0 {
        return Ok(Some(InstanceLock { _file: file }));
    }
    let err = std::io::Error::last_os_error();
    if err.kind() == std::io::ErrorKind::WouldBlock {
        Ok(None)
    } else {
        Err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> DeskConfig {
        DeskConfig::parse(
            text,
            Some(Path::new("/home/someone")),
            PathBuf::from("/home/someone/Desktop"),
        )
    }

    /// Saying nothing is the ordinary case and has to mean the defaults.
    #[test]
    fn no_desk_section_means_the_defaults() {
        let config = parse("[sidebar]\nhide = [\"music\"]\n");
        assert_eq!(
            config,
            DeskConfig::with_folder(PathBuf::from("/home/someone/Desktop"))
        );
        assert_eq!(config.arrange, Arrange::Grid);
        assert_eq!(config.sort, SortKey::Name);
        assert_eq!(config.anchor, Anchor::Fill);
        assert_eq!(config.padding, DEFAULT_PADDING);
        assert_eq!(config.icon_size, crate::view::DEFAULT_GRID_ICON);
    }

    /// The shape the spec documents, read back.
    #[test]
    fn a_full_section_is_read() {
        let config = parse(
            r#"
            [desk]
            folder = "~/Stuff"
            arrange = "free"
            sort = "modified"
            anchor = "bottom-right"
            size = [800, "60%"]
            padding = 12
            icon_size = 96.0
            "#,
        );
        assert_eq!(config.folder, PathBuf::from("/home/someone/Stuff"));
        assert_eq!(config.arrange, Arrange::Free);
        assert_eq!(config.sort, SortKey::Modified);
        assert_eq!(config.anchor, Anchor::BottomRight);
        assert_eq!(config.size, [Extent::Points(800.0), Extent::Percent(60.0)]);
        assert_eq!(config.padding, 12.0);
        assert_eq!(config.icon_size, 96.0);
    }

    /// One bad value costs that key and no more.
    #[test]
    fn a_bad_value_keeps_its_default_and_nothing_else() {
        let config = parse(
            r#"
            [desk]
            arrange = "diagonal"
            sort = "size"
            anchor = "somewhere"
            size = [800]
            padding = -3
            icon_size = "huge"
            folder = "/srv/desk"
            "#,
        );
        assert_eq!(config, DeskConfig::with_folder(PathBuf::from("/srv/desk")));
    }

    /// A file that does not parse at all is the defaults, not an error.
    #[test]
    fn a_broken_file_is_the_defaults() {
        let config = parse("[desk\nfolder = ");
        assert_eq!(
            config,
            DeskConfig::with_folder(PathBuf::from("/home/someone/Desktop"))
        );
    }

    /// Keys that belong to somebody else — the session's own switch — are
    /// not errors here.
    #[test]
    fn unknown_keys_are_ignored() {
        let config = parse("[desk]\nenabled = true\nsort = \"kind\"\n");
        assert_eq!(config.sort, SortKey::Kind);
    }

    #[test]
    fn sizes_are_clamped_to_something_usable() {
        let config = parse("[desk]\nicon_size = 8\npadding = 10000\n");
        assert_eq!(config.icon_size, ICON_SIZE_RANGE.0);
        assert_eq!(config.padding, MAX_PADDING);
        let config = parse("[desk]\nicon_size = 1000\n");
        assert_eq!(config.icon_size, ICON_SIZE_RANGE.1);
    }

    #[test]
    fn every_anchor_is_spelled_the_way_the_spec_spells_it() {
        for (text, anchor) in [
            ("fill", Anchor::Fill),
            ("top-left", Anchor::TopLeft),
            ("top", Anchor::Top),
            ("top-right", Anchor::TopRight),
            ("left", Anchor::Left),
            ("center", Anchor::Center),
            ("right", Anchor::Right),
            ("bottom-left", Anchor::BottomLeft),
            ("bottom", Anchor::Bottom),
            ("bottom-right", Anchor::BottomRight),
        ] {
            assert_eq!(Anchor::parse(text), Some(anchor), "{text}");
        }
    }

    #[test]
    fn fill_is_the_whole_surface_whatever_the_size() {
        let rect = panel_rect(Anchor::Fill, [Extent::Points(10.0); 2], (1440.0, 900.0));
        assert_eq!(rect, Rect::from_wh(1440.0, 900.0));
    }

    #[test]
    fn an_anchored_panel_sits_at_its_edge() {
        let size = [Extent::Points(400.0), Extent::Percent(50.0)];
        let surface = (1000.0, 800.0);
        assert_eq!(
            panel_rect(Anchor::TopLeft, size, surface),
            Rect::from_xywh(0.0, 0.0, 400.0, 400.0)
        );
        assert_eq!(
            panel_rect(Anchor::BottomRight, size, surface),
            Rect::from_xywh(600.0, 400.0, 400.0, 400.0)
        );
        assert_eq!(
            panel_rect(Anchor::Center, size, surface),
            Rect::from_xywh(300.0, 200.0, 400.0, 400.0)
        );
        assert_eq!(
            panel_rect(Anchor::Right, size, surface),
            Rect::from_xywh(600.0, 200.0, 400.0, 400.0)
        );
    }

    /// A panel never reaches past the usable area.
    #[test]
    fn a_panel_bigger_than_the_area_is_clamped_to_it() {
        let rect = panel_rect(
            Anchor::Bottom,
            [Extent::Points(5000.0), Extent::Points(300.0)],
            (1280.0, 720.0),
        );
        assert_eq!(rect, Rect::from_xywh(0.0, 420.0, 1280.0, 300.0));
    }

    #[test]
    fn without_a_home_the_default_folder_is_the_root() {
        assert_eq!(default_folder(None), PathBuf::from("/"));
    }

    /// The second claim in a session is refused, and the first one's going
    /// frees the lock for the next.
    #[test]
    fn only_one_desk_holds_the_lock() {
        let path = std::env::temp_dir().join(format!("otto-desk-lock-{}", std::process::id()));
        let first = claim_instance_at(&path).unwrap();
        assert!(first.is_some());
        assert!(claim_instance_at(&path).unwrap().is_none());
        drop(first);
        assert!(claim_instance_at(&path).unwrap().is_some());
        let _ = std::fs::remove_file(&path);
    }
}
