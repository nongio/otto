//! The picker as a `lay-rs` scene.
//!
//! Built the way otto-launcher's card is: a small tree of layers positioned
//! once and then *changed*, so the selection slides between cells and the tab
//! marker slides between tabs on transitions the engine runs by itself. The
//! card's material — the frost, the corner radius, the shadow — is not drawn
//! here at all: the card is its own subsurface and asks the compositor for
//! those through `otto-surface-style`, exactly as the launcher and the bar's
//! menus do. What this file draws onto that material is the query field, the
//! category tabs, the grid, and the footer.
//!
//! The grid is one layer whose content is redrawn on every change, rather than
//! a layer per cell. There are seventy cells on screen and two thousand
//! behind them; the layer tree stays the same shape whatever is scrolled into
//! view, and a cell is an image blit — the emoji are rasterised once each and
//! kept.

use std::collections::HashMap;
use std::sync::Arc;

use layers::prelude::*;
use layers::types::{Color as LayerColor, Point as LayerPoint, Size as LayerSize};
use otto_kit::components::text_input::{TextInput, TextInputStyle};
use otto_kit::theme::Theme;
use otto_kit::typography::{get_font, get_font_with_fallback, styles};
use skia_safe::shaper::TextBlobBuilderRunHandler;
use skia_safe::{
    Canvas, Color, Color4f, Font, FontMgr, FontStyle, Image, Paint, Rect, SamplingOptions, Shaper,
};

use crate::data::{Tone, GROUPS};

/// Cells across. Ten is what fits a name in the footer and a screen of emoji
/// in the eye at once.
pub const COLUMNS: usize = 10;
/// One cell, square.
pub const CELL: f32 = 44.0;
/// Inset from the card's edge to the grid.
pub const PAD: f32 = 14.0;
/// Width of the card.
pub const CARD_W: f32 = PAD * 2.0 + COLUMNS as f32 * CELL;
/// Height of the query field.
pub const FIELD_H: f32 = 50.0;
/// Height of the tab strip.
pub const TABS_H: f32 = 40.0;
/// Rows of the grid on screen at once.
pub const GRID_ROWS: usize = 7;
pub const GRID_H: f32 = GRID_ROWS as f32 * CELL;
/// Air above and below the grid, so the first and last rows are not pressed
/// against the tab strip and the footer.
pub const GRID_PAD: f32 = 10.0;
/// Height of the footer: the name of what is under the cursor, and the tones.
pub const FOOTER_H: f32 = 36.0;
/// Corner radius of the card, applied by the compositor to the subsurface.
pub const RADIUS: f32 = 12.0;
/// The card, top to bottom, with a hairline between each part.
pub const CARD_H: f32 =
    FIELD_H + 1.0 + TABS_H + 1.0 + GRID_PAD + GRID_H + GRID_PAD + 1.0 + FOOTER_H;

/// Where the grid starts, from the top of the card.
pub const GRID_Y: f32 = FIELD_H + 1.0 + TABS_H + 1.0 + GRID_PAD;
/// Where the footer starts.
pub const FOOTER_Y: f32 = GRID_Y + GRID_H + GRID_PAD + 1.0;

/// Size of an emoji in a cell, in points.
const EMOJI_PT: f32 = 30.0;
/// And on a tab.
const TAB_EMOJI_PT: f32 = 21.0;
/// Diameter of a tone swatch, and the pitch between them.
const SWATCH: f32 = 16.0;
const SWATCH_PITCH: f32 = 24.0;

/// Where the top of the card sits, as a fraction of the output's height.
/// Above centre, as the launcher is: it is a dialog, not a window.
const TOP_FRACTION: f32 = 0.14;

/// One tab of the strip: the recent list, then every group.
pub const TABS: usize = GROUPS.len() + 1;
/// Width of one tab.
pub const TAB_W: f32 = (CARD_W - PAD * 2.0) / TABS as f32;

/// One cell of the grid: which emoji, and the text it stands for — tone
/// applied, or the exact text of a recent pick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub emoji: usize,
    pub text: String,
}

/// One category as a pane of the grid: a block of cells [`COLUMNS`] wide
/// filling the grid's width. Panes sit side by side and a horizontal scroll
/// pans between them, the way otto-files pans its columns; each pane scrolls
/// vertically on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pane {
    /// Which tab of the strip this pane belongs to.
    pub tab: usize,
    /// Index of this pane's first cell.
    pub first: usize,
    /// How many cells it holds.
    pub count: usize,
}

impl Pane {
    /// How many rows of cells the pane has.
    pub fn rows(&self) -> usize {
        self.count.div_ceil(COLUMNS)
    }

    /// How tall its content is, in points.
    pub fn height(&self) -> f32 {
        self.rows() as f32 * CELL
    }
}

/// The grid as panes side by side. Cell indices are global and run through the
/// panes in order, so a pane owns the contiguous range `first..first + count`.
#[derive(Debug, Default, Clone)]
pub struct Layout {
    pub panes: Vec<Pane>,
    /// `(pane, row, column)` for each cell.
    place: Vec<(usize, usize, usize)>,
}

impl Layout {
    /// Lay `panes` out left to right. They must cover the cells in order.
    pub fn new(panes: Vec<Pane>) -> Self {
        let mut place = Vec::new();
        for (index, pane) in panes.iter().enumerate() {
            debug_assert_eq!(
                pane.first,
                place.len(),
                "panes must cover the cells in order"
            );
            for cell in 0..pane.count {
                place.push((index, cell / COLUMNS, cell % COLUMNS));
            }
        }
        Self { panes, place }
    }

    pub fn pane_count(&self) -> usize {
        self.panes.len()
    }

    pub fn cell_count(&self) -> usize {
        self.place.len()
    }

    /// Which pane a cell is in, and where in it.
    pub fn place(&self, cell: usize) -> Option<(usize, usize, usize)> {
        self.place.get(cell).copied()
    }

    /// How many cells are on `row` of `pane` — the last row is usually short.
    fn row_width(&self, pane: usize, row: usize) -> Option<usize> {
        let pane = self.panes.get(pane)?;
        let rows = pane.rows();
        if row >= rows {
            return None;
        }
        Some(if row + 1 == rows {
            pane.count - row * COLUMNS
        } else {
            COLUMNS
        })
    }

