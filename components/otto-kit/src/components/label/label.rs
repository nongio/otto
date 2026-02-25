use skia_safe::{Canvas, Color, Font, Paint, Point};

use crate::common::Renderable;
use crate::typography::TextStyle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

/// A simple text label
pub struct Label {
    pub x: f32,
    pub y: f32,
    pub width: Option<f32>,
    pub text: String,
    pub font: Font,
    pub color: Color,
    pub align: TextAlign,
}

impl Label {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: None,
            text: text.into(),
            font: crate::typography::styles::BODY.font(),
            color: Color::BLACK,
            align: TextAlign::Left,
        }
    }

    pub fn at(mut self, x: f32, y: f32) -> Self {
        self.x = x;
        self.y = y;
        self
    }

    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.font = style.font();
        self
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn with_align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    pub fn with_width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn build(self) -> Self {
        self
    }
}

impl Renderable for Label {
    fn render(&self, canvas: &Canvas) {
        let mut paint = Paint::default();
        paint.set_color(self.color);
        paint.set_anti_alias(true);

        let (text_width, _) = self.font.measure_str(&self.text, None);
        let width = self.width.unwrap_or(text_width);

        let x = match self.align {
            TextAlign::Left => self.x,
            TextAlign::Center => self.x + (width - text_width) / 2.0,
            TextAlign::Right => self.x + width - text_width,
        };

        let y = self.y + self.font.size() * 0.8;

        canvas.draw_str(&self.text, Point::new(x, y), &self.font, &paint);
    }

    fn intrinsic_size(&self) -> Option<(f32, f32)> {
        let (text_width, _) = self.font.measure_str(&self.text, None);
        let width = self.width.unwrap_or(text_width);
        Some((width, self.font.size()))
    }
}

// Backwards compatibility alias
pub type LabelBuilder = Label;
