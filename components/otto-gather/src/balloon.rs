//! The "Ask about…" card: what is gathered, as Ask shows attachments, under
//! a title and over the shortcut that sends them to Ask.

// Rust guideline compliant 2026-02-21

use otto_kit::color_scheme::current_color_scheme;
use otto_kit::components::attachments::{self, AttachmentList, Options, HOVER_PAD};
use otto_kit::components::scroll::ScrollView;
use otto_kit::skia::font_style::{Slant, Weight, Width};
use otto_kit::skia::textlayout::{
    FontCollection, Paragraph, ParagraphBuilder, ParagraphStyle, TextStyle,
};
use otto_kit::skia::{
    surfaces, AlphaType, Color, ColorType, FontMgr, FontStyle, ImageInfo, Paint, PaintStyle, Point,
    RRect, Rect,
};
use otto_kit::theme::Theme;
use otto_kit::typography::styles;

use crate::request::Gathering;

// Everything in logical pixels.
const WIDTH: f32 = 360.0;
const PAD: f32 = 18.0;
const INNER: f32 = WIDTH - 2.0 * PAD;
const GAP: f32 = 16.0;
/// The title: the launcher's field size, heavier.
const TITLE_SIZE: f32 = 15.0;
const TITLE_LINE_H: f32 = 20.0;
/// Space between the count and the Clear button, and how far around the
/// button a press still hits it.
const COUNT_GAP: f32 = 12.0;
const CLEAR_SLOP: f32 = 6.0;
/// The card's corner radius, as the launcher's shows on screen.
pub const RADIUS: f32 = 20.0;
/// How solid the card is drawn when the compositor can't frost it.
const UNFROSTED_MIN_ALPHA: u8 = 0xF6;
/// The shortest the items' viewport gets, however little room there is.
const MIN_VIEW_H: f32 = 48.0;

/// What a press on the card does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// Take the item at this index out of the gathering.
    Remove(usize),
    /// Throw the whole gathering away.
    Clear,
}

/// Draws the card.
pub struct Balloon {
    fonts: FontCollection,
    list: AttachmentList,
    /// The colours of the scheme in use, as of the last layout.
    theme: Theme,
    /// Whether the compositor draws the card's material under it.
    pub frosted: bool,
    /// The shortcut that sends, as Otto shows it, when one is bound.
    pub send_shortcut: Option<String>,
}

/// A card laid out for some items, ready to draw.
pub struct Layout {
    /// The title and the shortcut, fixed at the top and the bottom.
    fixed: Vec<(Paragraph, Point)>,
    /// The items, scrolled within `viewport`; `None` when there are none.
    items: Option<attachments::Layout>,
    /// Where the items show on the card.
    pub viewport: Rect,
    /// The button that clears the gathering, when there is something to
    /// clear.
    clear: Option<Rect>,
    /// How tall the items are, scrolled or not.
    pub body_length: f32,
    /// Size in logical pixels.
    pub width: f32,
    pub height: f32,
}

impl Layout {
    /// Where `(x, y)` on the card falls on the items, scrolled by `offset`.
    fn on_items(&self, x: f32, y: f32, offset: f32) -> Option<(&attachments::Layout, f32, f32)> {
        if !(self.viewport.top..self.viewport.bottom).contains(&y) {
            return None;
        }
        let items = self.items.as_ref()?;
        Some((items, x - PAD, y - self.viewport.top + offset - HOVER_PAD))
    }

    /// The item at `(x, y)` on the card, with the items scrolled by `offset`.
    pub fn item_at(&self, x: f32, y: f32, offset: f32) -> Option<usize> {
        let (items, x, y) = self.on_items(x, y, offset)?;
        items.item_at(x, y)
    }

    /// What a press at `(x, y)` on the card does, with the items scrolled by
    /// `offset`.
    pub fn hit(&self, x: f32, y: f32, offset: f32) -> Option<Hit> {
        if self
            .clear
            .is_some_and(|c| (c.left..c.right).contains(&x) && (c.top..c.bottom).contains(&y))
        {
            return Some(Hit::Clear);
        }
        let (items, x, y) = self.on_items(x, y, offset)?;
        items.remove_at(x, y).map(Hit::Remove)
    }
}

impl Default for Balloon {
    fn default() -> Self {
        let mut fonts = FontCollection::new();
        fonts.set_default_font_manager(FontMgr::new(), None);
        Self {
            fonts,
            list: AttachmentList::default(),
            theme: Theme::for_scheme(current_color_scheme()),
            frosted: false,
            send_shortcut: None,
        }
    }
}

