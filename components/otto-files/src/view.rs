//! Window layout: full-height sidebar, a large header over the content, and
//! the file area — either a list or Miller columns.
//!
//! Geometry lives in the `*_rect` / `*_at` helpers so drawing and hit-testing
//! read the same numbers — the otto-kit convention. Nothing here holds state.

use otto_kit::components::icon::Icon;
use otto_kit::components::scroll::{ScrollRenderer, ScrollState};
use otto_kit::components::titlebar::{WindowControl, WindowControls, WindowControlsState};
use otto_kit::icons;
use otto_kit::prelude::*;
use skia_safe::{ClipOp, Contains, Paint, PathBuilder, Point, RRect};

use crate::model::{self, Column, Entry, Place, SortKey};

/// The size the window asks for on first map; after that the compositor is in
/// charge and everything draws against the configured size.
pub const WINDOW_W: f32 = 1100.0;
pub const WINDOW_H: f32 = 700.0;
pub const MIN_W: f32 = 640.0;
pub const MIN_H: f32 = 400.0;
pub const CORNER: f32 = 12.0;

/// [`CORNER`], or square on a desktop configured without rounded corners.
pub fn corner() -> f32 {
    otto_kit::corners::radius(CORNER)
}

/// Full-height sidebar, like Finder's — the header sits beside it, not above.
pub const SIDEBAR_W: f32 = 232.0;
/// The preview pane's width — a Miller column of its own, sitting right
/// after the last real one and panned into view exactly the way a freshly
/// opened directory column is, rather than docked outside the stack.
pub const PREVIEW_W: f32 = 280.0;
/// The "big header": tall enough for a large title with a subtitle under it,
/// which is what makes the window read as a document rather than a dialog.
pub const HEADER_H: f32 = 92.0;

/// The picker's action row along the bottom: filter control on the left,
/// Cancel and the accept button on the right. Zero in the browser, which has
/// no footer at all — see [`Frame::footer`].
pub const FOOTER_H: f32 = 60.0;
/// Column-name strip, list view only.
pub const COLUMNS_H: f32 = 28.0;

/// Rows are tight and abut with no gap, in every view: a listing is scanned
/// rather than read, and the fewer pixels between one name and the next, the
/// more of the directory the eye takes in at once.
pub const ROW_H: f32 = 24.0;
const CONTENT_PAD: f32 = 20.0;
pub const ICON_SIZE: f32 = 18.0;
const CONTROLS_INSET: f32 = 18.0;
/// Optical centres of the header's two text lines, within `HEADER_H`.
const TITLE_CY: f32 = 40.0;
const SUBTITLE_CY: f32 = 66.0;
/// The back/forward pair sits to the left of the title as one split button:
/// a single rounded capsule the width of both halves, divided by a hairline.
const NAV_BTN_W: f32 = 30.0;
const NAV_BTN_H: f32 = 26.0;
const NAV_GROUP_W: f32 = NAV_BTN_W * 2.0;
const NAV_RADIUS: f32 = 7.0;
const NAV_ICON_SIZE: f32 = 13.0;
/// How far the title's own text starts right of the sidebar, once the nav
/// pair and its trailing gap are accounted for.
const TITLE_X: f32 = SIDEBAR_W + CONTENT_PAD + NAV_GROUP_W + 10.0;

/// The picker toolbar's single row, optically centred in the header the way
/// the browser's two text lines are.
pub const TOOLBAR_CY: f32 = 52.0;
/// The location control's width. Wide enough for a deep directory name and
/// narrow enough to leave the switcher its corner.
const LOCATION_W: f32 = 260.0;

/// The picker's location control, centred on the toolbar row.
pub fn location_rect(width: f32) -> Rect {
    let left = ((width - SIDEBAR_W - LOCATION_W) / 2.0 + SIDEBAR_W)
        .max(SIDEBAR_W + CONTENT_PAD + NAV_GROUP_W + 16.0);
    Rect::from_ltrb(
        left,
        TOOLBAR_CY - 15.0,
        left + LOCATION_W,
        TOOLBAR_CY + 15.0,
    )
}
/// One Miller pane's default width. Every pane shares one width — a column
/// whose width changes as you descend is disorienting, and this is what makes
/// the view scannable — but that shared width is user-resizable.
pub const MILLER_W: f32 = 260.0;
/// Miller panes cannot be dragged narrower than this.
pub const MILLER_MIN_W: f32 = 140.0;
pub const MILLER_MAX_W: f32 = 520.0;

/// How close the pointer must be to a column boundary to grab it.
const COLUMN_GRAB: f32 = 4.0;
/// Column widths cannot be dragged below this — thinner and a header label
/// no longer fits.
pub const COLUMN_MIN_W: f32 = 48.0;

/// The list view's Size/Kind/Modified column widths — Name is not stored: it
/// is whatever is left between the sidebar and wherever Size starts, so it
/// grows and shrinks with the window the way Finder's does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListColumnWidths {
    pub size: f32,
    pub kind: f32,
    pub modified: f32,
}

impl Default for ListColumnWidths {
    fn default() -> Self {
        Self {
            size: 90.0,
            kind: 110.0,
            modified: 150.0,
        }
    }
}

/// Which divider between two list-view columns the pointer is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnBoundary {
    /// Between Name and Size — dragging it resizes Size (Name absorbs the
    /// rest, being unstored).
    Size,
    /// Between Size and Kind — dragging it resizes Kind (Size keeps its own
    /// width and slides with the boundary; Name absorbs the difference).
    Kind,
    /// Between Kind and Modified — dragging it resizes Modified the same way.
    Modified,
}
/// Gap between a Miller pane's top edge and its first row, so the row does not
/// touch the header hairline.
pub const MILLER_ROW_INSET: f32 = 8.0;

/// Icon grid metrics. Kept together and public because this geometry is the
/// reusable part: a desktop surface lays out the same cells against its own
/// rect, with no file-manager chrome around them.
pub const CELL_W: f32 = 112.0;
pub const CELL_H: f32 = 120.0;
pub const GRID_ICON: f32 = 64.0;
const GRID_PAD: f32 = 14.0;
/// Space between the bottom of the icon and the optical centre of the
/// caption's first line. Tuned so the icon's selection rectangle and the
/// caption's pill meet edge to edge: they read as one highlight, without
/// either overlapping the other.
const GRID_LABEL_GAP: f32 = 16.0;
/// Baseline-to-baseline distance between the caption's two lines.
const GRID_LABEL_LINE: f32 = 16.0;
/// Padding above and below the caption inside its selection pill. The pill is
/// built out from the line centres, so the text sits optically centred in it.
const GRID_LABEL_INSET: f32 = 10.0;
/// How far the icon's selection rectangle stands off the icon itself. Small,
/// so the highlight reads as belonging to the icon rather than to the cell.
const GRID_ICON_INSET: f32 = 6.0;

