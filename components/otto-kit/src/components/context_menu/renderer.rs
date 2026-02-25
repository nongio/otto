use super::{ContextMenuState, ContextMenuStyle};
use crate::components::menu_item::{MenuItem, MenuItemGroup, VisualState};
use skia_safe::{Canvas, Paint, RRect, Rect};

/// Pure rendering functions for ContextMenu
///
/// Stateless drawing - all data passed as parameters.
pub struct ContextMenuRenderer;

impl ContextMenuRenderer {
    /// Calculate menu dimensions from items and style
    ///
    /// Returns (width, height) in logical pixels.
    pub fn measure(state: &ContextMenuState, style: &ContextMenuStyle) -> (f32, f32) {
        Self::measure_items(state.items(), style)
    }

    /// Calculate dimensions for specific items (used for submenus)
    ///
    /// Returned dimensions are already multiplied by `style.draw_scale`.
    pub fn measure_items(items: &[MenuItem], style: &ContextMenuStyle) -> (f32, f32) {
        let s = style.draw_scale;

        // Calculate height from items (item heights are in logical pixels)
        let height: f32 = items.iter().map(|item| item.height).sum();
        let height = (height + style.vertical_padding * 2.0) * s;

        // Use provided width or minimum, then scale
        let width = style.width.unwrap_or(style.min_width) * s;

        (width, height)
    }

    /// Render items at a specific depth with specific selection
    ///
    /// `width` and `height` are the layer's actual pixel dimensions (already scaled).
    /// The canvas is scaled by `draw_scale` so all drawing uses unscaled logical coords.
    pub fn render_depth(
        canvas: &Canvas,
        items: &[MenuItem],
        selected: Option<usize>,
        style: &ContextMenuStyle,
        width: f32,
        height: f32,
    ) {
        let s = style.draw_scale;
        let logical_w = width / s;
        let logical_h = height / s;

        canvas.save();
        canvas.scale((s, s));

        // Draw background and border at logical (unscaled) dimensions
        Self::draw_background(canvas, style, logical_w, logical_h);

        // Draw menu items with states
        Self::draw_items_with_selection(canvas, items, selected, style, logical_w);

        canvas.restore();
    }

    /// Draw menu background and border
    fn draw_background(canvas: &Canvas, style: &ContextMenuStyle, width: f32, height: f32) {
        let popup_rect = RRect::new_rect_xy(
            Rect::from_xywh(0.0, 0.0, width, height),
            style.corner_radius,
            style.corner_radius,
        );

        // Draw background
        let mut bg_paint = Paint::default();
        bg_paint.set_color(style.background_color());
        bg_paint.set_anti_alias(true);
        canvas.draw_rrect(popup_rect, &bg_paint);

        // Draw border
        let mut border_paint = Paint::default();
        border_paint.set_color(style.border_color());
        border_paint.set_style(skia_safe::paint::Style::Stroke);
        border_paint.set_stroke_width(style.border_width);
        border_paint.set_anti_alias(true);
        canvas.draw_rrect(popup_rect, &border_paint);
    }

    /// Draw items with explicit selection (for depth-specific rendering)
    fn draw_items_with_selection(
        canvas: &Canvas,
        items: &[MenuItem],
        selected: Option<usize>,
        style: &ContextMenuStyle,
        width: f32,
    ) {
        // Save canvas state and translate for padding
        canvas.save();
        canvas.translate((style.horizontal_padding, style.vertical_padding));

        // Apply hover state to items and convert to MenuItem components
        let menu_items_with_state: Vec<MenuItem> = items
            .iter()
            .enumerate()
            .map(|(i, item_data)| {
                let mut data = item_data.clone();
                if Some(i) == selected {
                    data.set_visual_state(VisualState::Hovered);
                }
                data
            })
            .collect();

        // Render using MenuItemGroup at origin (canvas is already translated)
        MenuItemGroup::new()
            .at(0.0, 0.0)
            .with_width(width - style.horizontal_padding * 2.0)
            .items(menu_items_with_state)
            .render(canvas);

        // Restore canvas state
        canvas.restore();
    }

    /// Hit test to determine which menu item is at the given position
    ///
    /// Returns the index of the item at (x, y), or None if outside menu bounds.
    /// Considers padding and returns None for separators.
    pub fn hit_test(
        state: &ContextMenuState,
        style: &ContextMenuStyle,
        x: f32,
        y: f32,
    ) -> Option<usize> {
        Self::hit_test_items(state.items(), style, x, y)
    }

    /// Hit test specific items (for depth-specific testing)
    pub fn hit_test_items(
        items: &[MenuItem],
        style: &ContextMenuStyle,
        x: f32,
        y: f32,
    ) -> Option<usize> {
        // Check if inside horizontal bounds (with padding)
        let scale = 1.0;
        let total_width = (style.width.unwrap_or(style.min_width)) * scale;
        if x < style.horizontal_padding * scale
            || x > total_width - style.horizontal_padding * scale
        {
            return None;
        }

        // Check if inside vertical bounds (with padding)
        let total_height =
            (style.vertical_padding + items.iter().map(|item| item.height).sum::<f32>()) * scale;
        if y < style.vertical_padding * scale || y > total_height {
            return None;
        }

        // Calculate position relative to first item
        let mut current_y = style.vertical_padding * scale;

        for (i, item) in items.iter().enumerate() {
            let item_bottom = current_y + item.height * scale;

            if y >= current_y && y < item_bottom {
                // Found the item at this position
                // Return None for separators (not selectable)
                return if item.is_separator() { None } else { Some(i) };
            }

            current_y = item_bottom;
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_state() -> ContextMenuState {
        use crate::components::menu_item::{MenuItem, MenuItemKind};

        ContextMenuState::new(vec![
            MenuItem::new(MenuItemKind::Action {
                label: "Item 1".to_string(),
                shortcut: None,
                action_id: Some("action_1".to_string()),
            }),
            MenuItem::new(MenuItemKind::Separator),
            MenuItem::new(MenuItemKind::Action {
                label: "Item 2".to_string(),
                shortcut: None,
                action_id: Some("action_2".to_string()),
            }),
        ])
    }

    #[test]
    fn test_measure() {
        let state = create_test_state();
        let style = ContextMenuStyle::default();

        let (width, height) = ContextMenuRenderer::measure(&state, &style);

        assert!(width >= style.min_width);
        assert!(height > 0.0);
    }

    #[test]
    fn test_measure_with_custom_width() {
        let state = create_test_state();
        let style = ContextMenuStyle::default().with_width(300.0);

        let (width, _height) = ContextMenuRenderer::measure(&state, &style);

        assert_eq!(width, 300.0);
    }
}
