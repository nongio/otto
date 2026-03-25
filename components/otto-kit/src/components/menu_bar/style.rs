use skia_safe::Color;

use super::state::MenuBarItem;

/// Visual styling for MenuBar component
#[derive(Clone, Debug)]
pub struct MenuBarStyle {
    // Dimensions
    pub height: f32,
    pub item_padding_horizontal: f32,
    pub bar_padding_horizontal: f32,
    pub item_spacing: f32,

    // Icon
    pub icon_size: f32,
    pub icon_text_gap: f32,

    // Colors
    pub background_color: Color,
    pub text_color: Color,
    pub text_active_color: Color,
    pub hover_color: Color,
    pub active_color: Color,
    /// Tint color for icons in normal state
    pub icon_tint: Color,
    /// Tint color for icons when the item is active
    pub icon_active_tint: Color,

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

            // Icon
            icon_size: 16.0,
            icon_text_gap: 6.0,

            // Colors
            background_color: Color::from_rgb(240, 240, 240),
            text_color: Color::from_rgb(40, 40, 40),
            text_active_color: Color::WHITE,
            hover_color: Color::from_argb(20, 0, 0, 0),
            active_color: Color::from_argb(255, 0, 90, 220),
            icon_tint: Color::from_rgb(40, 40, 40),
            icon_active_tint: Color::WHITE,

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

    /// Calculate the content width for an item (icon + gap + text)
    pub fn item_content_width(&self, item: &MenuBarItem, font: &skia_safe::Font) -> f32 {
        let icon_w = if item.icon.is_some() {
            self.icon_size
        } else {
            0.0
        };
        let gap = if item.icon.is_some() && item.label.is_some() {
            self.icon_text_gap
        } else {
            0.0
        };
        let text_w = item
            .label
            .as_deref()
            .map(|l| self.text_width(l, font))
            .unwrap_or(0.0);
        icon_w + gap + text_w
    }

    /// Calculate the total width for an item including padding
    pub fn item_width(&self, content_width: f32) -> f32 {
        content_width + self.item_padding_horizontal * 2.0
    }

    /// Calculate the total width needed for all items
    pub fn total_width(&self, items: &[MenuBarItem], font: &skia_safe::Font) -> f32 {
        let mut width = self.bar_padding_horizontal * 2.0;

        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                width += self.item_spacing;
            }
            let content_w = self.item_content_width(item, font);
            width += self.item_width(content_w);
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