    /// The cell at `row` and `column` of `pane`, moved left onto the last cell
    /// of a short row rather than falling off it.
    pub fn cell_in(&self, pane: usize, row: usize, column: usize) -> Option<usize> {
        let width = self.row_width(pane, row)?;
        let first = self.panes.get(pane)?.first;
        Some(first + row * COLUMNS + column.min(width - 1))
    }

    /// A cell's rectangle inside its own pane, before any scrolling.
    pub fn cell_rect(&self, cell: usize) -> Option<Rect> {
        let (_, row, column) = self.place(cell)?;
        Some(Rect::from_xywh(
            PAD + column as f32 * CELL,
            row as f32 * CELL,
            CELL,
            CELL,
        ))
    }

    pub fn pane_height(&self, pane: usize) -> f32 {
        self.panes.get(pane).map(Pane::height).unwrap_or(0.0)
    }

    /// How far `pane` can scroll vertically.
    pub fn max_scroll(&self, pane: usize) -> f32 {
        (self.pane_height(pane) - GRID_H).max(0.0)
    }

    /// Where a pane's left edge sits, in content coordinates.
    pub fn pane_origin(pane: usize) -> f32 {
        pane as f32 * CARD_W
    }

    /// Every pane side by side.
    pub fn content_width(&self) -> f32 {
        self.panes.len() as f32 * CARD_W
    }

    /// How far the panes can be panned.
    pub fn max_pan(&self) -> f32 {
        (self.content_width() - CARD_W).max(0.0)
    }

    /// The pane nearest to resting under the viewport at `pan`.
    pub fn pane_at_pan(&self, pan: f32) -> usize {
        if self.panes.is_empty() {
            return 0;
        }
        ((pan / CARD_W).round() as usize).min(self.panes.len() - 1)
    }

    /// Which pane a point `x` across the grid falls in.
    pub fn pane_at(&self, x: f32, pan: f32) -> Option<usize> {
        let index = ((x + pan) / CARD_W).floor();
        if index < 0.0 {
            return None;
        }
        let index = index as usize;
        (index < self.panes.len()).then_some(index)
    }

    /// The tab a pane belongs to.
    pub fn pane_tab(&self, pane: usize) -> Option<usize> {
        self.panes.get(pane).map(|pane| pane.tab)
    }

    /// The pane showing `tab`.
    pub fn pane_for_tab(&self, tab: usize) -> Option<usize> {
        self.panes.iter().position(|pane| pane.tab == tab)
    }

    /// The cell at a point in grid coordinates: `x` across the grid and `y`
    /// down it, with the pane pan and that pane's own vertical scroll applied.
    pub fn cell_at(&self, x: f32, y: f32, pan: f32, scroll: f32) -> Option<usize> {
        let pane = self.pane_at(x, pan)?;
        let local_x = x + pan - Self::pane_origin(pane);
        if !(PAD..PAD + COLUMNS as f32 * CELL).contains(&local_x) {
            return None;
        }
        let column = ((local_x - PAD) / CELL) as usize;
        let content_y = y + scroll;
        if content_y < 0.0 {
            return None;
        }
        let row = (content_y / CELL) as usize;
        let width = self.row_width(pane, row)?;
        if column >= width {
            return None;
        }
        Some(self.panes[pane].first + row * COLUMNS + column)
    }
}

/// Where the card's top-left corner sits inside a parent surface `surface`
/// points across and down.
pub fn card_origin(surface: (f32, f32)) -> (f32, f32) {
    let (width, height) = surface;
    (
        ((width - CARD_W) / 2.0).round(),
        (height * TOP_FRACTION).round(),
    )
}

/// How much air to leave between the caret and the card.
const CARET_GAP: f32 = 8.0;
/// How much to keep clear of the screen's edges. The bottom is deeper than the
/// rest because the dock lives there: a layer surface anchored to the whole
/// output is told the output's size, not the part of it nothing is covering,
/// so the picker has to leave the room itself or sit under the dock.
const SCREEN_MARGIN: f32 = 8.0;
const BOTTOM_MARGIN: f32 = 96.0;
/// How wide the beak is where it meets the card, and how far it reaches out
/// from it — the proportions the dock's labels point at their icon with, so
/// the two read as the same desktop.
pub const BEAK: f32 = 16.0;
pub const BEAK_REACH: f32 = 9.0;
/// Clearance between the beak's base and the card's rounded corner. Without
/// it the corner's curve eats one side of the beak and the point comes out
/// lopsided.
const BEAK_CORNER_CLEARANCE: f32 = 6.0;

/// Which edge of the card the beak sits on — that is, which way the card is
/// from the caret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Beak {
    /// The card is below the caret and points up at it.
    Top,
    /// The card is above the caret and points down at it.
    Bottom,
}

/// Where the card goes, and where it points.
///
/// The card is one surface — body and beak together — because the compositor
/// draws the whole balloon: the frost, the border and the shadow follow the
/// outline, and a beak on a surface of its own would be a separate piece of
/// glass sitting next to the card. So the surface is taller than the body by
/// the beak's reach, and the body sits inside it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// The card body's top-left corner in the parent surface's coordinates.
    pub origin: (f32, f32),
    /// The centre of the beak, and which edge it is on. `None` when the card
    /// is not pointing at anything.
    pub beak: Option<((f32, f32), Beak)>,
}

impl Placement {
    /// The whole balloon's size: the body, plus the beak's reach on the edge
    /// it sticks out of.
    pub fn surface_size(&self) -> (f32, f32) {
        match self.beak {
            Some(_) => (CARD_W, CARD_H + BEAK_REACH),
            None => (CARD_W, CARD_H),
        }
    }

    /// Where the balloon's surface starts, which is above the body when the
    /// beak is on the top edge.
    pub fn surface_origin(&self) -> (f32, f32) {
        let (x, y) = self.origin;
        match self.beak {
            Some((_, Beak::Top)) => (x, y - BEAK_REACH),
            _ => (x, y),
        }
    }

    /// Where the body sits inside the surface. Everything the picker draws is
    /// shifted by this, so the beak's strip stays empty.
    pub fn body_offset(&self) -> (f32, f32) {
        match self.beak {
            Some((_, Beak::Top)) => (0.0, BEAK_REACH),
            _ => (0.0, 0.0),
        }
    }

