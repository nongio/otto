use skia_safe::Color;

/// Visual styling for MenuBar component
#[derive(Clone, Debug)]
pub struct MenuBarStyle {
    // Dimensions
    pub height: f32,
    pub item_padding_horizontal: f32,
    pub bar_padding_horizontal: f32,
    pub item_spacing: f32,

    // Colors
    pub background_color: Color,
    pub text_color: Color,
    pub hover_color: Color,
    pub active_color: Color,

    // Typography
    pub font_size: f32,
    pub font_weight: skia_safe::font_style::Weight,

    // Borders/Shapes
    pub item_corner_radius: f32,
}

impl Default for MenuBarStyle {
    fn default() -> Self {
        Self {
            // Dimensions
            height: 28.0,
            item_padding_horizontal: 12.0,
            bar_padding_horizontal: 8.0,
            item_spacing: 0.0,

            // Colors
            background_color: Color::from_rgb(240, 240, 240),
            text_color: Color::from_rgb(40, 40, 40),
            hover_color: Color::from_argb(20, 0, 0, 0),
            active_color: Color::from_argb(50, 0, 0, 0),

            // Typography
            font_size: 13.0,
            font_weight: skia_safe::font_style::Weight::SEMI_BOLD,

            // Borders/Shapes
            item_corner_radius: 4.0,
        }
    }
}

impl MenuBarStyle {
    /// Calculate the width needed for a text label
    pub fn text_width(&self, text: &str, font: &skia_safe::Font) -> f32 {
        let (_, bounds) = font.measure_str(text, None);
        bounds.width()
    }

    /// Calculate the total width for an item including padding
    pub fn item_width(&self, text_width: f32) -> f32 {
        text_width + self.item_padding_horizontal * 2.0
    }

    /// Calculate the total width needed for all items
    pub fn total_width(&self, items: &[String], font: &skia_safe::Font) -> f32 {
        let mut width = self.bar_padding_horizontal * 2.0;

        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                width += self.item_spacing;
            }
            let text_w = self.text_width(item, font);
            width += self.item_width(text_w);
        }

        width
    }

    /// Get the font style
    pub fn font_style(&self) -> skia_safe::FontStyle {
        skia_safe::FontStyle::new(
            self.font_weight,
            skia_safe::font_style::Width::NORMAL,
            skia_safe::font_style::Slant::Upright,
        )
    }
}
