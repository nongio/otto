//! The launcher as a `lay-rs` scene.
//!
//! Built the way otto-kit's other panels are: a tree of layers positioned once
//! and then *changed*, rather than a canvas redrawn from scratch. That is what
//! buys what an immediate-mode launcher could not have: the selection slides
//! between rows instead of jumping, on a transition the engine runs without
//! the app driving frames by hand.
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
//! Rows are allocated once, up to [`MAX_ROWS`], and reused: filtering swaps
//! each row's draw content, never the shape of the tree. Layout is in logical
//! points and absolute — a row appearing must not move the field being typed
//! into.

use std::collections::HashMap;
use std::sync::Arc;

use layers::prelude::*;
use layers::types::{Color as LayerColor, Point as LayerPoint, Size as LayerSize};
use otto_kit::components::text_input::{TextInput, TextInputStyle};
use otto_kit::icons::named_icon_sized;
use otto_kit::theme::Theme;
use otto_kit::typography::{get_font_with_fallback, styles};
use skia_safe::{Canvas, Color, Color4f, Font, FontStyle, Image, Paint, Rect, SamplingOptions};

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
/// Corner radius of the card, applied by the compositor to the subsurface.
pub const RADIUS: f32 = 10.0;
const ICON: f32 = 28.0;
const ROW_INSET: f32 = 8.0;
/// The card at its tallest, which is how big its buffer is allocated: a
/// shorter card is the same buffer with the compositor clipping it, so the
/// height can change without reallocating anything.
pub const MAX_CARD_H: f32 = FIELD_H + 1.0 + LIST_PAD * 2.0 + MAX_ROWS as f32 * ROW_H;

/// Where the top of the card sits, as a fraction of the output's height.
/// Above centre: the eye starts there, and the list grows downwards into
/// space that was already empty.
const TOP_FRACTION: f32 = 0.16;

/// A row as the scene needs it — what to draw, with the icon already resolved.
struct Row {
    layer: Layer,
}

pub struct Palette {
    engine: Arc<Engine>,
    /// Root of the card subsurface's scene. Its own background is left to the
    /// compositor — see the module docs.
    card: Layer,
    field: Layer,
    divider: Layer,
    list: Layer,
    highlight: Layer,
    rows: Vec<Row>,

    size: (f32, f32),
    /// Number of rows currently shown, which sets the card's height.
    visible: usize,
    /// Icons live as long as the launcher does. It is open for seconds, and
    /// decoding the same icon on every keystroke is the one thing that would
    /// make typing feel slow.
    icons: HashMap<String, Option<Image>>,
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
        let list = new_layer("launcher-list");
        let highlight = new_layer("launcher-highlight");

        let _ = card.add_sublayer(&field);
        let _ = card.add_sublayer(&divider);
        let _ = card.add_sublayer(&list);
        // Before the rows, so it is behind their text.
        let _ = list.add_sublayer(&highlight);

        let rows = (0..MAX_ROWS)
            .map(|_| {
                let layer = new_layer("launcher-row");
                let _ = list.add_sublayer(&layer);
                Row { layer }
            })
            .collect();

        let mut palette = Self {
            engine,
            card,
            field,
            divider,
            list,
            highlight,
            rows,
            size: (0.0, 0.0),
            visible: 0,
            icons: HashMap::new(),
            dark,
        };
        palette.style();
        palette
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

        self.divider.set_background_color(
            PaintColor::Solid {
                color: lay_color(if self.dark {
                    Color::from_argb(36, 255, 255, 255)
                } else {
                    Color::from_argb(24, 0, 0, 0)
                }),
            },
            None,
        );

        self.highlight.set_background_color(
            PaintColor::Solid {
                color: lay_color(if self.dark {
                    Color::from_argb(46, 255, 255, 255)
                } else {
                    Color::from_argb(20, 0, 0, 0)
                }),
            },
            None,
        );
        self.highlight
            .set_border_corner_radius(BorderRadius::new_single(9.0), None);
        self.highlight.set_opacity(0.0, None);
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
        self.divider
            .set_position(LayerPoint { x: 0.0, y: FIELD_H }, None);
        self.divider.set_size(LayerSize::points(CARD_W, 1.0), None);
        self.list.set_position(
            LayerPoint {
                x: 0.0,
                y: FIELD_H + 1.0 + LIST_PAD,
            },
            None,
        );

        for (index, row) in self.rows.iter().enumerate() {
            row.layer.set_position(
                LayerPoint {
                    x: 0.0,
                    y: index as f32 * ROW_H,
                },
                None,
            );
            row.layer.set_size(LayerSize::points(CARD_W, ROW_H), None);
        }
        self.highlight.set_size(
            LayerSize::points(CARD_W - ROW_INSET * 2.0, ROW_H - 4.0),
            None,
        );