    /// Where the beak's tip points, measured along its edge from the surface's
    /// top-left corner — which is how the protocol wants it.
    pub fn beak_offset(&self) -> Option<f32> {
        let (((bx, _), _), (sx, _)) = (self.beak?, self.surface_origin());
        Some(bx - sx)
    }
}

/// Where the card goes when nothing is known about the text cursor: centred,
/// above the middle, pointing at nothing.
pub fn placement_centred(surface: (f32, f32)) -> Placement {
    Placement {
        origin: card_origin(surface),
        beak: None,
    }
}

/// Where the card goes when the desktop knows where the text cursor is.
///
/// `caret` is the cursor's rectangle in the parent surface's coordinates,
/// which is the whole output. The card is put beside the caret and points at
/// it, choosing the side with room:
///
/// - Below the caret by preference, above it when the card would otherwise
///   run past the bottom of the usable screen.
/// - Starting at the caret when there is room to its right, and ending at it
///   when there is not, so the card grows into whichever side is free.
///
/// The caret itself is never covered: the point is to keep the character being
/// typed visible while choosing what to put next to it.
pub fn placement_at_caret(surface: (f32, f32), caret: (f32, f32, f32, f32)) -> Placement {
    let (width, height) = surface;
    let (caret_x, caret_y, caret_w, caret_h) = caret;

    // The lowest the card may reach. Short of the screen's edge, because the
    // dock is down there and a layer surface anchored to the whole output is
    // told the output's size, not the part of it nothing is covering.
    let floor = height - BOTTOM_MARGIN;
    let below = caret_y + caret_h + CARET_GAP;
    let above = caret_y - CARET_GAP - CARD_H;
    let (y, edge) = if below + CARD_H <= floor || above < SCREEN_MARGIN {
        (below, Beak::Top)
    } else {
        (above, Beak::Bottom)
    };

    // The card is placed so the beak lands on the caret rather than so an edge
    // does: the beak has to stay off the rounded corner, so the card is
    // offset by that much. It grows into whichever side has the room — right
    // from the caret while that fits, left from it when it does not.
    let inset = otto_kit::corners::radius(RADIUS) + BEAK / 2.0 + BEAK_CORNER_CLEARANCE;
    let centre = caret_x + caret_w / 2.0;
    let rightwards = centre - inset;
    let x = if rightwards + CARD_W <= width - SCREEN_MARGIN {
        rightwards
    } else {
        centre + inset - CARD_W
    };

    let x = x.clamp(
        SCREEN_MARGIN,
        (width - CARD_W - SCREEN_MARGIN).max(SCREEN_MARGIN),
    );
    let y = y.clamp(SCREEN_MARGIN, (floor - CARD_H).max(SCREEN_MARGIN));
    let (x, y) = (x.round(), y.round());

    // The beak points at the middle of the caret, but stays on the card's
    // straight edge rather than wandering into a rounded corner — which is
    // where it ends up when the screen's edge has pushed the card off the
    // caret.
    let beak_x = centre.clamp(x + inset, (x + CARD_W - inset).max(x + inset));
    let beak_y = match edge {
        Beak::Top => y,
        Beak::Bottom => y + CARD_H,
    };

    Placement {
        origin: (x, y),
        beak: Some(((beak_x.round(), beak_y.round()), edge)),
    }
}

/// The card's top-left corner alone — see [`placement_at_caret`].
pub fn card_origin_at_caret(surface: (f32, f32), caret: (f32, f32, f32, f32)) -> (f32, f32) {
    placement_at_caret(surface, caret).origin
}

/// The rectangle the picker should take pointer input over, in the parent
/// surface's coordinates: the card, and nothing else. The parent surface
/// covers the output, and left to itself would take every click on it.
pub fn input_rect(surface: (f32, f32)) -> (i32, i32, i32, i32) {
    let (x, y) = card_origin(surface);
    (
        x.floor() as i32,
        y.floor() as i32,
        CARD_W.ceil() as i32,
        CARD_H.ceil() as i32,
    )
}

/// Which tab is at `x` across the card, if any.
pub fn tab_at(x: f32) -> Option<usize> {
    if !(PAD..CARD_W - PAD).contains(&x) {
        return None;
    }
    let tab = ((x - PAD) / TAB_W) as usize;
    (tab < TABS).then_some(tab)
}

/// Where the swatches start, from the card's left edge.
fn swatch_x0() -> f32 {
    CARD_W - PAD - Tone::ALL.len() as f32 * SWATCH_PITCH + (SWATCH_PITCH - SWATCH) / 2.0
}

/// Which tone swatch is at `x` across the footer, if any.
pub fn tone_at(x: f32) -> Option<Tone> {
    let x0 = swatch_x0() - (SWATCH_PITCH - SWATCH) / 2.0;
    if x < x0 {
        return None;
    }
    let index = ((x - x0) / SWATCH_PITCH) as usize;
    Tone::ALL.get(index).copied()
}

/// Draws and keeps the emoji images.
///
/// An emoji is a sequence — a flag is two regional indicators, a family is
/// people joined by zero-width joiners — and the font turns the sequence into
/// one glyph through shaping. So each is shaped once, drawn into a small
/// raster, and kept: the grid blits images, and never shapes on the render
/// thread.
struct Atlas {
    font_mgr: FontMgr,
    shaper: Shaper,
    typeface: Option<skia_safe::Typeface>,
    images: HashMap<(String, u32), Image>,
    /// Physical pixels per point, so the rasters are crisp on the output
    /// they are for.
    scale: f32,
}

impl Atlas {
    fn new(scale: f32) -> Self {
        let font_mgr = FontMgr::new();
        // A colour emoji face by name where there is one, otherwise whatever
        // the system would use to draw an emoji.
        let typeface = ["Noto Color Emoji", "Twemoji", "JoyPixels", "Emoji One"]
            .iter()
            .find_map(|family| font_mgr.match_family_style(family, FontStyle::normal()))
            .or_else(|| {
                font_mgr.match_family_style_character("", FontStyle::normal(), &[], 0x1F600)
            });
        if typeface.is_none() {
            tracing::warn!("no emoji font; the palette will be boxes");
        }
        Self {
            shaper: Shaper::new(font_mgr.clone()),
            font_mgr,
            typeface,
            images: HashMap::new(),
            scale: scale.max(1.0),
        }
    }

