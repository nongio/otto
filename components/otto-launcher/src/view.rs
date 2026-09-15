//! The launcher as a `lay-rs` scene.
//!
//! Built the way otto-kit's other panels are: a tree of layers positioned once
//! and then *changed*, rather than a canvas redrawn from scratch. That is what
//! buys what an immediate-mode launcher could not have: the card grows and
//! shrinks with the list on a transition the engine runs without the app
//! driving frames by hand.
//!
//! There is deliberately no dimming behind the card. A scrim painted by this
//! client would sit between the desktop and the card, and the compositor's
//! blur samples exactly that — the frost would be a blur of flat grey, and the
//! desktop behind it would vanish rather than soften. The parent surface stays
//! transparent, and the card is separated by its own material and shadow.
//!
//! The card's *material* is not drawn here at all. A client cannot blur the
//! desktop behind itself — only the compositor can see what is back there — so
//! the card is its own subsurface and its frost, corner radius, border and
//! shadow are asked for through `otto-surface-style`, exactly as otto-bar's
//! menus and the islands do. What this file draws onto that material is the
//! query, the divider, and the rows.
//!
//! The result rows are not in this scene. They are a scroll pane over the card
//! — see `main.rs` — whose band [`Palette::paint_rows`] paints, with the
//! selection a highlight the pane slides under them: scrolling the list or
//! moving the selection repaints neither the card nor the rows. Layout is in
//! logical points and absolute — a row appearing must not move the field being
//! typed into.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use layers::prelude::*;
use layers::types::{Color as LayerColor, Point as LayerPoint, Size as LayerSize};
use otto_kit::components::scroll::RowLayout;
use otto_kit::components::text_input::{TextInput, TextInputStyle};
use otto_kit::icons::named_icon_sized;
use otto_kit::theme::Theme;
use otto_kit::typography::{draw_runs, get_font_with_fallback, measure_runs, styles};
use skia_safe::font_style::{Slant, Weight, Width};
use skia_safe::{Canvas, Color, Color4f, Font, FontStyle, Image, Paint, Rect, SamplingOptions};

use crate::log::{Line, Style};
use crate::source::Item;

/// Width of the card. Wide enough for a window title and its application, and
/// narrow enough to stay a dialog rather than become a page.
pub const CARD_W: f32 = 620.0;
/// Height of the query field.
pub const FIELD_H: f32 = 58.0;
/// Height of one result row.
pub const ROW_H: f32 = 46.0;
/// How many rows are on screen at once. Past this the list scrolls, because a
/// list taller than this stops being scannable and the card starts to be the
/// screen.
pub const MAX_ROWS: usize = 8;
/// Padding above and below the list.
const LIST_PAD: f32 = 8.0;
/// Where the list starts, down the card.
pub const LIST_TOP: f32 = FIELD_H + 1.0 + LIST_PAD;
/// Corner radius of the card, applied by the compositor to the subsurface.
pub const RADIUS: f32 = 10.0;
const ICON: f32 = 28.0;
/// The icon on a compact row, such as a file attached to an ask request.
const SMALL_ICON: f32 = 18.0;
const ROW_INSET: f32 = 8.0;
/// Corner radius of the selection's highlight.
pub const HIGHLIGHT_RADIUS: f32 = 9.0;
/// The card at its tallest, which is how big its buffer is allocated: a
/// shorter card is the same buffer with the compositor clipping it, so the
/// height can change without reallocating anything.
pub const MAX_CARD_H: f32 = MAX_LOG_BLOCK + FIELD_H + 1.0 + LIST_PAD * 2.0 + MAX_LIST_H;
/// The most of the list — rows, or the ask log — on screen at once.
const MAX_LIST_H: f32 = MAX_ROWS as f32 * ROW_H;
/// The ask log above the field at its tallest, with its padding and divider.
const MAX_LOG_BLOCK: f32 = log_block(MAX_LIST_H);

/// How much of the card an ask log `log` points tall takes above the field.
const fn log_block(log: f32) -> f32 {
    if log <= 0.0 {
        0.0
    } else {
        LOG_TOP_PAD + log + LIST_PAD + 1.0
    }
}