/// The selection rectangle behind a grid icon, given its cell and the top of
/// the icon inside it. Public because a desktop surface draws the same
/// highlight against its own cells.
pub fn grid_icon_highlight_rect(cell: Rect, icon_top: f32) -> Rect {
    Rect::from_xywh(
        cell.center_x() - GRID_ICON / 2.0 - GRID_ICON_INSET,
        icon_top - GRID_ICON_INSET,
        GRID_ICON + GRID_ICON_INSET * 2.0,
        GRID_ICON + GRID_ICON_INSET * 2.0,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    List,
    Columns,
    Grid,
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// The file area: right of the sidebar, below the header (and the column strip
/// in list view).
pub fn content_viewport(width: f32, height: f32, mode: ViewMode) -> Rect {
    let top = match mode {
        ViewMode::List => HEADER_H + COLUMNS_H,
        ViewMode::Columns | ViewMode::Grid => HEADER_H,
    };
    Rect::from_ltrb(SIDEBAR_W, top, width, height)
}

// --- Icon grid geometry -----------------------------------------------------
//
// These four functions know nothing about the browser: give them an area and a
// cell index and they answer where it goes. That is what makes the grid
// reusable for a desktop surface, which has the same cells and none of the
// chrome. They are the shape `icon_grid` takes when it moves into otto-kit.

/// How many cells fit across `area`. Never zero, so a very narrow window
/// degrades to one column rather than dividing by it.
pub fn grid_columns(area: Rect) -> usize {
    (((area.width() - GRID_PAD) / CELL_W).floor() as usize).max(1)
}

/// The cell rect for `index`, in `area`, scrolled by `scroll`.
pub fn grid_cell_rect(area: Rect, index: usize, scroll: f32) -> Rect {
    let cols = grid_columns(area);
    let (row, col) = (index / cols, index % cols);
    Rect::from_xywh(
        area.left + GRID_PAD + col as f32 * CELL_W,
        area.top + GRID_PAD + row as f32 * CELL_H - scroll,
        CELL_W,
        CELL_H,
    )
}

/// The cell index under `(x, y)`, if any.
pub fn grid_cell_at(area: Rect, x: f32, y: f32, count: usize, scroll: f32) -> Option<usize> {
    if !area.contains(Point::new(x, y)) {
        return None;
    }
    let cols = grid_columns(area);
    let local_x = x - area.left - GRID_PAD;
    let local_y = y - area.top - GRID_PAD + scroll;
    if local_x < 0.0 || local_y < 0.0 {
        return None;
    }
    let col = (local_x / CELL_W) as usize;
    let row = (local_y / CELL_H) as usize;
    if col >= cols {
        return None;
    }
    let index = row * cols + col;
    (index < count).then_some(index)
}

/// The cells that intersect `band` — the visible strip of the grid, in the
/// same window coordinates [`grid_cell_rect`] places cells in.
///
/// Widened to whole rows of cells, so the answer is one contiguous range a
/// caller can walk without testing each cell again. As with [`RowStrip`], only
/// the vertical extent is compared: every column of the grid is on screen
/// whenever any of them is.
pub fn grid_visible_range(
    area: Rect,
    count: usize,
    scroll: f32,
    band: Rect,
) -> std::ops::Range<usize> {
    if count == 0 || band.is_empty() {
        return 0..0;
    }
    let cols = grid_columns(area);
    let top = area.top + GRID_PAD - scroll;
    let first_row = ((band.top - top) / CELL_H).floor().max(0.0) as usize;
    let last_row = ((band.bottom - top) / CELL_H).floor().min(count as f32);
    if last_row < 0.0 {
        return 0..0;
    }
    let first = (first_row * cols).min(count);
    let end = ((last_row as usize + 1) * cols).min(count);
    first..end.max(first)
}

/// The cells `band` touches — the rubber band's hit test, the counterpart of
/// [`grid_cell_at`] for a rectangle rather than a point.
///
/// Closed-form over the rows and columns the band spans, so sweeping a band
/// across a directory of ten thousand files costs what the band covers rather
/// than what the directory holds. A cell counts as caught the moment the band
/// touches its rect at all, which is what makes flicking a thin band through a
/// row of icons select them: requiring containment would mean drawing a box
/// carefully around each one.
pub fn grid_cells_in_rect(area: Rect, count: usize, scroll: f32, band: Rect) -> Vec<usize> {
    // A band with no extent at all catches nothing, even sitting squarely
    // over a cell: that band is a click on empty space, and a click on empty
    // space means nothing is selected. A band flat in *one* axis is still a
    // drag — a pointer swept straight across a row rarely moves a whole pixel
    // down — and catches what the line crosses.
    if count == 0 || (band.width() <= 0.0 && band.height() <= 0.0) {
        return Vec::new();
    }
    let cols = grid_columns(area);
    let origin_x = area.left + GRID_PAD;
    let origin_y = area.top + GRID_PAD - scroll;

    let span = |lo: f32, hi: f32, pitch: f32, origin: f32| {
        let first = ((lo - origin) / pitch).floor().max(0.0);
        let end = ((hi - origin) / pitch).ceil().max(0.0);
        (first as usize, end as usize)
    };
    let (first_col, end_col) = span(band.left, band.right, CELL_W, origin_x);
    let (first_row, end_row) = span(band.top, band.bottom, CELL_H, origin_y);
    let end_col = end_col.min(cols);

    let mut hit = Vec::new();
    for row in first_row..end_row {
        for col in first_col..end_col {
            let index = row * cols + col;
            if index >= count {
                return hit;
            }
            hit.push(index);
        }
    }
    hit
}

/// Total height `count` cells need in `area`.
pub fn grid_content_height(area: Rect, count: usize) -> f32 {
    let cols = grid_columns(area);
    let rows = count.div_ceil(cols);
    rows as f32 * CELL_H + GRID_PAD * 2.0
}

pub fn place_rect(index: usize) -> Rect {
    const FIRST_Y: f32 = 78.0;
    const STEP: f32 = 30.0;
    Rect::from_xywh(10.0, FIRST_Y + index as f32 * STEP, SIDEBAR_W - 20.0, 26.0)
}

pub fn place_at(x: f32, y: f32, count: usize) -> Option<usize> {
    (0..count).find(|i| place_rect(*i).contains(Point::new(x, y)))
}

/// The split button holding both arrows, before the title.
pub fn nav_group_rect() -> Rect {
    Rect::from_xywh(
        SIDEBAR_W + CONTENT_PAD,
        TITLE_CY - NAV_BTN_H / 2.0,
        NAV_GROUP_W,
        NAV_BTN_H,
    )
}

/// The Back half of the split button.
pub fn nav_back_rect() -> Rect {
    let group = nav_group_rect();
    Rect::from_xywh(group.left, group.top, NAV_BTN_W, group.height())
}

/// The Forward half of the split button.
pub fn nav_forward_rect() -> Rect {
    let group = nav_group_rect();
    Rect::from_xywh(group.left + NAV_BTN_W, group.top, NAV_BTN_W, group.height())
}

/// Which nav arrow, if either, sits under `(x, y)`.
pub fn nav_button_at(x: f32, y: f32) -> Option<NavButton> {
    let p = Point::new(x, y);
    if nav_back_rect().contains(p) {
        Some(NavButton::Back)
    } else if nav_forward_rect().contains(p) {
        Some(NavButton::Forward)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavButton {
    Back,
    Forward,
}

/// The three view-switcher segments, in the header's top right.
pub fn switcher_rect(width: f32) -> Rect {
    Rect::from_xywh(width - CONTENT_PAD - 114.0, 24.0, 114.0, 26.0)
}

pub const SWITCHER_MODES: [ViewMode; 3] = [ViewMode::List, ViewMode::Grid, ViewMode::Columns];

/// Which view the switcher segment at `(x, y)` selects, if any.
pub fn switcher_at(x: f32, y: f32, width: f32) -> Option<ViewMode> {
    let rect = switcher_rect(width);
    if !rect.contains(Point::new(x, y)) {
        return None;
    }
    let segment = ((x - rect.left) / (rect.width() / 3.0)) as usize;
    SWITCHER_MODES.get(segment.min(2)).copied()
}

pub(crate) fn column_edges(width: f32, widths: ListColumnWidths) -> (f32, f32, f32) {
    let content_w = width - SIDEBAR_W;
    let modified_x = width - CONTENT_PAD - widths.modified;
    let kind_x = modified_x - widths.kind;
    let size_x = kind_x - widths.size;
    let min_name = SIDEBAR_W + CONTENT_PAD + 120.0;
    if size_x < min_name {
        let spread = (content_w - 140.0).max(90.0) / 3.0;
        let base = SIDEBAR_W + CONTENT_PAD + 120.0;
        (base, base + spread, base + spread * 2.0)
    } else {
        (size_x, kind_x, modified_x)
    }
}

/// The divider the pointer is over, in the column-name strip — `None` once
/// the layout has fallen back to the narrow-window spread (those edges are
/// not draggable, since they are not real column widths).
pub fn column_boundary_at(
    x: f32,
    y: f32,
    width: f32,
    widths: ListColumnWidths,
) -> Option<ColumnBoundary> {
    if !(HEADER_H..=HEADER_H + COLUMNS_H).contains(&y) || x < SIDEBAR_W {
        return None;
    }
    let (size_x, kind_x, modified_x) = column_edges(width, widths);
    let modified_x_stored = width - CONTENT_PAD - widths.modified;
    if (modified_x - modified_x_stored).abs() > 0.5 {
        // The narrow-window fallback spread is active — those edges are not
        // real column widths, so there is nothing to drag.
        return None;
    }
    for (edge, boundary) in [
        (size_x, ColumnBoundary::Size),
        (kind_x, ColumnBoundary::Kind),
        (modified_x, ColumnBoundary::Modified),
    ] {
        if (x - edge).abs() <= COLUMN_GRAB {
            return Some(boundary);
        }
    }
    None
}

/// The Name column's left edge, in list-view row coordinates — where the
/// text starts, past the icon.
pub(crate) fn name_text_x() -> f32 {
    SIDEBAR_W + CONTENT_PAD + ICON_SIZE + 10.0
}

/// The `size` width that makes the Name column exactly wide enough for
/// `longest_name` (measured in points), for a double-click on the Name/Size
/// divider — Finder's "fit to content" gesture.
pub fn fit_size_column(width: f32, widths: ListColumnWidths, longest_name: f32) -> f32 {
    let kind_x = width - CONTENT_PAD - widths.modified - widths.kind;
    let desired_size_x = name_text_x() + longest_name + 12.0;
    let max_w = (kind_x - SIDEBAR_W - CONTENT_PAD).max(COLUMN_MIN_W);
    (kind_x - desired_size_x).clamp(COLUMN_MIN_W, max_w)
}

/// Where an in-place rename's text field sits over row `index` of the list —
/// the same span the name label would otherwise occupy.
pub fn list_rename_rect(
    width: f32,
    list_columns: ListColumnWidths,
    count: usize,
    scroll: f32,
    index: usize,
) -> Rect {
    let row = RowStrip::list(width, count, scroll).rect(index);
    let (size_x, ..) = column_edges(width, list_columns);
    let name_x = name_text_x();
    Rect::from_ltrb(name_x - 4.0, row.top + 3.0, size_x - 8.0, row.bottom - 3.0)
}

/// Where an in-place rename's text field sits over row `index` of Miller
/// pane `depth` — the same span the name label would otherwise occupy.
pub fn miller_rename_rect(
    height: f32,
    pan: f32,
    miller_w: f32,
    depth: usize,
    count: usize,
    scroll: f32,
    index: usize,
    is_dir: bool,
) -> Rect {
    let pane = miller_pane_rect(depth, height, pan, miller_w);
    let row = RowStrip::miller(pane, count, scroll).rect(index);
    let name_x = row.left + 14.0 + ICON_SIZE + 8.0;
    let trailing = if is_dir { 24.0 } else { 8.0 };
    Rect::from_ltrb(
        name_x - 4.0,
        row.top + 3.0,
        row.right - trailing,
        row.bottom - 3.0,
    )
}

/// Where an in-place rename's text field sits over grid cell `index` — over
/// the caption, sized like the selection pill it replaces so the cell does not
/// visibly change shape when the field appears.
pub fn grid_rename_rect(width: f32, height: f32, scroll: f32, index: usize) -> Rect {
    let area = content_viewport(width, height, ViewMode::Grid);
    let cell = grid_cell_rect(area, index, scroll);
    let center_y = cell.top + 8.0 + GRID_ICON + GRID_LABEL_GAP;
    Rect::from_ltrb(
        cell.left + 2.0,
        center_y - GRID_LABEL_INSET - 2.0,
        cell.right - 2.0,
        center_y + GRID_LABEL_LINE + GRID_LABEL_INSET + 2.0,
    )
}

/// A pane's rows, as a uniform-pitch strip in window coordinates with the
/// pane's scroll already subtracted.
///
/// Every consumer of row geometry is derived from this one description — the
/// render walk, the hit test, the keyboard's idea of where a row is, and the
/// Quick View anchor — so what is painted and what is clickable cannot drift
/// apart. Because the pitch is fixed the walk is closed-form rather than a
/// fold over the entries, which is what lets a directory of ten thousand
/// files cost a frame no more than one of ten.
#[derive(Debug, Clone, Copy)]
pub struct RowStrip {
    /// Top of row 0, scroll already applied.
    top: f32,
    left: f32,
    width: f32,
    count: usize,
}

impl RowStrip {
    /// The list view's single strip: full content width, flush under the
    /// column-name band.
    pub fn list(width: f32, count: usize, scroll: f32) -> Self {
        Self {
            top: HEADER_H + COLUMNS_H - scroll,
            left: SIDEBAR_W,
            width: width - SIDEBAR_W,
            count,
        }
    }

    /// One Miller pane's strip. Rows start a little way down the pane so the
    /// first one does not touch the header hairline.
    pub(crate) fn miller(pane: Rect, count: usize, scroll: f32) -> Self {
        Self {
            top: pane.top + MILLER_ROW_INSET - scroll,
            left: pane.left,
            width: pane.width(),
            count,
        }
    }

    pub fn rect(&self, index: usize) -> Rect {
        Rect::from_xywh(
            self.left,
            self.top + index as f32 * ROW_H,
            self.width,
            ROW_H,
        )
    }

    /// The row `y` falls on, if any. Rows above the strip and past its last
    /// entry are both misses.
    pub fn index_at(&self, y: f32) -> Option<usize> {
        let local = y - self.top;
        if local < 0.0 {
            return None;
        }
        let index = (local / ROW_H) as usize;
        (index < self.count).then_some(index)
    }

    /// The rows that intersect `band` — the visible band of the pane, in the
    /// same window coordinates the rows are placed in.
    ///
    /// Only the vertical extent is compared: a row always spans its pane's
    /// full width, so a horizontal test would reject nothing while risking
    /// rejecting something (a chevron, a date column) that reaches past the
    /// nominal edge. The comparison is inclusive at both ends so a row
    /// resting exactly on an edge survives, along with the stripe or hairline
    /// it contributes there.
    ///
    /// An empty band — a Miller pane panned off screen — yields no rows at
    /// all, which is the whole point: that pane costs nothing.
    pub fn visible(&self, band: Rect) -> std::ops::Range<usize> {
        if self.count == 0 || band.is_empty() {
            return 0..0;
        }
        let first = (((band.top - self.top) / ROW_H).floor().max(0.0) as usize).min(self.count);
        // Clamped before the cast: a band far past the end of a short strip
        // would otherwise turn into an index no `usize` can hold.
        let last = ((band.bottom - self.top) / ROW_H)
            .floor()
            .min(self.count as f32);
        if last < 0.0 {
            return 0..0;
        }
        let end = (last as usize + 1).min(self.count);
        first..end.max(first)
    }
}

/// The entry index under `(x, y)` in list view.
pub fn row_at(x: f32, y: f32, width: f32, height: f32, count: usize, scroll: f32) -> Option<usize> {
    if !content_viewport(width, height, ViewMode::List).contains(Point::new(x, y)) {
        return None;
    }
    RowStrip::list(width, count, scroll).index_at(y)
}

/// The untruncated pane rect, for laying rows out — drawing clips it, but a row
/// must not shift because its pane is half off-screen.
pub fn miller_pane_rect(depth: usize, height: f32, pan: f32, miller_w: f32) -> Rect {
    let left = SIDEBAR_W + depth as f32 * miller_w - pan;
    Rect::from_ltrb(left, HEADER_H, left + miller_w, height)
}

/// The preview pane's untruncated rect — a trailing member of the stack, one
/// `miller_w`-wide slot past the last real column, but its own [`PREVIEW_W`]
/// wide rather than sharing the columns' width.
pub fn preview_pane_rect(columns_len: usize, height: f32, pan: f32, miller_w: f32) -> Rect {
    let left = SIDEBAR_W + columns_len as f32 * miller_w - pan;
    Rect::from_ltrb(left, HEADER_H, left + PREVIEW_W, height)
}

/// What a drag hovering the window would drop onto, so it can be outlined.
///
/// Carries only what the view needs to find the rect again — the path the drop
/// would land in is the application's business, not the drawing's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropHighlight {
    /// A directory row or cell, which the files would go *into*.
    Row { depth: usize, index: usize },
    /// A pane's own directory: the drag is over its background, not any row.
    Pane { depth: usize },
    /// A sidebar place.
    Place { index: usize },
}

/// `rect` cut down to `clip`, or `None` when the two do not meet.
fn clipped_to(rect: Rect, clip: Rect) -> Option<Rect> {
    let mut out = rect;
    out.intersect(clip).then_some(out)
}

/// Where [`DropHighlight`] should be outlined, in window coordinates.
///
/// Runs the hit tests backwards — same strips, same pane rects — so the
/// outline lands exactly on the thing the drop will act on, in every view.
pub fn drop_highlight_rect(f: &Frame, target: DropHighlight) -> Option<Rect> {
    match target {
        DropHighlight::Place { index } => (index < f.places.len()).then(|| place_rect(index)),
        DropHighlight::Pane { depth } => {
            let viewport = content_viewport(f.width, f.height, f.mode);
            let pane = match f.mode {
                ViewMode::Columns => miller_pane_rect(depth, f.height, f.pan, f.miller_w),
                // One pane fills the content area in the flat views.
                _ => viewport,
            };
            // On the pane's own edges, where the divider between columns runs.
            // Held inside them the ring crosses the rows instead of bounding
            // them, which reads as a box drawn over the content rather than as
            // the column being picked out. Square corners need no room to
            // curve, so there is nothing to hold it off the edge for.
            clipped_to(pane, viewport)
        }
        DropHighlight::Row { depth, index } => {
            let pane = f.panes.get(depth)?;
            let rect = match f.mode {
                ViewMode::Grid => grid_cell_rect(
                    content_viewport(f.width, f.height, ViewMode::Grid),
                    index,
                    pane.scroll,
                ),
                ViewMode::List => {
                    RowStrip::list(f.width, pane.entries.len(), pane.scroll).rect(index)
                }
                ViewMode::Columns => {
                    let full = miller_pane_rect(depth, f.height, f.pan, f.miller_w);
                    RowStrip::miller(full, pane.entries.len(), pane.scroll).rect(index)
                }
            };
            // A row scrolled out from under the pointer has no outline rather
            // than one drawn over the header.
            clipped_to(rect, content_viewport(f.width, f.height, f.mode))
        }
    }
}

/// Outline what the drop would land in.
///
/// Drawn on the window canvas after the panes, which puts it over the rows in
/// every view — including Miller, whose rows are the scene's own layers,
/// composited under this canvas. The exception is `OTTO_FILES_PANE_SUBS=1`,
/// where the columns are subsurfaces *over* this canvas and the outline is
/// hidden behind them; that mode is opt-in and its own drop feedback is a
/// separate piece of work.
fn draw_drop_highlight(canvas: &Canvas, f: &Frame) {
    let Some(target) = f.drop_target else {
        return;
    };
    let Some(rect) = drop_highlight_rect(f, target) else {
        return;
    };
    if rect.is_empty() {
        return;
    }

    // The ring alone. A wash inside it as well says the same thing twice, and
    // over a pane it tints a whole column of rows to point at one target.
    let mut ring = Paint::default();
    ring.set_anti_alias(true);
    ring.set_style(skia_safe::paint::Style::Stroke);
    // Inset by half the stroke: a centred stroke on the rect's own edge would
    // spill a pixel outside it, over the neighbouring row.
    ring.set_stroke_width(2.0);
    ring.set_color(accent_light(f.theme));

    let radius = drop_ring_radius(f.mode, target);
    if radius > 0.0 {
        canvas.draw_rrect(
            RRect::new_rect_xy(rect.with_inset((1.0, 1.0)), radius, radius),
            &ring,
        );
    } else {
        canvas.draw_rect(rect.with_inset((1.0, 1.0)), &ring);
    }
}

/// The corner radius of the drop ring: the shape of the thing it outlines.
///
/// A Miller column and a pane have square edges, and a rounded ring drawn
/// around one reads as a different object floating over it rather than as that
/// column being picked out — those stay square. A grid cell and a sidebar place
/// are rounded shapes with a rounded highlight of their own, and a square ring
/// around either reads as a box that missed.
fn drop_ring_radius(mode: ViewMode, target: DropHighlight) -> f32 {
    match target {
        // The same radius the place's own selection is drawn with.
        DropHighlight::Place { .. } => 6.0,
        // A cell in the grid; a band abutting its neighbours anywhere else.
        DropHighlight::Row { .. } if mode == ViewMode::Grid => 8.0,
        DropHighlight::Row { .. } | DropHighlight::Pane { .. } => 0.0,
    }
}

/// One list-view row's rect, in window coordinates.
///
/// The strip the hit test uses, read forwards — so a caller that knows which
/// row was hit can find out where it is without rebuilding a frame.
pub fn list_row_rect(width: f32, count: usize, index: usize, scroll: f32) -> Rect {
    RowStrip::list(width, count, scroll).rect(index)
}

/// One Miller row's rect, in window coordinates.
pub fn miller_row_rect(
    depth: usize,
    height: f32,
    pan: f32,
    miller_w: f32,
    count: usize,
    index: usize,
    scroll: f32,
) -> Rect {
    let pane = miller_pane_rect(depth, height, pan, miller_w);
    RowStrip::miller(pane, count, scroll).rect(index)
}

/// The picture carried under the cursor while files are being dragged.
///
/// Sized for one row's worth of content: an icon, the name beside it, and a
/// count when more than one file is travelling. Its top-left corner is where
/// the pointer is, so the card reads as something held rather than something
/// the cursor is inside.
pub const DRAG_IMAGE_W: f32 = 240.0;
pub const DRAG_IMAGE_H: f32 = 36.0;

/// One file travelling in a drag: where its picture starts, and what to draw.
///
/// The start is where the entry actually is on screen, as an offset inside the
/// drag image. The end is the pile under the cursor. Between the two is the
/// gather — see [`draw_drag_gather`].
#[derive(Clone)]
pub struct DragItem {
    pub entry: Entry,
    pub thumb: Option<skia_safe::Image>,
    /// Top-left of this item's picture at the moment the drag began, relative
    /// to the drag image's own origin.
    pub start: (f32, f32),
}

/// How many pictures a drag shows at most.
///
/// The cap is about the surface as much as the clutter: the drag image is one
/// surface whose size is the bounding box of the pictures, so an uncapped
/// selection would ask for one as tall as the listing. Since the ones shown
/// are those nearest the grab, the box stays within this many rows of it
/// however large the selection is. The badge counts them all.
pub const DRAG_ITEMS_MAX: usize = 50;

/// The count badge's box, and how far it sits off the cursor.
const DRAG_BADGE_H: f32 = 20.0;
const DRAG_BADGE_GAP: f32 = 6.0;

/// The width of a badge showing `count`.
pub fn drag_badge_width(count: usize) -> f32 {
    18.0 + count.to_string().len() as f32 * 8.0
}

/// The drag image: the files being carried, each drawn where it sits in the
/// listing, and a count at the cursor.
///
/// No pile and no gathering. The files keep the places and the alignment they
/// have in the view they came from, so what lifts off the listing looks like
/// the rows that were selected — which is what says *which* files are moving.
/// The badge is the only thing added, and it rides the cursor rather than the
/// files, at its bottom right where it covers neither the names nor whatever
/// the drag is being held over.
pub fn draw_drag_image(
    canvas: &Canvas,
    theme: &Theme,
    mode: ViewMode,
    items: &[DragItem],
    anchor: (f32, f32),
    count: usize,
) {
    canvas.clear(Color::TRANSPARENT);

    for item in items {
        canvas.save();
        canvas.translate(item.start);
        draw_drag_entry(canvas, theme, mode, &item.entry, item.thumb.as_ref());
        canvas.restore();
    }

    if count > 1 {
        let width = drag_badge_width(count);
        let badge = Rect::from_xywh(
            anchor.0 + DRAG_BADGE_GAP,
            anchor.1 + DRAG_BADGE_GAP,
            width,
            DRAG_BADGE_H,
        );
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        // Red, not the accent: the names travelling under it are highlighted
        // in the accent, and a badge in the same colour reads as one more of
        // them instead of as the count of them.
        paint.set_color(theme.accent_red);
        canvas.draw_rrect(
            RRect::new_rect_xy(badge, DRAG_BADGE_H / 2.0, DRAG_BADGE_H / 2.0),
            &paint,
        );

        Label::new(count.to_string())
            .with_style(styles::FOOTNOTE_EMPHASIZED)
            .with_color(Color::WHITE)
            .centered_on(badge.center_x(), badge.center_y())
            .render(canvas);
    }
}

/// One travelling file: its icon, and its name on a highlight.
///
/// The highlight is behind the *name* only. The icon carries its own shape and
/// a block of colour behind it just muddies it; the name is text over whatever
/// the drag is passing across, and it needs the ground to stay readable.
fn draw_drag_entry(
    canvas: &Canvas,
    theme: &Theme,
    mode: ViewMode,
    entry: &Entry,
    thumb: Option<&skia_safe::Image>,
) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    let font = styles::BODY_MEDIUM.font();

    match mode {
        ViewMode::Grid => {
            let cell = Rect::from_wh(CELL_W, CELL_H);
            let icon_top = cell.top + 8.0;
            if let Some(image) = thumb {
                draw_thumbnail(
                    canvas,
                    image,
                    Rect::from_xywh(
                        cell.center_x() - GRID_ICON / 2.0,
                        icon_top,
                        GRID_ICON,
                        GRID_ICON,
                    ),
                    false,
                );
            } else {
                let chain = entry.icon_chain();
                let refs: Vec<&str> = chain.iter().map(String::as_str).collect();
                if let Some(image) =
                    icons::cached_icon_chain_at(&refs, GRID_ICON as i32, icons::FULL_COLOUR_SIZE)
                {
                    canvas.draw_image_rect(
                        &image,
                        None,
                        Rect::from_xywh(
                            cell.center_x() - GRID_ICON / 2.0,
                            icon_top,
                            GRID_ICON,
                            GRID_ICON,
                        ),
                        &Paint::default(),
                    );
                }
            }

            // The caption on its pill, the way a selected cell wears it.
            let center_y = icon_top + GRID_ICON + GRID_LABEL_GAP;
            let (first, second) = split_label(&entry.name, 13);
            let caption = styles::CALLOUT_EMPHASIZED.font();
            let text_w = caption
                .measure_str(&first, None)
                .0
                .max(caption.measure_str(&second, None).0);
            let width = (text_w + 12.0).min(cell.width() - 4.0);
            let lines = if second.is_empty() {
                0.0
            } else {
                GRID_LABEL_LINE
            };
            paint.set_color(accent(theme));
            canvas.draw_rrect(
                RRect::new_rect_xy(
                    Rect::from_xywh(
                        cell.center_x() - width / 2.0,
                        center_y - GRID_LABEL_INSET,
                        width,
                        lines + GRID_LABEL_INSET * 2.0,
                    ),
                    5.0,
                    5.0,
                ),
                &paint,
            );
            for (line, offset) in [(first.as_str(), 0.0), (second.as_str(), GRID_LABEL_LINE)] {
                if line.is_empty() {
                    continue;
                }
                Label::new(line)
                    .with_style(styles::CALLOUT_EMPHASIZED)
                    .with_color(Color::WHITE)
                    .centered_at(cell.center_x(), center_y + offset)
                    .render(canvas);
            }
        }
        ViewMode::List | ViewMode::Columns => {
            // The row's own alignment: the icon where the listing puts it, and
            // the name the same distance after it.
            let lead = row_icon_lead(mode);
            let center_y = ROW_H / 2.0;
            draw_entry_icon(canvas, entry, lead, center_y, false, thumb);

            let name_x = lead + ICON_SIZE + if mode == ViewMode::List { 10.0 } else { 8.0 };
            let name = ellipsize(&font, &entry.name, DRAG_IMAGE_W - name_x);
            let width = font.measure_str(&name, None).0;

            paint.set_color(accent(theme));
            canvas.draw_rrect(
                RRect::new_rect_xy(
                    Rect::from_xywh(name_x - 6.0, center_y - 9.0, width + 12.0, 18.0),
                    5.0,
                    5.0,
                ),
                &paint,
            );

            Label::new(name)
                .with_style(styles::BODY_MEDIUM)
                .with_color(Color::WHITE)
                .centered_on(name_x, center_y)
                .render(canvas);
        }
    }
}

/// Where the icon starts inside a row of `mode`, measured from the row's left
/// edge — the same inset the listing draws it at, so a travelling row lines up
/// with the one it lifted off.
fn row_icon_lead(mode: ViewMode) -> f32 {
    match mode {
        ViewMode::List => CONTENT_PAD,
        ViewMode::Columns => 14.0,
        ViewMode::Grid => 0.0,
    }
}

/// Where the icon starts inside a row-shaped travelling picture.
/// The size of the drag image for `mode`. What the cursor carries is the thing
/// that was picked up, so the icon grid carries a cell and the row views carry
/// a row-shaped card.
pub fn drag_image_size(mode: ViewMode) -> (f32, f32) {
    match mode {
        ViewMode::Grid => (CELL_W, CELL_H),
        ViewMode::List | ViewMode::Columns => (DRAG_IMAGE_W, DRAG_IMAGE_H),
    }
}

/// Which pane and row is under `(x, y)` in Miller view.
pub fn miller_at(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    columns: &[Column],
    counts: &[usize],
    pan: f32,
    miller_w: f32,
) -> Option<(usize, Option<usize>)> {
    if !content_viewport(width, height, ViewMode::Columns).contains(Point::new(x, y)) {
        return None;
    }
    for depth in 0..columns.len() {
        let pane = miller_pane_rect(depth, height, pan, miller_w);
        if x < pane.left || x >= pane.right {
            continue;
        }
        let strip = RowStrip::miller(pane, counts[depth], columns[depth].scroll.offset());
        return Some((depth, strip.index_at(y)));
    }
    None
}

/// How far the stack must be panned to bring the pane spanning
/// `[left, left + pane_w)` fully into a `width`-wide viewport.
fn pan_for(left: f32, pane_w: f32, width: f32, current: f32) -> f32 {
    let visible = width - SIDEBAR_W;
    let right = left + pane_w;
    if right - current > visible {
        (right - visible).max(0.0)
    } else if left < current {
        left
    } else {
        current
    }
}

/// How far the Miller stack must be panned to keep pane `depth` in view.
pub fn miller_pan_for(depth: usize, width: f32, current: f32, miller_w: f32) -> f32 {
    pan_for(depth as f32 * miller_w, miller_w, width, current)
}

/// How far the stack must be panned to bring the preview pane — sitting
/// right after the last of `columns_len` real columns — fully into view.
/// The same gesture [`miller_pan_for`] performs for a freshly opened column.
pub fn preview_pan_for(columns_len: usize, width: f32, current: f32, miller_w: f32) -> f32 {
    pan_for(columns_len as f32 * miller_w, PREVIEW_W, width, current)
}

/// Total width the whole stack wants: every real column, plus the preview
/// pane's own width when one is showing.
pub fn miller_content_width(depth: usize, miller_w: f32, preview_w: f32) -> f32 {
    depth as f32 * miller_w + preview_w
}

/// Is the pointer on the draggable edge between two Miller panes? All panes
/// share one width, so any divider found resizes them all — dragging one
/// divider is dragging the width. Returns the depth of the pane whose right
/// edge was grabbed: since that edge sits `(depth + 1) * miller_w` from the
/// sidebar, the caller needs it to turn pointer travel back into a width
/// delta that tracks the mouse exactly, however deep the divider is.
pub fn miller_boundary_at(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    pan: f32,
    pane_count: usize,
    miller_w: f32,
) -> Option<usize> {
    if !(HEADER_H..=height).contains(&y) || x < SIDEBAR_W {
        return None;
    }
    let viewport_right = width;
    (0..pane_count).find(|&depth| {
        let pane = miller_pane_rect(depth, height, pan, miller_w);
        pane.right >= SIDEBAR_W
            && pane.right <= viewport_right
            && (x - pane.right).abs() <= COLUMN_GRAB
    })
}

pub fn column_at(x: f32, y: f32, width: f32, widths: ListColumnWidths) -> Option<SortKey> {
    if !(HEADER_H..=HEADER_H + COLUMNS_H).contains(&y) || x < SIDEBAR_W {
        return None;
    }
    let (size_x, kind_x, modified_x) = column_edges(width, widths);
    Some(if x >= modified_x {
        SortKey::Modified
    } else if x >= kind_x {
        SortKey::Kind
    } else if x >= size_x {
        SortKey::Size
    } else {
        SortKey::Name
    })
}

pub fn content_height(count: usize) -> f32 {
    count as f32 * ROW_H
}

/// The scrolling viewport of one pane, whichever view is on.
///
/// In list and grid views there is one pane and it is the whole content area;
/// in Miller view each column scrolls on its own, so each gets its own strip.
/// This is what a pane's [`ScrollView`](otto_kit::components::scroll::ScrollView)
/// is given as its viewport, so the scrollbar lands on the right edge of the
/// pane the pointer is actually over.
pub fn pane_viewport(
    width: f32,
    height: f32,
    mode: ViewMode,
    depth: usize,
    pan: f32,
    miller_w: f32,
) -> Rect {
    match mode {
        ViewMode::List | ViewMode::Grid => content_viewport(width, height, mode),
        ViewMode::Columns => {
            let mut pane = miller_pane_rect(depth, height, pan, miller_w);
            // Clipped to what is actually on screen: a pane panned half out of
            // the window must not put its scrollbar under the sidebar.
            if !pane.intersect(content_viewport(width, height, mode)) {
                return Rect::new_empty();
            }
            pane
        }
    }
}

/// Which of a Miller column's rows are on screen — the half-open range the
/// scene records a picture for.
///
/// The same band [`draw_miller`] used to walk, lifted out so the scene can key
/// its cached picture on it: cross a row boundary and the column re-records,
/// scroll within one and it does not.
pub fn miller_visible_range(f: &Frame, depth: usize) -> (usize, usize) {
    let pane = &f.panes[depth];
    let full = miller_pane_rect(depth, f.height, f.pan, f.miller_w);
    let strip = RowStrip::miller(full, pane.entries.len(), pane.scroll);
    let band = pane.band(pane_viewport(
        f.width,
        f.height,
        ViewMode::Columns,
        depth,
        f.pan,
        f.miller_w,
    ));
    let range = strip.visible(band);
    (range.start, range.end)
}

// ---------------------------------------------------------------------------
// The picker's action row
// ---------------------------------------------------------------------------
//
// Geometry first, drawing second, and the hit test reads the same rects the
// paint does — the shape every other control in this file takes.

const FOOTER_PAD: f32 = 20.0;
const FOOTER_BTN_H: f32 = 30.0;
const FOOTER_BTN_MIN_W: f32 = 92.0;
const FOOTER_BTN_RADIUS: f32 = 8.0;
const FOOTER_FILTER_W: f32 = 220.0;
/// One row of the open filter menu.
const FOOTER_MENU_ROW_H: f32 = 26.0;

/// The action row's own strip, below the file area.
pub fn footer_rect(width: f32, window_height: f32) -> Rect {
    Rect::from_ltrb(0.0, window_height - FOOTER_H, width, window_height)
}

/// The accept button, hard against the right edge.
pub fn footer_accept_rect(width: f32, window_height: f32) -> Rect {
    let cy = window_height - FOOTER_H / 2.0;
    Rect::from_ltrb(
        width - FOOTER_PAD - FOOTER_BTN_MIN_W,
        cy - FOOTER_BTN_H / 2.0,
        width - FOOTER_PAD,
        cy + FOOTER_BTN_H / 2.0,
    )
}

/// Cancel, to the accept button's left.
pub fn footer_cancel_rect(width: f32, window_height: f32) -> Rect {
    let accept = footer_accept_rect(width, window_height);
    Rect::from_ltrb(
        accept.left - 10.0 - FOOTER_BTN_MIN_W,
        accept.top,
        accept.left - 10.0,
        accept.bottom,
    )
}

/// The filter control, on the left where the sidebar ends.
pub fn footer_filter_rect(window_height: f32) -> Rect {
    let cy = window_height - FOOTER_H / 2.0;
    Rect::from_ltrb(
        SIDEBAR_W + FOOTER_PAD,
        cy - FOOTER_BTN_H / 2.0,
        SIDEBAR_W + FOOTER_PAD + FOOTER_FILTER_W,
        cy + FOOTER_BTN_H / 2.0,
    )
}

/// One row of the filter menu, which opens *upwards* out of the control —
/// there is nothing below the action row to open into.
pub fn footer_filter_option_rect(window_height: f32, index: usize, count: usize) -> Rect {
    let control = footer_filter_rect(window_height);
    let height = count as f32 * FOOTER_MENU_ROW_H + 8.0;
    let top = control.top - 6.0 - height + 4.0 + index as f32 * FOOTER_MENU_ROW_H;
    Rect::from_ltrb(control.left, top, control.right, top + FOOTER_MENU_ROW_H)
}

/// What the action row has under `(x, y)`, if anything.
///
/// `window_height` is the whole window, not the file area. Takes the two
/// facts it needs rather than a [`FooterData`], so a caller can hit-test
/// without holding a borrow of the state it is about to mutate.
pub fn footer_at(
    x: f32,
    y: f32,
    width: f32,
    window_height: f32,
    filter_count: usize,
    filter_open: bool,
) -> Option<FooterButton> {
    let point = Point::new(x, y);

    // The open menu floats above the row and takes the pointer first,
    // exactly as a context menu would.
    if filter_open {
        for index in 0..filter_count {
            if footer_filter_option_rect(window_height, index, filter_count).contains(point) {
                return Some(FooterButton::FilterOption(index));
            }
        }
    }
    if footer_accept_rect(width, window_height).contains(point) {
        return Some(FooterButton::Accept);
    }
    if footer_cancel_rect(width, window_height).contains(point) {
        return Some(FooterButton::Cancel);
    }
    if filter_count > 0 && footer_filter_rect(window_height).contains(point) {
        return Some(FooterButton::Filter);
    }
    None
}

/// The name row's band, above the buttons: a labelled field and, under it,
/// room for the one line that says why accept is disabled.
pub const FOOTER_NAME_H: f32 = 58.0;
const FOOTER_NAME_LABEL_W: f32 = 76.0;
const FOOTER_NAME_FIELD_H: f32 = 30.0;

/// The whole name band. Only ever asked for in `Save` mode; in every other
/// mode the window has no such band and the caller does not draw one.
pub fn footer_name_band(width: f32, window_height: f32) -> Rect {
    let bottom = window_height - FOOTER_H;
    Rect::from_ltrb(0.0, bottom - FOOTER_NAME_H, width, bottom)
}

/// The text field itself, which the [`TextInput`] is rendered into.
///
/// [`TextInput`]: otto_kit::components::text_input::TextInput
pub fn footer_name_rect(width: f32, window_height: f32) -> Rect {
    let band = footer_name_band(width, window_height);
    let left = SIDEBAR_W + FOOTER_PAD + FOOTER_NAME_LABEL_W;
    Rect::from_ltrb(
        left,
        band.top + 8.0,
        (width - FOOTER_PAD).max(left + 40.0),
        band.top + 8.0 + FOOTER_NAME_FIELD_H,
    )
}

/// The name row's ground and label. The value is drawn afterwards, by the
/// text input that owns it.
fn draw_name_row(canvas: &Canvas, f: &Frame, footer: &FooterData<'_>, window_h: f32) {
    let theme = f.theme;
    let field = footer_name_rect(f.width, window_h);

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(content_ground());
    canvas.draw_rrect(RRect::new_rect_xy(field, 6.0, 6.0), &paint);
    paint.set_style(skia_safe::paint::Style::Stroke);
    paint.set_stroke_width(1.0);
    paint.set_color(theme.fill_tertiary);
    canvas.draw_rrect(RRect::new_rect_xy(field, 6.0, 6.0), &paint);

    Label::new(otto_kit::t!("files-picker-save-as-field"))
        .with_style(styles::BODY)
        .with_color(theme.text_secondary)
        .centered_on(SIDEBAR_W + FOOTER_PAD, field.center_y())
        .render(canvas);

    if let Some(problem) = footer.save_problem {
        Label::new(problem)
            .with_style(styles::CALLOUT)
            .with_color(warning_color())
            .centered_on(field.left, field.bottom + 11.0)
            .render(canvas);
    }
}

/// The tone a blocking message is written in. Not on [`Theme`] because
/// nothing else in the picker needs it yet; the day a second caller does, it
/// moves there rather than being copied.
fn warning_color() -> Color {
    if matches!(current_color_scheme(), ColorScheme::Dark) {
        Color::from_argb(0xFF, 0xFF, 0x45, 0x3A)
    } else {
        Color::from_argb(0xFF, 0xD7, 0x00, 0x15)
    }
}

fn draw_footer(canvas: &Canvas, f: &Frame) {
    let Some(footer) = &f.action_row else {
        return;
    };
    let theme = f.theme;
    let window_h = f.height + f.footer;

    // A hairline, not a filled strip: the row sits on the same material the
    // rest of the window does, and only needs separating from the listing.
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(theme.fill_tertiary);
    paint.set_stroke_width(1.0);
    canvas.draw_line(
        Point::new(0.0, window_h - f.footer),
        Point::new(f.width, window_h - f.footer),
        &paint,
    );

    if footer.save_name {
        draw_name_row(canvas, f, footer, window_h);
    }

    if !footer.filters.is_empty() {
        draw_filter_control(canvas, f, footer, window_h);
    }

    draw_footer_button(
        canvas,
        footer_cancel_rect(f.width, window_h),
        "Cancel",
        theme.fill_secondary,
        theme.text_primary,
        footer.pressed == Some(FooterButton::Cancel),
        true,
    );
    draw_footer_button(
        canvas,
        footer_accept_rect(f.width, window_h),
        footer.accept_label,
        accent(theme),
        Color::WHITE,
        footer.pressed == Some(FooterButton::Accept),
        footer.accept_enabled,
    );
}

/// The accent, lightened towards white.
///
/// A drop outline is a hint about where something would land, not a selection —
/// which is painted in the accent at full strength — so it says the same colour
/// more quietly.
fn accent_light(theme: &Theme) -> Color {
    let base = accent(theme);
    let lift = |channel: u8| (channel as f32 + (255.0 - channel as f32) * 0.45) as u8;
    Color::from_argb(base.a(), lift(base.r()), lift(base.g()), lift(base.b()))
}

/// The user's accent colour, or the theme's own selection tone when the
/// desktop has not set one.
fn accent(theme: &Theme) -> Color {
    otto_kit::accent::current_accent().unwrap_or(theme.material_selection_focused)
}

fn draw_footer_button(
    canvas: &Canvas,
    rect: Rect,
    label: &str,
    background: Color,
    text: Color,
    pressed: bool,
    enabled: bool,
) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    // Disabled and pressed are both expressed as alpha, so a themed accent
    // stays itself rather than being replaced by a grey that ignores the
    // user's colour. The fill's *own* alpha is scaled, never replaced: the
    // theme's secondary fill is 8% black, and forcing it opaque would draw a
    // black slab where a barely-there button belongs.
    let scale = match (enabled, pressed) {
        (false, _) => 0.35,
        (true, true) => 0.70,
        (true, false) => 1.0,
    };
    paint.set_color(Color::from_argb(
        (background.a() as f32 * scale) as u8,
        background.r(),
        background.g(),
        background.b(),
    ));
    canvas.draw_rrect(
        RRect::new_rect_xy(rect, FOOTER_BTN_RADIUS, FOOTER_BTN_RADIUS),
        &paint,
    );

    Label::new(label)
        .with_style(styles::BODY_EMPHASIZED)
        .with_color(if enabled {
            text
        } else {
            Color::from_argb(0x80, text.r(), text.g(), text.b())
        })
        .centered_at(rect.center_x(), rect.center_y())
        .render(canvas);
}

fn draw_filter_control(canvas: &Canvas, f: &Frame, footer: &FooterData<'_>, window_h: f32) {
    let theme = f.theme;
    let rect = footer_filter_rect(window_h);

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(theme.fill_secondary);
    canvas.draw_rrect(
        RRect::new_rect_xy(rect, FOOTER_BTN_RADIUS, FOOTER_BTN_RADIUS),
        &paint,
    );

    let label = footer
        .filters
        .get(footer.current_filter)
        .map(String::as_str)
        .unwrap_or_else(|| otto_kit::t!("files-picker-all-files"));
    Label::new(label)
        .with_style(styles::BODY)
        .with_color(theme.text_primary)
        .centered_on(rect.left + 12.0, rect.center_y())
        .render(canvas);

    // The disclosure chevron, pointing the way the menu opens.
    let mut chevron = Paint::default();
    chevron.set_anti_alias(true);
    chevron.set_color(theme.text_secondary);
    chevron.set_style(skia_safe::paint::Style::Stroke);
    chevron.set_stroke_width(1.6);
    chevron.set_stroke_cap(skia_safe::paint::Cap::Round);
    let cx = rect.right - 14.0;
    let cy = rect.center_y();
    let mut path = skia_safe::PathBuilder::new();
    path.move_to((cx - 4.0, cy + 2.0));
    path.line_to((cx, cy - 2.5));
    path.line_to((cx + 4.0, cy + 2.0));
    canvas.draw_path(&path.detach(), &chevron);

    if !footer.filter_open {
        return;
    }

    let count = footer.filters.len();
    let first = footer_filter_option_rect(window_h, 0, count);
    let last = footer_filter_option_rect(window_h, count - 1, count);
    let panel = Rect::from_ltrb(rect.left, first.top - 4.0, rect.right, last.bottom + 4.0);

    let mut menu = Paint::default();
    menu.set_anti_alias(true);
    menu.set_color(theme.material_popup);
    canvas.draw_rrect(RRect::new_rect_xy(panel, 8.0, 8.0), &menu);

    for (index, name) in footer.filters.iter().enumerate() {
        let row = footer_filter_option_rect(window_h, index, count);
        let hovered = footer.hovered == Some(FooterButton::FilterOption(index));
        if hovered {
            let mut hl = Paint::default();
            hl.set_anti_alias(true);
            hl.set_color(accent(theme));
            canvas.draw_rrect(
                RRect::new_rect_xy(row.with_inset((4.0, 0.0)), 5.0, 5.0),
                &hl,
            );
        }
        Label::new(name.as_str())
            .with_style(styles::BODY)
            .with_color(if hovered {
                Color::WHITE
            } else {
                theme.text_primary
            })
            .centered_on(row.left + 24.0, row.center_y())
            .render(canvas);

        if index == footer.current_filter {
            let mut tick = Paint::default();
            tick.set_anti_alias(true);
            tick.set_color(if hovered {
                Color::WHITE
            } else {
                theme.text_primary
            });
            tick.set_style(skia_safe::paint::Style::Stroke);
            tick.set_stroke_width(1.8);
            tick.set_stroke_cap(skia_safe::paint::Cap::Round);
            let mx = row.left + 13.0;
            let my = row.center_y();
            let mut p = skia_safe::PathBuilder::new();
            p.move_to((mx - 4.0, my));
            p.line_to((mx - 1.0, my + 3.5));
            p.line_to((mx + 4.5, my - 4.0));
            canvas.draw_path(&p.detach(), &tick);
        }
    }
}

/// Whether the current colour scheme is the dark one — what the materials
/// below switch on, and what the scene keys its styles on.
pub fn is_dark() -> bool {
    matches!(current_color_scheme(), ColorScheme::Dark)
}

/// How tall one pane's content is, for the same three views.
pub fn pane_content_height(width: f32, height: f32, mode: ViewMode, count: usize) -> f32 {
    match mode {
        ViewMode::Grid => grid_content_height(content_viewport(width, height, mode), count),
        // Miller rows start a little way down the pane; the list starts flush.
        ViewMode::Columns => content_height(count) + MILLER_ROW_INSET,
        ViewMode::List => content_height(count),
    }
}

/// Where item `index` sits in its pane's content coordinates — its top and its
/// height, before the pane's scroll offset is taken off. This is the half of
/// the geometry a "scroll the cursor into view" needs; the other half is the
/// pane's viewport height, from [`pane_viewport`].
pub fn item_span(width: f32, height: f32, mode: ViewMode, index: usize) -> (f32, f32) {
    match mode {
        ViewMode::List => (index as f32 * ROW_H, ROW_H),
        // Miller rows start a little way down the pane.
        ViewMode::Columns => (MILLER_ROW_INSET + index as f32 * ROW_H, ROW_H),
        ViewMode::Grid => {
            let cols = grid_columns(content_viewport(width, height, mode));
            (GRID_PAD + (index / cols) as f32 * CELL_H, CELL_H)
        }
    }
}

pub fn is_drag_area(x: f32, y: f32, width: f32) -> bool {
    if y > HEADER_H || x > width {
        return false;
    }
    if switcher_rect(width).contains(Point::new(x, y)) {
        return false;
    }
    if nav_group_rect().contains(Point::new(x, y)) {
        return false;
    }
    !Rect::from_xywh(CONTROLS_INSET - 4.0, CONTROLS_INSET - 4.0, 70.0, 20.0)
        .contains(Point::new(x, y))
}

pub fn control_at(x: f32, y: f32) -> Option<WindowControl> {
    const STEP: f32 = 20.0;
    const R: f32 = 6.0;
    for (i, control) in [
        WindowControl::Close,
        WindowControl::Minimize,
        WindowControl::Zoom,
    ]
    .into_iter()
    .enumerate()
    {
        let cx = CONTROLS_INSET + R + i as f32 * STEP;
        let cy = CONTROLS_INSET + R;
        if (x - cx).powi(2) + (y - cy).powi(2) <= (R + 3.0).powi(2) {
            return Some(control);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// One pane's worth of already-filtered, already-sorted entries.
pub struct PaneData<'a> {
    pub entries: Vec<&'a Entry>,
    /// One flag per entry, parallel to `entries`. A multi-selection has no
    /// single index, so this is a mask rather than a position.
    pub selected: Vec<bool>,
    /// Where the keyboard is. Drawn as a ring when it is not itself selected,
    /// so extending a selection with Ctrl+Arrow stays legible.
    pub cursor: Option<usize>,
    pub scroll: f32,
    /// The pane's scroll view state, for drawing its bar. `None` where there
    /// is no live view to read — the anchor tests, which care only about row
    /// geometry.
    pub bar: Option<&'a ScrollState>,
    pub loading: bool,
    pub error: Option<&'a str>,
}

impl PaneData<'_> {
    /// Draw this pane's scrollbar over whatever has been painted.
    ///
    /// The content is drawn by the pane itself, in window coordinates with
    /// `scroll` already applied, so the renderer is handed an empty closure:
    /// what is wanted here is the bar, the fade and the hover width, not the
    /// clipping.
    fn draw_scrollbar(&self, canvas: &Canvas, theme: &Theme) {
        if let Some(state) = self.bar {
            ScrollRenderer::draw(canvas, state, theme, |_, _| {});
        }
    }

    /// The band of this pane that can be seen, in the window coordinates its
    /// rows are laid out in — what the render walk is allowed to restrict
    /// itself to.
    ///
    /// This is the scroll view's own content rect, read back off the state so
    /// the two cannot disagree about where the pane is. `ScrollRenderer`
    /// reports the band in content-local coordinates, `(0, offset)` to
    /// `(width, offset + height)`; the rows here are placed in window
    /// coordinates with that same offset already subtracted, so the two are
    /// one band written from two origins, and mapping either way is just the
    /// viewport's own rect. That holds during a rubber-band overscroll too:
    /// the offset both sides read carries the overscroll, so the band and the
    /// rows move together and the pulled-past rows stay drawn.
    ///
    /// `fallback` is the viewport the pane would have been given, for callers
    /// with no live scroll view to ask — the anchor tests, which care only
    /// about row geometry.
    fn band(&self, fallback: Rect) -> Rect {
        match self.bar {
            Some(state) => {
                let viewport = state.viewport();
                let content = ScrollRenderer::visible_content_rect(state);
                content.with_offset((viewport.left, viewport.top - state.offset()))
            }
            None => fallback,
        }
    }
}

impl PaneData<'_> {
    pub fn is_selected(&self, index: usize) -> bool {
        self.selected.get(index).copied().unwrap_or(false)
    }
}

/// Everything the view needs for one frame. Rebuilt each frame; owns nothing.
pub struct Frame<'a> {
    pub width: f32,
    pub height: f32,
    pub theme: &'a Theme,
    pub title: &'a str,
    pub subtitle: String,
    pub places: &'a [Place],
    pub selected_place: Option<usize>,
    pub mode: ViewMode,
    /// One per column in the path stack. In list view only the last is drawn.
    pub panes: Vec<PaneData<'a>>,
    /// Which pane has the keyboard, and whose selection is "the" selection.
    pub active: usize,
    /// How far the Miller stack is panned left, in points.
    pub pan: f32,
    /// The Miller pan's scroll view state, for drawing the horizontal bar
    /// along the bottom of the stack. `None` outside Miller view, and in
    /// tests with no live view to read.
    pub pan_bar: Option<&'a ScrollState>,
    /// The Miller view's draggable pane width, shared by every pane.
    pub miller_w: f32,
    pub sort: SortKey,
    pub ascending: bool,
    /// The list view's draggable Size/Kind/Modified column widths.
    pub list_columns: ListColumnWidths,
    /// How far through the open pulse the entry being opened is, 0 to 1, and
    /// which pane it is in. `None` when nothing is opening. The entry itself is
    /// that pane's cursor — opening acts on the selection — so only the pane
    /// and the progress are needed here.
    pub opening: Option<(usize, f32)>,
    /// Depth and row index of an in-place rename in progress. That row's
    /// name label is skipped so the host's text field shows through instead.
    pub renaming: Option<(usize, usize)>,
    /// Paths marked by a cut, drawn dimmed until the paste happens.
    pub cut: Vec<std::path::PathBuf>,
    /// Hover and press state of the traffic lights.
    pub controls: WindowControlsState,
    /// Whether this is the focused window. An unfocused one steps back: gray
    /// traffic lights and a lighter title, the same depth cue the compositor's
    /// own decoration uses.
    pub focused: bool,
    /// Whether the compositor is blurring behind the window right now — which
    /// it does only for a focused window, and only where it can blur at all.
    /// The panel materials are translucent when it is and filled in when it is
    /// not; see [`opaque`].
    pub blurred: bool,
    /// Whether the back/forward arrows have anywhere to go.
    pub can_go_back: bool,
    pub can_go_forward: bool,
    /// The nav half being held down, drawn filled until it is released.
    pub nav_pressed: Option<NavButton>,
    /// The preview pane — a trailing member of the Miller stack, panned into
    /// and out of view the same as any real column — when one is showing.
    /// `None` outside Miller view or with no single file selected; drawn at
    /// [`PREVIEW_W`] by [`draw_miller`], the same way [`miller_w`] sizes
    /// every other pane rather than being carried on `Frame` itself.
    ///
    /// [`miller_w`]: Frame::miller_w
    pub preview: Option<PreviewData<'a>>,
    /// The picker's action row, when this is a picker window. `None` in the
    /// browser.
    ///
    /// Note that [`Frame::height`] is the *file area's* bottom, not the
    /// window's: the footer is subtracted from it before the frame is built,
    /// so every piece of geometry below already stops short of the action
    /// row without knowing the row exists. Chrome that genuinely spans the
    /// whole window adds [`Frame::footer`] back on.
    pub action_row: Option<FooterData<'a>>,
    /// How much of the window height the footer takes. `0.0` in the browser.
    pub footer: f32,
    /// Pointer is over Quick View's close button, so it lights up — the same
    /// hover behaviour the sheet's close dot has.
    pub quickview_close_hovered: bool,
    /// Thumbnails for the entries on screen, where any have been found. A
    /// file with one is drawn as itself instead of as its type's icon.
    ///
    /// `None` where there is no store to ask — the geometry tests, and any
    /// host drawing a listing without one. Every entry then falls back to its
    /// icon, which is what an entry with no thumbnail does anyway, so nothing
    /// downstream has to care which case it is in.
    pub thumbs: Option<&'a crate::thumbnails::Store>,
    /// What a drag now over the window would drop onto, outlined while it is
    /// over one. `None` when there is no drag, or it is over nothing that
    /// takes files.
    pub drop_target: Option<DropHighlight>,
    /// The rubber band being dragged out over the icon grid, in window
    /// coordinates. Grid view only: rows span their pane's whole width, so a
    /// band over a list or a Miller column could only ever say what dragging
    /// down the rows already says.
    pub marquee: Option<Rect>,
}

