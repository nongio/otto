use skia_safe::{Canvas, Color, Paint, PaintStyle, Rect};

use crate::common::Renderable;

/// The "this window is being shared" badge, drawn at the trailing end of a
/// titlebar the way macOS marks a window that is on a call.
///
/// A tinted pill with a display glyph inside it: readable at a glance without
/// competing with the title, and unmistakably *not* one of the window controls
/// (which live at the leading edge and are round).
#[derive(Debug, Clone)]
pub struct SharingIndicator {
    pub x: f32,
    pub y: f32,
    /// Height of the pill; the width follows from it
    pub height: f32,
    /// Unfocused windows dim the badge along with the rest of the bar
    pub active: bool,
    pub dark: bool,
}

impl Default for SharingIndicator {
    fn default() -> Self {
        Self::new()
    }
}

impl SharingIndicator {
    /// Default pill height in logical points
    pub const DEFAULT_HEIGHT: f32 = 18.0;
    /// Width as a multiple of the height — wide enough for the glyph plus the
    /// padding that makes it read as a badge rather than a stray icon.
    const ASPECT: f32 = 1.55;

    pub fn new() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            height: Self::DEFAULT_HEIGHT,
            active: true,
            dark: false,
        }
    }

    pub fn at(mut self, x: f32, y: f32) -> Self {
        self.x = x;
        self.y = y;
        self
    }

    pub fn with_height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    pub fn with_active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn with_dark(mut self, dark: bool) -> Self {
        self.dark = dark;
        self
    }

    pub fn width(&self) -> f32 {
        self.height * Self::ASPECT
    }

    pub fn bounds(&self) -> Rect {
        Rect::from_xywh(self.x, self.y, self.width(), self.height)
    }

    /// Accent of the badge: the system green macOS uses for an in-progress
    /// share, muted for an unfocused window.
    fn accent(&self) -> Color {
        if self.active {
            Color::from_rgb(0x28, 0xC8, 0x40)
        } else if self.dark {
            Color::from_rgb(0x6E, 0x8E, 0x74)
        } else {
            Color::from_rgb(0x9A, 0xB6, 0x9F)
        }
    }
}

impl Renderable for SharingIndicator {
    fn render(&self, canvas: &Canvas) {
        let accent = self.accent();
        let (w, h) = (self.width(), self.height);
        let pill = Rect::from_xywh(self.x, self.y, w, h);

        // Tinted pill behind the glyph, so the badge holds together over both a
        // light and a dark bar.
        let mut fill = Paint::default();
        fill.set_anti_alias(true);
        fill.set_color(Color::from_argb(
            if self.dark { 0x38 } else { 0x2E },
            accent.r(),
            accent.g(),
            accent.b(),
        ));
        canvas.draw_round_rect(pill, h / 2.0, h / 2.0, &fill);

        // Display glyph: a rounded screen with a stand, centered in the pill.
        let glyph_h = h * 0.52;
        let glyph_w = glyph_h * 1.35;
        let gx = self.x + (w - glyph_w) / 2.0;
        let gy = self.y + (h - glyph_h) / 2.0;
        // The stand takes the bottom sliver, so the screen sits slightly high.
        let screen_h = glyph_h * 0.70;
        let stroke = (h * 0.075).max(1.0);

        let mut line = Paint::default();
        line.set_anti_alias(true);
        line.set_style(PaintStyle::Stroke);
        line.set_stroke_width(stroke);
        line.set_stroke_cap(skia_safe::PaintCap::Round);
        line.set_color(accent);

        let screen = Rect::from_xywh(gx, gy, glyph_w, screen_h);
        let r = screen_h * 0.22;
        canvas.draw_round_rect(screen, r, r, &line);

        // Stand: a short foot centered under the screen.
        let foot_y = gy + glyph_h - stroke / 2.0;
        let foot_half = glyph_w * 0.22;
        let cx = gx + glyph_w / 2.0;
        canvas.draw_line((cx, screen.bottom), (cx, foot_y), &line);
        canvas.draw_line((cx - foot_half, foot_y), (cx + foot_half, foot_y), &line);

        // Filled dot in the screen — the "live" cue, the same language as a
        // recording light.
        let mut dot = Paint::default();
        dot.set_anti_alias(true);
        dot.set_color(accent);
        canvas.draw_circle(
            (cx, screen.top + screen_h / 2.0),
            (screen_h * 0.15).max(1.0),
            &dot,
        );
    }

    fn intrinsic_size(&self) -> Option<(f32, f32)> {
        Some((self.width(), self.height))
    }
}
