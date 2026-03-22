use skia_safe::{Canvas, Color, Paint, Rect};

use crate::common::Renderable;

/// A group of items in a titlebar (typically window controls)
pub struct TitlebarGroup {
    items: Vec<Box<dyn Renderable>>,
    spacing: f32,
}

impl TitlebarGroup {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            spacing: 8.0,
        }
    }

    pub fn with_spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    /// Add an item to the group
    pub fn add<T: Renderable + 'static>(mut self, item: T) -> Self {
        self.items.push(Box::new(item));
        self
    }

    pub fn build(self) -> Self {
        self
    }

    fn render_at(&self, canvas: &Canvas, x: f32, y: f32, height: f32) -> f32 {
        let mut current_x = x;

        for (i, item) in self.items.iter().enumerate() {
            let (item_width, item_height) = item.intrinsic_size().unwrap_or((32.0, height));

            // Center vertically within titlebar height
            let item_y = y + (height - item_height) / 2.0;

            // Render the item
            canvas.save();
            canvas.translate((current_x, item_y));
            item.render(canvas);
            canvas.restore();

            current_x += item_width;

            // Add spacing between items (except after last item)
            if i < self.items.len() - 1 {
                current_x += self.spacing;
            }
        }

        current_x - x // Return total width used
    }

    fn measure_width(&self, height: f32) -> f32 {
        let mut total_width = 0.0;
        for (i, item) in self.items.iter().enumerate() {
            let (item_width, _) = item.intrinsic_size().unwrap_or((32.0, height));
            total_width += item_width;

            if i < self.items.len() - 1 {
                total_width += self.spacing;
            }
        }
        total_width
    }
}

impl Default for TitlebarGroup {
    fn default() -> Self {
        Self::new()
    }
}

/// A horizontal titlebar component with centered text and trailing controls
pub struct Titlebar {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,

    // Styling
    background_color: Option<Color>,
    border_color: Option<Color>,
    padding: f32,

    // Content
    title: Option<Box<dyn Renderable>>,
    controls: Option<TitlebarGroup>,
}

impl Titlebar {
    /// Create a new titlebar
    pub fn new() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 28.0, // Compact titlebar height
            background_color: None,
            border_color: None,
            padding: 8.0,
            title: None,
            controls: None,
        }
    }

    pub fn at(mut self, x: f32, y: f32) -> Self {
        self.x = x;
        self.y = y;
        self
    }

    pub fn with_width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn with_height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    pub fn with_background(mut self, color: Color) -> Self {
        self.background_color = Some(color);
        self
    }

    pub fn with_border_bottom(mut self, color: Color) -> Self {
        self.border_color = Some(color);
        self
    }

    pub fn with_padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    /// Set the centered title
    pub fn with_title<T: Renderable + 'static>(mut self, title: T) -> Self {
        self.title = Some(Box::new(title));
        self
    }

    /// Set the window controls (right-aligned)
    pub fn with_controls(mut self, controls: TitlebarGroup) -> Self {
        self.controls = Some(controls);
        self
    }

    pub fn build(self) -> Self {
        self
    }
}

impl Default for Titlebar {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderable for Titlebar {
    fn render(&self, canvas: &Canvas) {
        // Save canvas state
        canvas.save();

        // Clip to titlebar bounds to prevent overflow
        let clip_rect = Rect::from_xywh(self.x, self.y, self.width, self.height);
        canvas.clip_rect(clip_rect, None, Some(true));

        // Draw background
        if let Some(bg_color) = self.background_color {
            let rect = Rect::from_xywh(self.x, self.y, self.width, self.height);
            let mut paint = Paint::default();
            paint.set_color(bg_color);
            paint.set_anti_alias(true);
            canvas.draw_rect(rect, &paint);
        }

        let content_height = self.height - self.padding * 2.0;

        // Render controls (right side) first to know how much space they take
        let controls_width = if let Some(ref controls) = self.controls {
            controls.measure_width(content_height)
        } else {
            0.0
        };

        // Calculate available space for title (accounting for controls and padding)
        let reserved_right = if controls_width > 0.0 {
            controls_width + self.padding * 2.0
        } else {
            0.0
        };

        // Render centered title
        if let Some(ref title) = self.title {
            if let Some((title_width, title_height)) = title.intrinsic_size() {
                // Center within available space (excluding controls area)
                let available_width = self.width - reserved_right;
                let center_x = self.x + (available_width - title_width) / 2.0;
                let center_y = self.y + (self.height - title_height) / 2.0;

                canvas.save();
                canvas.translate((center_x, center_y));
                title.render(canvas);
                canvas.restore();
            }
        }

        // Render controls on the right
        if let Some(ref controls) = self.controls {
            let controls_x = self.x + self.width - controls_width - self.padding;
            let controls_y = self.y + self.padding;
            controls.render_at(canvas, controls_x, controls_y, content_height);
        }

        // Restore before drawing border (so border isn't clipped)
        canvas.restore();

        // Draw bottom border (outside clip region to ensure full width)
        if let Some(border_color) = self.border_color {
            let mut paint = Paint::default();
            paint.set_color(border_color);
            paint.set_style(skia_safe::PaintStyle::Stroke);
            paint.set_stroke_width(1.0);
            paint.set_anti_alias(true);

            let y = self.y + self.height - 0.5;
            canvas.draw_line((self.x, y), (self.x + self.width, y), &paint);
        }
    }
}