impl Frame<'_> {
    /// The thumbnail to draw for an entry, if one is ready.
    fn thumbnail(&self, entry: &Entry) -> Option<&skia_safe::Image> {
        self.thumbs?.image(&entry.path, entry.modified)
    }
}

/// The picker's action row.
pub struct FooterData<'a> {
    pub accept_label: &'a str,
    pub accept_enabled: bool,
    /// The filter control's labels. Empty hides the control entirely.
    pub filters: &'a [String],
    pub current_filter: usize,
    /// The filter menu is open, so the control draws as pressed and its
    /// options are listed above it.
    pub filter_open: bool,
    pub hovered: Option<FooterButton>,
    pub pressed: Option<FooterButton>,
    /// Save mode: draw the name row above the buttons. The field's own text
    /// is not here — the [`TextInput`] renders itself over this rect after
    /// the chrome, the way an in-place rename does.
    ///
    /// [`TextInput`]: otto_kit::components::text_input::TextInput
    pub save_name: bool,
    /// Why accept is disabled, shown under the name field. `None` when there
    /// is nothing to explain.
    pub save_problem: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FooterButton {
    Accept,
    Cancel,
    Filter,
    /// One option of the open filter menu.
    FilterOption(usize),
}

/// What the preview pane shows for the current selection.
pub struct PreviewData<'a> {
    pub name: &'a str,
    pub icon_chain: Vec<String>,
    /// The decode, once it lands. `None` while it is still in flight, which
    /// is the big icon's cue to stand in for it.
    pub decoded: Option<&'a otto_kit::preview::Preview>,
    pub first_row: usize,
    /// What the column says about the file underneath its name — kind, size,
    /// when it was last written. Already formatted, because the listing has
    /// the same facts in the same words and they are formatted once.
    pub info: Vec<String>,
}

