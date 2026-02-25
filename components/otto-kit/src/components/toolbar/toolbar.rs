use skia_safe::{Canvas, Color, Paint, Rect};

use crate::common::Renderable;

/// A group of items in a toolbar
pub struct ToolbarGroup {
    items: Vec<ToolbarItem>,
    spacing: f32,
}

enum ToolbarItem {
    Renderable(Box<dyn Renderable>),
    Separator(ToolbarSeparator),
    Space(ToolbarSpace),
}

impl ToolbarGroup {
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
        self.items.push(ToolbarItem::Renderable(Box::new(item)));
        self
    }

    /// Add a separator (vertical line)
    pub fn add_separator(mut self) -> Self {
        self.items
            .push(ToolbarItem::Separator(ToolbarSeparator::new()));
        self
    }

    /// Add spacing (empty space)
    pub fn add_space(mut self, width: f32) -> Self {
        self.items
            .push(ToolbarItem::Space(ToolbarSpace::new(width)));
        self
    }

    pub fn build(self) -> Self {
        self
    }

    fn render_at(&self, canvas: &Canvas, x: f32, y: f32, height: f32) -> f32 {
        let mut current_x = x;

        for (i, item) in self.items.iter().enumerate() {
            match item {
                ToolbarItem::Space(space) => {
                    current_x += space.width;
                }
                ToolbarItem::Separator(sep) => {
                    // Draw separator
                    let sep_y1 = y + height * 0.2;
                    let sep_y2 = y + height * 0.8;

                    let mut paint = Paint::default();
                    paint.set_color(sep.color);
                    paint.set_style(skia_safe::PaintStyle::Stroke);
                    paint.set_stroke_width(1.0);
                    paint.set_anti_alias(true);

                    canvas.draw_line((current_x, sep_y1), (current_x, sep_y2), &paint);
                    current_x += 1.0;
                }
                ToolbarItem::Renderable(renderable) => {
                    // Get item size
                    let (item_width, item_height) =
                        renderable.intrinsic_size().unwrap_or((40.0, height));

                    // Center vertically within toolbar height
                    let item_y = y + (height - item_height) / 2.0;

                    // Render the item
                    canvas.save();
                    canvas.translate((current_x, item_y));
                    renderable.render(canvas);
                    canvas.restore();

                    current_x += item_width;
                }
            }

            // Add spacing between items (except after last item or separator)
            if i < self.items.len() - 1 && !matches!(item, ToolbarItem::Separator(_)) {
                current_x += self.spacing;
            }
        }

        current_x - x // Return total width used
    }
}

impl Default for ToolbarGroup {
    fn default() -> Self {
        Self::new()
    }
}

/// A horizontal toolbar component with leading and trailing groups
pub struct Toolbar {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,

    // Styling
    background_color: Option<Color>,
    border_color: Option<Color>,
    border_bottom_only: bool,
    padding: f32,

    // Content groups
    leading: Option<ToolbarGroup>,
    trailing: Option<ToolbarGroup>,
}

impl Toolbar {
    /// Create a new toolbar
    pub fn new() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 0.0, // Auto-calculated if 0
            height: 52.0,
            background_color: None,
            border_color: None,
            border_bottom_only: false,
            padding: 8.0,
            leading: None,
            trailing: None,
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

    pub fn with_border(mut self, color: Color) -> Self {
        self.border_color = Some(color);
        self
    }

    pub fn with_border_bottom(mut self, color: Color) -> Self {
        self.border_color = Some(color);
        self.border_bottom_only = true;
        self
    }

    pub fn with_padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    /// Set the leading (left-aligned) group of items
    pub fn with_leading(mut self, group: ToolbarGroup) -> Self {
        self.leading = Some(group);
        self
    }

    /// Set the trailing (right-aligned) group of items
    pub fn with_trailing(mut self, group: ToolbarGroup) -> Self {
        self.trailing = Some(group);
        self
    }

    pub fn build(self) -> Self {
        self
    }
}

impl Default for Toolbar {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderable for Toolbar {
    fn render(&self, canvas: &Canvas) {
        // Draw background
        if let Some(bg_color) = self.background_color {
            let rect = Rect::from_xywh(self.x, self.y, self.width, self.height);
            let mut paint = Paint::default();
            paint.set_color(bg_color);
            paint.set_anti_alias(true);
            canvas.draw_rect(rect, &paint);
        }

        // Draw border
        if let Some(border_color) = self.border_color {
            let mut paint = Paint::default();
            paint.set_color(border_color);
            paint.set_style(skia_safe::PaintStyle::Stroke);
            paint.set_stroke_width(1.0);
            paint.set_anti_alias(true);

            if self.border_bottom_only {
                // Draw only bottom border
                let y = self.y + self.height - 0.5;
                canvas.draw_line((self.x, y), (self.x + self.width, y), &paint);
            } else {
                // Draw full border
                let rect = Rect::from_xywh(self.x, self.y, self.width, self.height);
                canvas.draw_rect(rect, &paint);
            }
        }

        let content_x = self.x + self.padding;
        let content_y = self.y + self.padding;
        let content_height = self.height - self.padding * 2.0;

        // Render leading group (left side)
        if let Some(ref leading) = self.leading {
            leading.render_at(canvas, content_x, content_y, content_height);
        }

        // Render trailing group (right side)
        if let Some(ref trailing) = self.trailing {
            // Calculate x position for right-aligned content
            // For now, use a simple estimate - ideally would measure content first
            let trailing_x = self.x + self.width - self.padding - 200.0; // Rough estimate
            trailing.render_at(canvas, trailing_x, content_y, content_height);
        }
    }
}

/// A separator item for toolbars
struct ToolbarSeparator {
    color: Color,
}

impl ToolbarSeparator {
    fn new() -> Self {
        Self {
            color: Color::from_rgb(209, 213, 219),
        }
    }
}

/// A fixed-width space in a toolbar
struct ToolbarSpace {
    width: f32,
}

impl ToolbarSpace {
    fn new(width: f32) -> Self {
        Self { width }
    }
}
