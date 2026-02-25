use skia_safe::{Canvas, Color4f, Paint, Rect};

use super::{MenuBarState, MenuBarStyle};
use crate::typography;

/// Item bounds for hit testing
#[derive(Clone, Debug)]
pub struct ItemBounds {
    pub label: String,
    pub rect: Rect,
}

/// Result of rendering containing item bounds for hit testing
pub struct RenderResult {
    pub item_bounds: Vec<ItemBounds>,
}

/// Pure rendering functions for MenuBarNext
pub struct MenuBarRenderer;

impl MenuBarRenderer {
    /// Main render function
    /// Returns item bounds for hit testing
    pub fn render(
        canvas: &Canvas,
        state: &MenuBarState,
        style: &MenuBarStyle,
        _width: f32,
    ) -> RenderResult {
        // Clear background
        canvas.clear(Color4f::new(0.0, 0.0, 0.0, 0.0));

        // Get font
        let font = typography::get_font_with_fallback("Inter", style.font_style(), style.font_size);

        let mut item_bounds = Vec::new();
        let mut x_offset = style.bar_padding_horizontal;

        // Render each item
        for (index, item_label) in state.items().iter().enumerate() {
            let text_width = style.text_width(item_label, &font);
            let item_width = style.item_width(text_width);

            let item_rect = Rect::new(x_offset, 0.0, x_offset + item_width, style.height);

            // Draw background highlight if active or hovered
            if state.active_index() == Some(index) {
                Self::draw_item_background(canvas, &item_rect, style.active_color, style);
            } else if state.hover_index() == Some(index) {
                Self::draw_item_background(canvas, &item_rect, style.hover_color, style);
            }

            // Draw text
            Self::draw_item_text(
                canvas,
                item_label,
                x_offset + style.item_padding_horizontal,
                &font,
                style,
            );

            // Store bounds for hit testing
            item_bounds.push(ItemBounds {
                label: item_label.clone(),
                rect: item_rect,
            });

            x_offset += item_width + style.item_spacing;
        }

        RenderResult { item_bounds }
    }

    /// Draw background for an item
    fn draw_item_background(
        canvas: &Canvas,
        rect: &Rect,
        color: skia_safe::Color,
        style: &MenuBarStyle,
    ) {
        let mut paint = Paint::new(Color4f::from(color), None);
        paint.set_anti_alias(true);

        let rrect = skia_safe::RRect::new_rect_xy(
            *rect,
            style.item_corner_radius,
            style.item_corner_radius,
        );
        canvas.draw_rrect(rrect, &paint);
    }

    /// Draw text for an item
    fn draw_item_text(
        canvas: &Canvas,
        text: &str,
        x: f32,
        font: &skia_safe::Font,
        style: &MenuBarStyle,
    ) {
        let text_paint = Paint::new(Color4f::from(style.text_color), None);

        // Vertical centering
        let text_y = (style.height + style.font_size) / 2.0 - 2.0;

        canvas.draw_str(text, (x, text_y), font, &text_paint);
    }

    /// Calculate minimum width needed for the component
    pub fn measure_width(state: &MenuBarState, style: &MenuBarStyle) -> f32 {
        let font = typography::get_font_with_fallback("Inter", style.font_style(), style.font_size);

        style.total_width(state.items(), &font)
    }
}