        self.apply_card_height(self.visible, None);
    }

    /// Where the card subsurface belongs, in the parent surface's coordinates.
    pub fn card_origin(&self) -> (f32, f32) {
        let (width, height) = self.size;
        (
            ((width - CARD_W) / 2.0).round(),
            (height * TOP_FRACTION).round(),
        )
    }

    /// How much of the card is currently in use. The buffer stays
    /// [`MAX_CARD_H`] tall; this is what the compositor should show of it.
    pub fn card_size(&self) -> (f32, f32) {
        (CARD_W, self.card_height(self.visible))
    }

    /// Which visible row a point in the *card's own* coordinates is over.
    ///
    /// Pointer events on the card arrive relative to its subsurface, so this
    /// takes them as they come rather than in screen coordinates.
    pub fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        let (width, height) = self.card_size();
        if x < 0.0 || x > width || y < 0.0 || y > height {
            return None;
        }
        let local_y = y - (FIELD_H + 1.0 + LIST_PAD);
        if local_y < 0.0 {
            return None;
        }
        let row = (local_y / ROW_H) as usize;
        (row < self.visible).then_some(row)
    }

    /// Push the current query and results into the scene.
    ///
    /// `items` are the matches, already ranked; `offset` is the first of them
    /// that is on screen and `selected` the one that is highlighted, both as
    /// indices into `items`.
    /// `empty_message` is what to say when there is nothing to show — `None`
    /// when there is nothing to say, and the card is the field alone.
    pub fn update(
        &mut self,
        input: &TextInput,
        items: &[&Item],
        labels: &[&'static str],
        offset: usize,
        selected: usize,
        empty_message: Option<&str>,
    ) {
        let field = input.clone();
        self.field
            .set_draw_content(move |canvas: &Canvas, width: f32, height: f32| {
                field.render_at(canvas, width, height);
                Rect::from_wh(width, height)
            });

        let title_font = self.font(15.0, FontStyle::normal());
        let subtitle_font = self.font(11.5, FontStyle::normal());
        let badge_font = self.font(10.5, FontStyle::normal());
        let title_color = if self.dark {
            Color::from_argb(240, 255, 255, 255)
        } else {
            Color::from_argb(240, 12, 12, 14)
        };
        let subtitle_color = if self.dark {
            Color::from_argb(150, 255, 255, 255)
        } else {
            Color::from_argb(140, 0, 0, 0)
        };

        let shown = items.len().saturating_sub(offset).min(MAX_ROWS);

        // Nothing matched: the first row carries the message, so the card
        // keeps a shape instead of collapsing to a bare field.
        if items.is_empty() {
            // A query that matched nothing says so, in the first row, so the
            // card keeps a shape instead of collapsing under the answer. An
            // empty query has nothing to report — a launcher just opened has
            // not failed to find anything — and the card is the field alone.
            let rows = match empty_message {
                Some(message) => {
                    self.rows[0].layer.set_opacity(1.0, None);
                    self.rows[0].layer.set_draw_content(draw_message(
                        message.to_string(),
                        title_font.clone(),
                        subtitle_color,
                    ));
                    1
                }
                None => 0,
            };
            for row in self.rows.iter().skip(rows) {
                row.layer.set_opacity(0.0, None);
                row.layer.set_draw_content(draw_nothing());
            }
            self.highlight.set_opacity(0.0, None);
            self.visible = rows;
            self.apply_card_height(rows, Some(Transition::ease_out_quad(0.12)));
            return;
        }

        for (row_index, row) in self.rows.iter().enumerate() {
            let Some(item) = items.get(offset + row_index) else {
                row.layer.set_opacity(0.0, None);
                row.layer.set_draw_content(draw_nothing());
                continue;
            };

            let icon = item
                .icon
                .as_deref()
                .and_then(|name| resolve_icon(&mut self.icons, name));

            row.layer.set_opacity(1.0, None);
            row.layer.set_draw_content(draw_row(
                icon,
                item.title.clone(),
                item.subtitle.clone(),
                labels.get(item.origin.source).copied().unwrap_or(""),
                title_font.clone(),
                subtitle_font.clone(),
                badge_font.clone(),
                title_color,
                subtitle_color,
            ));
        }

        // The selection slides. `selected` is an index into the whole match
        // list; on screen it is however far it is past the scroll offset.
        let on_screen = selected.saturating_sub(offset);
        self.highlight.set_opacity(1.0, None);
        self.highlight.set_position(
            LayerPoint {
                x: ROW_INSET,
                y: on_screen as f32 * ROW_H + 2.0,
            },
            Some(Transition::ease_out_quad(0.11)),
        );

        self.visible = shown;
        self.apply_card_height(shown, Some(Transition::ease_out_quad(0.12)));
    }

    fn card_height(&self, rows: usize) -> f32 {
        if rows == 0 {
            // No list, so no divider and no padding around it either: the card
            // is the field.
            return FIELD_H;
        }
        FIELD_H + 1.0 + LIST_PAD * 2.0 + rows as f32 * ROW_H
    }

    fn apply_card_height(&self, rows: usize, transition: Option<Transition>) {
        self.divider
            .set_opacity(if rows == 0 { 0.0 } else { 1.0 }, None);
        let height = self.card_height(rows);
        self.card
            .set_size(LayerSize::points(CARD_W, height), transition);
        self.list
            .set_size(LayerSize::points(CARD_W, rows as f32 * ROW_H), None);
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
            let top = (height - ICON) / 2.0;
            canvas.draw_image_rect_with_sampling_options(
                image,
                None,
                Rect::from_xywh(ROW_INSET + 8.0, top, ICON, ICON),
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
}