    /// Whether the emoji font has a glyph for `codepoint`.
    fn can_draw(&self, codepoint: u32) -> bool {
        match &self.typeface {
            Some(typeface) => typeface.unichar_to_glyph(codepoint as i32) != 0,
            None => true,
        }
    }

    /// The emoji drawn `points` tall, rasterised at the output scale.
    fn image(&mut self, text: &str, points: f32) -> Option<Image> {
        let key = (text.to_string(), points.to_bits());
        if let Some(image) = self.images.get(&key) {
            return Some(image.clone());
        }
        let typeface = self.typeface.clone().or_else(|| {
            self.font_mgr
                .legacy_make_typeface(None, FontStyle::normal())
        })?;
        // The raster is a little wider than the glyph is tall: some emoji
        // (flags, keycaps) overhang their advance.
        let side = (points * 1.3 * self.scale).ceil() as i32;
        let font = Font::from_typeface(typeface, points * self.scale);
        let mut handler = TextBlobBuilderRunHandler::new(text, (0.0, 0.0));
        // One run, one font, whatever the sequence: the default iterators
        // split a joiner or a variation selector from the emoji beside it
        // and hand the pieces to HarfBuzz separately, which is how a family
        // comes out as four people and a white flag as a box and a flag.
        let bytes = text.len();
        let mut fonts = Shaper::new_trivial_font_run_iterator(&font, bytes);
        let mut bidi = skia_safe::shapers::primitive::trivial_bidi_run_iterator(0, bytes);
        let mut script = skia_safe::shapers::primitive::trivial_script_run_iterator(0, bytes);
        let mut language = Shaper::new_trivial_language_run_iterator("und", bytes);
        self.shaper.shape_with_iterators(
            text,
            &mut fonts,
            &mut bidi,
            &mut script,
            &mut language,
            f32::INFINITY,
            &mut handler,
        );
        let blob = handler.make_blob()?;

        let mut surface = skia_safe::surfaces::raster_n32_premul((side, side))?;
        let canvas = surface.canvas();
        canvas.clear(Color::TRANSPARENT);
        // The run handler lays the text out as a line — the baseline is
        // already an ascent down from the origin — so the blob's bounds are
        // the ink, and centring them is centring the emoji.
        let bounds = blob.bounds();
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        canvas.draw_text_blob(
            &blob,
            (
                (side as f32 - bounds.width()) / 2.0 - bounds.left,
                (side as f32 - bounds.height()) / 2.0 - bounds.top,
            ),
            &paint,
        );
        let image = surface.image_snapshot();
        self.images.insert(key, image.clone());
        Some(image)
    }
}

pub struct Palette {
    engine: Arc<Engine>,
    card: Layer,
    field: Layer,
    dividers: [Layer; 3],
    tabs: Layer,
    tab_marker: Layer,
    grid_clip: Layer,
    grid: Layer,
    highlight: Layer,
    footer: Layer,
    atlas: Atlas,

    size: (f32, f32),
    dark: bool,
    /// The desktop's text cursor, when it is known: the card sits beside it
    /// rather than in the middle of the output.
    caret: Option<(f32, f32, f32, f32)>,
    /// The images for the tab strip, made once.
    tab_icons: Vec<Option<Image>>,
    marker_placed: bool,
    highlight_placed: bool,
}

impl Palette {
    /// `card_parent` is the layer node of the card subsurface. The fullscreen
    /// parent surface has no scene of its own. `scale` is the output's, for
    /// the emoji rasters.
    pub fn new(engine: Arc<Engine>, card_parent: Option<&Layer>, dark: bool, scale: f32) -> Self {
        let new_layer = |key: &str| {
            let layer = engine.new_layer();
            layer.set_key(key);
            layer.set_layout_style(taffy::Style {
                position: taffy::style::Position::Absolute,
                ..Default::default()
            });
            layer
        };

        let card = new_layer("emoji-card");
        match card_parent {
            Some(parent) => {
                let _ = parent.add_sublayer(&card);
            }
            None => {
                let _ = engine.add_layer(&card);
            }
        }

        let field = new_layer("emoji-field");
        let dividers = [
            new_layer("emoji-divider"),
            new_layer("emoji-divider"),
            new_layer("emoji-divider"),
        ];
        let tabs = new_layer("emoji-tabs");
        let tab_marker = new_layer("emoji-tab-marker");
        let grid_clip = new_layer("emoji-grid-clip");
        let grid = new_layer("emoji-grid");
        let highlight = new_layer("emoji-highlight");
        let footer = new_layer("emoji-footer");

        let _ = card.add_sublayer(&field);
        for divider in &dividers {
            let _ = card.add_sublayer(divider);
        }
        // The marker before the icons, so it is behind them.
        let _ = card.add_sublayer(&tab_marker);
        let _ = card.add_sublayer(&tabs);
        let _ = card.add_sublayer(&grid_clip);
        let _ = grid_clip.add_sublayer(&highlight);
        let _ = grid_clip.add_sublayer(&grid);
        let _ = card.add_sublayer(&footer);

        let mut atlas = Atlas::new(scale);
        let recent_icon = "\u{1F558}".to_string();
        let tab_icons = std::iter::once(recent_icon)
            .chain(GROUPS.iter().map(|group| group.icon()))
            .map(|icon| atlas.image(&icon, TAB_EMOJI_PT))
            .collect();

        let mut palette = Self {
            engine,
            card,
            field,
            dividers,
            tabs,
            tab_marker,
            grid_clip,
            grid,
            highlight,
            footer,
            atlas,
            size: (0.0, 0.0),
            dark,
            caret: None,
            tab_icons,
            marker_placed: false,
            highlight_placed: false,
        };
        palette.style();
        palette.place();
        palette
    }

    /// Whether the emoji font can draw an emoji starting with `codepoint`.
    pub fn can_draw(&self, codepoint: u32) -> bool {
        self.atlas.can_draw(codepoint)
    }

    /// Switch the colour scheme. The portal answers after the palette is
    /// built, so this is the normal path into dark, not an edge case.
    pub fn set_dark(&mut self, dark: bool) {
        if self.dark == dark {
            return;
        }
        self.dark = dark;
        self.style();
    }

