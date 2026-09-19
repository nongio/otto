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
use std::path::{Path, PathBuf};
use std::sync::Arc;

use layers::prelude::*;
use layers::types::{Color as LayerColor, Point as LayerPoint, Size as LayerSize};
use otto_kit::components::scroll::RowLayout;
use otto_kit::components::text_input::{TextInput, TextInputStyle};
use otto_kit::icons::named_icon_sized;
use otto_kit::preview::document;
use otto_kit::theme::Theme;
use otto_kit::typography::{draw_runs, get_font_with_fallback, measure_runs, styles};
use skia_safe::font_style::{Slant, Weight, Width};
use skia_safe::{Canvas, Color, Color4f, Font, FontStyle, Image, Paint, Rect, SamplingOptions};

use crate::log::{Kind, Line, Style, BUBBLE_GAP, BUBBLE_PAD_X, BUBBLE_PAD_Y, FOOTER_H, IMAGE_PAD};
use crate::selection::Span;
use crate::source::{Activity, Item};

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
/// The activity dot beside a row with no icon, such as an agent session.
const DOT_RADIUS: f32 = 4.0;
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

/// Height of one line of plain text in the ask log.
pub const LOG_LINE_H: f32 = crate::log::LINE_H;
/// Size of the ask log's text: what the person asked and what the agent
/// answered, both sides of it. The answer is laid out by the toolkit's
/// document, which is told this size too, so one side of the conversation is
/// never quietly smaller than the other.
pub const LOG_TEXT: f32 = 14.0;

/// The prose style of an answer: the toolkit's body, at the log's size.
pub fn log_body() -> otto_kit::typography::TextStyle {
    otto_kit::typography::TextStyle {
        size: LOG_TEXT,
        ..styles::BODY
    }
}
/// Size of a note in the ask log — a tool call, a status line. Smaller than
/// the conversation, so what the agent said outranks what it is doing.
const LOG_NOTE_TEXT: f32 = 11.5;
/// Space either side of the ask log's text.
const LOG_INSET: f32 = 20.0;

/// Corner radius of a request's bubble; a one-line request is a pill.
const BUBBLE_RADIUS: f32 = 16.0;

/// Room either side of the mode's name inside its pill.
const PILL_PAD_X: f32 = 7.0;
/// How tall the mode's pill is.
const PILL_H: f32 = 16.0;
/// Space between the footer's pieces: the agent, its mode, the hint.
const FOOTER_GAP: f32 = 7.0;
/// Corner radius of the fill under a group of tool calls the pointer is on.
const STEPS_RADIUS: f32 = 6.0;
/// Room around that fill, so the words are not against its edge.
const STEPS_PAD: f32 = 4.0;

