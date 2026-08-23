use skia_safe::{Canvas, Color, Paint, PaintStyle, PathBuilder, Point, Rect};

use crate::common::Renderable;

/// Which window control a dot represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowControl {
    Close,
    Minimize,
    Zoom,
}

/// Traffic-light window controls: three round buttons that stay colored while
/// the window is active, gray out when it is not, and reveal their glyph when
/// the pointer is over the group.
#[derive(Debug, Clone)]
pub struct WindowControls {
    pub x: f32,
    pub y: f32,
    /// Diameter of a single dot
    pub size: f32,
    pub spacing: f32,
    /// Colored (focused window) vs gray (unfocused)
    pub active: bool,
    /// Glyphs appear when the pointer is anywhere over the group
    pub hovered: bool,
    /// Control currently pressed, drawn a shade darker
    pub pressed: Option<WindowControl>,
    /// Controls the window does not support are drawn gray even when active
    pub disabled: Vec<WindowControl>,
    /// Dark titlebars need a slightly lighter gray to read against
    pub dark: bool,
}

impl Default for WindowControls {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowControls {
    pub fn new() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            size: 12.0,
            spacing: 8.0,
            active: true,
            hovered: false,
            pressed: None,
            disabled: Vec::new(),
            dark: false,
        }
    }

    pub fn at(mut self, x: f32, y: f32) -> Self {
        self.x = x;
        self.y = y;
        self
    }

    pub fn with_size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn with_spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn with_active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn with_hovered(mut self, hovered: bool) -> Self {
        self.hovered = hovered;
        self
    }

    pub fn with_pressed(mut self, pressed: Option<WindowControl>) -> Self {
        self.pressed = pressed;
        self
    }

    pub fn with_disabled(mut self, disabled: Vec<WindowControl>) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn with_dark(mut self, dark: bool) -> Self {
        self.dark = dark;
        self
    }

    /// Total width of the group
    pub fn width(&self) -> f32 {
        self.size * 3.0 + self.spacing * 2.0
    }

    /// Bounding box of the group in the coordinate space it is rendered in
    pub fn bounds(&self) -> Rect {
        Rect::from_xywh(self.x, self.y, self.width(), self.size)
    }

    /// Which control (if any) sits under a point — for hit testing in the
    /// compositor or in a client-drawn titlebar.
    pub fn control_at(&self, px: f32, py: f32) -> Option<WindowControl> {
        if py < self.y || py > self.y + self.size {
            return None;
        }
        for (i, control) in Self::ORDER.iter().enumerate() {
            let cx = self.x + (self.size + self.spacing) * i as f32;
            if px >= cx && px <= cx + self.size {
                return Some(*control);
            }
        }
        None
    }

    const ORDER: [WindowControl; 3] = [
        WindowControl::Close,
        WindowControl::Minimize,
        WindowControl::Zoom,
    ];

    fn dot_color(&self, control: WindowControl) -> Color {
        if !self.active || self.disabled.contains(&control) {
            return if self.dark {
                Color::from_rgb(0x4E, 0x4E, 0x50)
            } else {
                Color::from_rgb(0xD3, 0xD3, 0xD5)
            };
        }
        // Tinted from the user's accent so a focused window reads as focused:
        // close takes a dark shade of it, the other two a light one.
        let accent =
            crate::accent::current_accent().unwrap_or_else(|| Color::from_rgb(0x0A, 0x84, 0xFF));
        let base = match control {
            WindowControl::Close => shade(accent, -0.28),
            WindowControl::Minimize | WindowControl::Zoom => shade(accent, 0.58),
        };
        if self.pressed == Some(control) {
            // ~20% darker while held
            Color::from_rgb(
                (base.0 as f32 * 0.8) as u8,
                (base.1 as f32 * 0.8) as u8,
                (base.2 as f32 * 0.8) as u8,
            )
        } else {
            Color::from_rgb(base.0, base.1, base.2)
        }
    }

    fn draw_glyph(&self, canvas: &Canvas, control: WindowControl, cx: f32, cy: f32) {
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_style(PaintStyle::Stroke);
        paint.set_stroke_width((self.size * 0.09).max(1.0));
        paint.set_stroke_cap(skia_safe::PaintCap::Round);
        // Close sits on a dark dot, so its glyph is light; the others stay dark
        paint.set_color(match control {
            WindowControl::Close => Color::from_argb(0xD0, 0xFF, 0xFF, 0xFF),
            _ => Color::from_argb(0xB0, 0x00, 0x00, 0x00),
        });

        // Glyphs are drawn inside ~44% of the dot so they never touch the rim
        let r = self.size * 0.22;
        match control {
            WindowControl::Close => {
                canvas.draw_line((cx - r, cy - r), (cx + r, cy + r), &paint);
                canvas.draw_line((cx + r, cy - r), (cx - r, cy + r), &paint);
            }
            WindowControl::Minimize => {
                canvas.draw_line((cx - r, cy), (cx + r, cy), &paint);
            }
            WindowControl::Zoom => {
                // Two opposing filled triangles pointing out of the corners
                paint.set_style(PaintStyle::Fill);
                let mut builder = PathBuilder::new();
                builder.move_to(Point::new(cx - r, cy - r));
                builder.line_to(Point::new(cx + r * 0.15, cy - r));
                builder.line_to(Point::new(cx - r, cy + r * 0.15));
                builder.close();
                builder.move_to(Point::new(cx + r, cy + r));
                builder.line_to(Point::new(cx - r * 0.15, cy + r));
                builder.line_to(Point::new(cx + r, cy - r * 0.15));
                builder.close();
                canvas.draw_path(&builder.detach(), &paint);
            }
        }
    }
}

/// Mix a colour toward black (`amount < 0`) or white (`amount > 0`).
fn shade(color: Color, amount: f32) -> (u8, u8, u8) {
    let mix = |c: u8| -> u8 {
        let c = c as f32;
        let target = if amount < 0.0 { 0.0 } else { 255.0 };
        (c + (target - c) * amount.abs()) as u8
    };
    (mix(color.r()), mix(color.g()), mix(color.b()))
}

impl Renderable for WindowControls {
    fn render(&self, canvas: &Canvas) {
        let radius = self.size / 2.0;
        for (i, control) in Self::ORDER.iter().enumerate() {
            let cx = self.x + (self.size + self.spacing) * i as f32 + radius;
            let cy = self.y + radius;
            let base = self.dot_color(*control);

            let mut paint = Paint::default();
            paint.set_anti_alias(true);
            paint.set_color(base);
            canvas.draw_circle((cx, cy), radius, &paint);

            // Hairline rim: keeps the dots from bleeding into a light bar
            let mut rim = Paint::default();
            rim.set_anti_alias(true);
            rim.set_style(PaintStyle::Stroke);
            rim.set_stroke_width(0.5);
            rim.set_color(Color::from_argb(0x33, 0x00, 0x00, 0x00));
            canvas.draw_circle((cx, cy), radius - 0.25, &rim);

            if self.hovered && self.active && !self.disabled.contains(control) {
                self.draw_glyph(canvas, *control, cx, cy);
            }
        }
    }

    fn intrinsic_size(&self) -> Option<(f32, f32)> {
        Some((self.width(), self.size))
    }
}