pub fn draw(canvas: &Canvas, f: &Frame) {
    let theme = f.theme;

    // No clear: the surface's buffer is cleared by otto-kit before this runs,
    // and [`crate::scene`] has already composited the panels into it.
    //
    // The panels' grounds are not painted here. The sidebar's frosted
    // material, the header's fainter version of it and the content area's
    // sheet of opaque paper are all *styles* on layers the engine composites
    // under this canvas — see [`crate::scene`]. What is left below is the
    // chrome drawn on top of them.
    let mut paint = Paint::default();
    paint.set_anti_alias(true);

    draw_sidebar(canvas, f);
    draw_header(canvas, f);

    match f.mode {
        ViewMode::List => {
            draw_column_strip(canvas, f);
            draw_list(canvas, f);
        }
        ViewMode::Columns => draw_miller(canvas, f),
        ViewMode::Grid => draw_grid(canvas, f),
    }

    // After the panes and before the chrome: over the rows it points at, under
    // the sidebar divider and the footer.
    draw_drop_highlight(canvas, f);

    // Over the rows, for the same reason: it is a ghost of one of them.
    draw_open_pulse(canvas, f);

    paint.set_color(theme.fill_tertiary);
    paint.set_stroke_width(1.0);
    canvas.draw_line(
        Point::new(SIDEBAR_W, 0.0),
        Point::new(SIDEBAR_W, f.height + f.footer),
        &paint,
    );

    if f.action_row.is_some() {
        draw_footer(canvas, f);
    }
}

// ---------------------------------------------------------------------------
// The replace confirmation
// ---------------------------------------------------------------------------
//
// A card over a dimmed window, drawn after everything else — it is modal, and
// the dim is what says so. Geometry first, drawing second, and the hit test
// reads the same rects the paint does.

const CONFIRM_W: f32 = 396.0;
const CONFIRM_H: f32 = 172.0;
const CONFIRM_PAD: f32 = 20.0;
const CONFIRM_BTN_H: f32 = 30.0;
const CONFIRM_BTN_W: f32 = 104.0;

/// What the sheet says and how its buttons are lit.
pub struct ConfirmData<'a> {
    pub message: &'a str,
    pub detail: &'a str,
    pub pressed: Option<ConfirmButton>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmButton {
    Replace,
    Cancel,
}

/// The card, centred on the whole window rather than the file area: it is a
/// question about the dialog, not about the listing.
pub fn confirm_card_rect(width: f32, window_height: f32) -> Rect {
    let w = CONFIRM_W.min(width - 32.0);
    let left = ((width - w) / 2.0).max(0.0);
    let top = ((window_height - CONFIRM_H) / 2.0).max(0.0);
    Rect::from_ltrb(left, top, left + w, top + CONFIRM_H)
}

/// Replace, hard against the card's bottom-right — the affirmative answer in
/// the position every other accept button in the picker takes.
pub fn confirm_replace_rect(width: f32, window_height: f32) -> Rect {
    let card = confirm_card_rect(width, window_height);
    Rect::from_ltrb(
        card.right - CONFIRM_PAD - CONFIRM_BTN_W,
        card.bottom - CONFIRM_PAD - CONFIRM_BTN_H,
        card.right - CONFIRM_PAD,
        card.bottom - CONFIRM_PAD,
    )
}

pub fn confirm_cancel_rect(width: f32, window_height: f32) -> Rect {
    let replace = confirm_replace_rect(width, window_height);
    Rect::from_ltrb(
        replace.left - 10.0 - CONFIRM_BTN_W,
        replace.top,
        replace.left - 10.0,
        replace.bottom,
    )
}

/// What the sheet has under `(x, y)`.
///
/// Returns `Some` for the two buttons only. The caller still has to treat a
/// click anywhere else as swallowed rather than falling through to the
/// listing: the sheet is modal, and a stray click must not select a file
/// behind it.
pub fn confirm_at(x: f32, y: f32, width: f32, window_height: f32) -> Option<ConfirmButton> {
    let point = Point::new(x, y);
    if confirm_replace_rect(width, window_height).contains(point) {
        return Some(ConfirmButton::Replace);
    }
    if confirm_cancel_rect(width, window_height).contains(point) {
        return Some(ConfirmButton::Cancel);
    }
    None
}

/// Draw the sheet over the finished window.
pub fn draw_confirm(
    canvas: &Canvas,
    theme: &Theme,
    width: f32,
    window_height: f32,
    data: &ConfirmData<'_>,
) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);

    // The dim, not a blur: the window behind is already on its own surfaces
    // and a blur here would cost a full read-back every frame the sheet is up.
    paint.set_color(Color::from_argb(0x66, 0, 0, 0));
    canvas.draw_rect(Rect::from_ltrb(0.0, 0.0, width, window_height), &paint);

    let card = confirm_card_rect(width, window_height);
    paint.set_color(header_material());
    canvas.draw_rrect(RRect::new_rect_xy(card, 14.0, 14.0), &paint);

    Label::new(data.message)
        .with_style(styles::BODY_EMPHASIZED)
        .with_color(theme.text_primary)
        .with_width(card.width() - CONFIRM_PAD * 2.0)
        .centered_on(card.left + CONFIRM_PAD, card.top + CONFIRM_PAD + 10.0)
        .render(canvas);

    Label::new(data.detail)
        .with_style(styles::CALLOUT)
        .with_color(theme.text_secondary)
        .with_width(card.width() - CONFIRM_PAD * 2.0)
        .centered_on(card.left + CONFIRM_PAD, card.top + CONFIRM_PAD + 40.0)
        .render(canvas);

    draw_footer_button(
        canvas,
        confirm_cancel_rect(width, window_height),
        "Cancel",
        theme.fill_secondary,
        theme.text_primary,
        data.pressed == Some(ConfirmButton::Cancel),
        true,
    );
    // Replacing a file is the destructive answer, so it is not the accent —
    // the accent means "the safe thing you probably want", and here that is
    // Cancel.
    draw_footer_button(
        canvas,
        confirm_replace_rect(width, window_height),
        "Replace",
        warning_color(),
        Color::WHITE,
        data.pressed == Some(ConfirmButton::Replace),
        true,
    );
}

/// The preview pane's content, as a closure the scene records into its own
/// layer's cached picture.
///
/// The panel is the layer's own box, so this draws from `(0, 0)` rather than
/// in window coordinates: the column's position on screen is the layer's, and
/// panning the Miller stack moves it without touching what was recorded.
///
/// Nothing here paints the pane's ground. That is `content_ground` — and *not*
/// `otto_kit::preview::background`, which is the translucent material Quick
/// View floats over a dimmed window with. This pane is not a card laid over
/// the browser; it reads as one more column on the same opaque paper the
/// listing sits on. The scene carries it as the layer's background style.
///
/// The decode is cloned in, because the picture outlives the frame that
/// recorded it. That only happens when the selection changes or a decode
/// lands, which is exactly when the picture had to be rebuilt anyway.
pub fn preview_content(
    data: &PreviewData<'_>,
    theme: Theme,
) -> impl Fn(&Canvas, f32, f32) + Send + Sync + 'static {
    let name = data.name.to_string();
    let icon_chain = data.icon_chain.clone();
    let decoded = data.decoded.cloned();
    let first_row = data.first_row;
    let info = data.info.clone();

    move |canvas: &Canvas, width: f32, height: f32| {
        let panel = Rect::from_wh(width, height);
        // The caption is laid out from the bottom up — the facts, then the
        // name above them — and whatever is left over is the preview's. That
        // way the name and the facts sit on the same line whatever the file
        // is, instead of riding up and down with the size of the thing above
        // them, and the preview gets every point that is not spoken for.
        draw_preview_caption(canvas, &theme, panel, &name, &info);
        let stage = preview_stage_rect(panel, info.len());
        draw_preview_stage(
            canvas,
            &theme,
            stage,
            decoded.as_ref(),
            &icon_chain,
            first_row,
        );
    }
}

/// The drag image for a file picked up by its preview: the picture the
/// column is already showing, and nothing else.
///
/// A drag out of the listing carries rows because rows are what the eye was
/// looking at. Out of the preview column, what the eye was looking at is the
/// picture — so that is what lifts off, at the size it is on screen, and the
/// file travels under it looking like itself rather than like a row it is
/// nowhere near.
pub fn preview_drag_picture(
    data: &PreviewData<'_>,
    theme: Theme,
) -> impl Fn(&Canvas, f32, f32) + Send + Sync + 'static {
    let icon_chain = data.icon_chain.clone();
    let decoded = data.decoded.cloned();
    let first_row = data.first_row;

    move |canvas: &Canvas, width: f32, height: f32| {
        canvas.clear(Color::TRANSPARENT);
        let stage = Rect::from_wh(width, height);
        draw_preview_stage(
            canvas,
            &theme,
            stage,
            decoded.as_ref(),
            &icon_chain,
            first_row,
        );
    }
}

/// The part of the preview column the previewed thing itself occupies: the
/// panel less the caption band at its foot.
///
/// Public because it is a *target* as well as a layout: pressing on the
/// picture picks the file up (see [`crate::app`]), so the hit test and the
/// drawing have to agree on where the picture is.
pub fn preview_stage_rect(panel: Rect, info_lines: usize) -> Rect {
    let caption = preview_caption_rect(panel, info_lines);
    Rect::from_ltrb(
        panel.left,
        panel.top,
        panel.right,
        (caption.top - PREVIEW_GAP).max(panel.top),
    )
}

/// The previewed thing, drawn into `stage` — a decode when one has landed,
/// and the file's own icon drawn large when one has not.
fn draw_preview_stage(
    canvas: &Canvas,
    theme: &Theme,
    stage: Rect,
    decoded: Option<&otto_kit::preview::Preview>,
    icon_chain: &[String],
    first_row: usize,
) {
    // Nothing the previewer produced may leave the stage. The decoders
    // bound what they return, and each drawing path lays itself out to
    // fit, but the content is a *file's* — an archive with hundreds of
    // long entry names, a text file with no line breaks — and the one
    // place that must not depend on the file being reasonable is the one
    // where overflow would draw over the caption below it.
    canvas.save();
    canvas.clip_rect(stage, None, false);
    match decoded {
        // A decoder that gave up (an unreadable archive, a format with no
        // previewer) is not a blank panel, and neither is one still
        // running: the file's own icon is still true, and drawn large it
        // is a preview of a kind rather than a placeholder apologising
        // for itself.
        Some(otto_kit::preview::Preview::Unavailable { .. }) | None => {
            draw_preview_icon(canvas, stage, icon_chain);
        }
        // A card is a description — a title, a subtitle and a table of
        // facts — and this column already carries every one of those in
        // the caption below. Drawn here it says the same things twice, in
        // the space meant for the thing itself. What the card was carrying
        // that the caption is not is its artwork: cover art, an embedded
        // thumbnail, an mp4's poster frame. That is shown as the picture
        // it is, and a card with none falls back to the file's own icon.
        Some(otto_kit::preview::Preview::Card { hero, .. }) => match hero {
            Some(pixels) => otto_kit::preview::draw(
                canvas,
                stage,
                &otto_kit::preview::Preview::Pixels {
                    pixels: pixels.clone(),
                    pages: 1,
                    page: 1,
                },
                theme,
                first_row,
                otto_kit::preview::Zoom::FIT,
                &|name, size| icons::cached_icon_chain_at(&[name], size, icons::FULL_COLOUR_SIZE),
            ),
            None => draw_preview_icon(canvas, stage, icon_chain),
        },
        Some(preview) => {
            otto_kit::preview::draw(
                canvas,
                stage,
                preview,
                theme,
                first_row,
                // The docked column is a glance, not a viewer: zooming
                // belongs to Quick View, which is the panel the user
                // opened deliberately.
                otto_kit::preview::Zoom::FIT,
                &|name, size| icons::cached_icon_chain_at(&[name], size, icons::FULL_COLOUR_SIZE),
            );
        }
    }
    canvas.restore();
}

/// Room between the preview and the name below it.
const PREVIEW_GAP: f32 = 14.0;
/// How far the caption's side margins hold its text off the column's edges.
const PREVIEW_PAD: f32 = 16.0;
/// The gap under the last line of facts, along the foot of the column. Wider
/// than the side margins: the column ends there, and text that stops as close
/// to the bottom edge as it does to the sides reads as having run out of
/// room rather than as having been placed.
const PREVIEW_FOOT: f32 = 32.0;
const PREVIEW_NAME_H: f32 = 22.0;
const PREVIEW_INFO_H: f32 = 18.0;

/// The band at the foot of the preview column: the name, and the facts under
/// it. Sized for however many facts there are, and pinned to the bottom.
fn preview_caption_rect(panel: Rect, lines: usize) -> Rect {
    let height = PREVIEW_NAME_H + lines as f32 * PREVIEW_INFO_H + PREVIEW_FOOT;
    Rect::from_ltrb(
        panel.left,
        (panel.bottom - height).max(panel.top),
        panel.right,
        panel.bottom,
    )
}

/// The file's name, and the facts about it, along the bottom of the column.
fn draw_preview_caption(canvas: &Canvas, theme: &Theme, panel: Rect, name: &str, info: &[String]) {
    let caption = preview_caption_rect(panel, info.len());
    let room = panel.width() - PREVIEW_PAD * 2.0;

    let name_font = styles::BODY_EMPHASIZED.font();
    Label::new(ellipsize(&name_font, name, room.max(40.0)))
        .with_style(styles::BODY_EMPHASIZED)
        .with_color(theme.text_primary)
        .centered_at(panel.center_x(), caption.top + PREVIEW_NAME_H / 2.0)
        .render(canvas);

    let info_font = styles::CALLOUT.font();
    let mut y = caption.top + PREVIEW_NAME_H + PREVIEW_INFO_H / 2.0;
    for line in info {
        Label::new(ellipsize(&info_font, line, room.max(40.0)))
            .with_style(styles::CALLOUT)
            .with_color(theme.text_tertiary)
            .centered_at(panel.center_x(), y)
            .render(canvas);
        y += PREVIEW_INFO_H;
    }
}

/// The file's icon, as large as the space above the caption allows.
///
/// It stands in for a preview that has not landed or cannot be made. Sized to
/// the stage rather than fixed at the listing's 16 points: this is the one
/// place the browser has room to show what a file *is* at a glance, and a
/// small icon marooned in a wide column reads as something missing.
fn draw_preview_icon(canvas: &Canvas, stage: Rect, icon_chain: &[String]) {
    const MAX: f32 = 128.0;
    let side = stage.width().min(stage.height()).min(MAX);
    if side < 16.0 {
        return;
    }
    let refs: Vec<&str> = icon_chain.iter().map(String::as_str).collect();
    let Some(image) = icons::cached_icon_chain_at(&refs, side as i32, icons::FULL_COLOUR_SIZE)
    else {
        return;
    };
    let dst = Rect::from_xywh(
        stage.center_x() - side / 2.0,
        stage.center_y() - side / 2.0,
        side,
        side,
    );
    canvas.draw_image_rect(&image, None, dst, &Paint::default());
}

fn draw_sidebar(canvas: &Canvas, f: &Frame) {
    let theme = f.theme;

    // The picker has no traffic lights. It is a dialog, not a document
    // window: it is dismissed with Cancel, and a close control beside it
    // would be a second, worse way to say the same thing — one that skips
    // the request's answer.
    if f.action_row.is_none() {
        f.controls
            .apply(
                WindowControls::new()
                    .at(CONTROLS_INSET, CONTROLS_INSET)
                    .with_active(f.focused)
                    .with_dark(is_dark()),
            )
            .render(canvas);
    }

    Label::new(otto_kit::t!("files-places"))
        .with_style(styles::SUBHEADLINE_EMPHASIZED)
        .with_color(theme.text_tertiary)
        .centered_on(20.0, 54.0)
        .render(canvas);

    for (i, place) in f.places.iter().enumerate() {
        let rect = place_rect(i);
        let selected = f.selected_place == Some(i);

        if selected {
            let mut paint = Paint::default();
            paint.set_anti_alias(true);
            paint.set_color(theme.material_selection_focused);
            canvas.draw_rrect(RRect::new_rect_xy(rect, 6.0, 6.0), &paint);
        }

        if let Some(image) = icons::cached_icon_chain(&[place.icon, "folder"], 16) {
            let dst = Rect::from_xywh(rect.left + 8.0, rect.center_y() - 8.0, 16.0, 16.0);
            // The theme's small-size art is a monochrome outline glyph baked
            // at whatever colour the theme authored it in — usually a dark
            // grey that all but vanishes on a dark sidebar. SrcIn recolours
            // it to the theme's own text tone while keeping its alpha, the
            // way a template/symbolic icon is meant to be drawn.
            let mut paint = Paint::default();
            paint.set_color_filter(skia_safe::color_filters::blend(
                theme.text_secondary,
                skia_safe::BlendMode::SrcIn,
            ));
            canvas.draw_image_rect(&image, None, dst, &paint);
        }

        Label::new(&place.label)
            .with_style(styles::BODY_EMPHASIZED)
            .with_color(if selected {
                Color::WHITE
            } else {
                theme.text_primary
            })
            .centered_on(rect.left + 32.0, rect.center_y())
            .render(canvas);
    }
}

fn draw_header(canvas: &Canvas, f: &Frame) {
    let theme = f.theme;
    let mut paint = Paint::default();
    paint.set_anti_alias(true);

    // The header's own ground — the content's, thinned just enough for the
    // frost to show through so the translucency runs across the whole top of
    // the window instead of stopping at the sidebar's edge — is the header
    // layer's background style. The hairline below is drawn here, because it
    // is what separates it from the opaque file area.
    draw_nav_buttons(canvas, f);

    if f.action_row.is_some() {
        // The picker's header is a toolbar, not a title bar: one row, no
        // window title, no item count. What the user needs here is where
        // they are and how to get elsewhere — the same things the macOS open
        // panel puts on its single toolbar row.
        draw_location_button(canvas, f);
    } else {
        // Both drop a step down the text scale while the window is in the
        // background — the title reads as a label rather than as the thing
        // being worked on.
        Label::new(f.title)
            .with_style(styles::TITLE_1_EMPHASIZED)
            .with_color(if f.focused {
                theme.text_primary
            } else {
                theme.text_secondary
            })
            .centered_on(TITLE_X, TITLE_CY)
            .render(canvas);

        Label::new(&f.subtitle)
            .with_style(styles::CALLOUT)
            .with_color(if f.focused {
                theme.text_secondary
            } else {
                theme.text_tertiary
            })
            .centered_on(SIDEBAR_W + CONTENT_PAD, SUBTITLE_CY)
            .render(canvas);
    }

    draw_switcher(canvas, f);

    paint.set_color(theme.fill_tertiary);
    paint.set_stroke_width(1.0);
    canvas.draw_line(
        Point::new(SIDEBAR_W, HEADER_H),
        Point::new(f.width, HEADER_H),
        &paint,
    );
}

/// The picker's location control: a folder icon and the current directory's
/// name in a soft capsule, centred on the toolbar row between the navigation
/// arrows and the view switcher.
///
/// It does not open a menu yet — it says where you are. The popup of ancestor
/// directories the reference layout has is the next thing it grows.
fn draw_location_button(canvas: &Canvas, f: &Frame) {
    let theme = f.theme;
    let cy = TOOLBAR_CY;
    let rect = location_rect(f.width);

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(theme.fill_tertiary);
    canvas.draw_rrect(RRect::new_rect_xy(rect, NAV_RADIUS, NAV_RADIUS), &paint);

    let mut text_x = rect.left + 10.0;
    if let Some(image) = icons::cached_icon_chain(&["folder"], 16) {
        let dst = Rect::from_xywh(text_x, cy - 8.0, 16.0, 16.0);
        canvas.draw_image_rect(&image, None, dst, &Paint::default());
        text_x += 22.0;
    }

    Label::new(f.title)
        .with_style(styles::BODY_EMPHASIZED)
        .with_color(if f.focused {
            theme.text_primary
        } else {
            theme.text_secondary
        })
        .centered_on(text_x, cy)
        .render(canvas);
}

/// The Back/Forward split button: one capsule, a hairline down the middle,
/// each half dimmed when there is nowhere for it to go. Drawn like the view
/// switcher on the other end of the header, so the two read as one family.
fn draw_nav_buttons(canvas: &Canvas, f: &Frame) {
    let theme = f.theme;
    let group = nav_group_rect();

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(theme.fill_tertiary);
    canvas.draw_rrect(RRect::new_rect_xy(group, NAV_RADIUS, NAV_RADIUS), &paint);

    // The held half darkens for as long as it is down. A disabled half is
    // never armed, so nothing here has to check whether it can go anywhere.
    if let Some(pressed) = f.nav_pressed {
        let (rect, back) = match pressed {
            NavButton::Back => (nav_back_rect(), true),
            NavButton::Forward => (nav_forward_rect(), false),
        };
        paint.set_color(theme.fill_secondary);
        canvas.draw_rrect(nav_half_rrect(rect, back), &paint);
    }

    // The divider stops short of the capsule's ends so it reads as a seam
    // between two halves rather than a cut through the shape.
    paint.set_color(theme.fill_secondary);
    paint.set_stroke_width(1.0);
    let split = group.left + NAV_BTN_W;
    canvas.draw_line(
        Point::new(split, group.top + 5.0),
        Point::new(split, group.bottom - 5.0),
        &paint,
    );

    draw_nav_button(
        canvas,
        theme,
        nav_back_rect(),
        "chevron-left",
        f.can_go_back,
        f.nav_pressed == Some(NavButton::Back),
    );
    draw_nav_button(
        canvas,
        theme,
        nav_forward_rect(),
        "chevron-right",
        f.can_go_forward,
        f.nav_pressed == Some(NavButton::Forward),
    );
}

