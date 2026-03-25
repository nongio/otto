use skia_safe::{Canvas, Color, Color4f, Data, Image, ImageInfo, Paint, Rect};

use super::state::{MenuBarIcon, MenuBarState};
use super::style::MenuBarStyle;
use crate::typography;

/// Item bounds for hit testing
#[derive(Clone, Debug)]
pub struct ItemBounds {
    pub index: usize,
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
        width: f32,
    ) -> RenderResult {
        // Draw background bar
        let bg_paint = Paint::new(Color4f::from(style.background_color), None);
        canvas.draw_rect(Rect::new(0.0, 0.0, width, style.height), &bg_paint);

        let font = typography::get_font_with_fallback("Inter", style.font_style(), style.font_size);

        let mut item_bounds = Vec::new();
        let mut x_offset = style.bar_padding_horizontal;

        for (index, item) in state.items().iter().enumerate() {
            let content_width = style.item_content_width(item, &font);
            let item_width = style.item_width(content_width);

            let item_rect = Rect::new(x_offset, 0.0, x_offset + item_width, style.height);

            let is_active = state.active_index() == Some(index);
            let is_hovered = state.hover_index() == Some(index);

            // Draw background highlight
            if is_active {
                Self::draw_item_background(canvas, &item_rect, style.active_color, style);
            } else if is_hovered {
                Self::draw_item_background(canvas, &item_rect, style.hover_color, style);
            }

            // Draw item content (icon and/or label)
            let mut cx = x_offset + style.item_padding_horizontal;

            if let Some(icon) = &item.icon {
                let tint = if is_active {
                    style.icon_active_tint
                } else {
                    style.icon_tint
                };
                Self::draw_icon(canvas, icon, cx, style, tint);
                cx += style.icon_size;
                if item.label.is_some() {
                    cx += style.icon_text_gap;
                }
            }

            if let Some(label) = &item.label {
                let text_color = if is_active {
                    style.text_active_color
                } else {
                    style.text_color
                };
                Self::draw_item_text(canvas, label, cx, &font, style, text_color);
            }

            item_bounds.push(ItemBounds {
                index,
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

    /// Draw an icon with a tint color
    fn draw_icon(
        canvas: &Canvas,
        icon: &MenuBarIcon,
        x: f32,
        style: &MenuBarStyle,
        tint: skia_safe::Color,
    ) {
        // Load icons at physical size for HiDPI crispness
        let scale = crate::app_runner::context::AppContext::scale_factor().max(1);
        let load_size = (style.icon_size as i32) * scale;
        let image = match icon {
            MenuBarIcon::Pixmap {
                data,
                width,
                height,
            } => Self::image_from_pixmap(data, *width, *height),
            MenuBarIcon::Named(name) => crate::icons::named_icon_sized(name, load_size),
            MenuBarIcon::File(path) => crate::icons::cached_file_icon(path, load_size),
        };

        if let Some(image) = image {
            let y = (style.height - style.icon_size) / 2.0;
            let dst = Rect::from_xywh(x, y, style.icon_size, style.icon_size);

            let mut paint = Paint::default();

            // Tint symbolic icons (monochrome SVGs designed for recoloring).
            // SrcIn replaces the icon's color with the tint while preserving alpha.
            let is_symbolic = matches!(icon, MenuBarIcon::File(p) if p.contains("-symbolic"))
                || matches!(icon, MenuBarIcon::Named(n) if n.contains("-symbolic"));
            if is_symbolic {
                paint.set_color_filter(skia_safe::color_filters::blend(
                    tint,
                    skia_safe::BlendMode::SrcIn,
                ));
            }

            canvas.draw_image_rect(&image, None, dst, &paint);
        }
    }

    /// Create a Skia Image from BGRA8888 pixel data
    fn image_from_pixmap(data: &[u8], width: i32, height: i32) -> Option<Image> {
        if width <= 0 || height <= 0 || data.is_empty() {
            return None;
        }
        let info = ImageInfo::new(
            (width, height),
            skia_safe::ColorType::BGRA8888,
            skia_safe::AlphaType::Unpremul,
            None,
        );
        let row_bytes = width as usize * 4;
        let skia_data = Data::new_copy(data);
        skia_safe::images::raster_from_data(&info, skia_data, row_bytes)
    }

    /// Draw text for an item
    fn draw_item_text(
        canvas: &Canvas,
        text: &str,
        x: f32,
        font: &skia_safe::Font,
        style: &MenuBarStyle,
        color: Color,
    ) {
        let text_paint = Paint::new(Color4f::from(color), None);
        let text_y = (style.height + style.font_size) / 2.0 - 2.0;
        canvas.draw_str(text, (x, text_y), font, &text_paint);
    }

    /// Calculate minimum width needed for the component
    pub fn measure_width(state: &MenuBarState, style: &MenuBarStyle) -> f32 {
        let font = typography::get_font_with_fallback("Inter", style.font_style(), style.font_size);
        style.total_width(state.items(), &font)
    }
}
