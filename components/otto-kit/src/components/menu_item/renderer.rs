use super::{MenuItem, MenuItemStyle, VisualState};
use crate::typography;
use crate::{components::icon::Icon, Renderable};
use skia_safe::{Canvas, Font, Paint, Point, RRect, Rect};

/// Pure rendering functions for MenuItem
pub struct MenuItemRenderer;

impl MenuItemRenderer {
    /// Render a menu item at the given position
    pub fn render(
        canvas: &Canvas,
        data: &MenuItem,
        style: &MenuItemStyle,
        x: f32,
        y: f32,
        width: f32,
    ) {
        if data.is_separator() {
            Self::draw_separator(canvas, style, x, y, width);
        } else {
            Self::draw_item(canvas, data, style, x, y, width);
        }
    }

    /// Draw a separator line
    fn draw_separator(canvas: &Canvas, style: &MenuItemStyle, x: f32, y: f32, width: f32) {
        let mut paint = Paint::default();
        paint.set_color(style.separator_color);
        paint.set_anti_alias(true);
        paint.set_stroke_width(1.0);

        let line_y = y + style.separator_height / 2.0;
        canvas.draw_line(
            Point::new(x + style.horizontal_padding, line_y),
            Point::new(x + width - style.horizontal_padding, line_y),
            &paint,
        );
    }

    /// Draw an action or submenu item
    fn draw_item(
        canvas: &Canvas,
        data: &MenuItem,
        style: &MenuItemStyle,
        x: f32,
        y: f32,
        width: f32,
    ) {
        let label = data.label().unwrap_or("");

        // Draw background if hovered
        if data.visual_state() == VisualState::Hovered {
            Self::draw_hover_background(canvas, style, x, y, width, data.height);
        }

        // Get text color based on state
        let text_color = match data.visual_state() {
            VisualState::Normal => style.text_color_normal,
            VisualState::Hovered => style.text_color_hovered,
            VisualState::Disabled => style.text_color_disabled,
        };

        // Create font
        let font = typography::styles::BODY.font();
        let shortcut_font = typography::styles::BODY.font();

        // Draw label
        Self::draw_label(canvas, label, &font, text_color, x, y, data.height, style);

        // Draw submenu indicator or shortcut
        if data.has_submenu() {
            Self::draw_submenu_indicator(canvas, text_color, x, y, width, data.height, style);
        } else if let Some(shortcut_text) = data.shortcut() {
            Self::draw_shortcut(
                canvas,
                shortcut_text,
                &shortcut_font,
                data.visual_state(),
                x,
                y,
                width,
                data.height,
                style,
            );
        }
    }

    /// Draw hover background
    fn draw_hover_background(
        canvas: &Canvas,
        style: &MenuItemStyle,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) {
        let mut bg_paint = Paint::default();
        bg_paint.set_color(style.bg_color_hovered);
        bg_paint.set_anti_alias(true);

        let bg_rect = RRect::new_rect_xy(
            Rect::from_xywh(x, y, width, height),
            style.border_radius,
            style.border_radius,
        );
        canvas.draw_rrect(&bg_rect, &bg_paint);
    }

    /// Draw item label
    fn draw_label(
        canvas: &Canvas,
        label: &str,
        font: &Font,
        color: skia_safe::Color,
        x: f32,
        y: f32,
        height: f32,
        style: &MenuItemStyle,
    ) {
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.set_anti_alias(true);

        // Center text vertically using font metrics
        let (_line_spacing, metrics) = font.metrics();
        let font_height = metrics.descent - metrics.ascent;
        let baseline_y = y + (height - font_height) / 2.0 - metrics.ascent;

        canvas.draw_str(
            label,
            Point::new(x + style.horizontal_padding, baseline_y),
            font,
            &paint,
        );
    }

    /// Draw submenu indicator (chevron)
    fn draw_submenu_indicator(
        canvas: &Canvas,
        color: skia_safe::Color,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        style: &MenuItemStyle,
    ) {
        let icon_size = 16.0;
        let icon_y = y + (height - icon_size) / 2.0;
        let icon_x = x + width - icon_size - style.horizontal_padding;

        canvas.save();
        canvas.translate((icon_x, icon_y));
        Icon::new("chevron-right")
            .with_size(icon_size)
            .with_color(color)
            .render(canvas);
        canvas.restore();
    }

    /// Draw shortcut text
    fn draw_shortcut(
        canvas: &Canvas,
        shortcut: &str,
        font: &Font,
        visual_state: VisualState,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        style: &MenuItemStyle,
    ) {
        let shortcut_color = match visual_state {
            VisualState::Hovered => style.shortcut_color_hovered,
            _ => style.shortcut_color_normal,
        };

        let mut paint = Paint::default();
        paint.set_color(shortcut_color);
        paint.set_anti_alias(true);

        let (shortcut_width, _) = font.measure_str(shortcut, None);

        let (_line_spacing, metrics) = font.metrics();
        let font_height = metrics.descent - metrics.ascent;
        let baseline_y = y + (height - font_height) / 2.0 - metrics.ascent;

        canvas.draw_str(
            shortcut,
            Point::new(
                x + width - shortcut_width - style.horizontal_padding,
                baseline_y,
            ),
            font,
            &paint,
        );
    }
}