/// A text style: Otto's body face, or monospace.
fn style(size: f32, line_h: f32, weight: i32, color: Color, mono: bool) -> TextStyle {
    let mut style = TextStyle::new();
    style.set_font_size(size);
    if mono {
        style.set_font_families(&["monospace"]);
    } else {
        style.set_font_families(&[styles::BODY.family, "sans-serif"]);
    }
    style.set_font_style(FontStyle::new(
        Weight::from(weight),
        Width::NORMAL,
        Slant::Upright,
    ));
    style.set_color(color);
    style.set_height(line_h / size);
    style.set_height_override(true);
    style
}

impl Balloon {
    /// One line of `text`, cut with an ellipsis at the card's inner width.
    fn line(&self, text: &str, style: &TextStyle) -> Paragraph {
        let mut paragraph_style = ParagraphStyle::new();
        paragraph_style.set_max_lines(1);
        paragraph_style.set_ellipsis("…");
        let mut builder = ParagraphBuilder::new(&paragraph_style, &self.fonts);
        builder.push_style(style);
        builder.add_text(text);
        let mut paragraph = builder.build();
        paragraph.layout(INNER);
        paragraph
    }

    /// Lay the card out for `gathering`, no taller than `max_height`: past
    /// that the items scroll between the title and the send shortcut. The
    /// item at `leaving.0` is on its way out, as much of it left as
    /// `leaving.1` says.
    pub fn layout(
        &mut self,
        gathering: &Gathering,
        leaving: Option<(usize, f32)>,
        max_height: f32,
    ) -> Layout {
        self.theme = Theme::for_scheme(current_color_scheme());
        let items = gathering.items.as_slice();
        let (primary, secondary) = (self.theme.text_primary, self.theme.text_secondary);

        let mut fixed = Vec::new();
        let title = self.line(
            "Ask about…",
            &style(TITLE_SIZE, TITLE_LINE_H, 700, primary, false),
        );
        fixed.push((title, Point::new(PAD, PAD)));
        let mut clear = None;
        if !items.is_empty() {
            // Clear, at the right of the title, and the count before it.
            let button = self.line("Clear", &style(13.0, TITLE_LINE_H, 500, secondary, false));
            let button_w = button.max_intrinsic_width().ceil();
            let button_x = PAD + INNER - button_w;
            clear = Some(
                Rect::from_xywh(button_x, PAD, button_w, TITLE_LINE_H)
                    .with_outset((CLEAR_SLOP, CLEAR_SLOP)),
            );
            fixed.push((button, Point::new(button_x, PAD)));

            let included = gathering.included();
            let count = if included == items.len() {
                included.to_string()
            } else {
                format!("{included} of {}", items.len())
            };
            let count = self.line(&count, &style(11.5, 18.0, 400, secondary, true));
            let x = button_x - COUNT_GAP - count.max_intrinsic_width().ceil();
            fixed.push((count, Point::new(x, PAD + 1.0)));
        }
        // The items' highlights reach up into the gap under the title.
        let head_h = PAD + TITLE_LINE_H + GAP - HOVER_PAD;

        let listed: Vec<_> = items
            .iter()
            .enumerate()
            .map(|(index, item)| (item, gathering.is_struck(index)))
            .collect();
        let list = (!items.is_empty()).then(|| {
            let options = Options {
                width: INNER,
                newest_first: true,
                removable: true,
            };
            self.list
                .layout_leaving(&listed, leaving, options, &self.theme)
        });
        let body_length = list
            .as_ref()
            .map_or(16.0, |list| list.height + 2.0 * HOVER_PAD);
        if list.is_none() {
            let hint = self.line(
                "Select something, then add it",
                &style(11.0, 16.0, 400, secondary, false),
            );
            fixed.push((hint, Point::new(PAD, head_h + HOVER_PAD)));
        }

        // The shortcut that sends, under the items.
        let hint = self.send_shortcut.clone().filter(|_| !items.is_empty());
        let foot_h = if hint.is_some() {
            GAP - HOVER_PAD + 18.0 + PAD
        } else {
            PAD - HOVER_PAD
        };
        let view_h = body_length
            .min(max_height - head_h - foot_h)
            .max(MIN_VIEW_H)
            .floor();
        let viewport = Rect::from_xywh(0.0, head_h, WIDTH, view_h);
        if let Some(keys) = hint {
            let y = viewport.bottom + GAP - HOVER_PAD;
            let label = self.line("Send to Ask", &style(11.0, 16.0, 400, secondary, false));
            fixed.push((label, Point::new(PAD, y + 1.0)));
            let keys = self.line(&keys, &style(11.5, 18.0, 500, primary, true));
            let x = PAD + INNER - keys.max_intrinsic_width().ceil();
            fixed.push((keys, Point::new(x, y)));
        }

        Layout {
            fixed,
            items: list,
            viewport,
            clear,
            body_length,
            width: WIDTH,
            height: (viewport.bottom + foot_h).ceil(),
        }
    }