    /// The card's scene root, for a host that draws it directly — the
    /// preview example renders this subtree into a raster surface.
    pub fn card_layer(&self) -> &Layer {
        &self.card
    }

    fn title_color(&self) -> Color {
        if self.dark {
            Color::from_argb(240, 255, 255, 255)
        } else {
            Color::from_argb(240, 12, 12, 14)
        }
    }

    fn subtitle_color(&self) -> Color {
        if self.dark {
            Color::from_argb(150, 255, 255, 255)
        } else {
            Color::from_argb(140, 0, 0, 0)
        }
    }

    /// Colours and radii, set once per scheme.
    fn style(&mut self) {
        let hairline = lay_color(if self.dark {
            Color::from_argb(36, 255, 255, 255)
        } else {
            Color::from_argb(24, 0, 0, 0)
        });
        for divider in &self.dividers {
            divider.set_background_color(PaintColor::Solid { color: hairline }, None);
        }
        let wash = lay_color(if self.dark {
            Color::from_argb(46, 255, 255, 255)
        } else {
            Color::from_argb(20, 0, 0, 0)
        });
        self.highlight
            .set_background_color(PaintColor::Solid { color: wash }, None);
        self.highlight
            .set_border_corner_radius(BorderRadius::new_single(9.0), None);
        self.tab_marker
            .set_background_color(PaintColor::Solid { color: wash }, None);
        self.tab_marker
            .set_border_corner_radius(BorderRadius::new_single(8.0), None);
    }

    /// Everything that does not move: the card is a fixed shape.
    fn place(&mut self) {
        self.card.set_size(LayerSize::points(CARD_W, CARD_H), None);
        self.field
            .set_size(LayerSize::points(CARD_W, FIELD_H), None);

        let divider_ys = [FIELD_H, FIELD_H + 1.0 + TABS_H, GRID_Y + GRID_H + GRID_PAD];
        for (divider, y) in self.dividers.iter().zip(divider_ys) {
            divider.set_position(LayerPoint { x: 0.0, y }, None);
            divider.set_size(LayerSize::points(CARD_W, 1.0), None);
        }

        let tabs_y = FIELD_H + 1.0;
        self.tabs
            .set_position(LayerPoint { x: 0.0, y: tabs_y }, None);
        self.tabs.set_size(LayerSize::points(CARD_W, TABS_H), None);
        self.tab_marker
            .set_size(LayerSize::points(TAB_W - 6.0, TABS_H - 10.0), None);
        self.tab_marker.set_opacity(0.0_f32, None);

        self.grid_clip
            .set_position(LayerPoint { x: 0.0, y: GRID_Y }, None);
        self.grid_clip
            .set_size(LayerSize::points(CARD_W, GRID_H), None);
        self.grid_clip.set_clip_children(true, None);
        self.grid.set_size(LayerSize::points(CARD_W, GRID_H), None);
        self.highlight
            .set_size(LayerSize::points(CELL - 4.0, CELL - 4.0), None);
        self.highlight.set_opacity(0.0_f32, None);

        self.footer.set_position(
            LayerPoint {
                x: 0.0,
                y: FOOTER_Y,
            },
            None,
        );
        self.footer
            .set_size(LayerSize::points(CARD_W, FOOTER_H), None);
    }

    /// The surface's size changed.
    pub fn set_size(&mut self, width: f32, height: f32) {
        if (width, height) == self.size {
            return;
        }
        self.size = (width, height);
        self.engine.scene_set_size(width, height);
    }

    /// Where the card subsurface belongs, in the parent surface's coordinates.
    pub fn card_origin(&self) -> (f32, f32) {
        self.placement().origin
    }

    /// Where the card sits and what it points at.
    pub fn placement(&self) -> Placement {
        match self.caret {
            Some(caret) => placement_at_caret(self.size, caret),
            None => placement_centred(self.size),
        }
    }

    /// Place the card at the desktop's text cursor, or centred again when
    /// `caret` is `None`.
    ///
    /// The scene follows: when the beak is on the top edge the body starts a
    /// beak's reach down the surface, and everything drawn moves with it.
    pub fn set_caret(&mut self, caret: Option<(f32, f32, f32, f32)>) {
        self.caret = caret;
        let (x, y) = self.placement().body_offset();
        self.card.set_position(LayerPoint { x, y }, None);
    }

    /// The card as the compositor should hit-test it — see [`input_rect`].
    pub fn input_rect(&self) -> (i32, i32, i32, i32) {
        let (x, y) = self.card_origin();
        (
            x.floor() as i32,
            y.floor() as i32,
            CARD_W.ceil() as i32,
            CARD_H.ceil() as i32,
        )
    }

    /// Push the query field into the scene.
    pub fn update_field(&mut self, input: &TextInput) {
        let field = input.clone();
        self.field
            .set_draw_content(move |canvas: &Canvas, width: f32, height: f32| {
                field.render_at(canvas, width, height);
                Rect::from_wh(width, height)
            });
    }

    /// Push the tab strip: `active` is the tab the marker sits under, or
    /// `None` while a search is showing and no tab applies.
    pub fn update_tabs(&mut self, active: Option<usize>) {
        let icons = self.tab_icons.clone();
        self.tabs
            .set_draw_content(move |canvas: &Canvas, _width: f32, height: f32| {
                let side = TAB_EMOJI_PT * 1.3;
                for (index, icon) in icons.iter().enumerate() {
                    let Some(icon) = icon else { continue };
                    let x = PAD + index as f32 * TAB_W + (TAB_W - side) / 2.0;
                    let y = (height - side) / 2.0;
                    let mut paint = Paint::default();
                    paint.set_alpha_f(if active == Some(index) { 1.0 } else { 0.62 });
                    canvas.draw_image_rect_with_sampling_options(
                        icon,
                        None,
                        Rect::from_xywh(x, y, side, side),
                        SamplingOptions::from(skia_safe::FilterMode::Linear),
                        &paint,
                    );
                }
                Rect::from_wh(CARD_W, height)
            });

        match active {
            Some(tab) => {
                let x = PAD + tab as f32 * TAB_W + 3.0;
                let y = 5.0 + FIELD_H + 1.0;
                // The first placement is a jump: nothing to slide from.
                let transition = self.marker_placed.then(|| Transition::ease_out_quad(0.16));
                self.tab_marker
                    .set_position(LayerPoint { x, y }, transition);
                self.tab_marker.set_opacity(1.0_f32, None);
                self.marker_placed = true;
            }
            None => {
                self.tab_marker.set_opacity(0.0_f32, None);
                self.marker_placed = false;
            }
        }
    }

