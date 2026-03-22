use otto_kit::prelude::*;
use otto_kit::typography;
use skia_safe::{Canvas, Paint, Rect, TextBlob};

use crate::clock::Clock;
use crate::config::*;

/// Renders the bar background and its three zones onto a Skia canvas.
pub struct Bar {
    pub clock: Clock,
    /// Focused app name shown in the left zone.
    pub app_name: String,
    /// Logical width of the bar (set on configure).
    pub width: f32,
    /// Logical height of the bar.
    pub height: f32,
}

impl Bar {
    pub fn new() -> Self {
        Self {
            clock: Clock::new(),
            app_name: "Otto".into(),
            width: BAR_WIDTH as f32,
            height: BAR_HEIGHT as f32,
        }
    }

    /// Draw the entire bar onto the given canvas. The canvas coordinate space
    /// is in physical pixels (already scaled by buffer_scale).
    pub fn draw(&self, canvas: &Canvas, scale: f32) {
        let theme = AppContext::current_theme();
        let w = self.width * scale;
        let h = self.height * scale;

        // Background fill (semi-transparent, composited with blur by the compositor)
        let mut bg = Paint::default();
        bg.set_anti_alias(true);
        bg.set_color(theme.material_titlebar);
        canvas.draw_rect(Rect::from_xywh(0.0, 0.0, w, h), &bg);

        // Left zone: app name
        self.draw_app_name(canvas, scale, &theme);

        // Right zone: clock
        self.draw_clock(canvas, scale, &theme);
    }

    fn draw_app_name(&self, canvas: &Canvas, scale: f32, theme: &Theme) {
        let font = typography::styles::FOOTNOTE_EMPHASIZED.font_scaled(scale);
        let x = BAR_PADDING_H * scale;
        let y = self.baseline_y(scale, &font);

        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(theme.text_primary);

        if let Some(blob) = TextBlob::new(&self.app_name, &font) {
            canvas.draw_text_blob(&blob, (x, y), &paint);
        }
    }

    fn draw_clock(&self, canvas: &Canvas, scale: f32, theme: &Theme) {
        let font = typography::styles::FOOTNOTE.font_scaled(scale);
        let text = &self.clock.text;
        let text_width = font.measure_str(text, None).0;

        let w = self.width * scale;
        let x = w - text_width - BAR_PADDING_H * scale;
        let y = self.baseline_y(scale, &font);

        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(theme.text_primary);

        if let Some(blob) = TextBlob::new(text, &font) {
            canvas.draw_text_blob(&blob, (x, y), &paint);
        }
    }

    /// Vertically center text using font metrics.
    fn baseline_y(&self, scale: f32, font: &skia_safe::Font) -> f32 {
        let (_, metrics) = font.metrics();
        let text_height = metrics.descent - metrics.ascent;
        let h = self.height * scale;
        (h - text_height) / 2.0 - metrics.ascent
    }
}