/// One half of the capsule as its own shape: rounded on the capsule's outside
/// edge, square against the seam, so a pressed half fills its corner of the
/// capsule instead of floating inside it as a pill.
fn nav_half_rrect(rect: Rect, back: bool) -> RRect {
    let round = Point::new(NAV_RADIUS, NAV_RADIUS);
    let square = Point::new(0.0, 0.0);
    // Radii run from the top left, clockwise.
    let radii = if back {
        [round, square, square, round]
    } else {
        [square, round, round, square]
    };
    RRect::new_rect_radii(rect, &radii)
}

fn draw_nav_button(
    canvas: &Canvas,
    theme: &Theme,
    rect: Rect,
    icon_name: &str,
    enabled: bool,
    pressed: bool,
) {
    let color = match (enabled, pressed) {
        // Held: the arrow comes forward with the fill under it, so the press
        // reads at a glance rather than as a faint change of ground.
        (true, true) => theme.text_primary,
        (true, false) => theme.text_secondary,
        (false, _) => theme.text_tertiary,
    };
    Icon::new(icon_name)
        .with_size(NAV_ICON_SIZE)
        .with_color(color)
        .at(
            rect.center_x() - NAV_ICON_SIZE / 2.0,
            rect.center_y() - NAV_ICON_SIZE / 2.0,
        )
        .render(canvas);
}

/// A three-segment control: list, icon grid, Miller columns. Glyphs are drawn
/// rather than themed so it stays legible with no icon-theme entry.
fn draw_switcher(canvas: &Canvas, f: &Frame) {
    let theme = f.theme;
    let rect = switcher_rect(f.width);
    let seg = rect.width() / 3.0;
    let mut paint = Paint::default();
    paint.set_anti_alias(true);

    paint.set_color(theme.fill_tertiary);
    canvas.draw_rrect(RRect::new_rect_xy(rect, 7.0, 7.0), &paint);

    let index = SWITCHER_MODES
        .iter()
        .position(|m| *m == f.mode)
        .unwrap_or(0);
    let active = Rect::from_xywh(
        rect.left + index as f32 * seg + 2.0,
        rect.top + 2.0,
        seg - 4.0,
        rect.height() - 4.0,
    );
    paint.set_color(theme.material_popup);
    canvas.draw_rrect(RRect::new_rect_xy(active, 5.0, 5.0), &paint);

    let cy = rect.center_y();
    for (i, mode) in SWITCHER_MODES.iter().enumerate() {
        let cx = rect.left + seg * i as f32 + seg / 2.0;
        paint.set_color(if *mode == f.mode {
            theme.text_primary
        } else {
            theme.text_secondary
        });
        match mode {
            // Three stacked lines.
            ViewMode::List => {
                for r in 0..3 {
                    let y = cy - 4.0 + r as f32 * 4.0;
                    canvas.draw_rrect(
                        RRect::new_rect_xy(Rect::from_xywh(cx - 7.0, y - 1.0, 14.0, 2.0), 1.0, 1.0),
                        &paint,
                    );
                }
            }
            // A 2x2 block of squares.
            ViewMode::Grid => {
                for r in 0..2 {
                    for c in 0..2 {
                        canvas.draw_rrect(
                            RRect::new_rect_xy(
                                Rect::from_xywh(
                                    cx - 6.0 + c as f32 * 7.0,
                                    cy - 6.0 + r as f32 * 7.0,
                                    5.0,
                                    5.0,
                                ),
                                1.2,
                                1.2,
                            ),
                            &paint,
                        );
                    }
                }
            }
            // Three vertical bars.
            ViewMode::Columns => {
                for c in 0..3 {
                    let x = cx - 7.5 + c as f32 * 5.5;
                    canvas.draw_rrect(
                        RRect::new_rect_xy(Rect::from_xywh(x, cy - 5.0, 4.0, 10.0), 1.0, 1.0),
                        &paint,
                    );
                }
            }
        }
    }
}

/// The icon grid. The cell drawing is deliberately free of browser state — it
/// takes an entry, a rect and whether it is selected — so a desktop surface can
/// call it with its own rects.
fn draw_grid(canvas: &Canvas, f: &Frame) {
    let theme = f.theme;
    let area = content_viewport(f.width, f.height, ViewMode::Grid);
    let Some(pane) = f.panes.last() else { return };

    canvas.save();
    canvas.clip_rect(area, ClipOp::Intersect, true);

    if let Some(error) = pane.error {
        draw_centered(canvas, area, error, theme.text_secondary);
    } else if pane.loading {
        draw_centered(
            canvas,
            area,
            otto_kit::t!("files-loading"),
            theme.text_tertiary,
        );
    } else if pane.entries.is_empty() {
        draw_centered(
            canvas,
            area,
            otto_kit::t!("files-folder-empty"),
            theme.text_tertiary,
        );
    } else {
        // Only the rows of cells the viewport is asking for are drawn.
        let band = pane.band(area);
        for index in grid_visible_range(area, pane.entries.len(), pane.scroll, band) {
            let cell = grid_cell_rect(area, index, pane.scroll);
            draw_grid_cell(
                canvas,
                theme,
                pane.entries[index],
                cell,
                pane.is_selected(index),
                f.renaming == Some((f.panes.len() - 1, index)),
                f.thumbnail(pane.entries[index]),
            );
        }
    }

    if let Some(band) = f.marquee {
        draw_marquee(canvas, theme, band);
    }

    canvas.restore();
    pane.draw_scrollbar(canvas, theme);
}

/// The rubber band: a wash of the accent over what it covers, with a hairline
/// edge so a band dragged out over a dark wallpaper still reads as a shape.
///
/// Drawn inside the grid's clip and after the cells, so it tints the icons it
/// has caught rather than hiding behind them.
pub fn draw_marquee(canvas: &Canvas, theme: &Theme, band: Rect) {
    let accent = accent(theme);
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(accent.with_a(40));
    canvas.draw_rrect(RRect::new_rect_xy(band, 2.0, 2.0), &paint);

    paint.set_color(accent.with_a(140));
    paint.set_style(skia_safe::paint::Style::Stroke);
    paint.set_stroke_width(1.0);
    canvas.draw_rrect(
        RRect::new_rect_xy(band.with_inset((0.5, 0.5)), 2.0, 2.0),
        &paint,
    );
}

/// One grid cell: icon over a centred, wrapped-to-two-lines name.
///
/// `renaming` suppresses the caption — and the pill behind it — while the
/// host's text field sits over it, the way a list row does.
pub fn draw_grid_cell(
    canvas: &Canvas,
    theme: &Theme,
    entry: &Entry,
    cell: Rect,
    selected: bool,
    renaming: bool,
    thumb: Option<&skia_safe::Image>,
) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);

    let icon_top = cell.top + 8.0;
    // The optical centre of the caption's first line, not its top: that is what
    // `Label::centered_at` wants, and the pill is measured off the same point.
    let label_center_y = icon_top + GRID_ICON + GRID_LABEL_GAP;

    if selected {
        // The highlight hugs the icon, not the cell — a cell-wide wash reads as
        // a block of colour rather than as one picked-out file.
        paint.set_color(theme.material_selection_focused);
        paint.set_alpha(60);
        canvas.draw_rrect(
            RRect::new_rect_xy(grid_icon_highlight_rect(cell, icon_top), 8.0, 8.0),
            &paint,
        );
    }

    let box_rect = Rect::from_xywh(
        cell.center_x() - GRID_ICON / 2.0,
        icon_top,
        GRID_ICON,
        GRID_ICON,
    );
    if let Some(image) = thumb {
        draw_thumbnail(canvas, image, box_rect, false);
    } else {
        let chain = entry.icon_chain();
        let refs: Vec<&str> = chain.iter().map(String::as_str).collect();
        if let Some(image) =
            icons::cached_icon_chain_at(&refs, GRID_ICON as i32, icons::FULL_COLOUR_SIZE)
        {
            canvas.draw_image_rect(&image, None, box_rect, &Paint::default());
        }
    }

    // Two lines at most, the second elided — a long name must not push the
    // grid out of alignment.
    let (first, second) = split_label(&entry.name, 13);
    let text_color = if selected {
        Color::WHITE
    } else {
        theme.text_primary
    };

    if selected && !renaming {
        let font = styles::CALLOUT_EMPHASIZED.font();
        let text_w = font
            .measure_str(&first, None)
            .0
            .max(font.measure_str(&second, None).0);
        let width = (text_w + 12.0).min(cell.width() - 4.0);
        let lines = if second.is_empty() {
            0.0
        } else {
            GRID_LABEL_LINE
        };
        let height = lines + GRID_LABEL_INSET * 2.0;
        paint.set_color(theme.material_selection_focused);
        paint.set_alpha(255);
        canvas.draw_rrect(
            RRect::new_rect_xy(
                Rect::from_xywh(
                    cell.center_x() - width / 2.0,
                    label_center_y - GRID_LABEL_INSET,
                    width,
                    height,
                ),
                5.0,
                5.0,
            ),
            &paint,
        );
    }

    for (line, offset) in [(first.as_str(), 0.0), (second.as_str(), GRID_LABEL_LINE)] {
        if renaming || line.is_empty() {
            continue;
        }
        Label::new(line)
            .with_style(styles::CALLOUT_EMPHASIZED)
            .with_color(text_color)
            .centered_at(cell.center_x(), label_center_y + offset)
            .render(canvas);
    }
}

/// Split a name across at most two lines of `per_line` characters, eliding the
/// tail. Character-count based rather than measured — good enough for a fixed
/// cell, and it keeps this callable without a font.
fn split_label(name: &str, per_line: usize) -> (String, String) {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= per_line {
        return (name.to_string(), String::new());
    }
    let first: String = chars[..per_line].iter().collect();
    let rest = &chars[per_line..];
    if rest.len() <= per_line {
        return (first, rest.iter().collect());
    }
    let second: String = rest[..per_line.saturating_sub(1)].iter().collect();
    (first, format!("{second}…"))
}

/// Truncate `text` with a trailing ellipsis so it measures no wider than
/// `max_width` under `font` — a long file name must stop before it runs into
/// the next column rather than drawing straight through it.
///
/// The toolkit's, re-exported: the preview panel crops archive listings by the
/// same rule, and two implementations of "how wide is too wide" would
/// eventually disagree.
pub use otto_kit::typography::ellipsize;

/// The width the Name column needs to show every given name in full — what a
/// double-click on the Name/Size divider fits it to, the way Finder does.
pub fn widest_name(names: impl Iterator<Item = impl AsRef<str>>) -> f32 {
    let font = styles::BODY_MEDIUM.font();
    names.fold(0.0f32, |max, name| {
        max.max(font.measure_str(name.as_ref(), None).0)
    })
}

/// The shared Miller pane width that fits `longest_name` without truncating
/// — a double-click on a pane's right edge, the way the Name/Size divider
/// fits in list view. `has_dirs` reserves room for the disclosure chevron,
/// since it sits at the end of every directory row in the pane.
pub fn fit_miller_width(longest_name: f32, has_dirs: bool) -> f32 {
    let trailing = if has_dirs { 24.0 } else { 8.0 };
    let content = 14.0 + ICON_SIZE + 8.0 + longest_name + trailing + 12.0;
    content.clamp(MILLER_MIN_W, MILLER_MAX_W)
}

fn draw_column_strip(canvas: &Canvas, f: &Frame) {
    let theme = f.theme;
    let (size_x, kind_x, modified_x) = column_edges(f.width, f.list_columns);
    // Optical centre of the column strip, not a baseline.
    let cy = HEADER_H + COLUMNS_H / 2.0;

    for (key, x) in [
        (SortKey::Name, SIDEBAR_W + CONTENT_PAD),
        (SortKey::Size, size_x),
        (SortKey::Kind, kind_x),
        (SortKey::Modified, modified_x),
    ] {
        let active = f.sort == key;
        Label::new(key.label())
            .with_style(styles::SUBHEADLINE)
            .with_color(if active {
                theme.text_primary
            } else {
                theme.text_tertiary
            })
            .centered_on(x, cy)
            .render(canvas);

        if active {
            let mut paint = Paint::default();
            paint.set_anti_alias(true);
            paint.set_color(theme.text_secondary);
            let label_w = styles::SUBHEADLINE.font().measure_str(key.label(), None).0;
            let cx = x + label_w + 8.0;
            let dir = if f.ascending { -1.0 } else { 1.0 };
            let mut builder = PathBuilder::new();
            builder.move_to(Point::new(cx - 3.5, cy - 2.0 * dir));
            builder.line_to(Point::new(cx + 3.5, cy - 2.0 * dir));
            builder.line_to(Point::new(cx, cy + 2.5 * dir));
            builder.close();
            canvas.draw_path(&builder.detach(), &paint);
        }
    }

    let mut paint = Paint::default();
    paint.set_color(theme.fill_tertiary);
    paint.set_stroke_width(1.0);
    canvas.draw_line(
        Point::new(SIDEBAR_W, HEADER_H + COLUMNS_H),
        Point::new(f.width, HEADER_H + COLUMNS_H),
        &paint,
    );
}

fn draw_list(canvas: &Canvas, f: &Frame) {
    let theme = f.theme;
    let viewport = content_viewport(f.width, f.height, ViewMode::List);
    let depth = f.panes.len().saturating_sub(1);
    let Some(pane) = f.panes.last() else { return };

    canvas.save();
    canvas.clip_rect(viewport, ClipOp::Intersect, true);

    if let Some(error) = pane.error {
        draw_centered(canvas, viewport, error, theme.text_secondary);
        canvas.restore();
        return;
    }
    if pane.loading {
        draw_centered(
            canvas,
            viewport,
            otto_kit::t!("files-loading"),
            theme.text_tertiary,
        );
        canvas.restore();
        return;
    }
    if pane.entries.is_empty() {
        draw_centered(
            canvas,
            viewport,
            &otto_kit::t_owned!("files-folder-empty"),
            theme.text_tertiary,
        );
        canvas.restore();
        return;
    }

    let (size_x, kind_x, modified_x) = column_edges(f.width, f.list_columns);

    // Only rows the viewport is asking for are laid out or drawn — the cost of
    // a frame must not grow with the size of the directory, and measuring and
    // shaping the text of a row nobody can see is most of what that would buy.
    let strip = RowStrip::list(f.width, pane.entries.len(), pane.scroll);
    let band = pane.band(viewport);

    for index in strip.visible(band) {
        let entry = pane.entries[index];
        let rect = strip.rect(index);
        let selected = pane.is_selected(index);

        draw_row_background(
            canvas,
            theme,
            rect,
            selected,
            RunEnds::of_pane(pane, index),
            index,
        );
        if pane.cursor == Some(index) && !selected {
            draw_cursor_ring(canvas, theme, rect, 8.0);
        }

        let (text_color, detail_color) = row_colors(theme, selected);
        let cut = f.cut.contains(&entry.path);
        let centre = rect.center_y();

        draw_entry_icon(
            canvas,
            entry,
            rect.left + CONTENT_PAD,
            rect.center_y(),
            cut,
            f.thumbnail(entry),
        );

        if f.renaming != Some((depth, index)) {
            let name_x = rect.left + CONTENT_PAD + ICON_SIZE + 10.0;
            let name_font = styles::BODY_MEDIUM.font();
            let name = ellipsize(&name_font, &entry.name, size_x - 12.0 - name_x);
            Label::new(name)
                .with_style(styles::BODY_MEDIUM)
                .with_color(if cut {
                    dim_color(text_color)
                } else {
                    text_color
                })
                .centered_on(name_x, centre)
                .render(canvas);
        }

        let size_text = if entry.is_dir {
            "--".to_string()
        } else {
            entry.size.map(model::format_size).unwrap_or_default()
        };
        for (text, x) in [
            (size_text, size_x),
            (entry.kind_label().to_string(), kind_x),
            (
                entry.modified.map(model::format_time).unwrap_or_default(),
                modified_x,
            ),
        ] {
            Label::new(&text)
                .with_style(styles::SUBHEADLINE)
                .with_color(detail_color)
                .centered_on(x, centre)
                .render(canvas);
        }
    }

    canvas.restore();
    pane.draw_scrollbar(canvas, theme);
}

/// Miller columns: the chrome over the stack.
///
/// The columns themselves — their ground, and every row in them — are layers
/// the engine composites under this canvas; see [`crate::scene`]. What is left
/// here is what sits *over* them and is cheap enough not to be worth a layer
/// of its own: each column's scrollbar, the hairline down its trailing edge,
/// and the stack's own horizontal bar.
fn draw_miller(canvas: &Canvas, f: &Frame) {
    let theme = f.theme;
    let viewport = content_viewport(f.width, f.height, ViewMode::Columns);

    canvas.save();
    canvas.clip_rect(viewport, ClipOp::Intersect, true);

    let mut divider = Paint::default();
    divider.set_color(theme.fill_tertiary);
    divider.set_stroke_width(1.0);

    let trailing_edges = (0..f.panes.len())
        .map(|depth| miller_pane_rect(depth, f.height, f.pan, f.miller_w))
        .chain(
            f.preview
                .is_some()
                .then(|| preview_pane_rect(f.panes.len(), f.height, f.pan, f.miller_w)),
        );

    for (depth, full) in trailing_edges.enumerate() {
        if full.right < viewport.left || full.left > viewport.right {
            continue;
        }
        // The bar belongs to the column, but it is drawn from here so it lies
        // over the column's content rather than being recorded into it — a
        // scroll then moves the bar without the column re-recording anything.
        // Unless the columns are in their own surfaces, in which case each
        // draws its own bar into its own buffer: those surfaces sit over this
        // canvas, so a bar drawn here would be hidden under them anyway, and
        // would be stale besides.
        if !crate::pane_surfaces::enabled() {
            if let Some(pane) = f.panes.get(depth) {
                pane.draw_scrollbar(canvas, theme);
            }
        }
        canvas.draw_line(
            Point::new(full.right, viewport.top),
            Point::new(full.right, viewport.bottom),
            &divider,
        );
    }

    // The stack's own bar, along the bottom of every pane: the panes scroll
    // vertically on their own bars, the stack scrolls sideways on this one.
    // Drawn last so it lies over the pane dividers rather than under them.
    // Unless it has a surface of its own over the columns — drawn here it
    // would be covered by them. See [`crate::pane_surfaces`].
    if !crate::pane_surfaces::enabled() {
        if let Some(state) = f.pan_bar {
            ScrollRenderer::draw(canvas, state, theme, |_, _| {});
        }
    }

    canvas.restore();
}

/// Where a selected row sits within its run of selected rows. Rows abut, so a
/// run has to be drawn as one shape: only its first row is rounded on top and
/// only its last is rounded on the bottom, and the rows between are square at
/// both ends. Rounding every row instead would draw a stack of separate pills
/// with pinched waists where they meet.
#[derive(Debug, Clone, Copy)]
pub struct RunEnds {
    pub first: bool,
    pub last: bool,
}

impl RunEnds {
    /// Read off the neighbours of `index`. A row at either end of the listing
    /// is an end of its run by definition.
    pub fn of_pane(pane: &PaneData<'_>, index: usize) -> Self {
        Self {
            first: index == 0 || !pane.is_selected(index - 1),
            last: !pane.is_selected(index + 1),
        }
    }
}

/// One row's share of a selection run, inset horizontally by `inset`.
pub fn draw_selection_run(canvas: &Canvas, rect: Rect, color: Color, inset: f32, ends: RunEnds) {
    const R: f32 = 6.0;
    let top = if ends.first { R } else { 0.0 };
    let bottom = if ends.last { R } else { 0.0 };
    let radii = [
        Point::new(top, top),
        Point::new(top, top),
        Point::new(bottom, bottom),
        Point::new(bottom, bottom),
    ];
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(color);
    canvas.draw_rrect(
        RRect::new_rect_radii(
            Rect::from_ltrb(rect.left + inset, rect.top, rect.right - inset, rect.bottom),
            &radii,
        ),
        &paint,
    );
}

fn draw_row_background(
    canvas: &Canvas,
    theme: &Theme,
    rect: Rect,
    selected: bool,
    ends: RunEnds,
    index: usize,
) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);

    if selected {
        // Full row height: the highlight of one row meets the next with no
        // gap, so a run of selected rows reads as one block.
        draw_selection_run(canvas, rect, theme.material_selection_focused, 8.0, ends);
    } else if index % 2 == 1 {
        // A faint stripe is what makes a wide row scannable across to the date.
        // The theme colour is already the right weight — raising its alpha here
        // would make a banded grey list rather than a hint.
        paint.set_color(theme.fill_quaternary);
        canvas.draw_rect(rect, &paint);
    }
}

/// The opaque ground everything right of the sidebar sits on: white in light
/// mode, near-black in dark.
///
/// Not taken from the theme's materials, which are all translucent by design
/// for the compositor's blur path. This surface deliberately is not.
pub fn content_ground() -> Color {
    if matches!(current_color_scheme(), ColorScheme::Dark) {
        Color::from_argb(0xFF, 0x1C, 0x1C, 0x1E)
    } else {
        Color::WHITE
    }
}

/// Styling for an in-place rename's text field.
///
/// [`TextInputStyle::with_theme`]'s default background is a near-transparent
/// fill meant for fields that float over a material — barely visible against
/// a row that already sits on [`content_ground`]. A rename field wants to
/// read as solid paper, the way Finder's does, with the focus ring alone
/// marking it editable.
pub fn rename_field_style(theme: Theme) -> TextInputStyle {
    let mut style = TextInputStyle::with_theme(theme);
    style.background = content_ground();
    style
}

/// The picker's name field. The row underneath draws the box and its border,
/// so the input itself paints no ground of its own — two rounded rects on
/// top of each other, one of them a hair off, is exactly the sort of seam a
/// dialog gets judged on.
pub fn save_field_style(theme: Theme) -> TextInputStyle {
    let mut style = TextInputStyle::with_theme(theme);
    style.background = Color::TRANSPARENT;
    style
}