    /// Push the grid: the panes panned to `pan`, each scrolled by its own
    /// entry in `scrolls`, with `selected` highlighted. `empty_message` is
    /// shown instead of a grid with nothing in it.
    pub fn update_grid(
        &mut self,
        cells: &[Cell],
        layout: &Layout,
        pan: f32,
        scrolls: &[f32],
        selected: Option<usize>,
        empty_message: Option<&str>,
    ) {
        // Only the panes on screen, and within them only the rows on screen,
        // with a cell of slack all round so a partly visible one is drawn
        // whole.
        let mut images: Vec<(Rect, Image)> = Vec::new();
        for (index, pane) in layout.panes.iter().enumerate() {
            let origin = Layout::pane_origin(index) - pan;
            if origin + CARD_W < -CELL || origin > CARD_W + CELL {
                continue;
            }
            let scroll = scrolls.get(index).copied().unwrap_or(0.0);
            let (top, bottom) = (scroll - CELL, scroll + GRID_H + CELL);
            for cell in pane.first..pane.first + pane.count {
                let Some(rect) = layout.cell_rect(cell) else {
                    continue;
                };
                if rect.bottom < top || rect.top > bottom {
                    continue;
                }
                let Some(cell) = cells.get(cell) else {
                    continue;
                };
                if let Some(image) = self.atlas.image(&cell.text, EMOJI_PT) {
                    let side = EMOJI_PT * 1.3;
                    let dst = Rect::from_xywh(
                        origin + rect.left + (CELL - side) / 2.0,
                        rect.top - scroll + (CELL - side) / 2.0,
                        side,
                        side,
                    );
                    images.push((dst, image));
                }
            }
        }

        let message_font = styles::BODY.font();
        let message_color = self.subtitle_color();
        let message = empty_message.map(str::to_string);
        self.grid
            .set_draw_content(move |canvas: &Canvas, width: f32, height: f32| {
                let mut paint = Paint::default();
                paint.set_anti_alias(true);
                for (dst, image) in &images {
                    canvas.draw_image_rect_with_sampling_options(
                        image,
                        None,
                        *dst,
                        SamplingOptions::from(skia_safe::FilterMode::Linear),
                        &paint,
                    );
                }
                if let Some(message) = &message {
                    let mut text_paint = Paint::new(Color4f::from(message_color), None);
                    text_paint.set_anti_alias(true);
                    let text_width = message_font.measure_str(message, Some(&text_paint)).0;
                    canvas.draw_str(
                        message,
                        ((width - text_width) / 2.0, height / 2.0 + 5.0),
                        &message_font,
                        &text_paint,
                    );
                }
                Rect::from_wh(width, height)
            });

        let spot = selected.and_then(|cell| {
            let (pane, _, _) = layout.place(cell)?;
            let rect = layout.cell_rect(cell)?;
            let scroll = scrolls.get(pane).copied().unwrap_or(0.0);
            Some(LayerPoint {
                x: Layout::pane_origin(pane) - pan + rect.left + 2.0,
                y: rect.top - scroll + 2.0,
            })
        });
        match spot {
            Some(position) => {
                let transition = self
                    .highlight_placed
                    .then(|| Transition::ease_out_quad(0.1));
                self.highlight.set_position(position, transition);
                self.highlight.set_opacity(1.0_f32, None);
                self.highlight_placed = true;
            }
            None => {
                self.highlight.set_opacity(0.0_f32, None);
                self.highlight_placed = false;
            }
        }
    }

    /// Push the footer: the name of the selected emoji, and the tone
    /// swatches with `tone` marked.
    pub fn update_footer(&mut self, name: &str, tone: Tone) {
        let font = styles::CALLOUT.font();
        let name = name.to_string();
        let title = self.title_color();
        let ring = self.title_color();
        let x0 = swatch_x0();
        self.footer
            .set_draw_content(move |canvas: &Canvas, _width: f32, height: f32| {
                let mut paint = Paint::new(Color4f::from(title), None);
                paint.set_anti_alias(true);
                let limit = x0 - PAD - 8.0;
                let shown = otto_kit::typography::ellipsize(&font, &name, limit - PAD - 4.0);
                canvas.draw_str(&shown, (PAD + 4.0, height / 2.0 + 5.0), &font, &paint);

                for (index, swatch) in Tone::ALL.iter().enumerate() {
                    let (r, g, b) = swatch.swatch();
                    let cx = x0 + index as f32 * SWATCH_PITCH + SWATCH / 2.0;
                    let cy = height / 2.0;
                    let mut fill = Paint::new(Color4f::from(Color::from_rgb(r, g, b)), None);
                    fill.set_anti_alias(true);
                    canvas.draw_circle((cx, cy), SWATCH / 2.0, &fill);
                    if *swatch == tone {
                        let mut stroke = Paint::new(Color4f::from(ring), None);
                        stroke.set_anti_alias(true);
                        stroke.set_style(skia_safe::PaintStyle::Stroke);
                        stroke.set_stroke_width(2.0);
                        canvas.draw_circle((cx, cy), SWATCH / 2.0 + 2.5, &stroke);
                    }
                }
                Rect::from_wh(CARD_W, height)
            });
    }

    /// Get a font for a caller that draws beside the palette.
    pub fn font(&self, size: f32, style: FontStyle) -> Font {
        get_font_with_fallback(styles::BODY.family, style, size)
    }
}

/// The query field's look: no box of its own, because it already sits in one.
pub fn field_style(dark: bool) -> TextInputStyle {
    let mut style = TextInputStyle::with_theme(if dark { Theme::dark() } else { Theme::light() });
    style.text_style = styles::TITLE_3;
    style.text_style.size = 17.0;
    style.horizontal_padding = PAD + 4.0;
    style.corner_radius = 0.0;
    style.focus_ring_width = 0.0;
    style.background = Color::TRANSPARENT;
    style.text_color = if dark {
        Color::from_argb(245, 255, 255, 255)
    } else {
        Color::from_argb(245, 10, 10, 12)
    };
    style.placeholder_color = if dark {
        Color::from_argb(110, 255, 255, 255)
    } else {
        Color::from_argb(100, 0, 0, 0)
    };
    style
}

