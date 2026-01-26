use super::data::{MenuItem, MenuStyle};
use skia_safe::{Canvas, Color4f, Font, FontMgr, Paint, PaintStyle, RRect, Rect};

/// Draw a complete menu with background and items
pub fn draw_menu(
    canvas: &Canvas,
    items: &[MenuItem],
    width: f32,
    hovered_index: Option<usize>,
    style: &MenuStyle,
) {
    // Clear to transparent
    canvas.clear(Color4f::new(0.0, 0.0, 0.0, 0.0));

    // Only draw background and border if sc-layer is not handling it
    if !style.sc_layer {
        // Draw background
        let height = style.calculate_menu_height(items);
        let bg = style.background_color;
        let bg_paint = Paint::new(Color4f::new(bg[0], bg[1], bg[2], bg[3]), None);
        let bg_rect = RRect::new_rect_xy(
            Rect::from_xywh(0.0, 0.0, width, height),
            style.corner_radius,
            style.corner_radius,
        );
        // canvas.draw_rrect(&bg_rect, &bg_paint);

        // Draw border
        let mut border_paint = Paint::new(Color4f::new(0.0, 0.0, 0.0, 0.1), None);
        border_paint.set_style(PaintStyle::Stroke);
        border_paint.set_stroke_width(1.0);
        border_paint.set_anti_alias(true);
        // canvas.draw_rrect(&bg_rect, &border_paint);
    }

    // Setup fonts
    let font_mgr = FontMgr::new();
    let font_style = skia_safe::FontStyle::new(
        skia_safe::font_style::Weight::MEDIUM,
        skia_safe::font_style::Width::NORMAL,
        skia_safe::font_style::Slant::Upright,
    );

    let typeface = font_mgr
        .match_family_style("Inter", font_style)
        .or_else(|| font_mgr.match_family_style("Inter UI", font_style))
        .or_else(|| font_mgr.match_family_style("system-ui", font_style))
        .unwrap_or_else(|| font_mgr.legacy_make_typeface(None, font_style).unwrap());

    let mut menu_font = Font::from_typeface(typeface.clone(), style.font_size);
    menu_font.set_subpixel(true);
    menu_font.set_edging(skia_safe::font::Edging::SubpixelAntiAlias);

    let mut shortcut_font = Font::from_typeface(typeface, style.shortcut_font_size);
    shortcut_font.set_subpixel(true);
    shortcut_font.set_edging(skia_safe::font::Edging::SubpixelAntiAlias);

    // Draw items
    let mut y = style.padding_vertical;

    for (item_index, item) in items.iter().enumerate() {
        let is_hovered = !item.is_separator() && hovered_index == Some(item_index);

        y = draw_menu_item(
            canvas,
            item,
            y,
            width,
            is_hovered,
            style,
            &menu_font,
            &shortcut_font,
        );
    }
}