/// The header band: [`content_ground`] with a little alpha taken out of it.
/// Much more opaque than the sidebar — the title sits here and the file list
/// starts a hairline below, so the backdrop may only be hinted at.
/// The same colour, made opaque.
///
/// The panel materials are translucent because they are meant to sit over the
/// compositor's blur. An unfocused window has no blur behind it — see
/// `FilesApp::on_app_ready` — and a translucent material over the bare desktop
/// is not a softer version of itself: it is the wallpaper showing through,
/// which drags the contrast of everything drawn on top down with it. Without
/// the blur the materials are filled in instead.
pub fn opaque(color: Color) -> Color {
    Color::from_argb(0xFF, color.r(), color.g(), color.b())
}

pub fn header_material() -> Color {
    if matches!(current_color_scheme(), ColorScheme::Dark) {
        Color::from_argb(0xE6, 0x1C, 0x1C, 0x1E)
    } else {
        Color::from_argb(0xE6, 0xFF, 0xFF, 0xFF)
    }
}

pub fn row_colors(theme: &Theme, selected: bool) -> (Color, Color) {
    if selected {
        (Color::WHITE, Color::from_argb(200, 255, 255, 255))
    } else {
        (theme.text_primary, theme.text_secondary)
    }
}

fn draw_entry_icon(
    canvas: &Canvas,
    entry: &Entry,
    left: f32,
    center_y: f32,
    cut: bool,
    thumb: Option<&skia_safe::Image>,
) {
    let box_rect = Rect::from_xywh(left, center_y - ICON_SIZE / 2.0, ICON_SIZE, ICON_SIZE);
    if let Some(image) = thumb {
        draw_thumbnail(canvas, image, box_rect, cut);
        return;
    }

    let chain = entry.icon_chain();
    let refs: Vec<&str> = chain.iter().map(String::as_str).collect();
    if let Some(image) =
        icons::cached_icon_chain_at(&refs, ICON_SIZE as i32, icons::FULL_COLOUR_SIZE)
    {
        let dst = box_rect;
        let mut paint = Paint::default();
        if cut {
            // A cut entry is still there — it does not move until the paste —
            // so it is dimmed rather than hidden.
            paint.set_alpha(110);
        }
        canvas.draw_image_rect(&image, None, dst, &paint);
    }
}

/// A thumbnail, fitted into the box an icon would have had.
///
/// Fitted rather than filled: a thumbnail is the file, and cropping it to a
/// square would be showing the user the middle of their photograph and calling
/// it the photograph. The picture keeps its own proportions and is centred in
/// the box, so a panorama and a portrait both sit on the same baseline as the
/// icons around them.
///
/// The hairline matters more than it looks. A photograph with a white sky and
/// no border does not end anywhere — it bleeds into the window and stops
/// reading as an object in a grid — and one drawn hard against a dark
/// background reads as a hole. A single low-contrast edge is enough to close
/// it, and is cheaper than the drop shadow the same problem is usually solved
/// with.
pub(crate) fn draw_thumbnail(canvas: &Canvas, image: &skia_safe::Image, box_rect: Rect, cut: bool) {
    let (w, h) = (image.width() as f32, image.height() as f32);
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    // Never enlarged past its own pixels: a 32-pixel favicon blown up to fill
    // a grid cell is a blurry square where a crisp icon would have been.
    let scale = (box_rect.width() / w).min(box_rect.height() / h).min(1.0);
    let (dst_w, dst_h) = (w * scale, h * scale);
    let dst = Rect::from_xywh(
        box_rect.center_x() - dst_w / 2.0,
        // Sat on the box's bottom edge rather than its middle, so a row of
        // mixed shapes shares a baseline the way the icons they replace do.
        box_rect.bottom - dst_h,
        dst_w,
        dst_h,
    );

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    if cut {
        // Dimmed exactly as the icon it stands in for would be.
        paint.set_alpha(110);
    }
    canvas.draw_image_rect(image, None, dst, &paint);

    let mut edge = Paint::default();
    edge.set_anti_alias(true);
    edge.set_style(skia_safe::paint::Style::Stroke);
    edge.set_stroke_width(1.0);
    edge.set_color(skia_safe::Color::from_argb(
        if cut { 20 } else { 46 },
        0,
        0,
        0,
    ));
    canvas.draw_rect(dst.with_inset((0.5, 0.5)), &edge);
}

/// The keyboard cursor when it is not itself part of the selection.
pub fn draw_cursor_ring(canvas: &Canvas, theme: &Theme, rect: Rect, inset: f32) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_style(skia_safe::paint::Style::Stroke);
    paint.set_stroke_width(1.5);
    paint.set_color(theme.material_selection_focused);
    canvas.draw_rrect(
        RRect::new_rect_xy(
            Rect::from_ltrb(
                rect.left + inset,
                rect.top + 1.0,
                rect.right - inset,
                rect.bottom - 1.0,
            ),
            6.0,
            6.0,
        ),
        &paint,
    );
}

/// Half-strength, for entries marked by a pending cut.
pub fn dim_color(color: Color) -> Color {
    Color::from_argb(color.a() / 2, color.r(), color.g(), color.b())
}

fn draw_centered(canvas: &Canvas, area: Rect, text: &str, color: Color) {
    Label::new(text)
        .with_style(styles::BODY)
        .with_color(color)
        .centered_at(area.center_x(), area.center_y())
        .render(canvas);
}

// ---------------------------------------------------------------------------
// Get Info modal
// ---------------------------------------------------------------------------

/// The Get Info panel's size, in logical points. It is a window in its own
/// right, so this is the size that window is created at — and it does not
/// resize: the layout inside it is fixed.
pub const INFO_W: f32 = 380.0;
/// Tall enough for every detail row a symlink can produce *above* the
/// permissions rule, which is fixed to the bottom of the panel. The rows are
/// body text now and step 23 points, so a window sized for the old caption
/// text ran the last of them into the section header.
pub const INFO_H: f32 = 520.0;

/// The strip the panel is dragged by: its top edge, beside the close dot.
///
/// It carries no title and draws nothing of its own — the icon and the name
/// below it already say what the panel is about — so this is a hit target,
/// not a titlebar. Everything below it is content, and dragging from there
/// would fight the permission checkboxes.
pub fn info_titlebar_rect(sheet: Rect) -> Rect {
    Rect::from_ltrb(sheet.left, sheet.top, sheet.right, sheet.top + 40.0)
}

/// The close button, top-left of the sheet, matching the window's own controls.
pub fn info_close_rect(sheet: Rect) -> Rect {
    Rect::from_xywh(sheet.left + 14.0, sheet.top + 14.0, 12.0, 12.0)
}

/// Where the permissions grid starts inside the sheet: the baseline of the
/// section, with the column headers above it and the rows below.
fn perm_origin(sheet: Rect) -> (f32, f32) {
    (sheet.left + 24.0, sheet.bottom - 118.0)
}

/// Narrowest the grid is ever drawn: the English metrics, which every wider
/// language grows from rather than being squeezed into.
const PERM_COL_W: f32 = 62.0;
const PERM_ROW_H: f32 = 26.0;
const PERM_BOX: f32 = 15.0;
/// Left edge of the first checkbox column, from the grid origin.
const PERM_COL_X: f32 = 96.0;
/// Drop from the origin to the first checkbox row. The column headers live in
/// the gap; without it they are drawn underneath the first row of boxes.
const PERM_ROWS_TOP: f32 = 14.0;
/// Clear space kept between a label and whatever is next to it, so a column
/// that only just fits still reads as two things rather than one.
const PERM_GAP: f32 = 10.0;

/// The widest field name in the Get Info panel, plus its clear space.
///
/// Every row is measured, not just the ones a particular file has, so the
/// values line up down the panel whether or not the file is a symlink.
fn info_label_width() -> f32 {
    let font = styles::BODY.font();
    [
        "files-info-where",
        "files-info-kind",
        "files-info-modified",
        "files-info-created",
        "files-info-accessed",
        "files-info-owner",
        "files-info-links-to",
    ]
    .iter()
    .map(|key| font.measure_str(otto_kit::t!(key), None).0 + PERM_GAP)
    .fold(0.0f32, f32::max)
}

/// The grid's horizontal metrics for the language it is being drawn in: the
/// left edge of the first checkbox column, and the pitch between columns.
///
/// Measured rather than fixed. "Read" is half the width of the column it is
/// centred over; "Esecuzione" and "Выполнение" are wider than it, and a label
/// centred over a column narrower than itself reaches into its neighbours —
/// the header row stops reading as three headings over three columns and
/// starts reading as one run of text. The same for the row labels: "Everyone"
/// clears the 96pt gap easily, "Proprietario" less so.
///
/// Both are floors, never ceilings, so English is drawn exactly as it was
/// designed and only a language that needs more room gets it. The columns are
/// then hung off the panel's right margin rather than off the label column, so
/// they cannot run out of the sheet however wide the headers get — a checkbox
/// drawn outside the panel cannot be clicked.
fn perm_metrics(sheet: Rect) -> (f32, f32) {
    let widest = |font: &skia_safe::Font, keys: [&str; 3]| {
        keys.iter()
            .map(|key| font.measure_str(otto_kit::t!(key), None).0)
            .fold(0.0f32, f32::max)
    };

    let labels = widest(
        &styles::BODY.font(),
        [
            "files-perm-owner",
            "files-perm-group",
            "files-perm-everyone",
        ],
    );
    let headers = widest(
        &styles::CALLOUT.font(),
        ["files-perm-read", "files-perm-write", "files-perm-exec"],
    );

    let label_gap = (labels + PERM_GAP).max(PERM_COL_X);
    let col_w = (headers + PERM_GAP).max(PERM_COL_W);

    // The group is hung off the right margin — the same edge the octal above
    // it ends on — rather than off the label column, so the grid lines up with
    // the panel instead of trailing away into empty space partway across it.
    // The left edge is the floor: a language whose row labels are wide enough
    // to reach it pushes the columns back out to the right, which costs the
    // symmetry but keeps the label readable.
    let last = sheet.right - 24.0 - PERM_BOX;
    let first = (last - 2.0 * col_w).max(sheet.left + 24.0 + label_gap);
    (first, col_w)
}

/// The checkbox rect for `who` (0 owner, 1 group, 2 other) and `what`
/// (0 read, 1 write, 2 execute).
pub fn perm_box_rect(sheet: Rect, who: usize, what: usize) -> Rect {
    let (_, oy) = perm_origin(sheet);
    let (first, col_w) = perm_metrics(sheet);
    Rect::from_xywh(
        first + what as f32 * col_w,
        oy + PERM_ROWS_TOP + who as f32 * PERM_ROW_H - PERM_BOX / 2.0 + 1.0,
        PERM_BOX,
        PERM_BOX,
    )
}

/// The permission cell under `(x, y)`, if any. Returns `(who, what)`.
pub fn perm_box_at(sheet: Rect, x: f32, y: f32) -> Option<(usize, usize)> {
    for who in 0..3 {
        for what in 0..3 {
            // A slightly generous target: a 15pt box is small for a pointer.
            if perm_box_rect(sheet, who, what)
                .with_outset((4.0, 4.0))
                .contains(Point::new(x, y))
            {
                return Some((who, what));
            }
        }
    }
    None
}

/// Draw the Quick View panel over a dimmed window.
///
/// The panel is drawn into this window's own surface rather than into a
/// separate previewer's, which is what lets it grow out of the row the user
/// pressed Space on: [`quickview_anchor`] is already in these coordinates.
/// The content itself comes from [`otto_kit::preview`], canvas-pure and shared
/// with every other file view.
///
/// `resting` is where the panel comes to rest. It is passed in rather than
/// computed here because it is not always the window's middle: a panel centred
/// on the display rests somewhere only the caller knows, and the drawing and
/// the surface it is drawn into must agree about it or the card is painted at
/// one size inside a surface of another.
/// Quick View's close button: a round dot in the panel's top-*right*.
///
/// Right, not left, because it is not a titlebar control. The card has no
/// titlebar and no other controls to sit in a group with, and the browser's
/// own traffic lights are already down the window's left edge — a second dot
/// in the same corner would read as one of them.
pub fn quickview_close_rect(panel: Rect) -> Rect {
    const D: f32 = 17.0;
    const INSET: f32 = 10.0;
    let strip = quickview_titlebar_rect(panel);
    Rect::from_xywh(panel.right - INSET - D, strip.center_y() - D / 2.0, D, D)
}

/// The panel's title strip: the file's name and the close button.
///
/// Chrome, not content. The preview is drawn below it, so a close button in
/// the corner is never sitting on top of the thing being previewed — which
/// is both hard to see against a busy image and easy to mis-click.
pub fn quickview_titlebar_rect(panel: Rect) -> Rect {
    Rect::from_ltrb(
        panel.left,
        panel.top,
        panel.right,
        panel.top + crate::quickview::TITLEBAR_H,
    )
}

/// What is left of the panel for the preview itself.
///
/// Inset on every side, not just below the title strip: the card reads as a
/// mount around the content rather than a window whose content is jammed
/// against the glass, and an image that reaches the edge no longer collides
/// with the panel's own rounded corners.
///
/// Narrow, because it is not the only inset the content gets: the preview
/// pads itself as well ([`otto_kit::preview::PADDING`], and less than that
/// for a picture). This one only has to clear the corners and the border.
pub fn quickview_content_rect(panel: Rect) -> Rect {
    const INSET: f32 = 4.0;
    Rect::from_ltrb(
        panel.left + INSET,
        panel.top + crate::quickview::TITLEBAR_H + INSET,
        panel.right - INSET,
        panel.bottom - INSET,
    )
}

/// Paint the close button. Split out so the panel's own draw stays readable.
fn draw_quickview_close(canvas: &Canvas, theme: &Theme, panel: Rect, hovered: bool, opacity: f32) {
    let close = quickview_close_rect(panel);
    let centre = Point::new(close.center_x(), close.center_y());

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    // A tint of the theme's own fill rather than the traffic lights' red:
    // this dismisses a preview, it does not close a window, and borrowing
    // the window-close colour would overstate it.
    paint.set_color(fade(
        if hovered {
            theme.fill_primary
        } else {
            theme.fill_secondary
        },
        opacity,
    ));
    canvas.draw_circle(centre, close.width() / 2.0, &paint);

    // The glyph is always drawn, unlike the traffic lights' reveal-on-hover:
    // a bare dot in a corner with no group around it does not say "close".
    let mut glyph = Paint::default();
    glyph.set_anti_alias(true);
    glyph.set_style(skia_safe::paint::Style::Stroke);
    glyph.set_stroke_width(1.5);
    glyph.set_stroke_cap(skia_safe::PaintCap::Round);
    glyph.set_color(fade(theme.text_secondary, opacity));
    let r = close.width() * 0.24;
    canvas.draw_line(
        (centre.x - r, centre.y - r),
        (centre.x + r, centre.y + r),
        &glyph,
    );
    canvas.draw_line(
        (centre.x + r, centre.y - r),
        (centre.x - r, centre.y + r),
        &glyph,
    );
}

/// How much of the panel's chrome is drawn at the size the panel is now.
///
/// The strip and the close dot are absolute: 30 points tall and 17 points
/// across whatever the card measures. At rest that is chrome around a
/// preview; on the first frames of the entrance the card is the file's own
/// row, and the same 30 points are the entire card — so what the eye catches
/// is an enormous titlebar with a huge dot in it, growing, rather than a
/// preview opening. The card's *content* has no such floor: it is laid out
/// into whatever box it is given and simply arrives small.
///
/// So the chrome fades in as the card grows into a size that can carry it,
/// and both ends of the ramp are reached well below [`PANEL_MIN`] — a panel
/// at rest, however small the window, always has its titlebar.
fn quickview_chrome_opacity(panel: Rect) -> f32 {
    // Two strips tall before any of it shows, four before all of it does.
    const HEIGHT_FROM: f32 = crate::quickview::TITLEBAR_H * 2.0;
    const HEIGHT_TO: f32 = crate::quickview::TITLEBAR_H * 4.0;
    // Width matters too: a card wide enough for the dot but not for a name
    // is a card the strip has nothing to say in.
    const WIDTH_FROM: f32 = 140.0;
    const WIDTH_TO: f32 = 240.0;

    fn ramp(value: f32, from: f32, to: f32) -> f32 {
        ((value - from) / (to - from)).clamp(0.0, 1.0)
    }

    ramp(panel.height(), HEIGHT_FROM, HEIGHT_TO).min(ramp(panel.width(), WIDTH_FROM, WIDTH_TO))
}

/// A colour at a fraction of its own alpha. Scaled, never replaced: the
/// theme's fills are translucent by design and forcing one opaque to fade it
/// in would make it darker at the end than it is at rest.
fn fade(color: Color, opacity: f32) -> Color {
    Color::from_argb(
        (color.a() as f32 * opacity) as u8,
        color.r(),
        color.g(),
        color.b(),
    )
}

pub fn draw_quickview(
    canvas: &Canvas,
    f: &Frame,
    session: &crate::quickview::Session,
    resting: Rect,
) {
    let panel = session.panel(resting);

    let mut paint = Paint::default();
    paint.set_anti_alias(true);

    // No dim behind it, and no fade on it. The panel is a window laid over the
    // browser, not a modal sheet the browser is answering: it carries its own
    // shadow, and darkening everything else to make it read as focused would
    // say the opposite of what it is. It arrives opaque, so what the eye
    // follows is one card moving rather than a shape resolving out of nothing.

    // No shadow. A Gaussian across the whole card, recorded again on every
    // frame of a zoom that resizes the surface as it goes, is the most
    // expensive thing this panel was doing — and it bought a soft edge nobody
    // asked for. A hairline is enough to part the card from what is behind it.
    paint.set_color(otto_kit::preview::background(f.theme));
    canvas.draw_rrect(RRect::new_rect_xy(panel, 12.0, 12.0), &paint);

    let mut edge = Paint::default();
    edge.set_anti_alias(true);
    edge.set_style(skia_safe::paint::Style::Stroke);
    edge.set_stroke_width(1.0);
    edge.set_color(f.theme.fill_tertiary);
    canvas.draw_rrect(RRect::new_rect_xy(panel, 12.0, 12.0), &edge);

    // The chrome, but only once the card is big enough to be a card with
    // chrome on it rather than a titlebar with a card behind it. The content
    // keeps its place either way — the strip's room is reserved whether or
    // not the strip is drawn — so what appears mid-entrance appears where it
    // will rest, and nothing under it moves when it does.
    let chrome = quickview_chrome_opacity(panel);
    if chrome > 0.0 {
        // The title strip first, so the content's clip can exclude it.
        let strip = quickview_titlebar_rect(panel);
        let mut strip_paint = Paint::default();
        strip_paint.set_anti_alias(true);
        strip_paint.set_color(fade(f.theme.fill_quaternary, chrome));
        canvas.save();
        // Clipped to the panel so the strip's own corners follow the card's.
        canvas.clip_rrect(RRect::new_rect_xy(panel, 12.0, 12.0), None, true);
        canvas.draw_rect(strip, &strip_paint);
        canvas.restore();

        let mut rule = Paint::default();
        rule.set_anti_alias(true);
        rule.set_color(fade(f.theme.fill_tertiary, chrome));
        rule.set_stroke_width(1.0);
        canvas.draw_line(
            Point::new(strip.left, strip.bottom),
            Point::new(strip.right, strip.bottom),
            &rule,
        );

        // The name, centred in the strip and clamped so a long one cannot run
        // under the close button.
        if !session.name.is_empty() {
            // The same size the browser's own header gives a title. The strip
            // is a titlebar and the name is what it is for, so it is read at
            // the distance a window title is read at rather than a caption's.
            let font = styles::BODY_EMPHASIZED.font();
            let room = strip.width() - 80.0;
            Label::new(ellipsize(&font, &session.name, room.max(40.0)))
                .with_style(styles::BODY_EMPHASIZED)
                .with_color(fade(f.theme.text_secondary, chrome))
                .centered_at(strip.center_x(), strip.center_y())
                .render(canvas);
        }

        draw_quickview_close(canvas, f.theme, panel, f.quickview_close_hovered, chrome);
    }

    let content = quickview_content_rect(panel);
    canvas.save();
    canvas.clip_rrect(RRect::new_rect_xy(panel, 12.0, 12.0), None, true);
    canvas.clip_rect(content, None, true);
    otto_kit::preview::draw(
        canvas,
        content,
        &session.preview,
        f.theme,
        session.first_row,
        session.zoom,
        &|name: &str, size: i32| {
            icons::cached_icon_chain_at(&[name], size, icons::FULL_COLOUR_SIZE)
        },
    );

    // The pan's bars, inside the same clip as the picture they belong to.
    // Nothing is drawn unless there is a zoomed picture with something under
    // the fold: a state with no content past its viewport has no thumb.
    let (horizontal, vertical) = session.pan_bars();
    ScrollRenderer::draw(canvas, horizontal, f.theme, |_, _| {});
    ScrollRenderer::draw(canvas, vertical, f.theme, |_, _| {});
    canvas.restore();
}

