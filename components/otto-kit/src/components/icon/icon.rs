use skia_safe::{Canvas, Color, Paint, Path, Rect};

use crate::common::Renderable;

/// Common icon shapes that can be rendered
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconShape {
    Circle,
    Square,
    Triangle,
    Star,
    Heart,
    Check,
    Cross,
    Plus,
    Minus,
    ChevronUp,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
}

/// Icon styling options
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IconStyle {
    /// Filled icon with solid color
    Filled,
    /// Outlined icon with stroke
    Outlined { stroke_width: f32 },
}

/// A simple icon component
pub struct Icon {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub shape: IconShape,
    pub color: Color,
    pub style: IconStyle,
}

impl Icon {
    pub fn new(shape: IconShape) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            size: 24.0,
            shape,
            color: Color::BLACK,
            style: IconStyle::Filled,
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

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn with_style(mut self, style: IconStyle) -> Self {
        self.style = style;
        self
    }

    pub fn filled(mut self) -> Self {
        self.style = IconStyle::Filled;
        self
    }

    pub fn outlined(mut self, stroke_width: f32) -> Self {
        self.style = IconStyle::Outlined { stroke_width };
        self
    }

    pub fn build(self) -> Self {
        self
    }

    /// Create the path for the icon shape
    fn create_path(&self) -> Path {
        let mut path = Path::new();
        let center_x = self.x + self.size / 2.0;
        let center_y = self.y + self.size / 2.0;
        let radius = self.size / 2.0;

        match self.shape {
            IconShape::Circle => {
                path.add_circle((center_x, center_y), radius, None);
            }
            IconShape::Square => {
                path.add_rect(Rect::from_xywh(self.x, self.y, self.size, self.size), None);
            }
            IconShape::Triangle => {
                path.move_to((center_x, self.y));
                path.line_to((self.x + self.size, self.y + self.size));
                path.line_to((self.x, self.y + self.size));
                path.close();
            }
            IconShape::Star => {
                // 5-pointed star
                let outer_radius = radius;
                let inner_radius = radius * 0.4;
                for i in 0..10 {
                    let angle =
                        (i as f32 * std::f32::consts::PI / 5.0) - std::f32::consts::PI / 2.0;
                    let r = if i % 2 == 0 {
                        outer_radius
                    } else {
                        inner_radius
                    };
                    let x = center_x + r * angle.cos();
                    let y = center_y + r * angle.sin();
                    if i == 0 {
                        path.move_to((x, y));
                    } else {
                        path.line_to((x, y));
                    }
                }
                path.close();
            }
            IconShape::Heart => {
                let w = self.size;
                let h = self.size;
                path.move_to((center_x, self.y + h * 0.3));
                path.cubic_to(
                    (center_x, self.y + h * 0.15),
                    (self.x + w * 0.25, self.y),
                    (center_x, self.y + h * 0.15),
                );
                path.cubic_to(
                    (self.x + w * 0.75, self.y),
                    (self.x + w, self.y + h * 0.15),
                    (center_x, self.y + h * 0.3),
                );
                path.line_to((center_x, self.y + h));
                path.line_to((center_x, self.y + h * 0.3));
                path.close();
            }
            IconShape::Check => {
                path.move_to((self.x + self.size * 0.2, center_y));
                path.line_to((self.x + self.size * 0.4, self.y + self.size * 0.7));
                path.line_to((self.x + self.size * 0.8, self.y + self.size * 0.3));
                // For check, we'll use stroke mode even if filled
                return path;
            }
            IconShape::Cross => {
                let inset = self.size * 0.2;
                path.move_to((self.x + inset, self.y + inset));
                path.line_to((self.x + self.size - inset, self.y + self.size - inset));
                path.move_to((self.x + self.size - inset, self.y + inset));
                path.line_to((self.x + inset, self.y + self.size - inset));
            }
            IconShape::Plus => {
                let thickness = self.size * 0.15;
                path.move_to((center_x - thickness / 2.0, self.y));
                path.line_to((center_x + thickness / 2.0, self.y));
                path.line_to((center_x + thickness / 2.0, self.y + self.size));
                path.line_to((center_x - thickness / 2.0, self.y + self.size));
                path.close();
                path.move_to((self.x, center_y - thickness / 2.0));
                path.line_to((self.x + self.size, center_y - thickness / 2.0));
                path.line_to((self.x + self.size, center_y + thickness / 2.0));
                path.line_to((self.x, center_y + thickness / 2.0));
                path.close();
            }
            IconShape::Minus => {
                let thickness = self.size * 0.15;
                path.move_to((self.x, center_y - thickness / 2.0));
                path.line_to((self.x + self.size, center_y - thickness / 2.0));
                path.line_to((self.x + self.size, center_y + thickness / 2.0));
                path.line_to((self.x, center_y + thickness / 2.0));
                path.close();
            }
            IconShape::ChevronUp => {
                path.move_to((self.x + self.size * 0.2, self.y + self.size * 0.7));
                path.line_to((center_x, self.y + self.size * 0.3));
                path.line_to((self.x + self.size * 0.8, self.y + self.size * 0.7));
            }
            IconShape::ChevronDown => {
                path.move_to((self.x + self.size * 0.2, self.y + self.size * 0.3));
                path.line_to((center_x, self.y + self.size * 0.7));
                path.line_to((self.x + self.size * 0.8, self.y + self.size * 0.3));
            }
            IconShape::ChevronLeft => {
                path.move_to((self.x + self.size * 0.7, self.y + self.size * 0.2));
                path.line_to((self.x + self.size * 0.3, center_y));
                path.line_to((self.x + self.size * 0.7, self.y + self.size * 0.8));
            }
            IconShape::ChevronRight => {
                path.move_to((self.x + self.size * 0.3, self.y + self.size * 0.2));
                path.line_to((self.x + self.size * 0.7, center_y));
                path.line_to((self.x + self.size * 0.3, self.y + self.size * 0.8));
            }
        }

        path
    }
}

impl Renderable for Icon {
    fn render(&self, canvas: &Canvas) {
        let mut paint = Paint::default();
        paint.set_color(self.color);
        paint.set_anti_alias(true);

        match self.style {
            IconStyle::Filled => {
                paint.set_style(skia_safe::PaintStyle::Fill);
            }
            IconStyle::Outlined { stroke_width } => {
                paint.set_style(skia_safe::PaintStyle::Stroke);
                paint.set_stroke_width(stroke_width);
            }
        }

        // Special handling for stroke-only icons
        if matches!(
            self.shape,
            IconShape::Check
                | IconShape::Cross
                | IconShape::ChevronUp
                | IconShape::ChevronDown
                | IconShape::ChevronLeft
                | IconShape::ChevronRight
        ) {
            paint.set_style(skia_safe::PaintStyle::Stroke);
            let stroke_width = match self.style {
                IconStyle::Outlined { stroke_width } => stroke_width,
                IconStyle::Filled => self.size * 0.1,
            };
            paint.set_stroke_width(stroke_width);
            paint.set_stroke_cap(skia_safe::PaintCap::Round);
            paint.set_stroke_join(skia_safe::PaintJoin::Round);
        }

        let path = self.create_path();
        canvas.draw_path(&path, &paint);
    }
}
