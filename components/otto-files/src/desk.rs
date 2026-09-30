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
//! position = ["10%", 40]  # optional top-left corner; overrides the anchor
//! padding = 24            # points, inside the panel
//! icon_size = 64          # points
//! overflow = "scroll"     # scroll | stack
//! ```
//!
//! Every key is optional, and a value that cannot be read is a warning in the
//! log that leaves that one key at its default. Whether the desk runs at all
//! is not in here: that is a session setting, and the compositor starts and
//! stops `otto-files --desk` to follow it.
//!
//! The size and position can also be set with the pointer, in edit mode (see
//! [`Grip`] and [`stored_geometry`]), which writes them back into this file
//! with [`with_geometry`].
//!
//! What happens to icons the panel has no room for is [`Overflow`]: the grid
//! scrolls, or the last cell becomes a [`Pile`] that opens into a [`Fan`].

// Rust guideline compliant 2026-02-21

use std::path::{Path, PathBuf};

use serde::Deserialize;
use skia_safe::{Contains, Rect};

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

/// What the desk does with icons its panel has no cell for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    /// The grid runs on past the panel's bottom edge and scrolls.
    Scroll,
    /// The grid stops at the panel's edge, and its last cell piles up
    /// everything that did not fit. See [`pile`].
    Stack,
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
    /// The name the config file spells this anchor with.
    fn name(self) -> &'static str {
        match self {
            Self::Fill => "fill",
            Self::TopLeft => "top-left",
            Self::Top => "top",
            Self::TopRight => "top-right",
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
            Self::BottomLeft => "bottom-left",
            Self::Bottom => "bottom",
            Self::BottomRight => "bottom-right",
        }
    }

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

    /// An offset: like a size, except that zero is a place to be.
    fn parse_offset(value: &toml::Value) -> Option<Self> {
        let zero = match value {
            toml::Value::Integer(0) => Some(Self::Points(0.0)),
            toml::Value::Float(points) if *points == 0.0 => Some(Self::Points(0.0)),
            toml::Value::String(text) => {
                let text = text.trim();
                let number = text.trim_end_matches('%').trim().parse::<f32>().ok();
                (number == Some(0.0)).then(|| {
                    if text.ends_with('%') {
                        Self::Percent(0.0)
                    } else {
                        Self::Points(0.0)
                    }
                })
            }
            _ => None,
        };
        zero.or_else(|| Self::parse(value))
    }

    /// How the config file spells this extent: a number of points, or a
    /// string ending in `%`.
    fn to_toml(self) -> toml_edit::Value {
        match self {
            Self::Points(points) => toml_edit::Value::from(i64::from(points.round() as i32)),
            Self::Percent(percent) => toml_edit::Value::from(format!("{}%", round_tenth(percent))),
        }
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
    /// The panel's top-left corner inside the usable area. When set, it
    /// places the panel instead of the anchor's edge or corner; ignored under
    /// [`Anchor::Fill`]. Edit mode writes it.
    pub position: Option<[Extent; 2]>,
    /// Room between the panel's edges and the icons, in points.
    pub padding: f32,
    /// The grid icon's size, in points.
    pub icon_size: f32,
    /// What happens to icons past the last cell that fits.
    pub overflow: Overflow,
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
            position: None,
            padding: DEFAULT_PADDING,
            icon_size: crate::view::DEFAULT_GRID_ICON,
            overflow: Overflow::Scroll,
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
            position: Option<toml::Value>,
            padding: Option<toml::Value>,
            icon_size: Option<toml::Value>,
            overflow: Option<String>,
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
            match pair(size, Extent::parse) {
                Some(size) => config.size = size,
                None => warn_key("size", &size.to_string()),
            }
        }
        if let Some(position) = raw.position.as_ref() {
            match pair(position, Extent::parse_offset) {
                Some(position) => config.position = Some(position),
                None => warn_key("position", &position.to_string()),
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
        if let Some(overflow) = raw.overflow.as_deref() {
            match overflow.trim().to_ascii_lowercase().as_str() {
                "scroll" => config.overflow = Overflow::Scroll,
                "stack" => config.overflow = Overflow::Stack,
                _ => warn_key("overflow", overflow),
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

/// A two-element array read with `parse`, as `size` and `position` are.
fn pair(value: &toml::Value, parse: fn(&toml::Value) -> Option<Extent>) -> Option<[Extent; 2]> {
    match value.as_array()?.as_slice() {
        [first, second] => Some([parse(first)?, parse(second)?]),
        _ => None,
    }
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
///
/// A `position` puts the panel's top-left corner there instead of at the
/// anchor's edge or corner, moved back inside the area if the panel would
/// otherwise hang off it.
pub fn panel_rect(
    anchor: Anchor,
    size: [Extent; 2],
    position: Option<[Extent; 2]>,
    surface: (f32, f32),
) -> Rect {
    let (width, height) = (surface.0.max(0.0), surface.1.max(0.0));
    if anchor == Anchor::Fill {
        return Rect::from_wh(width, height);
    }
    let panel_w = size[0].resolve(width);
    let panel_h = size[1].resolve(height);
    let (x, y) = match position {
        Some([x, y]) => (
            x.resolve(width).min(width - panel_w),
            y.resolve(height).min(height - panel_h),
        ),
        None => {
            let (ax, ay) = anchor.alignment();
            ((width - panel_w) * ax, (height - panel_h) * ay)
        }
    };
    Rect::from_xywh(x.round(), y.round(), panel_w, panel_h)
}

// ---------------------------------------------------------------------------
// Stacking: the pile in the last cell, and the fan it opens into
// ---------------------------------------------------------------------------

/// The run of items the last cell holds when [`Overflow::Stack`] has more
/// items than cells: entries `first` to `first + count - 1`, in the grid's
/// order. The cell itself is cell `first`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pile {
    /// The pile's first item, which is on top, and the cell the pile sits in.
    pub first: usize,
    /// How many items it holds; never fewer than two.
    pub count: usize,
}

impl Pile {
    /// Whether entry `index` is in the pile.
    pub fn contains(&self, index: usize) -> bool {
        index >= self.first && index < self.first + self.count
    }

    /// The pile's entries, as a range of indices.
    pub fn range(&self) -> std::ops::Range<usize> {
        self.first..self.first + self.count
    }
}

/// The pile `count` items make in a grid of `capacity` cells, if they make
/// one: the grid fills in order, and once there are more items than cells,
/// the last cell takes its own item and every one after it.
///
/// No cells at all is no pile, since there is nowhere to put one.
pub fn pile(count: usize, capacity: usize) -> Option<Pile> {
    (capacity > 0 && count > capacity).then(|| Pile {
        first: capacity - 1,
        count: count - capacity + 1,
    })
}

/// Room between the fan's edge and its cells, in points.
pub const FAN_PAD: f32 = 12.0;

/// Room between the fan and the pile it opens from, in points.
pub const FAN_GAP: f32 = 8.0;

/// The most columns the fan lays out, so a big pile opens as a block rather
/// than a strip across the whole screen.
pub const FAN_MAX_COLUMNS: usize = 6;

/// The fan: the pile's items laid out as a small grid near it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fan {
    /// Where the fan sits, in the same space as the pile's cell.
    pub rect: Rect,
    /// How many cells across.
    pub columns: usize,
    /// How tall all its rows are, padding included; more than the rect's
    /// height when it scrolls.
    pub content_h: f32,
    /// A cell's width and height, in points.
    pub cell: (f32, f32),
}

impl Fan {
    /// The fan for `items` cells of `cell` points, opening from the pile at
    /// `pile` and kept inside `bounds`.
    ///
    /// It prefers the space above the pile, since the pile is the grid's last
    /// cell, then below it, and past that takes the tallest it can inside
    /// `bounds` and scrolls. Across, it is centred on the pile as far as
    /// `bounds` allows.
    pub fn new(bounds: Rect, pile: Rect, items: usize, cell: (f32, f32)) -> Self {
        let room = ((bounds.width() - FAN_PAD * 2.0) / cell.0).floor().max(1.0) as usize;
        let columns = items.clamp(1, FAN_MAX_COLUMNS).min(room);
        let rows = items.div_ceil(columns).max(1);
        let width = (columns as f32 * cell.0 + FAN_PAD * 2.0).min(bounds.width());
        let content_h = rows as f32 * cell.1 + FAN_PAD * 2.0;
        let height = content_h.min(bounds.height());
        let left = (pile.center_x() - width / 2.0)
            .min(bounds.right - width)
            .max(bounds.left);
        let top = if pile.top - FAN_GAP - height >= bounds.top {
            pile.top - FAN_GAP - height
        } else if pile.bottom + FAN_GAP + height <= bounds.bottom {
            pile.bottom + FAN_GAP
        } else {
            (bounds.bottom - height).max(bounds.top)
        };
        Self {
            rect: Rect::from_xywh(left.round(), top.round(), width, height),
            columns,
            content_h,
            cell,
        }
    }

    /// How far the fan can scroll: zero when every row fits.
    pub fn max_scroll(&self) -> f32 {
        (self.content_h - self.rect.height()).max(0.0)
    }

    /// The cell for the pile's `k`th item, scrolled by `scroll`.
    pub fn cell_rect(&self, k: usize, scroll: f32) -> Rect {
        let (column, row) = (k % self.columns, k / self.columns);
        Rect::from_xywh(
            self.rect.left + FAN_PAD + column as f32 * self.cell.0,
            self.rect.top + FAN_PAD + row as f32 * self.cell.1 - scroll,
            self.cell.0,
            self.cell.1,
        )
    }

    /// Which of `items` is under (`x`, `y`), scrolled by `scroll`. `None`
    /// outside the fan, and on its padding or an empty cell.
    pub fn index_at(&self, x: f32, y: f32, items: usize, scroll: f32) -> Option<usize> {
        if !self.rect.contains(skia_safe::Point::new(x, y)) {
            return None;
        }
        let dx = x - self.rect.left - FAN_PAD;
        let dy = y - self.rect.top - FAN_PAD + scroll;
        if dx < 0.0 || dy < 0.0 {
            return None;
        }
        let column = (dx / self.cell.0) as usize;
        if column >= self.columns {
            return None;
        }
        let k = (dy / self.cell.1) as usize * self.columns + column;
        (k < items).then_some(k)
    }

    /// The items whose cells show in the fan, scrolled by `scroll`.
    pub fn visible(&self, items: usize, scroll: f32) -> std::ops::Range<usize> {
        let first_row = ((scroll - FAN_PAD) / self.cell.1).floor().max(0.0) as usize;
        let last_row = ((scroll + self.rect.height() - FAN_PAD) / self.cell.1).ceil() as usize;
        (first_row * self.columns).min(items)..(last_row * self.columns).min(items)
    }

    /// The scroll that shows item `k` whole, starting from `scroll`: as
    /// little movement as that takes.
    pub fn reveal(&self, k: usize, scroll: f32) -> f32 {
        let top = FAN_PAD + (k / self.columns) as f32 * self.cell.1;
        let bottom = top + self.cell.1;
        let view = self.rect.height();
        let scroll = if top - FAN_PAD < scroll {
            top - FAN_PAD
        } else if bottom + FAN_PAD > scroll + view {
            bottom + FAN_PAD - view
        } else {
            scroll
        };
        scroll.clamp(0.0, self.max_scroll())
    }
}

// ---------------------------------------------------------------------------
// Edit mode: moving and resizing the panel with the pointer
// ---------------------------------------------------------------------------

/// The smallest panel edit mode leaves, in points: room for an icon with its
/// caption, and for edit mode's Cancel and Done buttons side by side.
pub const MIN_PANEL: f32 = 180.0;

/// How far either side of an edge a press still takes that edge, in points.
/// The outline is a hairline; a target that thin could not be hit.
pub const GRIP_REACH: f32 = 8.0;

/// What a press in edit mode took hold of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grip {
    /// The whole panel, to move it.
    Move,
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Grip {
    /// Which horizontal and vertical edges the grip drags: -1 the leading
    /// one, 1 the trailing one, 0 neither.
    fn edges(self) -> (i8, i8) {
        match self {
            Self::Move => (0, 0),
            Self::Left => (-1, 0),
            Self::Right => (1, 0),
            Self::Top => (0, -1),
            Self::Bottom => (0, 1),
            Self::TopLeft => (-1, -1),
            Self::TopRight => (1, -1),
            Self::BottomLeft => (-1, 1),
            Self::BottomRight => (1, 1),
        }
    }

    /// The handles drawn on the outline, as their centres on `panel`: the
    /// four corners and the middle of each side.
    pub fn handles(panel: Rect) -> [(f32, f32); 8] {
        let (cx, cy) = (panel.center_x(), panel.center_y());
        [
            (panel.left, panel.top),
            (cx, panel.top),
            (panel.right, panel.top),
            (panel.right, cy),
            (panel.right, panel.bottom),
            (cx, panel.bottom),
            (panel.left, panel.bottom),
            (panel.left, cy),
        ]
    }
}

/// What a press at (`x`, `y`) takes hold of: an edge or a corner within
/// [`GRIP_REACH`] of the outline, the panel itself inside it, or nothing.
pub fn grip_at(panel: Rect, x: f32, y: f32) -> Option<Grip> {
    let within_x = x >= panel.left - GRIP_REACH && x <= panel.right + GRIP_REACH;
    let within_y = y >= panel.top - GRIP_REACH && y <= panel.bottom + GRIP_REACH;
    if !within_x || !within_y {
        return None;
    }
    let side = |at: f32, lead: f32, trail: f32| -> i8 {
        if (at - lead).abs() <= GRIP_REACH {
            -1
        } else if (at - trail).abs() <= GRIP_REACH {
            1
        } else {
            0
        }
    };
    Some(
        match (
            side(x, panel.left, panel.right),
            side(y, panel.top, panel.bottom),
        ) {
            (-1, -1) => Grip::TopLeft,
            (1, -1) => Grip::TopRight,
            (-1, 1) => Grip::BottomLeft,
            (1, 1) => Grip::BottomRight,
            (-1, _) => Grip::Left,
            (1, _) => Grip::Right,
            (_, -1) => Grip::Top,
            (_, 1) => Grip::Bottom,
            _ if panel.contains(skia_safe::Point::new(x, y)) => Grip::Move,
            _ => return None,
        },
    )
}

/// `start` after dragging `grip` by (`dx`, `dy`) points, kept inside an area
/// of `bounds` points and no smaller than [`MIN_PANEL`] (or the area, where
/// the area is smaller than that).
///
/// A move keeps the size and stops at the area's edges. A resize moves only
/// the edges it holds; the opposite edges stay where they were.
pub fn dragged(start: Rect, grip: Grip, dx: f32, dy: f32, bounds: (f32, f32)) -> Rect {
    let (width, height) = (bounds.0.max(0.0), bounds.1.max(0.0));
    if grip == Grip::Move {
        let w = start.width().min(width);
        let h = start.height().min(height);
        let x = (start.left + dx).clamp(0.0, width - w);
        let y = (start.top + dy).clamp(0.0, height - h);
        return Rect::from_xywh(x, y, w, h);
    }
    let (horizontal, vertical) = grip.edges();
    let (left, right) = drag_axis(start.left, start.right, horizontal, dx, width);
    let (top, bottom) = drag_axis(start.top, start.bottom, vertical, dy, height);
    Rect::from_ltrb(left, top, right, bottom)
}

/// One axis of [`dragged`]: move the `edge` end (-1 leading, 1 trailing, 0
/// neither) of `lead..trail` by `delta`, inside `0..extent`.
fn drag_axis(lead: f32, trail: f32, edge: i8, delta: f32, extent: f32) -> (f32, f32) {
    let min = MIN_PANEL.min(extent);
    let lead = lead.clamp(0.0, extent);
    let trail = trail.clamp(0.0, extent);
    match edge {
        -1 => ((lead + delta).clamp(0.0, (trail - min).max(0.0)), trail),
        1 => (
            lead,
            (trail + delta).clamp((lead + min).min(extent), extent),
        ),
        _ => (lead, trail),
    }
}

/// How a panel edited to `panel` is written to the config, in an area of
/// `bounds` points: `(anchor, size, position)`.
///
/// Percentages of the usable area, so the panel keeps its place when the
/// output changes size or a bar comes and goes. A panel that covers the whole
/// area is `fill`, which is what it is.
pub fn stored_geometry(
    panel: Rect,
    bounds: (f32, f32),
) -> (Anchor, [Extent; 2], Option<[Extent; 2]>) {
    let (width, height) = bounds;
    // Half a point either way: a drag to the edge lands on a whole point
    // near it, and a fill panel must not become a 99.9% one.
    let covers = panel.left <= 0.5
        && panel.top <= 0.5
        && panel.right >= width - 0.5
        && panel.bottom >= height - 0.5;
    if covers || width <= 0.0 || height <= 0.0 {
        return (Anchor::Fill, DEFAULT_SIZE, None);
    }
    let percent = |part: f32, whole: f32| Extent::Percent(round_tenth(part / whole * 100.0));
    (
        Anchor::TopLeft,
        [
            percent(panel.width(), width),
            percent(panel.height(), height),
        ],
        Some([percent(panel.left, width), percent(panel.top, height)]),
    )
}

/// A percentage to one decimal: finer than a point on any screen, and short
/// enough to read in the file.
fn round_tenth(value: f32) -> f32 {
    (value * 10.0).round() / 10.0
}

/// `text`, a whole `files.toml`, with the `[desk]` panel geometry replaced.
///
/// Everything else in the file, comments included, stays as it was. Under
/// `fill` the size and position are taken out rather than left to mislead.
///
/// # Errors
///
/// Fails when `text` is not TOML, so a file that already does not parse is
/// never overwritten with one that does but has lost everything else.
pub fn with_geometry(
    text: &str,
    anchor: Anchor,
    size: [Extent; 2],
    position: Option<[Extent; 2]>,
) -> Result<String, toml_edit::TomlError> {
    let mut doc: toml_edit::DocumentMut = text.parse()?;
    if !doc.contains_table("desk") {
        doc["desk"] = toml_edit::table();
    }
    let as_array = |pair: [Extent; 2]| {
        let mut array = toml_edit::Array::new();
        array.push(pair[0].to_toml());
        array.push(pair[1].to_toml());
        toml_edit::value(array)
    };
    let desk = &mut doc["desk"];
    desk["anchor"] = toml_edit::value(anchor.name());
    let placed = (anchor != Anchor::Fill).then_some(size);
    let position = position.filter(|_| placed.is_some());
    match placed {
        Some(size) => desk["size"] = as_array(size),
        None => remove_key(desk, "size"),
    }
    match position {
        Some(position) => desk["position"] = as_array(position),
        None => remove_key(desk, "position"),
    }
    Ok(doc.to_string())
}

fn remove_key(table: &mut toml_edit::Item, key: &str) {
    if let Some(table) = table.as_table_like_mut() {
        table.remove(key);
    }
}

/// Write the panel geometry into `~/.config/otto/files.toml`, keeping the
/// rest of the file. The desk notices its own write and re-reads it, which
/// changes nothing.
///
/// # Errors
///
/// Fails when there is no home to find the file in, the file cannot be read
/// or written, or it does not parse.
pub fn save_geometry(
    anchor: Anchor,
    size: [Extent; 2],
    position: Option<[Extent; 2]>,
) -> std::io::Result<()> {
    let path = places_config::config_path()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no home folder"))?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err),
    };
    let next = with_geometry(&text, anchor, size, position)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Written beside and renamed over, so the desk's own watch never reads a
    // half-written file.
    let temp = path.with_extension(format!("toml.tmp.{}", std::process::id()));
    std::fs::write(&temp, next)?;
    std::fs::rename(&temp, &path)
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
        let rect = panel_rect(
            Anchor::Fill,
            [Extent::Points(10.0); 2],
            None,
            (1440.0, 900.0),
        );
        assert_eq!(rect, Rect::from_wh(1440.0, 900.0));
    }

    #[test]
    fn an_anchored_panel_sits_at_its_edge() {
        let size = [Extent::Points(400.0), Extent::Percent(50.0)];
        let surface = (1000.0, 800.0);
        assert_eq!(
            panel_rect(Anchor::TopLeft, size, None, surface),
            Rect::from_xywh(0.0, 0.0, 400.0, 400.0)
        );
        assert_eq!(
            panel_rect(Anchor::BottomRight, size, None, surface),
            Rect::from_xywh(600.0, 400.0, 400.0, 400.0)
        );
        assert_eq!(
            panel_rect(Anchor::Center, size, None, surface),
            Rect::from_xywh(300.0, 200.0, 400.0, 400.0)
        );
        assert_eq!(
            panel_rect(Anchor::Right, size, None, surface),
            Rect::from_xywh(600.0, 200.0, 400.0, 400.0)
        );
    }

    /// A panel never reaches past the usable area.
    #[test]
    fn a_panel_bigger_than_the_area_is_clamped_to_it() {
        let rect = panel_rect(
            Anchor::Bottom,
            [Extent::Points(5000.0), Extent::Points(300.0)],
            None,
            (1280.0, 720.0),
        );
        assert_eq!(rect, Rect::from_xywh(0.0, 420.0, 1280.0, 300.0));
    }

    #[test]
    fn a_position_places_the_corner_and_stays_inside() {
        let size = [Extent::Percent(50.0), Extent::Percent(50.0)];
        let at = Some([Extent::Percent(10.0), Extent::Points(20.0)]);
        assert_eq!(
            panel_rect(Anchor::TopLeft, size, at, (1000.0, 800.0)),
            Rect::from_xywh(100.0, 20.0, 500.0, 400.0)
        );
        // A corner too far right is pulled back so the panel fits.
        let far = Some([Extent::Percent(90.0), Extent::Percent(90.0)]);
        assert_eq!(
            panel_rect(Anchor::TopLeft, size, far, (1000.0, 800.0)),
            Rect::from_xywh(500.0, 400.0, 500.0, 400.0)
        );
        // Fill ignores it.
        assert_eq!(
            panel_rect(Anchor::Fill, size, far, (1000.0, 800.0)),
            Rect::from_wh(1000.0, 800.0)
        );
    }

    #[test]
    fn a_position_is_read_and_zero_is_allowed() {
        let config = parse("[desk]\nposition = [0, \"12.5%\"]\n");
        assert_eq!(
            config.position,
            Some([Extent::Points(0.0), Extent::Percent(12.5)])
        );
        let config = parse("[desk]\nposition = [-4, 3]\n");
        assert_eq!(config.position, None);
    }

    const AREA: (f32, f32) = (1000.0, 800.0);

    fn panel() -> Rect {
        Rect::from_xywh(200.0, 100.0, 400.0, 300.0)
    }

    #[test]
    fn a_press_finds_the_corner_the_edge_or_the_inside() {
        let p = panel();
        assert_eq!(grip_at(p, 200.0, 100.0), Some(Grip::TopLeft));
        assert_eq!(grip_at(p, 605.0, 398.0), Some(Grip::BottomRight));
        assert_eq!(grip_at(p, 400.0, 96.0), Some(Grip::Top));
        assert_eq!(grip_at(p, 197.0, 250.0), Some(Grip::Left));
        assert_eq!(grip_at(p, 400.0, 250.0), Some(Grip::Move));
        assert_eq!(grip_at(p, 100.0, 250.0), None);
        assert_eq!(grip_at(p, 400.0, 500.0), None);
    }

    #[test]
    fn a_move_keeps_the_size_and_stops_at_the_edges() {
        let p = panel();
        assert_eq!(
            dragged(p, Grip::Move, 50.0, -20.0, AREA),
            Rect::from_xywh(250.0, 80.0, 400.0, 300.0)
        );
        assert_eq!(
            dragged(p, Grip::Move, 5000.0, 5000.0, AREA),
            Rect::from_xywh(600.0, 500.0, 400.0, 300.0)
        );
        assert_eq!(
            dragged(p, Grip::Move, -5000.0, -5000.0, AREA),
            Rect::from_xywh(0.0, 0.0, 400.0, 300.0)
        );
    }

    #[test]
    fn a_resize_moves_only_the_edges_it_holds() {
        let p = panel();
        assert_eq!(
            dragged(p, Grip::BottomRight, 100.0, 50.0, AREA),
            Rect::from_ltrb(200.0, 100.0, 700.0, 450.0)
        );
        assert_eq!(
            dragged(p, Grip::Left, -50.0, 999.0, AREA),
            Rect::from_ltrb(150.0, 100.0, 600.0, 400.0)
        );
        assert_eq!(
            dragged(p, Grip::Top, 0.0, -500.0, AREA),
            Rect::from_ltrb(200.0, 0.0, 600.0, 400.0)
        );
    }

    #[test]
    fn a_resize_stops_at_the_area_and_at_the_smallest_panel() {
        let p = panel();
        assert_eq!(dragged(p, Grip::Right, 5000.0, 0.0, AREA).right, AREA.0);
        let shrunk = dragged(p, Grip::TopLeft, 5000.0, 5000.0, AREA);
        assert_eq!(shrunk.width(), MIN_PANEL);
        assert_eq!(shrunk.height(), MIN_PANEL);
        assert_eq!((shrunk.right, shrunk.bottom), (p.right, p.bottom));
    }

    #[test]
    fn an_edited_panel_is_stored_as_percentages_and_reads_back() {
        let edited = Rect::from_xywh(250.0, 80.0, 400.0, 300.0);
        let (anchor, size, position) = stored_geometry(edited, AREA);
        assert_eq!(anchor, Anchor::TopLeft);
        assert_eq!(size, [Extent::Percent(40.0), Extent::Percent(37.5)]);
        assert_eq!(
            position,
            Some([Extent::Percent(25.0), Extent::Percent(10.0)])
        );
        assert_eq!(panel_rect(anchor, size, position, AREA), edited);
    }

    #[test]
    fn a_panel_over_the_whole_area_is_stored_as_fill() {
        let (anchor, _, position) = stored_geometry(Rect::from_wh(AREA.0, AREA.1), AREA);
        assert_eq!(anchor, Anchor::Fill);
        assert_eq!(position, None);
    }

    #[test]
    fn the_geometry_is_written_into_the_file_and_the_rest_is_kept() {
        let text = "# mine\n[sidebar]\nhide = [\"music\"]\n\n[desk]\nsort = \"kind\" # by kind\n";
        let (anchor, size, position) =
            stored_geometry(Rect::from_xywh(250.0, 80.0, 400.0, 300.0), AREA);
        let written = with_geometry(text, anchor, size, position).unwrap();
        assert!(written.contains("# mine"));
        assert!(written.contains("sort = \"kind\" # by kind"));
        let config = parse(&written);
        assert_eq!(config.anchor, Anchor::TopLeft);
        assert_eq!(config.size, size);
        assert_eq!(config.position, position);
        assert_eq!(config.sort, SortKey::Kind);

        // Back to fill: the size and position go with it.
        let filled = with_geometry(&written, Anchor::Fill, DEFAULT_SIZE, None).unwrap();
        assert!(!filled.contains("position"));
        assert!(!filled.contains("size"));
        assert_eq!(parse(&filled).anchor, Anchor::Fill);
    }

    #[test]
    fn a_file_that_does_not_parse_is_not_overwritten() {
        assert!(with_geometry("[desk\n", Anchor::Fill, DEFAULT_SIZE, None).is_err());
    }

    #[test]
    fn overflow_is_read_and_defaults_to_scroll() {
        assert_eq!(parse("").overflow, Overflow::Scroll);
        assert_eq!(
            parse("[desk]\noverflow = \"stack\"\n").overflow,
            Overflow::Stack
        );
        assert_eq!(
            parse("[desk]\noverflow = \"Scroll\"\n").overflow,
            Overflow::Scroll
        );
        // A value it does not know costs that key and nothing else.
        let config = parse("[desk]\noverflow = \"hide\"\nsort = \"kind\"\n");
        assert_eq!(config.overflow, Overflow::Scroll);
        assert_eq!(config.sort, SortKey::Kind);
    }

    /// Everything fits: no pile, whatever the capacity.
    #[test]
    fn items_that_fit_make_no_pile() {
        assert_eq!(pile(0, 12), None);
        assert_eq!(pile(11, 12), None);
        assert_eq!(pile(12, 12), None);
    }

    /// One too many: the last cell holds its own item and the one after it.
    #[test]
    fn the_last_cell_takes_its_item_and_every_one_after() {
        let pile = pile(13, 12).unwrap();
        assert_eq!(
            pile,
            Pile {
                first: 11,
                count: 2
            }
        );
        assert!(!pile.contains(10));
        assert!(pile.contains(11));
        assert!(pile.contains(12));
        assert!(!pile.contains(13));
        assert_eq!(pile.range(), 11..13);

        let big = super::pile(500, 12).unwrap();
        assert_eq!(big.first, 11);
        assert_eq!(big.first + big.count, 500);
    }

    #[test]
    fn a_single_cell_is_all_pile_and_no_cells_is_none() {
        assert_eq!(pile(5, 1), Some(Pile { first: 0, count: 5 }));
        assert_eq!(pile(5, 0), None);
    }

    const CELL: (f32, f32) = (100.0, 120.0);

    /// The pile sits at the bottom-right of the panel, so the fan opens
    /// above it, inside the panel.
    #[test]
    fn the_fan_opens_above_the_pile_and_inside_the_bounds() {
        let bounds = Rect::from_wh(1000.0, 800.0);
        let pile_cell = Rect::from_xywh(900.0, 680.0, CELL.0, CELL.1);
        let fan = Fan::new(bounds, pile_cell, 8, CELL);
        assert_eq!(fan.columns, FAN_MAX_COLUMNS);
        assert!(fan.rect.bottom <= pile_cell.top - FAN_GAP + 0.5);
        assert!(fan.rect.right <= bounds.right);
        assert!(fan.rect.left >= bounds.left);
        assert_eq!(fan.max_scroll(), 0.0);
    }

    #[test]
    fn a_small_pile_opens_one_row_as_wide_as_it_is() {
        let bounds = Rect::from_wh(1000.0, 800.0);
        let pile_cell = Rect::from_xywh(450.0, 680.0, CELL.0, CELL.1);
        let fan = Fan::new(bounds, pile_cell, 3, CELL);
        assert_eq!(fan.columns, 3);
        assert_eq!(fan.rect.width(), 3.0 * CELL.0 + FAN_PAD * 2.0);
        assert_eq!(fan.rect.height(), CELL.1 + FAN_PAD * 2.0);
        // Centred on the pile.
        assert_eq!(fan.rect.center_x(), pile_cell.center_x());
    }

    /// Items map to cells and back, gaps and padding are nothing.
    #[test]
    fn a_fan_cell_is_found_where_it_is_drawn() {
        let bounds = Rect::from_wh(1000.0, 800.0);
        let pile_cell = Rect::from_xywh(900.0, 680.0, CELL.0, CELL.1);
        let fan = Fan::new(bounds, pile_cell, 8, CELL);
        for k in 0..8 {
            let cell = fan.cell_rect(k, 0.0);
            assert_eq!(
                fan.index_at(cell.center_x(), cell.center_y(), 8, 0.0),
                Some(k)
            );
        }
        // The empty cells after the last item, and the padding.
        let empty = fan.cell_rect(9, 0.0);
        assert_eq!(
            fan.index_at(empty.center_x(), empty.center_y(), 8, 0.0),
            None
        );
        assert_eq!(
            fan.index_at(fan.rect.left + 2.0, fan.rect.top + 2.0, 8, 0.0),
            None
        );
        assert_eq!(fan.index_at(0.0, 0.0, 8, 0.0), None);
    }

    /// More rows than fit scroll, and revealing an item brings its row in.
    #[test]
    fn a_tall_fan_scrolls_to_what_it_reveals() {
        let bounds = Rect::from_wh(700.0, 400.0);
        let pile_cell = Rect::from_xywh(600.0, 280.0, CELL.0, CELL.1);
        let fan = Fan::new(bounds, pile_cell, 60, CELL);
        assert!(fan.rect.height() <= bounds.height());
        assert!(fan.max_scroll() > 0.0);
        assert_eq!(fan.visible(60, 0.0).start, 0);
        let last = fan.reveal(59, 0.0);
        assert_eq!(last, fan.max_scroll());
        assert!(fan.visible(60, last).contains(&59));
        let cell = fan.cell_rect(59, last);
        assert!(cell.bottom <= fan.rect.bottom && cell.top >= fan.rect.top);
        assert_eq!(fan.reveal(0, last), 0.0);
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