/// How rounded a picture in the log is: the corner every other surface in
/// the card wears.
const IMAGE_RADIUS: f32 = 8.0;
/// How wide a line of the ask log may run.
pub const LOG_W: f32 = CARD_W - LOG_INSET * 2.0;

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
    /// Whether the card is centred on the output, as it is in ask and agents
    /// mode, rather than resting above centre.
    centered: bool,
    /// How far the card has been dragged from where it rests, in points.
    moved: (f32, f32),
    /// Icons live as long as the launcher does. It is open for seconds, and
    /// decoding the same icon on every keystroke is the one thing that would
    /// make typing feel slow. Behind a cell because painting a band only
    /// borrows the palette.
    icons: RefCell<HashMap<String, Option<Image>>>,
    /// Pictures the agent sent, decoded once. Kept beside the icons and for the
    /// same reason: the log is laid out again on every chunk of an answer, and
    /// each pass asks every picture how large it is.
    pictures: RefCell<HashMap<PathBuf, Option<Image>>>,
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
            centered: false,
            moved: (0.0, 0.0),
            icons: RefCell::new(HashMap::new()),
            pictures: RefCell::new(HashMap::new()),
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

    /// Centre the card on the output, whatever is on it: a conversation, or a
    /// list of sessions. As either grows, the card grows both ways and stays
    /// centred.
    pub fn set_centered(&mut self, centered: bool) {
        self.centered = centered;
    }

    /// Where the card subsurface belongs, in the parent surface's coordinates.
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
        if !self.centered {
            return (x, y);
        }
        let (_, height) = self.card_size();
        (x, ((self.size.1 - height) / 2.0).round().max(0.0))
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

    /// How large the picture at `path` is, in its own pixels, for the log to
    /// scale into the card. `None` when there is no reading it — the file is
    /// gone, or it is not a picture — and the log says its name instead.
    pub fn picture_size(&self, path: &Path) -> Option<(f32, f32)> {
        let image = self.picture(path)?;
        Some((image.width() as f32, image.height() as f32))
    }

    /// The picture at `path`, decoded once and kept.
    ///
    /// Decoded eagerly into raster pixels: `Image::from_encoded` is lazy, and a
    /// picture left lazy is decoded again on every band the log paints.
    fn picture(&self, path: &Path) -> Option<Image> {
        if let Some(cached) = self.pictures.borrow().get(path) {
            return cached.clone();
        }
        let decoded = std::fs::read(path)
            .ok()
            .and_then(|bytes| Image::from_encoded(skia_safe::Data::new_copy(&bytes)))
            .and_then(|image| image.make_raster_image(None, None));
        self.pictures
            .borrow_mut()
            .insert(path.to_path_buf(), decoded.clone());
        decoded
    }

    /// How wide `text` is in the ask log, drawn in `style`.
    pub fn measure_log(&self, text: &str, style: Style) -> f32 {
        measure_runs(&self.log_font(style), text)
    }

    fn log_font(&self, style: Style) -> Font {
        let weight = match style {
            Style::Prompt => Weight::SEMI_BOLD,
            Style::Request | Style::Answer | Style::Note => Weight::NORMAL,
        };
        let size = match style {
            Style::Note => LOG_NOTE_TEXT,
            Style::Request | Style::Prompt | Style::Answer => LOG_TEXT,
        };
        self.font(size, FontStyle::new(weight, Width::NORMAL, Slant::Upright))
    }

    /// Every piece of text in the ask log, in reading order, with the box it
    /// is painted in — what [`crate::selection`] selects over.
    ///
    /// This walks the log exactly as [`Palette::paint_log`] does, because a
    /// highlight that does not sit on the words is worse than no highlight at
    /// all: the two have to agree about where each line was put.
    pub fn log_spans(&self, lines: &[Line]) -> Vec<Span> {
        let plain = self.log_font(Style::Answer);
        let mut spans = Vec::new();
        let mut row = 0usize;
        for line in lines {
            match &line.kind {
                Kind::Text { text, style } => {
                    let font = self.log_font(*style);
                    let width = measure_runs(&font, text);
                    spans.push(Span {
                        rect: Rect::from_xywh(LOG_INSET, line.top, width, line.height),
                        text: text.clone(),
                        font,
                        line: row,
                    });
                    row += 1;
                }
                Kind::Bubble {
                    lines: words,
                    width,
                    ..
                } => {
                    let left = LOG_INSET + LOG_W - width + BUBBLE_PAD_X;
                    for (index, words) in words.iter().enumerate() {
                        let top = line.top + BUBBLE_PAD_Y + index as f32 * LOG_LINE_H;
                        spans.push(Span {
                            rect: Rect::from_xywh(
                                left,
                                top,
                                measure_runs(&plain, words),
                                LOG_LINE_H,
                            ),
                            text: words.clone(),
                            font: plain.clone(),
                            line: row,
                        });
                        row += 1;
                    }
                }
                Kind::Document(doc) => {
                    for text_line in doc {
                        for run in &text_line.runs {
                            spans.push(Span {
                                rect: Rect::from_xywh(
                                    LOG_INSET + run.x,
                                    line.top + text_line.top,
                                    run.width,
                                    text_line.height,
                                ),
                                text: run.text.clone(),
                                font: run.font(),
                                line: row,
                            });
                        }
                        row += 1;
                    }
                }
                Kind::Steps { lines: words, .. } => {
                    let font = self.log_font(Style::Note);
                    let height = Style::Note.line_h();
                    for (index, words) in words.iter().enumerate() {
                        spans.push(Span {
                            rect: Rect::from_xywh(
                                LOG_INSET,
                                line.top + index as f32 * height,
                                measure_runs(&font, words),
                                height,
                            ),
                            text: words.clone(),
                            font: font.clone(),
                            line: row,
                        });
                        row += 1;
                    }
                }
                // Neither a picture nor the footer has words to select over —
                // the footer is the card talking about itself, not the
                // conversation — but each is a line of the log all the same,
                // so the numbering carries on past it.
                Kind::Image { .. } | Kind::Footer { .. } => row += 1,
            }
        }
        spans
    }

    /// The group of tool calls under `point` in the ask log, as the log line
    /// holding it and the request it belongs to. `point` is in the log's
    /// content coordinates.
    pub fn steps_at(&self, lines: &[Line], point: (f32, f32)) -> Option<(usize, usize)> {
        lines.iter().enumerate().find_map(|(index, line)| {
            let Kind::Steps { block, .. } = &line.kind else {
                return None;
            };
            let rect = Self::steps_rect(line);
            let (x, y) = point;
            (x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom)
                .then_some((index, *block))
        })
    }

    /// What a group of tool calls answers the pointer over: its words and the
    /// padding the fill is drawn in.
    fn steps_rect(line: &Line) -> Rect {
        Rect::from_xywh(
            LOG_INSET - STEPS_PAD,
            line.top - STEPS_PAD / 2.0,
            LOG_W + STEPS_PAD * 2.0,
            line.height + STEPS_PAD,
        )
    }

    /// The code block under `point` in the ask log, as the log line holding
    /// the answer and the block within it, and whether the point is on the
    /// block's copy button. `point` is in the log's content coordinates.
    pub fn code_at(&self, lines: &[Line], point: (f32, f32)) -> Option<(usize, document::CodeHit)> {
        lines.iter().enumerate().find_map(|(index, line)| {
            let Kind::Document(doc) = &line.kind else {
                return None;
            };
            let content = Rect::from_xywh(LOG_INSET, line.top, LOG_W, line.height);
            document::code_at(content, doc, 0.0, point).map(|hit| (index, hit))
        })
    }

    /// The link under `point` in the ask log, as its destination. `point` is
    /// in the log's content coordinates, as for [`Palette::code_at`].
    pub fn link_at<'a>(&self, lines: &'a [Line], point: (f32, f32)) -> Option<&'a str> {
        lines.iter().find_map(|line| {
            let Kind::Document(doc) = &line.kind else {
                return None;
            };
            let content = Rect::from_xywh(LOG_INSET, line.top, LOG_W, line.height);
            document::link_at_scrolled(content, doc, 0.0, point)
        })
    }

    /// The text of the `block`th code block of the answer on log line `line`.
    pub fn code_text(lines: &[Line], line: usize, block: usize) -> Option<String> {
        let Kind::Document(doc) = &lines.get(line)?.kind else {
            return None;
        };
        document::code_blocks(doc)
            .into_iter()
            .nth(block)
            .map(|block| block.text)
    }

    /// Paint the lines of the ask log that fall inside `band`, in the list's
    /// content coordinates. `copy` is the copy button to show on a code
    /// block, as the log line holding its answer and the button. `steps` is
    /// the log line of the group of tool calls the pointer is on, which is
    /// drawn as something to click.
    pub fn paint_log(
        &self,
        canvas: &Canvas,
        band: Rect,
        lines: &[Line],
        selection: &[Rect],
        copy: Option<(usize, document::CopyButton)>,
        steps: Option<usize>,
    ) {
        let prompt_font = self.log_font(Style::Prompt);
        let note_font = self.log_font(Style::Note);
        let font = self.log_font(Style::Answer);
        let mut text = Paint::new(Color4f::from(self.title_color()), None);
        text.set_anti_alias(true);
        let mut dim = Paint::new(Color4f::from(self.subtitle_color()), None);
        dim.set_anti_alias(true);

        let theme = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        let mut request = Paint::new(Color4f::from(theme.text_primary), None);
        request.set_anti_alias(true);

        // The highlight goes down first, so the words sit on top of it.
        let mut highlight = Paint::new(Color4f::from(selection_color(&theme)), None);
        highlight.set_anti_alias(true);
        for rect in selection {
            if rect.bottom < band.top || rect.top > band.bottom {
                continue;
            }
            canvas.draw_round_rect(*rect, 2.0, 2.0, &highlight);
        }
        let mut bubble_fill = Paint::new(Color4f::from(theme.fill_secondary), None);
        bubble_fill.set_anti_alias(true);

        for (index, line) in lines.iter().enumerate() {
            if line.top + line.height < band.top || line.top > band.bottom {
                continue;
            }
            match &line.kind {
                Kind::Text { text: words, .. } if words.is_empty() => {}
                Kind::Text { text: words, style } => {
                    let baseline = line.top + line.height * 0.72;
                    let (font, paint) = match style {
                        Style::Prompt => (&prompt_font, &text),
                        Style::Request | Style::Answer => (&font, &text),
                        Style::Note => (&note_font, &dim),
                    };
                    draw_runs(canvas, words, (LOG_INSET, baseline), font, paint);
                }
                Kind::Bubble {
                    lines: words,
                    width,
                    ..
                } => {
                    let bubble = Rect::from_xywh(
                        LOG_INSET + LOG_W - width,
                        line.top,
                        *width,
                        line.height - BUBBLE_GAP,
                    );
                    let radius = BUBBLE_RADIUS.min(bubble.height() / 2.0);
                    canvas.draw_round_rect(bubble, radius, radius, &bubble_fill);
                    for (index, words) in words.iter().enumerate() {
                        let baseline =
                            line.top + BUBBLE_PAD_Y + index as f32 * LOG_LINE_H + LOG_LINE_H * 0.72;
                        draw_runs(
                            canvas,
                            words,
                            (bubble.left + BUBBLE_PAD_X, baseline),
                            &font,
                            &request,
                        );
                    }
                }
                Kind::Steps { lines: words, .. } => {
                    // Under the pointer the group takes a fill: painted text
                    // says nothing about being clickable on its own, and the
                    // cursor alone is only found by the person already there.
                    if steps == Some(index) {
                        let mut fill = Paint::new(Color4f::from(theme.fill_secondary), None);
                        fill.set_anti_alias(true);
                        canvas.draw_round_rect(
                            Self::steps_rect(line),
                            STEPS_RADIUS,
                            STEPS_RADIUS,
                            &fill,
                        );
                    }
                    let height = Style::Note.line_h();
                    for (row, words) in words.iter().enumerate() {
                        let baseline = line.top + row as f32 * height + height * 0.72;
                        draw_runs(canvas, words, (LOG_INSET, baseline), &note_font, &dim);
                    }
                }
                Kind::Footer { agent, mode, hint } => {
                    let baseline = line.top + FOOTER_H * 0.68;
                    let mut x = LOG_INSET;
                    draw_runs(canvas, agent, (x, baseline), &note_font, &dim);
                    x += measure_runs(&note_font, agent) + FOOTER_GAP;

                    // The mode is the one thing here that can be changed, so
                    // it is the one thing wearing a control's shape.
                    let width = measure_runs(&note_font, mode) + PILL_PAD_X * 2.0;
                    let pill =
                        Rect::from_xywh(x, line.top + (FOOTER_H - PILL_H) / 2.0, width, PILL_H);
                    let mut fill = Paint::new(Color4f::from(theme.fill_secondary), None);
                    fill.set_anti_alias(true);
                    canvas.draw_round_rect(pill, PILL_H / 2.0, PILL_H / 2.0, &fill);
                    draw_runs(canvas, mode, (x + PILL_PAD_X, baseline), &note_font, &text);
                    x += width + FOOTER_GAP;

                    if let Some(hint) = hint {
                        draw_runs(canvas, hint, (x, baseline), &note_font, &dim);
                    }
                }
                Kind::Document(doc) => {
                    let content = Rect::from_xywh(LOG_INSET, line.top, LOG_W, line.height);
                    let copy = copy
                        .filter(|(on_line, _)| *on_line == index)
                        .map(|(_, button)| button);
                    document::draw_scrolled(canvas, content, doc, 0.0, &theme, copy);
                }
                Kind::Image {
                    path,
                    label,
                    width,
                    height,
                } => {
                    let Some(image) = self.picture(path) else {
                        // The layout only makes a picture line for a file it
                        // could read; if it has gone since, its name goes in
                        // its place rather than a hole in the log.
                        let baseline = line.top + LOG_LINE_H * 0.72;
                        let words = format!("picture: {label}");
                        draw_runs(canvas, &words, (LOG_INSET, baseline), &note_font, &dim);
                        continue;
                    };
                    let box_ = Rect::from_xywh(LOG_INSET, line.top + IMAGE_PAD, *width, *height);
                    let radius = IMAGE_RADIUS.min(box_.height() / 2.0);
                    canvas.save();
                    // Rounded like every other surface in the card, and clipped
                    // rather than drawn rounded: the picture's own edge pixels
                    // must not bleed past the corner.
                    canvas.clip_rrect(
                        skia_safe::RRect::new_rect_xy(box_, radius, radius),
                        None,
                        true,
                    );
                    let source = (image.width(), image.height());
                    canvas.draw_image_rect_with_sampling_options(
                        &image,
                        None,
                        box_,
                        otto_kit::utils::icon_sampling(source, (*width, *height)),
                        &Paint::default(),
                    );
                    canvas.restore();
                    // A hairline border, so a picture that is nearly the colour
                    // of the card still reads as a picture.
                    let mut edge = Paint::new(Color4f::from(theme.hairline), None);
                    edge.set_anti_alias(true);
                    edge.set_style(skia_safe::paint::Style::Stroke);
                    edge.set_stroke_width(1.0);
                    canvas.draw_round_rect(box_, radius, radius, &edge);
                }
            }
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
        let theme = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
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
            let dot = item.activity.map(|activity| match activity {
                Activity::Working => theme.accent,
                Activity::Idle => theme.text_tertiary,
                Activity::Waiting => theme.accent_yellow,
            });
            let draw = draw_row(
                icon,
                dot,
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
    dot: Option<Color>,
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

        if let Some(color) = dot {
            // In the icon's place, centred where a full-size icon would be.
            let centre = (ROW_INSET + 8.0 + ICON / 2.0, height / 2.0);
            let mut dot_paint = Paint::new(Color4f::from(color), None);
            dot_paint.set_anti_alias(true);
            canvas.draw_circle(centre, DOT_RADIUS, &dot_paint);
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

/// What selected text in the log sits on: the accent, faint enough to read
/// through.
fn selection_color(theme: &Theme) -> Color {
    let accent = theme.accent;
    Color::from_argb(90, accent.r(), accent.g(), accent.b())
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
    /// In ask and agents mode the card is centred on the output, and stays
    /// centred as the log above the field or the list under it grows.
    #[test]
    fn the_ask_card_stays_centred_as_it_grows() {
        let mut palette = Palette::new(Engine::create(CARD_W, MAX_CARD_H), None, true);
        palette.set_centered(true);
        palette.set_size(1920.0, 1080.0);
        let input = TextInput::editing("", field_style(true));
        let centred = |palette: &Palette| {
            let (_, y) = palette.card_origin();
            let (_, height) = palette.card_size();
            ((y + height / 2.0) - 540.0).abs() <= 0.5
        };

        palette.update(&input, 0, None);
        assert_eq!(
            palette.field_top(),
            0.0,
            "no log, so the field tops the card"
        );
        assert!(centred(&palette), "the field alone");

        palette.update(&input, MAX_ROWS, None);
        assert!(centred(&palette), "a full list of sessions");

        palette.set_log(LOG_LINE_H * 3.0);
        palette.update(&input, 2, None);
        assert!(centred(&palette), "a log and answers");
        let log = palette.log_rect();
        assert!(log.height() > 0.0 && log.bottom <= palette.field_top());
        assert!(palette.list_rect().top >= palette.field_top() + FIELD_H);

        palette.set_log(10_000.0);
        palette.update(&input, 0, None);
        assert!(centred(&palette), "the tallest log");
        let (_, y, _, height) = palette.input_rect();
        assert!(y >= 0 && y + height <= 1080, "the card stays on the output");
    }

    /// An agent that shares a link writes it bare, so the log has to find one
    /// under the pointer — a link that cannot be clicked is not a link.
    #[test]
    fn a_bare_link_in_the_log_is_under_the_pointer() {
        let palette = Palette::new(Engine::create(CARD_W, MAX_CARD_H), None, true);
        let (blocks, _) = otto_md_kit::parse_capped(
            "the notes are at https://example.com/a now",
            otto_md_kit::MAX_BLOCKS,
        );
        let doc = otto_kit::preview::document::wrap_at(&blocks, LOG_W, log_body());
        let run = doc
            .iter()
            .flat_map(|line| line.runs.iter().map(move |run| (line, run)))
            .find(|(_, run)| run.style.link)
            .expect("the bare link is a link");
        let (text_line, run) = run;
        let height = doc
            .last()
            .map(|line| line.top + line.height)
            .expect("a laid-out answer");
        let lines = vec![Line {
            top: 0.0,
            height,
            kind: Kind::Document(doc.clone()),
        }];

        let point = (
            LOG_INSET + run.x + run.width / 2.0,
            text_line.top + text_line.height / 2.0,
        );
        assert_eq!(
            palette.link_at(&lines, point),
            Some("https://example.com/a")
        );
        assert_eq!(
            palette.link_at(&lines, (LOG_INSET + 1.0, point.1)),
            None,
            "the words before the link go nowhere"
        );
    }

    /// The card is dragged by its field or its log, and never off the output.
    #[test]
    fn a_dragged_card_moves_and_stays_on_the_output() {
        let mut palette = Palette::new(Engine::create(CARD_W, MAX_CARD_H), None, true);
        palette.set_centered(true);
        palette.set_size(1920.0, 1080.0);
        let input = TextInput::editing("", field_style(true));
        palette.set_log(LOG_LINE_H * 3.0);
        palette.update(&input, 2, None);

        let log = palette.log_rect();
        assert!(palette.drags_at(log.top + 1.0), "the log is a handle");
        assert!(
            palette.drags_at(palette.field_top() + 1.0),
            "so is the field"
        );
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

    /// Selecting text in the log is hit-testing against the boxes this file
    /// says each piece of text was painted in, so those boxes have to hold
    /// every kind of line the log draws — a request in its bubble, an answer
    /// laid out as a document, a tool call, the status — and be where the
    /// words are.
    #[test]
    fn every_kind_of_line_in_the_log_can_be_pointed_at() {
        let palette = Palette::new(Engine::create(CARD_W, MAX_CARD_H), None, true);
        let steps = ["✓ ls".to_string()];
        let answer = [crate::ask::Said::Text(
            "Run `cargo build` first\n\n- then the tests".to_owned(),
        )];
        let blocks = [crate::log::Block {
            prompt: "how do I build it",
            attachments: None,
            answer: &answer,
            steps: &steps,
            steps_expanded: false,

            inputs: &[],
            question: None,
            action: &[],
            note: None,
        }];
        let lines = crate::log::lay_out(
            &blocks,
            Some("Working…"),
            None,
            LOG_W,
            |text, style| palette.measure_log(text, style),
            |path| palette.picture_size(path),
        );
        let spans = palette.log_spans(&lines);
        // All of it, copied, reads as the conversation does on screen: the
        // request, the answer with its code and its list, the tool call and
        // the status, each on its own line.
        let all = crate::selection::everything(&spans).expect("something to select");
        assert_eq!(
            crate::selection::text(&spans, all),
            "how do I build it\nRun cargo build first\n• then the tests\n✓ ls\n\nWorking…"
        );

        let length = crate::log::length(&lines);
        for span in &spans {
            assert!(
                span.rect.left >= 0.0 && span.rect.right <= CARD_W + 0.5,
                "{:?} is drawn off the card",
                span.text
            );
            assert!(
                span.rect.top >= 0.0 && span.rect.bottom <= length + 0.5,
                "{:?} is drawn outside the log",
                span.text
            );
        }
        // Reading order: a span never starts above the one before it.
        for pair in spans.windows(2) {
            assert!(pair[1].rect.top >= pair[0].rect.top - 0.5);
        }
        // And a press in the middle of a span lands in that span.
        let request = spans
            .iter()
            .position(|span| span.text.contains("how do I build it"))
            .expect("the request is there");
        let rect = spans[request].rect;
        let caret =
            crate::selection::caret_at(&spans, (rect.left + rect.width() / 2.0, rect.center_y()))
                .expect("a press on the words selects them");
        assert_eq!(caret.span, request);
        assert!(caret.byte > 0 && caret.byte < spans[request].text.len());
    }

    /// A picture has no words in it, but it is still a line of the log. The
    /// spans on either side of it have to keep the line numbers they would have
    /// had, or copying an answer with a picture in it puts the words in the
    /// wrong order.
    #[test]
    fn a_picture_keeps_its_line_without_adding_a_span() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let path = dir.path().join("shot.png");
        std::fs::write(&path, PIXEL_PNG).expect("written");
        let palette = Palette::new(Engine::create(CARD_W, MAX_CARD_H), None, true);
        let answer = [
            crate::ask::Said::Text("here:".to_owned()),
            crate::ask::Said::Image(crate::ask::Picture {
                path: path.clone(),
                label: "shot".to_owned(),
            }),
            crate::ask::Said::Text("that is all".to_owned()),
        ];
        let blocks = [crate::log::Block {
            prompt: "draw",
            attachments: None,
            answer: &answer,
            steps: &[],
            steps_expanded: false,

            inputs: &[],
            question: None,
            action: &[],
            note: None,
        }];
        let lines = crate::log::lay_out(
            &blocks,
            None,
            None,
            LOG_W,
            |text, style| palette.measure_log(text, style),
            |path| palette.picture_size(path),
        );
        assert!(
            matches!(lines[2].kind, Kind::Image { .. }),
            "the picture is laid out: {:?}",
            lines[2].kind
        );

        let spans = palette.log_spans(&lines);
        let all = crate::selection::everything(&spans).expect("something to select");
        // The picture has nothing to copy, so it contributes no line of its
        // own — but it takes a line number, which is what keeps the words after
        // it from being run onto the words before it.
        assert_eq!(
            crate::selection::text(&spans, all),
            "draw\nhere:\nthat is all"
        );
        let mut rows: Vec<usize> = spans.iter().map(|span| span.line).collect();
        rows.dedup();
        assert_eq!(rows, [0, 1, 3], "the picture is line 2");
    }

    /// A one-pixel PNG, so the decoder has something real to read.
    const PIXEL_PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xfc,
        0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0xab, 0xce, 0x36, 0x89, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

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