fn lay_color(color: Color) -> LayerColor {
    LayerColor::new_rgba255(color.r(), color.g(), color.b(), color.a())
}

/// Whether the system has any emoji font at all, checked without a scene.
pub fn has_emoji_font() -> bool {
    get_font("Noto Color Emoji", FontStyle::normal(), 12.0).is_some()
        || FontMgr::new()
            .match_family_style_character("", FontStyle::normal(), &[], 0x1F600)
            .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout_of(counts: &[usize]) -> Layout {
        let mut panes = Vec::new();
        let mut first = 0;
        for (tab, count) in counts.iter().enumerate() {
            panes.push(Pane {
                tab,
                first,
                count: *count,
            });
            first += count;
        }
        Layout::new(panes)
    }

    #[test]
    fn a_pane_wraps_its_cells_into_rows() {
        let layout = layout_of(&[12, 3]);
        assert_eq!(layout.pane_count(), 2);
        assert_eq!(layout.cell_count(), 15);
        assert_eq!(layout.panes[0].rows(), 2);
        assert_eq!(layout.panes[1].rows(), 1);
        assert_eq!(layout.pane_height(0), CELL * 2.0);
        // Cell 11 is the second row of the first pane, second column.
        assert_eq!(layout.place(11), Some((0, 1, 1)));
        // Cell 12 opens the second pane.
        assert_eq!(layout.place(12), Some((1, 0, 0)));
    }

    #[test]
    fn panes_sit_side_by_side() {
        let layout = layout_of(&[10, 10, 10]);
        assert_eq!(Layout::pane_origin(0), 0.0);
        assert_eq!(Layout::pane_origin(2), CARD_W * 2.0);
        assert_eq!(layout.content_width(), CARD_W * 3.0);
        assert_eq!(layout.max_pan(), CARD_W * 2.0);
        // The pane under the viewport is the nearest one, so a pan that has
        // come to rest just past half way reads as the next pane.
        assert_eq!(layout.pane_at_pan(0.0), 0);
        assert_eq!(layout.pane_at_pan(CARD_W * 0.6), 1);
        assert_eq!(layout.pane_at_pan(CARD_W * 9.0), 2, "clamped to the last");
    }

    #[test]
    fn cells_are_found_where_they_are_drawn() {
        let layout = layout_of(&[12, 5]);
        // First pane, no pan: cell 11 sits on the second row.
        let rect = layout.cell_rect(11).unwrap();
        assert_eq!(
            layout.cell_at(rect.center_x(), rect.center_y(), 0.0, 0.0),
            Some(11)
        );
        // Column 5 of that short row holds nothing.
        assert_eq!(
            layout.cell_at(PAD + 5.5 * CELL, rect.center_y(), 0.0, 0.0),
            None
        );
        // The inset either side of the cells is not a cell.
        assert_eq!(layout.cell_at(1.0, CELL / 2.0, 0.0, 0.0), None);
        // Panned to the second pane, the same screen point is its first cell.
        assert_eq!(
            layout.cell_at(PAD + CELL / 2.0, CELL / 2.0, CARD_W, 0.0),
            Some(12)
        );
        // And that pane's own vertical scroll moves what is under the point.
        assert_eq!(
            layout.cell_at(PAD + CELL / 2.0, CELL / 2.0, CARD_W, CELL),
            None,
            "the second pane has only one row, scrolled past"
        );
    }

    #[test]
    fn a_short_row_catches_the_column_moving_onto_it() {
        let layout = layout_of(&[13]);
        // Row 1 holds three cells; asking for column 7 lands on the last.
        assert_eq!(layout.cell_in(0, 1, 7), Some(12));
        assert_eq!(layout.cell_in(0, 1, 1), Some(11));
        assert_eq!(layout.cell_in(0, 2, 0), None, "there is no third row");
    }

    #[test]
    fn a_pane_shorter_than_the_grid_does_not_scroll() {
        let layout = layout_of(&[5, 400]);
        assert_eq!(layout.max_scroll(0), 0.0);
        assert!(layout.max_scroll(1) > 0.0);
        assert_eq!(layout.max_scroll(1), layout.pane_height(1) - GRID_H);
    }

    #[test]
    fn a_pane_knows_its_tab_and_back() {
        let layout = layout_of(&[4, 4, 4]);
        assert_eq!(layout.pane_tab(2), Some(2));
        assert_eq!(layout.pane_for_tab(1), Some(1));
        assert_eq!(layout.pane_for_tab(9), None);
    }

    #[test]
    fn tabs_and_tones_are_hit_where_they_are_drawn() {
        assert_eq!(tab_at(PAD + 0.5 * TAB_W), Some(0));
        assert_eq!(tab_at(PAD + (TABS as f32 - 0.5) * TAB_W), Some(TABS - 1));
        assert_eq!(tab_at(1.0), None);
        assert_eq!(tab_at(CARD_W - 1.0), None);

        assert_eq!(tone_at(swatch_x0() + SWATCH / 2.0), Some(Tone::None));
        assert_eq!(
            tone_at(swatch_x0() + 5.0 * SWATCH_PITCH + SWATCH / 2.0),
            Some(Tone::Dark)
        );
        assert_eq!(tone_at(PAD + 10.0), None);
    }

    #[test]
    fn the_card_points_at_the_caret_from_below() {
        let p = placement_at_caret((1920.0, 1080.0), (400.0, 200.0, 2.0, 18.0));
        let ((bx, by), edge) = p.beak.expect("a caret is known, so it points at it");
        assert_eq!(edge, Beak::Top, "the card is below, so it points up");
        assert_eq!(by, p.origin.1, "the beak sits on the card's top edge");
        assert_eq!(bx, 401.0, "and at the middle of the caret");
        assert!(
            p.origin.0 < 401.0,
            "the card is offset so the beak reaches the caret, not butted against it"
        );
    }

    #[test]
    fn the_card_points_down_when_it_sits_above() {
        let p = placement_at_caret((1920.0, 1080.0), (400.0, 900.0, 2.0, 18.0));
        let ((_, by), edge) = p.beak.expect("beak");
        assert_eq!(edge, Beak::Bottom);
        assert_eq!(by, p.origin.1 + CARD_H, "on the card's bottom edge");
    }

    #[test]
    fn the_card_grows_into_whichever_side_has_room() {
        let screen = (1920.0, 1080.0);
        // Room to the right: the card grows rightwards from the caret.
        let left = placement_at_caret(screen, (300.0, 200.0, 2.0, 18.0));
        assert!(left.origin.0 < 301.0 && left.origin.0 > 301.0 - CARD_W / 2.0);
        let ((bx, _), _) = left.beak.expect("beak");
        assert_eq!(bx, 301.0, "pointing at the caret");
        // No room: it ends at the caret instead of hanging off.
        let right = placement_at_caret(screen, (1850.0, 200.0, 2.0, 18.0));
        assert!(right.origin.0 + CARD_W <= 1920.0 - SCREEN_MARGIN);
        assert!(right.origin.0 < 1850.0, "right-aligned with the caret");
    }

    #[test]
    fn the_beak_stays_on_the_straight_part_of_the_edge() {
        let screen = (1920.0, 1080.0);
        // A caret hard against the right edge drags the card left, but the
        // beak must not end up in the rounded corner.
        let p = placement_at_caret(screen, (1918.0, 200.0, 2.0, 18.0));
        let ((bx, _), _) = p.beak.expect("beak");
        let inset = otto_kit::corners::radius(RADIUS) + BEAK / 2.0;
        assert!(
            bx >= p.origin.0 + inset - 0.5 && bx <= p.origin.0 + CARD_W - inset + 0.5,
            "beak at {bx} is off the straight edge of a card at {}",
            p.origin.0
        );
    }

    #[test]
    fn the_balloon_surface_holds_the_body_and_the_beak() {
        let screen = (1920.0, 1080.0);
        // Pointing up: the surface starts a beak above the body, and the body
        // is pushed down inside it by the same amount.
        let up = placement_at_caret(screen, (400.0, 200.0, 2.0, 18.0));
        assert_eq!(up.beak.map(|(_, edge)| edge), Some(Beak::Top));
        assert_eq!(up.surface_size(), (CARD_W, CARD_H + BEAK_REACH));
        assert_eq!(up.surface_origin(), (up.origin.0, up.origin.1 - BEAK_REACH));
        assert_eq!(up.body_offset(), (0.0, BEAK_REACH));

        // Pointing down: the beak is past the body, so the surface starts with
        // it and nothing is offset.
        let down = placement_at_caret(screen, (400.0, 900.0, 2.0, 18.0));
        assert_eq!(down.beak.map(|(_, edge)| edge), Some(Beak::Bottom));
        assert_eq!(down.surface_origin(), down.origin);
        assert_eq!(down.body_offset(), (0.0, 0.0));

        // The offset handed to the compositor is measured along the edge from
        // the surface's corner, and lands on the caret.
        let ((bx, _), _) = up.beak.unwrap();
        assert_eq!(up.beak_offset(), Some(bx - up.surface_origin().0));
    }

    #[test]
    fn a_card_with_no_caret_is_a_plain_rectangle() {
        let p = placement_centred((1920.0, 1080.0));
        assert_eq!(p.surface_size(), (CARD_W, CARD_H));
        assert_eq!(p.surface_origin(), p.origin);
        assert_eq!(p.body_offset(), (0.0, 0.0));
        assert_eq!(p.beak_offset(), None);
    }

    #[test]
    fn a_centred_card_points_at_nothing() {
        let p = placement_centred((1920.0, 1080.0));
        assert!(p.beak.is_none());
        assert_eq!(p.origin, card_origin((1920.0, 1080.0)));
    }

    #[test]
    fn the_card_sits_under_the_caret() {
        let screen = (1920.0, 1080.0);
        // A caret near the top: the card hangs below it, offset just enough
        // that its beak reaches the caret without sitting in a corner.
        let (x, y) = card_origin_at_caret(screen, (400.0, 200.0, 2.0, 18.0));
        assert_eq!(y, 200.0 + 18.0 + CARET_GAP);
        assert!(x < 401.0 && x > 401.0 - CARD_W / 2.0, "got {x}");
    }

    #[test]
    fn a_caret_near_the_bottom_flips_the_card_above_it() {
        let screen = (1920.0, 1080.0);
        let caret_y = 900.0;
        let (_, y) = card_origin_at_caret(screen, (400.0, caret_y, 2.0, 18.0));
        assert!(
            y + CARD_H <= caret_y,
            "the card should clear the caret, not cover it"
        );
        assert_eq!(y, caret_y - CARET_GAP - CARD_H);
    }

    #[test]
    fn the_card_is_kept_on_screen() {
        let screen = (1920.0, 1080.0);
        // Hard against the right edge: slid back rather than hanging off.
        let (x, _) = card_origin_at_caret(screen, (1900.0, 200.0, 2.0, 18.0));
        assert!(x + CARD_W <= 1920.0);
        // And against the left.
        let (x, _) = card_origin_at_caret(screen, (-50.0, 200.0, 2.0, 18.0));
        assert!(x >= 0.0);
        // A screen too short for the card anywhere still gives a position on
        // it rather than a negative one.
        let (_, y) = card_origin_at_caret((1920.0, 200.0), (400.0, 100.0, 2.0, 18.0));
        assert!(y >= 0.0);
        // And the card never sits in the strip the dock occupies.
        let (_, y) = card_origin_at_caret((1920.0, 1080.0), (400.0, 600.0, 2.0, 18.0));
        assert!(
            y + CARD_H <= 1080.0 - BOTTOM_MARGIN,
            "the card should stay clear of the dock, got {y}"
        );
    }

    #[test]
    fn the_card_is_centred_above_the_middle() {
        let (x, y) = card_origin((1920.0, 1080.0));
        assert_eq!(x, ((1920.0 - CARD_W) / 2.0_f32).round());
        assert!(y < 1080.0 / 2.0 - CARD_H / 2.0);
        let (ix, iy, iw, ih) = input_rect((1920.0, 1080.0));
        assert_eq!((ix, iy), (x as i32, y as i32));
        assert_eq!((iw, ih), (CARD_W.ceil() as i32, CARD_H.ceil() as i32));
    }
}
