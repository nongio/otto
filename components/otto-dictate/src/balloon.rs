//! The balloon under the caret: the equaliser on the left and the words heard
//! so far on the right, settled words bright and the unsettled tail dimmed.
//! It grows with the text up to a few lines, then shows the latest ones.

// Rust guideline compliant 2026-02-21

use otto_kit::typography::styles;
use otto_kit::skia::font_style::{Slant, Weight, Width};
use otto_kit::skia::textlayout::{FontCollection, Paragraph, ParagraphBuilder, ParagraphStyle, TextStyle};
use otto_kit::skia::{
    AlphaType, Color, ColorType, FontMgr, FontStyle, ImageInfo, Paint, PathBuilder, RRect, Rect,
};

/// Everything in logical pixels.
const TAIL_H: f32 = 6.0;
const TAIL_W: f32 = 10.0;
/// Where the tail's tip sits from the left edge: the popup's left edge is the
/// caret, and a tip right at the corner would cut into the rounding.
const TAIL_TIP_X: f32 = 6.0;
const RADIUS: f32 = 10.0;
const PAD_X: f32 = 10.0;
const PAD_Y: f32 = 6.0;
const BARS_W: f32 = 26.0;
const BARS_H: f32 = 14.0;
const GAP: f32 = 8.0;
const LINE_H: f32 = 18.0;
const MAX_TEXT_W: f32 = 340.0;
const MAX_LINES: f32 = 3.0;

/// Draws the balloon.
pub struct Balloon {
    fonts: FontCollection,
}

/// A balloon laid out for some text, ready to draw.
pub struct Layout {
    paragraph: Paragraph,
    /// Size in logical pixels, tail included.
    pub width: f32,
    pub height: f32,
}

impl Default for Balloon {
    fn default() -> Self {
        let mut fonts = FontCollection::new();
        fonts.set_default_font_manager(FontMgr::new(), None);
        Self { fonts }
    }
}

impl Balloon {
    /// Lay out `settled` and `tentative` text; with neither, a hint.
    pub fn layout(&self, settled: &str, tentative: &str) -> Layout {
        let mut builder = ParagraphBuilder::new(&ParagraphStyle::new(), &self.fonts);
        // Otto's body text, wrapped by Skia's paragraph layout.
        let body = styles::BODY;
        let style = |alpha: u8| {
            let mut style = TextStyle::new();
            style.set_font_size(body.size);
            style.set_font_families(&[body.family, "sans-serif"]);
            style.set_font_style(FontStyle::new(
                Weight::from(body.weight),
                Width::NORMAL,
                Slant::Upright,
            ));
            style.set_color(Color::from_argb(alpha, 255, 255, 255));
            style.set_height(LINE_H / body.size);
            style.set_height_override(true);
            style
        };
        if settled.is_empty() && tentative.is_empty() {
            builder.push_style(&style(120));
            builder.add_text("Listening…");
        } else {
            builder.push_style(&style(255));
            builder.add_text(settled);
            builder.push_style(&style(140));
            builder.add_text(tentative);
        }
        let mut paragraph = builder.build();
        paragraph.layout(MAX_TEXT_W);
        let text_w = paragraph.longest_line().ceil();
        let text_h = paragraph.height().min(LINE_H * MAX_LINES);
        let width = PAD_X + BARS_W + GAP + text_w + PAD_X;
        let height = TAIL_H + PAD_Y + text_h.max(LINE_H) + PAD_Y;
        Layout {
            paragraph,
            width: width.ceil(),
            height: height.ceil(),
        }
    }

    /// Draw `layout` with bars at `levels` into `pixels`, an ARGB8888 buffer
    /// of `width` × `height` pixels at `scale` pixels per logical pixel.
    pub fn draw(
        &self,
        layout: &Layout,
        levels: &[f32],
        pixels: &mut [u8],
        width: i32,
        height: i32,
        scale: f32,
    ) {
        // ARGB8888 on little endian is B, G, R, A in memory.
        let info = ImageInfo::new((width, height), ColorType::BGRA8888, AlphaType::Premul, None);
        let Some(mut surface) =
            otto_kit::skia::surfaces::wrap_pixels(&info, pixels, None, None)
        else {
            tracing::warn!("cannot draw into the popup buffer");
            return;
        };
        let canvas = surface.canvas();
        canvas.clear(Color::TRANSPARENT);
        canvas.scale((scale, scale));

        let mut fill = Paint::default();
        fill.set_anti_alias(true);
        fill.set_color(Color::from_argb(225, 22, 22, 24));
        let body = Rect::from_xywh(0.0, TAIL_H, layout.width, layout.height - TAIL_H);
        canvas.draw_rrect(RRect::new_rect_xy(body, RADIUS, RADIUS), &fill);
        let mut tail = PathBuilder::new();
        tail.move_to((TAIL_TIP_X, 0.0));
        tail.line_to((TAIL_TIP_X + TAIL_W / 2.0, TAIL_H + 1.0));
        tail.line_to((TAIL_TIP_X - TAIL_W / 2.0 + 2.0, TAIL_H + 1.0));
        tail.close();
        canvas.draw_path(&tail.detach(), &fill);

        // Bars, vertically centred on the first line.
        let first_line_mid = TAIL_H + PAD_Y + LINE_H / 2.0;
        let count = levels.len().max(1) as f32;
        let bar_w = 3.0;
        let step = (BARS_W - bar_w) / (count - 1.0).max(1.0);
        let mut bar = Paint::default();
        bar.set_anti_alias(true);
        bar.set_color(Color::WHITE);
        for (i, level) in levels.iter().enumerate() {
            let h = (BARS_H * level).max(bar_w);
            let x = PAD_X + i as f32 * step;
            let rect = Rect::from_xywh(x, first_line_mid - h / 2.0, bar_w, h);
            canvas.draw_rrect(RRect::new_rect_xy(rect, bar_w / 2.0, bar_w / 2.0), &bar);
        }

        // The text, scrolled so the latest lines show when it outgrows the
        // balloon.
        let text_x = PAD_X + BARS_W + GAP;
        let text_top = TAIL_H + PAD_Y;
        let visible = LINE_H * MAX_LINES;
        let scroll = (layout.paragraph.height() - visible).max(0.0);
        canvas.save();
        canvas.clip_rect(
            Rect::from_xywh(text_x, text_top, MAX_TEXT_W, visible),
            None,
            true,
        );
        layout.paragraph.paint(canvas, (text_x, text_top - scroll));
        canvas.restore();
    }
}