/// Draw the Get Info panel.
///
/// A panel, not a modal sheet: it is drawn into a surface of its own, the
/// window behind it is not dimmed, and it goes on taking input while the
/// browser underneath does too. The user drags it by its top strip and
/// dismisses it with its close dot, which is the whole of what makes it feel
/// like a window rather than a sheet.
///
/// `sheet` is where it currently sits, in window coordinates — the caller
/// owns that, because the user moves it.
///
/// `shadow` paints one under the card. Only for the fallback path that draws
/// into the window's own canvas: on its own surface the compositor casts the
/// shadow outside the card's bounds, which no client-side one can do.
pub fn draw_info(
    canvas: &Canvas,
    theme: &Theme,
    sheet: Rect,
    info: &model::FileInfo,
    error: Option<&str>,
    close_hovered: bool,
    shadow: bool,
) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);

    if shadow {
        paint.set_color(theme.shadow);
        canvas.draw_rrect(
            RRect::new_rect_xy(sheet.with_offset((0.0, 6.0)), 14.0, 14.0),
            &paint,
        );
    }
    paint.set_color(content_ground());
    canvas.draw_rrect(RRect::new_rect_xy(sheet, 14.0, 14.0), &paint);

    // Close control, in the same red as the window's own — and, on hover,
    // the same revealed × glyph as the window's own traffic lights.
    let close = info_close_rect(sheet);
    paint.set_color(Color::from_argb(0xFF, 0xFF, 0x5F, 0x57));
    canvas.draw_circle(
        Point::new(close.center_x(), close.center_y()),
        close.width() / 2.0,
        &paint,
    );
    if close_hovered {
        let mut glyph = Paint::default();
        glyph.set_anti_alias(true);
        glyph.set_style(skia_safe::paint::Style::Stroke);
        glyph.set_stroke_width((close.width() * 0.09).max(1.0));
        glyph.set_stroke_cap(skia_safe::PaintCap::Round);
        glyph.set_color(Color::from_argb(0xB0, 0x00, 0x00, 0x00));
        let r = close.width() * 0.22;
        let (cx, cy) = (close.center_x(), close.center_y());
        canvas.draw_line((cx - r, cy - r), (cx + r, cy + r), &glyph);
        canvas.draw_line((cx + r, cy - r), (cx - r, cy + r), &glyph);
    }

    // Icon and name.
    let chain = if info.is_dir {
        vec!["folder".to_string(), "inode-directory".to_string()]
    } else {
        otto_kit::filetype::icon_names(&info.mime)
    };
    let refs: Vec<&str> = chain.iter().map(String::as_str).collect();
    if let Some(image) = icons::cached_icon_chain(&refs, 64) {
        let dst = Rect::from_xywh(sheet.center_x() - 32.0, sheet.top + 34.0, 64.0, 64.0);
        canvas.draw_image_rect(&image, None, dst, &Paint::default());
    }

    Label::new(elide(&info.name, 30))
        .with_style(styles::TITLE_2_EMPHASIZED)
        .with_color(theme.text_primary)
        .centered_at(sheet.center_x(), sheet.top + 118.0)
        .render(canvas);

    let subtitle = if info.is_dir {
        otto_kit::t_owned!("files-kind-folder")
    } else {
        format!("{} — {}", info.kind.label(), model::format_size(info.size))
    };
    Label::new(&subtitle)
        .with_style(styles::CALLOUT)
        .with_color(theme.text_secondary)
        .centered_at(sheet.center_x(), sheet.top + 144.0)
        .render(canvas);

    // Detail rows. Body text, not a caption: these are the panel's content —
    // the path, the dates, who owns the file — and they were being set two
    // steps smaller than the same facts are shown at in the browser itself.
    let mut y = sheet.top + 180.0;
    let label_x = sheet.left + 24.0;
    // Where the values start is measured, not fixed. Neither column elides —
    // the label because it is meant to be short, the value because it is
    // ellipsized to whatever is left — so a field name wider than the gap it
    // was given simply runs into the value beside it, which is what "Ultimo
    // accesso" did to its date. 94pt is what English needs; a language that
    // needs more takes it out of the value column, which has room to give.
    let value_x = label_x + info_label_width().max(94.0);
    let value_w = sheet.right - 24.0 - value_x;
    let value_font = styles::BODY.font();

    let row = |label: &str, value: String, canvas: &Canvas, y: &mut f32| {
        if value.is_empty() {
            return;
        }
        Label::new(label)
            .with_style(styles::BODY)
            .with_color(theme.text_tertiary)
            .centered_on(label_x, *y)
            .render(canvas);
        // Measured against the font rather than counted in characters: a
        // per-character width estimate is a guess that has to be revisited
        // every time the size changes, and the column is narrow enough that
        // being wrong by a few characters runs the value under the edge.
        Label::new(ellipsize(&value_font, &value, value_w))
            .with_style(styles::BODY)
            .with_color(theme.text_primary)
            .centered_on(value_x, *y)
            .render(canvas);
        *y += 23.0;
    };

    let where_path = info
        .path
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    row(otto_kit::t!("files-info-where"), where_path, canvas, &mut y);
    row(
        otto_kit::t!("files-info-kind"),
        info.mime.clone(),
        canvas,
        &mut y,
    );
    row(
        otto_kit::t!("files-info-modified"),
        info.modified.map(model::format_time).unwrap_or_default(),
        canvas,
        &mut y,
    );
    row(
        otto_kit::t!("files-info-created"),
        info.created.map(model::format_time).unwrap_or_default(),
        canvas,
        &mut y,
    );
    row(
        otto_kit::t!("files-info-accessed"),
        info.accessed.map(model::format_time).unwrap_or_default(),
        canvas,
        &mut y,
    );
    row(
        otto_kit::t!("files-info-owner"),
        format!("{} : {}", info.owner, info.group),
        canvas,
        &mut y,
    );
    if let Some(target) = &info.link_target {
        row(
            otto_kit::t!("files-info-links-to"),
            target.to_string_lossy().into_owned(),
            canvas,
            &mut y,
        );
    }

    // A file that could not be read at all says so instead of showing a
    // permissions grid for a mode it never managed to load.
    if let Some(reason) = &info.error {
        Label::new(elide(reason, 40))
            .with_style(styles::CALLOUT)
            .with_color(Color::from_argb(0xFF, 0xD7, 0x3A, 0x2E))
            .centered_on(sheet.left + 24.0, y + 8.0)
            .render(canvas);
        return;
    }

    draw_permissions(canvas, theme, sheet, info, error);
}

fn draw_permissions(
    canvas: &Canvas,
    theme: &Theme,
    sheet: Rect,
    info: &model::FileInfo,
    error: Option<&str>,
) {
    let (ox, oy) = perm_origin(sheet);
    let mut paint = Paint::default();
    paint.set_anti_alias(true);

    paint.set_color(theme.fill_tertiary);
    paint.set_stroke_width(1.0);
    canvas.draw_line(
        Point::new(sheet.left + 20.0, oy - 40.0),
        Point::new(sheet.right - 20.0, oy - 40.0),
        &paint,
    );

    Label::new(otto_kit::t!("files-info-permissions"))
        .with_style(styles::BODY_EMPHASIZED)
        .with_color(theme.text_secondary)
        .centered_on(ox, oy - 18.0)
        .render(canvas);

    // The octal, because anyone changing permissions deliberately thinks in it.
    let octal = format!("{}  {}", info.mode_string(), info.mode_octal());
    let octal_w = styles::CALLOUT.font().measure_str(&octal, None).0;
    Label::new(&octal)
        .with_style(styles::CALLOUT)
        .with_color(theme.text_tertiary)
        .centered_on(sheet.right - 24.0 - octal_w, oy - 18.0)
        .render(canvas);

    // Headers sit in the gap above the first row, centred over their column.
    let (first, col_w) = perm_metrics(sheet);
    for (what, header) in [
        otto_kit::t!("files-perm-read"),
        otto_kit::t!("files-perm-write"),
        otto_kit::t!("files-perm-exec"),
    ]
    .into_iter()
    .enumerate()
    {
        let column_centre = first + what as f32 * col_w + PERM_BOX / 2.0;
        Label::new(header)
            .with_style(styles::CALLOUT)
            .with_color(theme.text_tertiary)
            .centered_at(column_centre, oy - 3.0)
            .render(canvas);
    }

    for (who, label) in [
        otto_kit::t!("files-perm-owner"),
        otto_kit::t!("files-perm-group"),
        otto_kit::t!("files-perm-everyone"),
    ]
    .into_iter()
    .enumerate()
    {
        let y = oy + PERM_ROWS_TOP + who as f32 * PERM_ROW_H;
        Label::new(label)
            .with_style(styles::BODY)
            .with_color(theme.text_primary)
            .centered_on(ox, y)
            .render(canvas);

        for what in 0..3 {
            let box_rect = perm_box_rect(sheet, who, what);
            let on = info.permission(who, what);

            paint.set_style(skia_safe::paint::Style::Fill);
            paint.set_color(if on {
                theme.material_selection_focused
            } else {
                theme.fill_tertiary
            });
            canvas.draw_rrect(RRect::new_rect_xy(box_rect, 4.0, 4.0), &paint);

            if on {
                // A tick, drawn rather than themed.
                paint.set_color(Color::WHITE);
                paint.set_style(skia_safe::paint::Style::Stroke);
                paint.set_stroke_width(1.8);
                paint.set_stroke_cap(skia_safe::paint::Cap::Round);
                let mut builder = PathBuilder::new();
                builder.move_to(Point::new(box_rect.left + 3.5, box_rect.center_y()));
                builder.line_to(Point::new(box_rect.center_x() - 0.5, box_rect.bottom - 4.0));
                builder.line_to(Point::new(box_rect.right - 3.5, box_rect.top + 4.5));
                canvas.draw_path(&builder.detach(), &paint);
                paint.set_style(skia_safe::paint::Style::Fill);
            }
        }
    }

    // A refused chmod says why, in place, rather than silently reverting.
    if let Some(error) = error {
        Label::new(elide(error, 42))
            .with_style(styles::CALLOUT)
            .with_color(Color::from_argb(0xFF, 0xD7, 0x3A, 0x2E))
            .centered_on(ox, oy + PERM_ROWS_TOP + 3.0 * PERM_ROW_H + 12.0)
            .render(canvas);
    }
}

/// Truncate to `max` characters with an ellipsis. Character-count based, which
/// is good enough for a fixed-width sheet and needs no font.
fn elide(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    let head: String = chars[..max.saturating_sub(1)].iter().collect();
    format!("{head}…")
}

// ---------------------------------------------------------------------------
// Quick View anchor
// ---------------------------------------------------------------------------

/// The rect quick view should grow its card out of: the cursor's row, in this
/// surface's own coordinates, with scrolling already applied.
///
/// Returns an **empty rect** when there is nothing to grow from — no cursor, or
/// a cursor scrolled out of view. That is a documented answer meaning "open in
/// place", not a missing value, and it is the case worth getting right: a stale
/// rect for an off-screen selection makes the preview appear to erupt from a
/// file that is not on screen.
///
/// Surface-local, never screen coordinates. A Wayland client cannot discover
/// where its own surface sits on an output, and Otto exposes nothing that would
/// tell it — `set_position` in `otto-surface-style` and `sc-layer` is a request,
/// and no protocol carries a position back. Resolving this to the screen is the
/// compositor's job. See `specs/file-browser.md`.
/// How long the open pulse runs. Long enough to register as an answer to the
/// double-click, short enough not to sit between the user and the application
/// they just asked for.
pub const OPEN_PULSE: std::time::Duration = std::time::Duration::from_millis(280);

/// The scale and alpha of the open pulse at `t`, 0 to 1.
///
/// A ghost of the icon grows out of the icon and fades as it goes — the icon
/// itself stays put underneath, so the row does not appear to leave with it.
/// Eased out: most of the movement is in the first third, which is what makes
/// it read as a thing springing open rather than drifting.
pub fn open_pulse(t: f32) -> (f32, u8) {
    let eased = 1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3);
    (1.0 + 0.6 * eased, (255.0 * (1.0 - eased)) as u8)
}

/// Draw the open pulse over everything: the whole selected entry — its
/// highlight, its icon and its name — growing out of itself and fading.
///
/// The entry is drawn again, selected, into a faded layer scaled about its own
/// centre, by the same code that drew it in the listing. Echoing the selection
/// rather than the icon alone is what makes it read as *that file* opening: in
/// the grid the caption pill goes with it, and in the row views the highlight
/// band does.
pub fn draw_open_pulse(canvas: &Canvas, f: &Frame) {
    let Some((depth, t)) = f.opening else { return };
    let Some(pane) = f.panes.get(depth) else {
        return;
    };
    let Some(index) = pane.cursor else { return };
    let Some(entry) = pane.entries.get(index) else {
        return;
    };

    let rect = cursor_entry_rect(f.width, f.height, f.mode, pane, depth, f.pan, f.miller_w);
    if rect.is_empty() {
        return;
    }

    let (scale, alpha) = open_pulse(t);
    let thumb = f.thumbnail(entry);

    // The layer is the grown rect, so nothing is clipped as it swells, and the
    // fade applies to the whole ghost at once rather than to each piece of it —
    // overlapping shapes inside must not show through one another.
    let grown = Rect::from_xywh(
        rect.center_x() - rect.width() * scale / 2.0,
        rect.center_y() - rect.height() * scale / 2.0,
        rect.width() * scale,
        rect.height() * scale,
    );
    canvas.save_layer_alpha(Some(grown), alpha as u32);
    canvas.translate((rect.center_x(), rect.center_y()));
    canvas.scale((scale, scale));
    canvas.translate((-rect.center_x(), -rect.center_y()));

    match f.mode {
        ViewMode::Grid => draw_grid_cell(canvas, f.theme, entry, rect, true, false, thumb),
        ViewMode::List | ViewMode::Columns => {
            // One row on its own is a run of one: rounded at both ends.
            draw_row_background(
                canvas,
                f.theme,
                rect,
                true,
                RunEnds {
                    first: true,
                    last: true,
                },
                index,
            );
            let (text_color, _) = row_colors(f.theme, true);
            let icon = entry_icon_rect(rect, f.mode);
            draw_entry_icon(canvas, entry, icon.left, icon.center_y(), false, thumb);

            let name_x = icon.right + if f.mode == ViewMode::List { 10.0 } else { 8.0 };
            let font = styles::BODY_MEDIUM.font();
            Label::new(ellipsize(
                &font,
                &entry.name,
                (rect.right - 12.0 - name_x).max(20.0),
            ))
            .with_style(styles::BODY_MEDIUM)
            .with_color(text_color)
            .centered_on(name_x, rect.center_y())
            .render(canvas);
        }
    }

    canvas.restore();
}

pub fn quickview_anchor(
    width: f32,
    height: f32,
    mode: ViewMode,
    pane: &PaneData,
    depth: usize,
    pan: f32,
    miller_w: f32,
) -> Rect {
    let Some(index) = pane.cursor else {
        return Rect::new_empty();
    };
    if index >= pane.entries.len() {
        return Rect::new_empty();
    }

    let rect = cursor_entry_rect(width, height, mode, pane, depth, pan, miller_w);
    if rect.is_empty() {
        return rect;
    }

    // Grow from the icon, not the whole row. The row is a full-width band and
    // the panel is a card, so a zoom out of the row reads as a stripe
    // unfurling; the icon is already the thing on screen that stands for the
    // file, and it is the shape the panel is closest to.
    entry_icon_rect(rect, mode)
}

/// The whole rect of the pane's cursor entry — the row band, or the grid cell —
/// in surface coordinates with scrolling applied.
///
/// Empty when there is nothing there to point at: no cursor, or one scrolled
/// out of the viewport. [`quickview_anchor`] narrows this to the icon; the open
/// pulse uses it whole, because what it echoes is the selection.
pub fn cursor_entry_rect(
    width: f32,
    height: f32,
    mode: ViewMode,
    pane: &PaneData,
    depth: usize,
    pan: f32,
    miller_w: f32,
) -> Rect {
    let Some(index) = pane.cursor else {
        return Rect::new_empty();
    };
    if index >= pane.entries.len() {
        return Rect::new_empty();
    }

    let viewport = content_viewport(width, height, mode);
    let count = pane.entries.len();
    let rect = match mode {
        ViewMode::List => RowStrip::list(width, count, pane.scroll).rect(index),
        ViewMode::Grid => grid_cell_rect(viewport, index, pane.scroll),
        ViewMode::Columns => RowStrip::miller(
            miller_pane_rect(depth, height, pan, miller_w),
            count,
            pane.scroll,
        )
        .rect(index),
    };

    // Clipped away entirely, or scrolled out of the viewport: nothing to grow
    // from. A partially visible row still counts — the user can see it.
    // `Rect::intersect` reports overlap and mutates in place, so test a copy.
    let mut probe = rect;
    if probe.intersect(viewport) {
        rect
    } else {
        Rect::new_empty()
    }
}

/// Where the icon sits inside a row or a grid cell — the same geometry the
/// drawing uses, read back so the Quick View entrance can start there.
pub(crate) fn entry_icon_rect(rect: Rect, mode: ViewMode) -> Rect {
    match mode {
        ViewMode::Grid => Rect::from_xywh(
            rect.center_x() - GRID_ICON / 2.0,
            rect.top + 8.0,
            GRID_ICON,
            GRID_ICON,
        ),
        // The list insets its icon by the content padding; a Miller column,
        // which has no such padding, by its own fixed inset.
        ViewMode::List => Rect::from_xywh(
            rect.left + CONTENT_PAD,
            rect.center_y() - ICON_SIZE / 2.0,
            ICON_SIZE,
            ICON_SIZE,
        ),
        ViewMode::Columns => Rect::from_xywh(
            rect.left + 14.0,
            rect.center_y() - ICON_SIZE / 2.0,
            ICON_SIZE,
            ICON_SIZE,
        ),
    }
}

#[cfg(test)]
mod fit_tests {
    //! The Get Info panel's two narrow columns, measured against every
    //! catalogue rather than against the English the layout was drawn for.
    //!
    //! Both columns size themselves to the language they are drawn in, so the
    //! question is not whether a label fits a fixed width — it is whether the
    //! panel still has room once it has. A translation long enough to push the
    //! last checkbox off the sheet, or to leave the values with nothing to be
    //! drawn in, is a layout that has run out, and the only way to find that
    //! out is to measure every locale rather than the one on this machine.

    use super::*;

    /// The value of every `key = value` line in a catalogue.
    ///
    /// Read off disk rather than through the locale chain: the chain is a
    /// process-wide `OnceLock`, so a test that went through it could only ever
    /// examine whichever locale won the race, and what matters here is all of
    /// them at once.
    fn catalogue(locale: &str) -> std::collections::HashMap<String, String> {
        let path = format!(
            "{}/../../resources/locales/{locale}.ftl",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let (key, value) = line.split_once(" = ")?;
                (!key.starts_with('#') && !key.starts_with(' '))
                    .then(|| (key.trim().to_string(), value.trim().to_string()))
            })
            .collect()
    }

    const LOCALES: &[&str] = &["en-GB", "de", "es", "fr", "it", "pl", "pt-BR", "ru", "uk"];

    /// The widest of `keys` in `locale`, measured in `font`.
    fn widest(locale: &str, font: &skia_safe::Font, keys: &[&str]) -> (f32, String) {
        let catalogue = catalogue(locale);
        keys.iter()
            .filter_map(|key| catalogue.get(*key))
            .map(|text| (font.measure_str(text, None).0, text.clone()))
            .fold((0.0, String::new()), |a, b| if b.0 > a.0 { b } else { a })
    }

    /// The row labels never grow far enough to unseat the right alignment.
    ///
    /// Mirrors `perm_metrics`, which cannot be called directly: it reads the
    /// live locale chain, and this has to answer for all nine at once.
    #[test]
    fn the_permissions_grid_fits_the_panel() {
        let mut over = Vec::new();
        for locale in LOCALES {
            let (labels, widest_label) = widest(
                locale,
                &styles::BODY.font(),
                &[
                    "files-perm-owner",
                    "files-perm-group",
                    "files-perm-everyone",
                ],
            );
            let (headers, widest_header) = widest(
                locale,
                &styles::CALLOUT.font(),
                &["files-perm-read", "files-perm-write", "files-perm-exec"],
            );
            let label_gap = (labels + PERM_GAP).max(PERM_COL_X);
            let col_w = (headers + PERM_GAP).max(PERM_COL_W);
            // The grid hangs off the right margin, so it never overflows —
            // what overflows is the label column, which pushes the columns
            // back out to the right and takes the alignment with them. 24pt of
            // margin at each edge, then the labels, two column gaps, and the
            // last checkbox.
            let needed = 24.0 + label_gap + 2.0 * col_w + PERM_BOX + 24.0;
            if needed > INFO_W {
                over.push(format!(
                    "{locale}: needs {needed:.0}pt of {INFO_W:.0}                      ({widest_label:?}, {widest_header:?})"
                ));
            }
        }
        assert!(
            over.is_empty(),
            "the permissions grid is wider than the panel:\n{}",
            over.join("\n")
        );
    }

    /// And the field names leave the values something to be drawn in.
    ///
    /// The values are ellipsized, so they never overflow — they just stop
    /// saying anything useful. A path or a MIME type needs most of the panel.
    #[test]
    fn info_field_names_leave_room_for_their_values() {
        const VALUE_MIN: f32 = 200.0;
        let mut tight = Vec::new();
        for locale in LOCALES {
            let (labels, widest_label) = widest(
                locale,
                &styles::BODY.font(),
                &[
                    "files-info-where",
                    "files-info-kind",
                    "files-info-modified",
                    "files-info-created",
                    "files-info-accessed",
                    "files-info-owner",
                    "files-info-links-to",
                ],
            );
            let value_x = 24.0 + (labels + PERM_GAP).max(94.0);
            let value_w = INFO_W - 24.0 - value_x;
            if value_w < VALUE_MIN {
                tight.push(format!(
                    "{locale}: {value_w:.0}pt left for values ({widest_label:?})"
                ));
            }
        }
        assert!(
            tight.is_empty(),
            "less than {VALUE_MIN:.0}pt left for the values:\n{}",
            tight.join("\n")
        );
    }
}

#[cfg(test)]
mod geometry_tests {
    use super::*;

    /// The rubber band's hit test has to agree with where the cells are
    /// drawn — a band around a cell's own rect catches that cell and only it.
    #[test]
    fn a_band_catches_the_cells_it_is_drawn_around() {
        let area = content_viewport(900.0, 600.0, ViewMode::Grid);
        let count = 30;
        let cell = grid_cell_rect(area, 7, 0.0);

        assert_eq!(
            grid_cells_in_rect(area, count, 0.0, cell.with_inset((1.0, 1.0))),
            vec![7]
        );

        // Widened over its neighbour, and the neighbour comes along.
        let pair = Rect::from_ltrb(
            cell.left + 1.0,
            cell.top + 1.0,
            cell.right + 4.0,
            cell.bottom - 1.0,
        );
        assert_eq!(grid_cells_in_rect(area, count, 0.0, pair), vec![7, 8]);
    }