    /// The files on the card whose thumbnails are still to be made.
    pub fn thumbnails_wanted(&mut self) -> Vec<std::path::PathBuf> {
        self.list.thumbnails_wanted()
    }

    /// Hand in the thumbnail of `path`, or that it has none.
    pub fn set_thumbnail(&mut self, path: &std::path::Path, image: Option<otto_kit::skia::Image>) {
        self.list.set_thumbnail(path, image);
    }

    /// Draw `layout` into `pixels`, an ARGB8888 buffer of `width` × `height`
    /// pixels at `scale` pixels per logical pixel, with the items scrolled
    /// as `scroll` has them.
    pub fn draw(
        &self,
        layout: &Layout,
        scroll: &ScrollView,
        hovered: Option<usize>,
        pixels: &mut [u8],
        (width, height): (i32, i32),
        scale: f32,
    ) {
        // ARGB8888 on little endian is B, G, R, A in memory.
        let info = ImageInfo::new(
            (width, height),
            ColorType::BGRA8888,
            AlphaType::Premul,
            None,
        );
        let Some(mut surface) = surfaces::wrap_pixels(&info, pixels, None, None) else {
            tracing::warn!("cannot draw into the balloon buffer");
            return;
        };
        let canvas = surface.canvas();
        canvas.clear(Color::TRANSPARENT);
        canvas.scale((scale, scale));

        // The compositor frosts the card, rounds it and edges it with the
        // hairline, as it does the launcher's; drawn here only when there is
        // no one to do it.
        if !self.frosted {
            let radius = otto_kit::corners::radius(RADIUS);
            let card = Rect::from_xywh(0.0, 0.0, layout.width, layout.height);
            let material = self.theme.material_popup;
            let mut fill = Paint::default();
            fill.set_anti_alias(true);
            fill.set_color(Color::from_argb(
                material.a().max(UNFROSTED_MIN_ALPHA),
                material.r(),
                material.g(),
                material.b(),
            ));
            canvas.draw_rrect(RRect::new_rect_xy(card, radius, radius), &fill);
            let mut stroke = fill;
            stroke.set_style(PaintStyle::Stroke);
            stroke.set_stroke_width(1.0);
            stroke.set_color(self.theme.hairline);
            let edge = card.with_inset((0.5, 0.5));
            let edge_radius = (radius - 0.5).max(0.0);
            canvas.draw_rrect(RRect::new_rect_xy(edge, edge_radius, edge_radius), &stroke);
        }

        for (paragraph, at) in &layout.fixed {
            paragraph.paint(canvas, *at);
        }
        if let Some(items) = layout.items.as_ref() {
            scroll.render(canvas, &self.theme, |canvas, _| {
                canvas.save();
                canvas.translate((PAD, HOVER_PAD));
                self.list.paint(canvas, items, &self.theme, hovered);
                canvas.restore();
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Item;

    #[test]
    fn remove_buttons_and_items_can_be_hit() {
        let mut balloon = Balloon::default();
        let mut gathering = Gathering::default();
        gathering.add(Item::Text("one".into()));
        gathering.add(Item::Text("two".into()));
        let layout = balloon.layout(&gathering, None, 800.0);
        // The newest leads, with its remove button beside its first line.
        let top = layout.viewport.top + HOVER_PAD;
        assert_eq!(
            layout.hit(PAD + INNER - 6.0, top + 11.0, 0.0),
            Some(Hit::Remove(1))
        );
        assert_eq!(layout.item_at(PAD + 20.0, top + 11.0, 0.0), Some(1));
        assert_eq!(layout.hit(1.0, 1.0, 0.0), None);
        // Clear sits at the right end of the title line.
        assert_eq!(
            layout.hit(PAD + INNER - 4.0, PAD + TITLE_LINE_H / 2.0, 0.0),
            Some(Hit::Clear)
        );
        let empty = balloon.layout(&Gathering::default(), None, 800.0);
        assert_eq!(
            empty.hit(PAD + INNER - 4.0, PAD + TITLE_LINE_H / 2.0, 0.0),
            None
        );
    }

    #[test]
    fn many_items_scroll_within_the_height() {
        let mut balloon = Balloon::default();
        let mut gathering = Gathering::default();
        for i in 0..30 {
            gathering.add(Item::Text(format!("item {i}")));
        }
        let layout = balloon.layout(&gathering, None, 500.0);
        assert!(layout.height <= 500.0);
        assert!(layout.body_length > layout.viewport.height());
        // Scrolled to the end, the oldest item is under the pointer at the
        // bottom of the viewport.
        let offset = layout.body_length - layout.viewport.height();
        let y = layout.viewport.bottom - HOVER_PAD - 8.0;
        assert_eq!(layout.item_at(PAD + 20.0, y, offset), Some(0));
    }
}