/// Space above the ask log, between the card's top edge and its first line.
const LOG_TOP_PAD: f32 = 20.0;

/// Height of one line of the ask log.
pub const LOG_LINE_H: f32 = 21.0;
/// Size of the ask log's text.
const LOG_TEXT: f32 = 14.0;
/// Space either side of the ask log's text.
const LOG_INSET: f32 = 20.0;
/// How wide a line of the ask log may run.
pub const LOG_W: f32 = CARD_W - LOG_INSET * 2.0;

/// How tall the ask log is with `lines` lines in it.
pub fn log_length(lines: usize) -> f32 {
    lines as f32 * LOG_LINE_H
}

/// Where the top of the card sits, as a fraction of the output's height.
/// Above centre: the eye starts there, and the list grows downwards into
/// space that was already empty.
const TOP_FRACTION: f32 = 0.16;

/// How tall the card is with `rows` rows on it, in logical points.
///
/// Free of the [`Palette`] on purpose. This is the shape the compositor is
/// told to hit-test the pointer against, and the launcher has to be able to
/// work it out from a row count alone — the buffer it is drawn into says
/// nothing about it.
pub fn card_height(rows: usize) -> f32 {
    card_height_for(rows as f32 * ROW_H)
}

/// How tall the card is with `list` points of list under the field.
fn card_height_for(list: f32) -> f32 {
    if list <= 0.0 {
        // No list, so no divider and no padding around it either: the card is
        // the field.
        return FIELD_H;
    }
    FIELD_H + 1.0 + LIST_PAD * 2.0 + list
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

/// The rectangle the launcher should take pointer input over, in the parent
/// surface's coordinates: the card as it is drawn, and nothing else.
///
/// Neither surface's own geometry answers this. The card's buffer is allocated
/// at [`MAX_CARD_H`] however few rows are on it, and the parent surface covers
/// the whole output — so a launcher that says nothing about its input region
/// claims the screen with one surface and eight rows it is not showing with
/// the other. The shadow is deliberately outside the rectangle: a shadow is
/// something to see past, not something to click.
///
/// Rounded outwards, because a region is in whole pixels and a card that lost
/// its bottom row of them would have a one-pixel dead strip along the edge the
/// pointer arrives at.
pub fn input_rect(surface: (f32, f32), rows: usize) -> (i32, i32, i32, i32) {
    input_rect_for(surface, rows as f32 * ROW_H)
}

fn input_rect_for(surface: (f32, f32), list: f32) -> (i32, i32, i32, i32) {
    let (x, y) = card_origin(surface);
    outward(x, y, CARD_W, card_height_for(list))
}

/// A rectangle in points as whole pixels, rounded outwards.
fn outward(x: f32, y: f32, width: f32, height: f32) -> (i32, i32, i32, i32) {
    let (left, top) = (x.floor(), y.floor());
    (
        left as i32,
        top as i32,
        ((x + width).ceil() - left) as i32,
        ((y + height).ceil() - top) as i32,
    )
}

pub struct Palette {
    engine: Arc<Engine>,
    /// Root of the card subsurface's scene. Its own background is left to the
    /// compositor — see the module docs.
    card: Layer,
    field: Layer,
    divider: Layer,
    /// The line that stands in for the list when nothing matched.
    message: Layer,
    /// The line between the ask log and the field.
    log_divider: Layer,

    size: (f32, f32),
    /// How much of the list is under the field, in points: the rows on
    /// screen, or the message's line.
    list_h: f32,
    /// How tall the rows' pane is: `list_h`, unless that is the message.
    pane_h: f32,
    /// How much of the ask log is on screen above the field, in points.
    log_h: f32,
    /// Whether the field sits low enough to leave the ask log room above it.
    log_room: bool,
    /// How far the card has been dragged from where it rests, in points.
    moved: (f32, f32),
    /// Icons live as long as the launcher does. It is open for seconds, and
    /// decoding the same icon on every keystroke is the one thing that would
    /// make typing feel slow. Behind a cell because painting a band only
    /// borrows the palette.
    icons: RefCell<HashMap<String, Option<Image>>>,
    dark: bool,
}

impl Palette {
    /// `card_parent` is the layer node of the card subsurface. The fullscreen
    /// parent surface has no scene of its own — see the module docs.
    pub fn new(engine: Arc<Engine>, card_parent: Option<&Layer>, dark: bool) -> Self {
        let new_layer = |key: &str| {
            let layer = engine.new_layer();
            layer.set_key(key);
            layer.set_layout_style(taffy::Style {
                position: taffy::style::Position::Absolute,
                ..Default::default()
            });
            layer
        };

        let card = new_layer("launcher-card");
        match card_parent {
            Some(parent) => {
                let _ = parent.add_sublayer(&card);
            }
            None => {
                let _ = engine.add_layer(&card);
            }
        }

        let field = new_layer("launcher-field");
        let divider = new_layer("launcher-divider");
        let message = new_layer("launcher-message");
        let log_divider = new_layer("launcher-log-divider");

        let _ = card.add_sublayer(&field);
        let _ = card.add_sublayer(&divider);
        let _ = card.add_sublayer(&message);
        let _ = card.add_sublayer(&log_divider);

        let mut palette = Self {
            engine,
            card,
            field,
            divider,
            message,
            log_divider,
            size: (0.0, 0.0),
            list_h: 0.0,
            pane_h: 0.0,
            log_h: 0.0,
            log_room: false,
            moved: (0.0, 0.0),
            icons: RefCell::new(HashMap::new()),
            dark,
        };
        palette.style();
        palette
    }

    /// Switch the colour scheme. The portal answers after the palette is
    /// built, so this is the normal path into dark, not an edge case. Only the
    /// two layer colours are set here — the row text reads `dark` on every
    /// `update`, and the caller marks itself dirty.
    pub fn set_dark(&mut self, dark: bool) {
        if self.dark == dark {
            return;
        }
        self.dark = dark;
        self.style();
    }

    /// The card's scene root, for a host that needs to draw it directly — the
    /// preview example renders this subtree into a raster surface.
    pub fn card_layer(&self) -> &Layer {
        &self.card
    }

    /// Colours and radii, set once — `update` only touches what changes.
    fn style(&mut self) {
        // The card itself paints nothing: the compositor draws the frosted
        // material under this scene, and anything painted here would sit on
        // top of it as a second, unblurred pane.
        self.card
            .set_size(LayerSize::points(CARD_W, MAX_CARD_H), None);

        let line = if self.dark {
            Color::from_argb(36, 255, 255, 255)
        } else {
            Color::from_argb(24, 0, 0, 0)
        };
        for divider in [&self.divider, &self.log_divider] {
            divider.set_background_color(
                PaintColor::Solid {
                    color: lay_color(line),
                },
                None,
            );
        }
    }

    /// The selection's wash, which the list pane draws under the rows.
    pub fn highlight_color(&self) -> Color {
        if self.dark {
            Color::from_argb(46, 255, 255, 255)
        } else {
            Color::from_argb(20, 0, 0, 0)
        }
    }

    /// Where the highlight goes for row `index`, in the list's content
    /// coordinates.
    pub fn highlight_rect(index: usize) -> Rect {
        Rect::from_xywh(
            ROW_INSET,
            index as f32 * ROW_H + 2.0,
            CARD_W - ROW_INSET * 2.0,
            ROW_H - 4.0,
        )
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

    /// The surface's size changed. Everything that does not depend on the
    /// result list is placed here.
    pub fn set_size(&mut self, width: f32, height: f32) {
        if (width, height) == self.size {
            return;
        }
        self.size = (width, height);

        self.engine.scene_set_size(width, height);

        // The card's scene starts at the origin of its own surface; where that
        // surface sits is [`Palette::card_origin`], which the app applies to
        // the subsurface.
        self.field
            .set_size(LayerSize::points(CARD_W, FIELD_H), None);
        self.divider.set_size(LayerSize::points(CARD_W, 1.0), None);
        self.log_divider
            .set_size(LayerSize::points(CARD_W, 1.0), None);

        self.apply_card_height(None);
    }

    /// Leave room above the field for the ask log, so the field being typed
    /// into stays where it is as the log appears and grows.
    pub fn reserve_log_room(&mut self) {
        self.set_log_room(true);
    }

    /// Whether to leave that room. A list of sessions has no log, and without
    /// the room it sits where the launcher's other lists do, higher up.
    pub fn set_log_room(&mut self, room: bool) {
        self.log_room = room;
    }

    /// Where the card subsurface belongs, in the parent surface's coordinates.
    ///
    /// With room for the ask log, the field sits where the tallest log would
    /// put it, and the card grows upwards over the space above as the log
    /// grows.
    ///
    /// A card that has been dragged sits that far from there, kept on the
    /// output.
    pub fn card_origin(&self) -> (f32, f32) {
        let (x, y) = self.resting_origin();
        let (width, height) = self.card_size();
        let (dx, dy) = self.moved;
        (
            (x + dx).clamp(0.0, (self.size.0 - width).max(0.0)),
            (y + dy).clamp(0.0, (self.size.1 - height).max(0.0)),
        )
    }

    /// Drag the card's top-left corner to `(x, y)` on the output, or as near
    /// as the output's edges allow. Absolute, as Files' palette is dragged:
    /// the pointer's position arrives relative to where the card was last
    /// placed, so adding up steps would count a step again for every event
    /// that arrives before the card has moved.
    pub fn move_card_to(&mut self, x: f32, y: f32) {
        let (rest_x, rest_y) = self.resting_origin();
        self.moved = (x - rest_x, y - rest_y);
        // What is kept is where the card ended up, so dragging past an edge
        // and back moves it straight away.
        let (x, y) = self.card_origin();
        self.moved = (x - rest_x, y - rest_y);
    }

    /// Whether a press at `y`, in the card's coordinates, lands on what the
    /// card is dragged by: the field, or the ask log above it.
    pub fn drags_at(&self, y: f32) -> bool {
        let field = self.field_top();
        let on_field = (field..field + FIELD_H).contains(&y);
        let log = self.log_rect();
        on_field || (self.log_h > 0.0 && (log.top..log.bottom).contains(&y))
    }

    /// Where the card sits before it is dragged anywhere.
    fn resting_origin(&self) -> (f32, f32) {
        let (x, y) = card_origin(self.size);
        if !self.log_room {
            return (x, y);
        }
        let under_field = MAX_CARD_H - MAX_LOG_BLOCK;
        let field = (y + MAX_LOG_BLOCK).min(self.size.1 - under_field).max(0.0);
        (x, (field - self.field_top()).max(0.0))
    }

    /// Where the field starts, down the card: under the ask log, if there is
    /// one.
    pub fn field_top(&self) -> f32 {
        log_block(self.log_h)
    }

    /// How much of the card is currently in use. The buffer stays
    /// [`MAX_CARD_H`] tall; this is what the compositor should show of it.
    pub fn card_size(&self) -> (f32, f32) {
        (CARD_W, self.field_top() + card_height_for(self.list_h))
    }

    /// The card as the compositor should hit-test it — see [`input_rect`].
    pub fn input_rect(&self) -> (i32, i32, i32, i32) {
        let (x, y) = self.card_origin();
        let (width, height) = self.card_size();
        outward(x, y, width, height)
    }

    /// The rows' pane, in the card's own coordinates: under the field, and
    /// empty when the card shows no rows.
    pub fn list_rect(&self) -> Rect {
        Rect::from_xywh(0.0, self.field_top() + LIST_TOP, CARD_W, self.pane_h)
    }

    /// The ask log's pane, above the field, and empty when there is no log.
    pub fn log_rect(&self) -> Rect {
        Rect::from_xywh(0.0, LOG_TOP_PAD, CARD_W, self.log_h)
    }

    /// Push the current query into the scene, and size the card for `count`
    /// results.
    ///
    /// `empty_message` is what to say when there is nothing to show — `None`
    /// when there is nothing to say, and the card is the field alone.
    pub fn update(&mut self, input: &TextInput, count: usize, empty_message: Option<&str>) {
        let field = input.clone();
        self.field
            .set_draw_content(move |canvas: &Canvas, width: f32, height: f32| {
                field.render_at(canvas, width, height);
                Rect::from_wh(width, height)
            });

        let transition = Some(Transition::ease_out_quad(0.12));
        if count == 0 {
            // A query that matched nothing says so, in the first line, so the
            // card keeps a shape instead of collapsing under the answer. An
            // empty query has nothing to report — a launcher just opened has
            // not failed to find anything — and the card is the field alone.
            let list = match empty_message {
                Some(message) => {
                    self.message.set_opacity(1.0_f32, None);
                    self.message.set_draw_content(draw_message(
                        message.to_string(),
                        self.font(15.0, FontStyle::normal()),
                        self.subtitle_color(),
                    ));
                    ROW_H
                }
                None => {
                    self.message.set_opacity(0.0_f32, None);
                    self.message.set_draw_content(draw_nothing());
                    0.0
                }
            };
            self.list_h = list;
            self.pane_h = 0.0;
            self.apply_card_height(transition);
            return;
        }

        self.message.set_opacity(0.0_f32, None);
        self.message.set_draw_content(draw_nothing());
        self.list_h = count.min(MAX_ROWS) as f32 * ROW_H;
        self.pane_h = self.list_h;
        self.apply_card_height(transition);
    }

    /// Make room above the field for an ask log `length` points long: all of
    /// it, up to the list's tallest, past which the log scrolls. Zero for no
    /// log. Takes effect with the next [`Palette::update`].
    pub fn set_log(&mut self, length: f32) {
        self.log_h = if length <= 0.0 {
            0.0
        } else {
            length.clamp(LOG_LINE_H, MAX_LIST_H)
        };
    }

    /// How wide `text` is in the ask log, drawn in `style`.
    pub fn measure_log(&self, text: &str, style: Style) -> f32 {
        measure_runs(&self.log_font(style), text)
    }

    fn log_font(&self, style: Style) -> Font {
        let weight = match style {
            Style::Prompt => Weight::SEMI_BOLD,
            Style::Answer | Style::Note => Weight::NORMAL,
        };
        self.font(
            LOG_TEXT,
            FontStyle::new(weight, Width::NORMAL, Slant::Upright),
        )
    }

    /// Paint the lines of the ask log that fall inside `band`, in the list's
    /// content coordinates.
    pub fn paint_log(&self, canvas: &Canvas, band: Rect, lines: &[Line]) {
        let prompt_font = self.log_font(Style::Prompt);
        let font = self.log_font(Style::Answer);
        let mut text = Paint::new(Color4f::from(self.title_color()), None);
        text.set_anti_alias(true);
        let mut dim = Paint::new(Color4f::from(self.subtitle_color()), None);
        dim.set_anti_alias(true);

        let first = (band.top / LOG_LINE_H).floor().max(0.0) as usize;
        let last = ((band.bottom / LOG_LINE_H).ceil().max(0.0) as usize).min(lines.len());
        for (index, line) in lines.iter().enumerate().take(last).skip(first) {
            if line.text.is_empty() {
                continue;
            }
            let baseline = index as f32 * LOG_LINE_H + LOG_LINE_H * 0.72;
            let (font, paint) = match line.style {
                Style::Prompt => (&prompt_font, &text),
                Style::Answer => (&font, &text),
                Style::Note => (&font, &dim),
            };
            draw_runs(canvas, &line.text, (LOG_INSET, baseline), font, paint);
        }
    }

    /// Paint the rows of `items` that fall inside `band`, in the list's
    /// content coordinates — row 0 at the top — for the list pane's band.
    /// `labels` name each item's source. Items from `compact_source` are drawn
    /// with smaller text and a smaller icon: they are there to be read, not
    /// picked.
    pub fn paint_rows(
        &self,
        canvas: &Canvas,
        band: Rect,
        items: &[&Item],
        labels: &[&'static str],
        compact_source: Option<usize>,
    ) {
        let title_font = self.font(15.0, FontStyle::normal());
        let subtitle_font = self.font(11.5, FontStyle::normal());
        let small_title_font = self.font(12.5, FontStyle::normal());
        let small_subtitle_font = self.font(10.0, FontStyle::normal());
        let badge_font = self.font(10.5, FontStyle::normal());
        let (title_color, subtitle_color) = (self.title_color(), self.subtitle_color());
        let layout = RowLayout::new(ROW_H, items.len());
        for index in layout.visible(band) {
            let item = items[index];
            let compact = compact_source == Some(item.origin.source);
            let (icon_size, title_font, subtitle_font) = if compact {
                (SMALL_ICON, &small_title_font, &small_subtitle_font)
            } else {
                (ICON, &title_font, &subtitle_font)
            };
            let icon = item
                .icon
                .as_deref()
                .and_then(|name| resolve_icon(&mut self.icons.borrow_mut(), name));
            let draw = draw_row(
                icon,
                icon_size,
                item.title.clone(),
                item.subtitle.clone(),
                labels.get(item.origin.source).copied().unwrap_or(""),
                title_font.clone(),
                subtitle_font.clone(),
                badge_font.clone(),
                title_color,
                subtitle_color,
            );
            let row = layout.rect(index, CARD_W);
            canvas.save();
            canvas.translate((row.left, row.top));
            draw(canvas, row.width(), row.height());
            canvas.restore();
        }
    }

    /// Size the card for what is on it, and place what sits under the log.
    ///
    /// Positions change at once, in step with the card's origin, which moves
    /// up as the log grows: together they keep the field still on screen.
    fn apply_card_height(&self, transition: Option<Transition>) {
        let top = self.field_top();
        let log_shown = if self.log_h > 0.0 { 1.0_f32 } else { 0.0_f32 };
        self.log_divider.set_opacity(log_shown, None);
        self.log_divider.set_position(
            LayerPoint {
                x: 0.0,
                y: (top - 1.0).max(0.0),
            },
            None,
        );
        self.field.set_position(LayerPoint { x: 0.0, y: top }, None);
        self.divider.set_position(
            LayerPoint {
                x: 0.0,
                y: top + FIELD_H,
            },
            None,
        );
        self.divider
            .set_opacity(if self.list_h <= 0.0 { 0.0_f32 } else { 1.0_f32 }, None);
        self.message.set_position(
            LayerPoint {
                x: 0.0,
                y: top + LIST_TOP,
            },
            None,
        );
        self.message
            .set_size(LayerSize::points(CARD_W, self.list_h), None);
        let (_, height) = self.card_size();
        self.card
            .set_size(LayerSize::points(CARD_W, height), transition);
    }

    fn font(&self, size: f32, style: FontStyle) -> Font {
        get_font_with_fallback(styles::BODY.family, style, size)
    }
}

// ---------------------------------------------------------------------------
// Content draw functions
//
// Each returns a closure the engine calls with the layer's own size. They run
// on the renderer thread, so everything they need — fonts, decoded icons — is
// resolved here and moved in.
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn draw_row(
    icon: Option<Image>,
    icon_size: f32,
    title: String,
    subtitle: Option<String>,
    badge: &'static str,
    title_font: Font,
    subtitle_font: Font,
    badge_font: Font,
    title_color: Color,
    subtitle_color: Color,
) -> impl Fn(&Canvas, f32, f32) -> Rect + Send + Sync {
    move |canvas, width, height| {
        let mut paint = Paint::new(Color4f::from(title_color), None);
        paint.set_anti_alias(true);

        if let Some(image) = &icon {
            let top = (height - icon_size) / 2.0;
            // Centred in the space a full-size icon takes, so the text lines
            // up with the rows around it.
            let left = ROW_INSET + 8.0 + (ICON - icon_size) / 2.0;
            canvas.draw_image_rect_with_sampling_options(
                image,
                None,
                Rect::from_xywh(left, top, icon_size, icon_size),
                SamplingOptions::default(),
                &paint,
            );
        }

        // The badge is measured first: the title is clipped to what is left,
        // so a long window title cannot run underneath it.
        let mut badge_paint = Paint::new(Color4f::from(subtitle_color), None);
        badge_paint.set_anti_alias(true);
        let badge_width = if badge.is_empty() {
            0.0
        } else {
            badge_font.measure_str(badge, Some(&badge_paint)).0
        };
        if !badge.is_empty() {
            canvas.draw_str(
                badge,
                (width - ROW_INSET - 8.0 - badge_width, height / 2.0 + 4.0),
                &badge_font,
                &badge_paint,
            );
        }

        let text_x = ROW_INSET + 8.0 + ICON + 12.0;
        let text_width = (width - ROW_INSET - 20.0 - badge_width - text_x).max(0.0);
        canvas.save();
        canvas.clip_rect(
            Rect::from_xywh(text_x, 0.0, text_width, height),
            None,
            Some(true),
        );

        match &subtitle {
            Some(subtitle) if !subtitle.is_empty() => {
                canvas.draw_str(&title, (text_x, height / 2.0 - 1.0), &title_font, &paint);
                let mut sub = Paint::new(Color4f::from(subtitle_color), None);
                sub.set_anti_alias(true);
                canvas.draw_str(
                    subtitle,
                    (text_x, height / 2.0 + 14.0),
                    &subtitle_font,
                    &sub,
                );
            }
            _ => {
                canvas.draw_str(&title, (text_x, height / 2.0 + 5.0), &title_font, &paint);
            }
        }
        canvas.restore();

        Rect::from_wh(width, height)
    }
}

fn draw_message(
    message: String,
    font: Font,
    color: Color,
) -> impl Fn(&Canvas, f32, f32) -> Rect + Send + Sync {
    move |canvas, width, height| {
        let mut paint = Paint::new(Color4f::from(color), None);
        paint.set_anti_alias(true);
        let text_width = font.measure_str(&message, Some(&paint)).0;
        canvas.draw_str(
            &message,
            ((width - text_width) / 2.0, height / 2.0 + 5.0),
            &font,
            &paint,
        );
        Rect::from_wh(width, height)
    }
}

/// Decode an icon once and keep it. Misses are remembered too — an app whose
/// icon the theme does not have must not be looked up again on every keystroke.
fn resolve_icon(cache: &mut HashMap<String, Option<Image>>, name: &str) -> Option<Image> {
    cache
        .entry(name.to_string())
        .or_insert_with(|| named_icon_sized(name, (ICON * 2.0) as i32))
        .clone()
}

/// Draws nothing. Given to rows that have no item, whose opacity is zero
/// anyway — the engine wants a content function, not the absence of one.
fn draw_nothing() -> impl Fn(&Canvas, f32, f32) -> Rect + Send + Sync {
    move |_canvas, width, height| Rect::from_wh(width, height)
}

/// The query field's look: no box of its own, because it already sits in one.
pub fn field_style(dark: bool) -> TextInputStyle {
    let mut style = TextInputStyle::with_theme(if dark { Theme::dark() } else { Theme::light() });
    style.text_style = styles::TITLE_3;
    style.text_style.size = 19.0;
    style.horizontal_padding = 20.0;
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

#[cfg(test)]
mod tests {
    use super::*;
    use otto_kit::components::text_input::{KeyMods, TextInputKey};

    /// A field that was never given a box scrolls whatever is typed into it out
    /// of its own clip: `ensure_caret_visible` keeps the caret inside a width
    /// of zero by scrolling the full width of the text. The launcher looked
    /// like it was ignoring the keyboard — the query filtered the list, and the
    /// field stayed empty.
    #[test]
    fn the_query_field_keeps_typed_text_inside_its_box() {
        let mut input = TextInput::editing("", field_style(true));
        input.set_size(CARD_W, FIELD_H);
        for c in "terminal".chars() {
            input.on_key(TextInputKey::Char(c), KeyMods::default());
        }
        assert_eq!(input.value(), "terminal");
        assert_eq!(
            input.state.scroll_px, 0.0,
            "a query this short fits, so nothing should be scrolled out of view"
        );
    }

    /// The launcher's buffer is the card at its tallest whatever is on it, so
    /// the input region — not the buffer — is the only thing that says where
    /// the launcher ends. A card showing nothing but the field must claim the
    /// field and no more, or the eight rows' worth of transparent buffer under
    /// it goes on swallowing the presses and hovers meant for the dock.
    /// In ask mode the log grows above the field. The card's top edge moves
    /// up to make room, and the field being typed into must not move at all.
    #[test]
    fn the_field_stays_put_as_the_ask_log_grows_above_it() {
        let mut palette = Palette::new(Engine::create(CARD_W, MAX_CARD_H), None, true);
        palette.reserve_log_room();
        palette.set_size(1920.0, 1080.0);
        let input = TextInput::editing("", field_style(true));
        let field_on_screen = |palette: &Palette| palette.card_origin().1 + palette.field_top();

        palette.update(&input, 0, None);
        let resting = field_on_screen(&palette);
        assert_eq!(
            palette.field_top(),
            0.0,
            "no log, so the field tops the card"
        );

        // A question's two answers under the field don't move it either.
        palette.set_log(LOG_LINE_H * 3.0);
        palette.update(&input, 2, None);
        assert_eq!(field_on_screen(&palette), resting);
        let log = palette.log_rect();
        assert!(log.height() > 0.0 && log.bottom <= palette.field_top());
        assert!(palette.list_rect().top >= palette.field_top() + FIELD_H);

        palette.set_log(10_000.0);
        palette.update(&input, 0, None);
        assert_eq!(
            field_on_screen(&palette),
            resting,
            "even at the tallest log"
        );
        let (_, y, _, height) = palette.input_rect();
        assert!(y >= 0 && y + height <= 1080, "the card stays on the output");
    }

    /// The card is dragged by its field or its log, and never off the output.
    #[test]
    fn a_dragged_card_moves_and_stays_on_the_output() {
        let mut palette = Palette::new(Engine::create(CARD_W, MAX_CARD_H), None, true);
        palette.reserve_log_room();
        palette.set_size(1920.0, 1080.0);
        let input = TextInput::editing("", field_style(true));
        palette.set_log(LOG_LINE_H * 3.0);
        palette.update(&input, 2, None);

        let log = palette.log_rect();
        assert!(palette.drags_at(log.top + 1.0), "the log is a handle");
        assert!(palette.drags_at(palette.field_top() + 1.0), "so is the field");
        assert!(
            !palette.drags_at(palette.list_rect().top + 1.0),
            "the rows are not"
        );

        let (x, y) = palette.card_origin();
        palette.move_card_to(x - 100.0, y + 40.0);
        assert_eq!(palette.card_origin(), (x - 100.0, y + 40.0));

        palette.move_card_to(-10_000.0, -10_000.0);
        assert_eq!(palette.card_origin(), (0.0, 0.0), "held at the corner");
        palette.move_card_to(5.0, 5.0);
        assert_eq!(
            palette.card_origin(),
            (5.0, 5.0),
            "and back off the edge at once"
        );
    }

    #[test]
    fn the_input_rect_is_the_card_that_is_drawn() {
        let surface = (1920.0, 1080.0);
        let (x, y, width, field_only) = input_rect(surface, 0);
        assert_eq!((x, y), (650, 173), "the card is centred, above centre");
        assert_eq!(width, CARD_W as i32);
        assert_eq!(field_only, FIELD_H as i32, "no rows, no list to point at");
        assert!(
            (field_only as f32) < MAX_CARD_H,
            "the empty card must not claim the whole buffer"
        );

        // And it grows and shrinks with the list, rather than staying at
        // whichever height it was first asked about.
        let (_, _, _, one_row) = input_rect(surface, 1);
        let (_, _, _, eight_rows) = input_rect(surface, MAX_ROWS);
        assert!(one_row > field_only);
        assert_eq!(eight_rows, card_height(MAX_ROWS) as i32);
        assert_eq!(
            input_rect(surface, 0).3,
            field_only,
            "emptying the list gives the height back"
        );
    }
}