    /// A band that touches an icon at all catches it: dragging a thin band
    /// through a row selects the row, rather than needing to enclose it.
    #[test]
    fn a_band_grazing_a_cell_still_catches_it() {
        let area = content_viewport(900.0, 600.0, ViewMode::Grid);
        let cell = grid_cell_rect(area, 3, 0.0);
        let sliver = Rect::from_ltrb(
            cell.left + 2.0,
            cell.top + 2.0,
            cell.left + 3.0,
            cell.top + 3.0,
        );

        assert_eq!(grid_cells_in_rect(area, 30, 0.0, sliver), vec![3]);
    }

    /// A band of no size catches nothing, whatever it is over — which is what
    /// makes a click on empty space mean an empty selection.
    #[test]
    fn a_band_of_no_size_catches_nothing() {
        let area = content_viewport(900.0, 600.0, ViewMode::Grid);
        let cell = grid_cell_rect(area, 2, 0.0);
        let point = Rect::from_xywh(cell.center_x(), cell.center_y(), 0.0, 0.0);

        assert!(grid_cells_in_rect(area, 30, 0.0, point).is_empty());
    }

    /// Past the last entry there is nothing to catch, however far the band is
    /// dragged into the empty part of the pane.
    #[test]
    fn a_band_below_the_last_cell_catches_nothing_more() {
        let area = content_viewport(900.0, 600.0, ViewMode::Grid);
        let below = Rect::from_ltrb(
            area.left,
            area.bottom - 40.0,
            area.right,
            area.bottom + 400.0,
        );

        assert!(grid_cells_in_rect(area, 3, 0.0, below).is_empty());
    }

    /// The drop ring takes the shape of what it outlines: rounded around a
    /// grid cell or a sidebar place, square around a column or a row band.
    #[test]
    fn the_drop_ring_follows_the_shape_it_outlines() {
        let cell = DropHighlight::Row { depth: 0, index: 3 };
        assert!(drop_ring_radius(ViewMode::Grid, cell) > 0.0);
        assert_eq!(drop_ring_radius(ViewMode::List, cell), 0.0);
        assert_eq!(drop_ring_radius(ViewMode::Columns, cell), 0.0);

        let pane = DropHighlight::Pane { depth: 0 };
        for mode in [ViewMode::List, ViewMode::Grid, ViewMode::Columns] {
            assert_eq!(drop_ring_radius(mode, pane), 0.0);
        }

        let place = DropHighlight::Place { index: 1 };
        assert!(drop_ring_radius(ViewMode::Columns, place) > 0.0);
    }

    /// A drag carries the shape of the view it started in: a cell in the icon
    /// grid, a row card everywhere else.
    #[test]
    fn the_drag_image_is_shaped_like_the_view_it_came_from() {
        assert_eq!(drag_image_size(ViewMode::Grid), (CELL_W, CELL_H));
        for mode in [ViewMode::List, ViewMode::Columns] {
            assert_eq!(drag_image_size(mode), (DRAG_IMAGE_W, DRAG_IMAGE_H));
        }
    }

    /// The open pulse starts as the icon itself and ends invisible, having
    /// grown the whole way — a ghost leaving, not a second icon appearing.
    #[test]
    fn the_open_pulse_grows_out_of_the_icon_and_fades() {
        let (scale, alpha) = open_pulse(0.0);
        assert_eq!(scale, 1.0);
        assert_eq!(alpha, 255);

        let (scale, alpha) = open_pulse(1.0);
        assert!(scale > 1.5);
        assert_eq!(alpha, 0);

        // Eased out: past halfway through the growth before halfway in time.
        let (mid, _) = open_pulse(0.5);
        assert!(mid > 1.0 + 0.6 / 2.0);
    }

    /// The grid's selection rectangle belongs to the icon: it stands a little
    /// off it on every side, and stays well inside the cell it lives in.
    #[test]
    fn a_selected_grid_icon_is_highlighted_at_icon_size() {
        let cell = grid_cell_rect(Rect::from_xywh(0.0, 0.0, 800.0, 600.0), 0, 0.0);
        let rect = grid_icon_highlight_rect(cell, cell.top + 8.0);

        assert_eq!(rect.width(), GRID_ICON + GRID_ICON_INSET * 2.0);
        assert_eq!(rect.height(), rect.width());
        assert_eq!(rect.center_x(), cell.center_x());
        assert!(rect.width() < cell.width() - 20.0);
    }

    /// And it meets the caption's pill edge to edge — the two read as one
    /// highlight, with neither overlapping the other.
    #[test]
    fn the_icon_highlight_meets_the_caption_pill() {
        let cell = grid_cell_rect(Rect::from_xywh(0.0, 0.0, 800.0, 600.0), 0, 0.0);
        let icon_top = cell.top + 8.0;
        let pill_top = icon_top + GRID_ICON + GRID_LABEL_GAP - GRID_LABEL_INSET;

        assert_eq!(grid_icon_highlight_rect(cell, icon_top).bottom, pill_top);
    }

    /// The first frames of the entrance are a card the size of the file's own
    /// row. The chrome is absolute, so drawing it there would fill the card
    /// with titlebar; it stays away until the card can carry it.
    #[test]
    fn a_tiny_card_carries_no_chrome() {
        let row = Rect::from_xywh(240.0, 180.0, 260.0, 24.0);
        assert_eq!(quickview_chrome_opacity(row), 0.0);
    }

    /// And every panel that is actually at rest has it in full, however
    /// small the window it rests in — both ends of the ramp sit below the
    /// smallest panel the browser will lay out.
    #[test]
    fn a_resting_panel_always_has_its_titlebar() {
        for (w, h) in [(1100.0, 700.0), (320.0, 240.0), (480.0, 360.0)] {
            let panel = crate::quickview::panel_rect(w, h);
            assert_eq!(
                quickview_chrome_opacity(panel),
                1.0,
                "{w}x{h} window, panel {panel:?}"
            );
        }
    }

    /// In between it ramps rather than popping, so the strip arrives with the
    /// card instead of appearing on top of one already in motion.
    #[test]
    fn the_chrome_fades_in_as_the_card_grows() {
        let resting = crate::quickview::panel_rect(1100.0, 700.0);
        let row = Rect::from_xywh(240.0, 180.0, 260.0, 24.0);
        let mut last = 0.0;
        for step in 0..=20 {
            let t = step as f32 / 20.0;
            let opacity = quickview_chrome_opacity(crate::quickview::entrance_at(row, resting, t));
            assert!(opacity >= last - 0.001, "went backwards at t={t}");
            last = opacity;
        }
        assert_eq!(last, 1.0);
    }

    /// The panel opens centred, and moving it moves the whole card — the
    /// close dot and the permission grid travel with it, because every piece
    /// of its geometry is derived from the rect rather than the window.
    #[test]
    fn the_info_panel_travels_with_its_rect() {
        let home = Rect::from_wh(INFO_W, INFO_H);
        let moved = Rect::from_xywh(40.0, 12.0, INFO_W, INFO_H);
        let dx = moved.left - home.left;
        let dy = moved.top - home.top;
        assert!(
            (info_close_rect(moved).left - info_close_rect(home).left - dx).abs() < 0.01,
            "the close dot stayed behind"
        );
        let (a, b) = (perm_box_rect(home, 1, 2), perm_box_rect(moved, 1, 2));
        assert!((b.left - a.left - dx).abs() < 0.01, "{a:?} {b:?}");
        assert!((b.top - a.top - dy).abs() < 0.01, "{a:?} {b:?}");
    }

    /// The strip the panel is dragged by must not sit over anything that is
    /// clicked for another reason, or a drag would toggle a permission.
    #[test]
    fn the_drag_strip_holds_nothing_clickable() {
        let sheet = Rect::from_wh(INFO_W, INFO_H);
        let strip = info_titlebar_rect(sheet);
        for who in 0..3 {
            for what in 0..3 {
                let mut box_rect = perm_box_rect(sheet, who, what);
                assert!(
                    !box_rect.intersect(strip),
                    "permission box {who},{what} is under the drag strip"
                );
            }
        }
        // The close dot is the exception: it is *in* the strip, and is
        // hit-tested before the drag.
        assert!(strip.contains(skia_safe::Point::new(
            info_close_rect(sheet).center_x(),
            info_close_rect(sheet).center_y()
        )));
    }

    use crate::model::Entry;
    use otto_kit::filetype::Kind;

    /// A solid-red image of the given shape, standing in for a thumbnail.
    fn red_image(w: i32, h: i32) -> skia_safe::Image {
        let info = skia_safe::ImageInfo::new(
            (w, h),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Premul,
            None,
        );
        let pixels: Vec<u8> = (0..w * h).flat_map(|_| [255u8, 0, 0, 255]).collect();
        skia_safe::images::raster_from_data(
            &info,
            skia_safe::Data::new_copy(&pixels),
            w as usize * 4,
        )
        .expect("raster image")
    }

    /// Draw one grid cell into a bitmap and hand back the pixels, so a test
    /// can ask what actually landed rather than what was meant to.
    fn grid_cell_pixels(thumb: Option<&skia_safe::Image>) -> (skia_safe::Surface, Rect) {
        let cell = Rect::from_xywh(0.0, 0.0, CELL_W, CELL_H);
        let mut surface =
            skia_safe::surfaces::raster_n32_premul((CELL_W as i32, CELL_H as i32)).unwrap();
        let theme = Theme::light();
        let entry = &entries(1)[0];
        surface.canvas().clear(skia_safe::Color::WHITE);
        draw_grid_cell(surface.canvas(), &theme, entry, cell, false, false, thumb);
        (surface, cell)
    }

    fn pixel_at(surface: &mut skia_safe::Surface, x: i32, y: i32) -> (u8, u8, u8) {
        let info = skia_safe::ImageInfo::new(
            (1, 1),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Unpremul,
            None,
        );
        let mut px = [0u8; 4];
        assert!(
            surface.image_snapshot().read_pixels(
                &info,
                &mut px,
                4,
                (x, y),
                skia_safe::image::CachingHint::Allow
            ),
            "reading pixel at {x},{y}"
        );
        (px[0], px[1], px[2])
    }

    /// A thumbnail is drawn where the icon would have been — the picture
    /// reaches the box, rather than the type icon being drawn over or under
    /// it.
    #[test]
    fn a_grid_cell_with_a_thumbnail_draws_the_picture() {
        let image = red_image(64, 64);
        let (mut surface, cell) = grid_cell_pixels(Some(&image));

        // Centre of the icon box: a square thumbnail fills it, so this is red.
        let x = cell.center_x() as i32;
        let y = (cell.top + 8.0 + GRID_ICON / 2.0) as i32;
        let (r, g, b) = pixel_at(&mut surface, x, y);
        assert!(
            r > 200 && g < 60 && b < 60,
            "expected the thumbnail at the icon box's centre, got ({r},{g},{b})"
        );
    }

    /// Fitted, not filled: a wide picture keeps its proportions, so the box's
    /// top corners stay empty rather than being covered by a stretched or
    /// cropped image.
    #[test]
    fn a_wide_thumbnail_keeps_its_shape() {
        // Twice as wide as tall: fitted to the box's width, it occupies the
        // bottom half of the box and leaves the top half alone.
        let image = red_image(128, 64);
        let (mut surface, cell) = grid_cell_pixels(Some(&image));

        let icon_top = cell.top + 8.0;
        let x = cell.center_x() as i32;

        // Just below the box's bottom edge minus a quarter of its height: in
        // the picture.
        let inside = (icon_top + GRID_ICON * 0.75) as i32;
        let (r, _, _) = pixel_at(&mut surface, x, inside);
        assert!(r > 200, "expected picture in the lower half of the box");

        // The top of the box: above a half-height picture sitting on the
        // box's baseline, so still the background.
        let above = (icon_top + 2.0) as i32;
        let (r, g, b) = pixel_at(&mut surface, x, above);
        assert!(
            r > 200 && g > 200 && b > 200,
            "a fitted wide picture must not reach the top of the box, got ({r},{g},{b})"
        );
    }

    fn entries(n: usize) -> Vec<Entry> {
        (0..n)
            .map(|i| Entry {
                name: format!("f{i}"),
                path: std::path::PathBuf::from(format!("f{i}")),
                is_dir: false,
                is_symlink: false,
                hidden: false,
                kind: Kind::Text,
                size: Some(0),
                modified: None,
            })
            .collect()
    }

    fn pane<'a>(owned: &'a [Entry], cursor: Option<usize>, scroll: f32) -> PaneData<'a> {
        PaneData {
            entries: owned.iter().collect(),
            selected: vec![false; owned.len()],
            cursor,
            scroll,
            bar: None,
            loading: false,
            error: None,
        }
    }

    #[test]
    fn no_cursor_means_no_anchor() {
        let owned = entries(10);
        let anchor = quickview_anchor(
            1100.0,
            700.0,
            ViewMode::List,
            &pane(&owned, None, 0.0),
            0,
            0.0,
            MILLER_W,
        );
        assert!(anchor.is_empty(), "expected an empty rect, got {anchor:?}");
    }

    #[test]
    fn a_visible_row_anchors_to_itself() {
        let owned = entries(10);
        let anchor = quickview_anchor(
            1100.0,
            700.0,
            ViewMode::List,
            &pane(&owned, Some(0), 0.0),
            0,
            0.0,
            MILLER_W,
        );
        assert!(!anchor.is_empty());
        // Inside the window, and below the header — a real place on screen.
        assert!(anchor.top >= HEADER_H, "{anchor:?}");
        assert!(anchor.left >= SIDEBAR_W, "{anchor:?}");
        // The icon, not the row: a square the size the icon is drawn at,
        // rather than a band running the width of the file area.
        assert!((anchor.width() - ICON_SIZE).abs() < 0.5, "{anchor:?}");
        assert!((anchor.height() - ICON_SIZE).abs() < 0.5, "{anchor:?}");
    }

    /// Every mode anchors to something icon-shaped, and inside the row it
    /// belongs to — the entrance must not start from a rect the file is not
    /// actually drawn in.
    #[test]
    fn every_mode_anchors_to_a_square_inside_its_row() {
        let owned = entries(10);
        for mode in [ViewMode::List, ViewMode::Grid, ViewMode::Columns] {
            let anchor = quickview_anchor(
                1100.0,
                700.0,
                mode,
                &pane(&owned, Some(0), 0.0),
                0,
                0.0,
                MILLER_W,
            );
            assert!(!anchor.is_empty(), "{mode:?}: {anchor:?}");
            assert!(
                (anchor.width() - anchor.height()).abs() < 0.5,
                "{mode:?}: {anchor:?}"
            );
            let viewport = content_viewport(1100.0, 700.0, mode);
            let mut probe = anchor;
            assert!(probe.intersect(viewport), "{mode:?}: {anchor:?}");
        }
    }

    #[test]
    fn a_row_scrolled_out_of_view_anchors_nowhere() {
        // The case that would look wrong once the zoom lands: the selection is
        // real, but it is not on screen, so there is nothing to grow from.
        let owned = entries(500);
        let anchor = quickview_anchor(
            1100.0,
            700.0,
            ViewMode::List,
            &pane(&owned, Some(0), 5_000.0),
            0,
            0.0,
            MILLER_W,
        );
        assert!(anchor.is_empty(), "expected an empty rect, got {anchor:?}");
    }

    #[test]
    fn a_cursor_past_the_end_anchors_nowhere() {
        let owned = entries(3);
        let anchor = quickview_anchor(
            1100.0,
            700.0,
            ViewMode::List,
            &pane(&owned, Some(9), 0.0),
            0,
            0.0,
            MILLER_W,
        );
        assert!(anchor.is_empty());
    }

    #[test]
    fn every_view_mode_produces_an_anchor() {
        let owned = entries(10);
        for mode in [ViewMode::List, ViewMode::Grid, ViewMode::Columns] {
            let anchor = quickview_anchor(
                1100.0,
                700.0,
                mode,
                &pane(&owned, Some(1), 0.0),
                0,
                0.0,
                MILLER_W,
            );
            assert!(!anchor.is_empty(), "{mode:?} produced no anchor");
        }
    }

    /// The band a pane of `count` rows would be asked for when the window is
    /// `height` tall and scrolled to `offset`, through the same scroll state a
    /// live pane carries.
    fn scrolled(count: usize, height: f32, offset: f32) -> ScrollState {
        let viewport = content_viewport(1100.0, height, ViewMode::List);
        let mut state = ScrollState::new(viewport);
        state.set_content_length(pane_content_height(1100.0, height, ViewMode::List, count));
        state.set_offset(offset);
        state
    }

    #[test]
    fn a_strip_offers_only_the_rows_the_band_covers() {
        let strip = RowStrip::list(1100.0, 4_000, 0.0);
        let band = content_viewport(1100.0, 700.0, ViewMode::List);
        let visible = strip.visible(band);

        // A 580pt band of 30pt rows: twenty or so, never four thousand.
        assert!(visible.len() < 30, "{visible:?}");
        assert_eq!(visible.start, 0);
        // Every row offered really does touch the band, and the ones just
        // outside it really are left out.
        for index in visible.clone() {
            let rect = strip.rect(index);
            assert!(
                rect.bottom >= band.top && rect.top <= band.bottom,
                "{index}"
            );
        }
        assert!(strip.rect(visible.end).top > band.bottom);
    }

    #[test]
    fn scrolling_moves_the_offered_rows_without_widening_them() {
        let band = content_viewport(1100.0, 700.0, ViewMode::List);
        // A thousand rows down, expressed in rows rather than points so the
        // test says what it means when the pitch changes.
        let scroll = 1_000.0 * ROW_H;
        let top = RowStrip::list(1100.0, 4_000, 0.0).visible(band);
        let deep = RowStrip::list(1100.0, 4_000, scroll).visible(band);

        assert_eq!(top.len(), deep.len());
        assert_eq!(deep.start, 1_000);
    }

    #[test]
    fn a_rubber_banded_pane_still_offers_the_rows_pulled_into_view() {
        // Overscrolled past the top: the rows are pushed down the window, and
        // the first of them must still be drawn or the pull looks like a wipe.
        let band = content_viewport(1100.0, 700.0, ViewMode::List);
        let pulled = RowStrip::list(1100.0, 4_000, -80.0).visible(band);
        assert_eq!(pulled.start, 0);
        assert!(pulled.len() > 1);
    }

    #[test]
    fn an_off_screen_miller_pane_offers_nothing() {
        let strip = RowStrip::miller(Rect::from_xywh(-500.0, HEADER_H, MILLER_W, 600.0), 500, 0.0);
        assert!(strip.visible(Rect::new_empty()).is_empty());
    }

    #[test]
    fn the_grid_offers_whole_rows_of_cells_and_no_more() {
        let area = content_viewport(1100.0, 700.0, ViewMode::Grid);
        let band = area;
        let visible = grid_visible_range(area, 4_000, 0.0, band);
        assert!(!visible.is_empty());
        assert!(visible.len() < 100, "{visible:?}");
        for index in visible.clone() {
            let cell = grid_cell_rect(area, index, 0.0);
            assert!(
                cell.bottom >= band.top && cell.top <= band.bottom,
                "{index}"
            );
        }
    }

    #[test]
    fn the_band_a_pane_is_asked_for_matches_its_scroll_view() {
        // The band is the scroll view's own content rect, mapped back into the
        // window coordinates the rows live in — so it lands exactly on the
        // viewport the scroll view was given, scrolled or not.
        let owned = entries(4_000);
        for offset in [0.0, 900.0] {
            let state = scrolled(owned.len(), 700.0, offset);
            let mut pane = pane(&owned, None, state.offset());
            pane.bar = Some(&state);
            assert_eq!(pane.band(Rect::new_empty()), state.viewport());
        }
    }

    /// Draw ops `draw_list` emits for one window height, scrolled to `offset`.
    fn list_ops(owned: &[Entry], height: f32, offset: f32) -> usize {
        let state = scrolled(owned.len(), height, offset);
        let mut data = pane(owned, None, state.offset());
        data.bar = Some(&state);

        let theme = Theme::light();
        let frame = Frame {
            action_row: None,
            footer: 0.0,
            quickview_close_hovered: false,
            drop_target: None,
            marquee: None,
            width: 1100.0,
            height,
            theme: &theme,
            title: "Home",
            subtitle: String::new(),
            places: &[],
            selected_place: None,
            mode: ViewMode::List,
            panes: vec![data],
            active: 0,
            pan: 0.0,
            pan_bar: None,
            miller_w: MILLER_W,
            sort: SortKey::Name,
            ascending: true,
            list_columns: ListColumnWidths::default(),
            opening: None,
            renaming: None,
            cut: Vec::new(),
            controls: WindowControlsState::new(),
            focused: true,
            blurred: true,
            can_go_back: false,
            can_go_forward: false,
            nav_pressed: None,
            preview: None,
            // No store: this measures the cost of drawing rows, and every
            // entry falling back to its icon is the case being counted.
            thumbs: None,
        };

        let mut recorder = skia_safe::PictureRecorder::new();
        let canvas = recorder.begin_recording(Rect::from_wh(1100.0, height), false);
        draw_list(canvas, &frame);
        recorder
            .finish_recording_as_picture(None)
            .expect("recorded picture")
            .approximate_op_count()
    }

    #[test]
    fn a_scrolled_tall_listing_draws_far_less_than_the_whole_of_it() {
        let owned = entries(3_000);
        // A window tall enough to hold every row is the "draw it all" case:
        // the band covers the whole content, so nothing is skipped.
        let whole = HEADER_H + COLUMNS_H + content_height(owned.len());

        let all = list_ops(&owned, whole, 0.0);
        let windowed = list_ops(&owned, 700.0, 15_000.0);

        assert!(
            windowed * 20 < all,
            "a 700pt window into 3000 rows cost {windowed} ops against {all} for all of them"
        );
    }

    #[test]
    fn a_miller_column_panned_off_screen_anchors_nowhere() {
        let owned = entries(10);
        let anchor = quickview_anchor(
            1100.0,
            700.0,
            ViewMode::Columns,
            &pane(&owned, Some(0), 0.0),
            0,
            5_000.0,
            MILLER_W,
        );
        assert!(anchor.is_empty(), "expected an empty rect, got {anchor:?}");
    }
}