/// Draw a single menu item
fn draw_menu_item(
    canvas: &Canvas,
    item: &MenuItem,
    y_position: f32,
    width: f32,
    is_hovered: bool,
    style: &MenuStyle,
    menu_font: &Font,
    shortcut_font: &Font,
) -> f32 {
    match item {
        MenuItem::Separator => {
            // Draw separator line
            let y = y_position + style.separator_height / 2.0;

            let sep = style.separator_color;
            let mut separator_paint =
                Paint::new(Color4f::new(sep[0], sep[1], sep[2], sep[3]), None);
            separator_paint.set_anti_alias(true);
            separator_paint.set_stroke_width(1.0);
            separator_paint.set_style(PaintStyle::Stroke);

            canvas.draw_line(
                (style.padding_left, y),
                (width - style.padding_right, y),
                &separator_paint,
            );

            y_position + style.separator_height
        }
        MenuItem::Action {
            label,
            shortcut,
            enabled,
            ..
        } => {
            // Draw highlight background if hovered and enabled
            if is_hovered && *enabled {
                let hover = style.item_hover_background;
                let highlight_paint =
                    Paint::new(Color4f::new(hover[0], hover[1], hover[2], hover[3]), None);
                let highlight_rect = RRect::new_rect_xy(
                    Rect::from_xywh(
                        style.highlight_h_padding,
                        y_position,
                        width - 2.0 * style.highlight_h_padding,
                        style.item_height,
                    ),
                    5.0,
                    5.0,
                );
                canvas.draw_rrect(&highlight_rect, &highlight_paint);
            }

            // Calculate baseline position for text
            let baseline_y = y_position + style.item_height * 0.68;

            // Choose text color based on state
            let text_color = if !enabled {
                style.disabled_text_color
            } else if is_hovered {
                style.item_hover_text
            } else {
                style.item_text_color
            };

            let shortcut_color = if !enabled {
                style.disabled_text_color
            } else if is_hovered {
                style.item_hover_text
            } else {
                style.disabled_text_color // Shortcuts always lighter
            };

            let mut text_paint = Paint::new(
                Color4f::new(text_color[0], text_color[1], text_color[2], text_color[3]),
                None,
            );
            text_paint.set_anti_alias(true);

            let mut shortcut_paint = Paint::new(
                Color4f::new(
                    shortcut_color[0],
                    shortcut_color[1],
                    shortcut_color[2],
                    shortcut_color[3],
                ),
                None,
            );
            shortcut_paint.set_anti_alias(true);

            // Draw menu item label
            canvas.draw_str(
                label,
                (style.padding_left, baseline_y),
                menu_font,
                &text_paint,
            );

            // Draw shortcut text
            if let Some(shortcut_text) = shortcut {
                // Measure shortcut text to right-align it
                let (shortcut_width, _) =
                    shortcut_font.measure_str(shortcut_text, Some(&shortcut_paint));
                let shortcut_x = width - style.padding_right - shortcut_width;
                canvas.draw_str(
                    shortcut_text,
                    (shortcut_x, baseline_y),
                    shortcut_font,
                    &shortcut_paint,
                );
            }

            y_position + style.item_height
        }
        MenuItem::Submenu { label, enabled, .. } => {
            // Draw highlight background if hovered and enabled
            if is_hovered && *enabled {
                let hover = style.item_hover_background;
                let highlight_paint =
                    Paint::new(Color4f::new(hover[0], hover[1], hover[2], hover[3]), None);
                let highlight_rect = RRect::new_rect_xy(
                    Rect::from_xywh(
                        style.highlight_h_padding,
                        y_position,
                        width - 2.0 * style.highlight_h_padding,
                        style.item_height,
                    ),
                    5.0,
                    5.0,
                );
                canvas.draw_rrect(&highlight_rect, &highlight_paint);
            }

            // Calculate baseline position for text
            let baseline_y = y_position + style.item_height * 0.68;

            // Choose text color based on state
            let text_color = if !enabled {
                style.disabled_text_color
            } else if is_hovered {
                style.item_hover_text
            } else {
                style.item_text_color
            };

            let shortcut_color = if !enabled {
                style.disabled_text_color
            } else if is_hovered {
                style.item_hover_text
            } else {
                style.disabled_text_color
            };

            let mut text_paint = Paint::new(
                Color4f::new(text_color[0], text_color[1], text_color[2], text_color[3]),
                None,
            );
            text_paint.set_anti_alias(true);

            let mut shortcut_paint = Paint::new(
                Color4f::new(
                    shortcut_color[0],
                    shortcut_color[1],
                    shortcut_color[2],
                    shortcut_color[3],
                ),
                None,
            );
            shortcut_paint.set_anti_alias(true);

            // Draw menu item label
            canvas.draw_str(
                label,
                (style.padding_left, baseline_y),
                menu_font,
                &text_paint,
            );

            // Draw submenu arrow
            let arrow = "▶";
            let (arrow_width, _) = shortcut_font.measure_str(arrow, Some(&shortcut_paint));
            let arrow_x = width - style.padding_right - arrow_width;
            canvas.draw_str(arrow, (arrow_x, baseline_y), shortcut_font, &shortcut_paint);

            y_position + style.item_height
        }
    }
}
