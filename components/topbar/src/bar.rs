use otto_kit::prelude::*;
use otto_kit::typography;
use skia_safe::{
    Canvas, ColorType, Data, Image, ImageInfo, Paint, Rect, TextBlob,
};

use crate::clock::Clock;
use crate::config::*;
use crate::tray;

/// Left panel: app name + menus.
pub struct LeftPanel {
    pub app_name: String,
    pub width: f32,
    pub height: f32,
}

/// Right panel: tray icons + clock.
pub struct RightPanel {
    pub clock: Clock,
    pub width: f32,
    pub height: f32,
}

impl LeftPanel {
    pub fn new() -> Self {
        Self {
            app_name: "Otto".into(),
            width: LEFT_WIDTH as f32,
            height: BAR_HEIGHT as f32,
        }
    }

    pub fn draw(&self, canvas: &Canvas) {
        let theme = AppContext::current_theme();
        let h = self.height;

        let font = typography::TextStyle {
            family: "Inter",
            weight: 700,
            size: 13.0,
        }.font();
        let x = BAR_PADDING_H;
        let y = baseline_y(h, &font);

        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(theme.text_primary);

        if let Some(blob) = TextBlob::new(&self.app_name, &font) {
            canvas.draw_text_blob(&blob, (x, y), &paint);
        }
    }
}

impl RightPanel {
    pub fn new() -> Self {
        Self {
            clock: Clock::new(),
            width: RIGHT_WIDTH as f32,
            height: BAR_HEIGHT as f32,
        }
    }

    pub fn draw(&self, canvas: &Canvas) {
        let theme = AppContext::current_theme();

        // Clock on the right edge
        let clock_width = self.draw_clock(canvas, &theme);

        // Tray icons to the left of the clock
        self.draw_tray_icons(canvas, &theme, clock_width);
    }

    fn draw_clock(&self, canvas: &Canvas, theme: &Theme) -> f32 {
        let font = typography::styles::BODY.font();
        let text = &self.clock.text;
        let text_width = font.measure_str(text, None).0;

        let x = self.width - text_width - BAR_PADDING_H;
        let y = baseline_y(self.height, &font);

        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(theme.text_primary);

        if let Some(blob) = TextBlob::new(text, &font) {
            canvas.draw_text_blob(&blob, (x, y), &paint);
        }

        text_width + BAR_PADDING_H
    }

    fn draw_tray_icons(&self, canvas: &Canvas, theme: &Theme, clock_width: f32) {
        let items = tray::current_items();
        if items.is_empty() {
            return;
        }

        let icon_size = TRAY_ICON_SIZE;
        let y = (self.height - icon_size) / 2.0;
        let mut x = self.width - clock_width - TRAY_ICON_SPACING;

        for item in items.iter().rev() {
            x -= icon_size;

            if let Some(image) = tray_icon_image(item) {
                let dst = Rect::from_xywh(x, y, icon_size, icon_size);
                // Tint symbolic icons with the theme text color
                let mut paint = Paint::default();
                paint.set_anti_alias(true);
                paint.set_color(theme.text_primary);
                paint.set_color_filter(skia_safe::color_filters::blend(
                    theme.text_primary,
                    skia_safe::BlendMode::SrcIn,
                ));
                canvas.draw_image_rect(
                    &image,
                    None,
                    dst,
                    &paint,
                );
            } else {
                // Placeholder circle for items with no loadable icon
                let mut paint = Paint::default();
                paint.set_anti_alias(true);
                paint.set_color(theme.text_secondary);
                let cx = x + icon_size / 2.0;
                let cy = y + icon_size / 2.0;
                canvas.draw_circle((cx, cy), icon_size / 2.0 - 1.0, &paint);
            }

            x -= TRAY_ICON_SPACING;
        }
    }

    /// Compute the ideal panel width based on current clock text and tray icon count.
    pub fn target_width(&self) -> f32 {
        let font = typography::styles::BODY.font();
        let clock_text_width = font.measure_str(&self.clock.text, None).0;
        let num_tray = tray::current_items().len() as f32;
        let tray_width = if num_tray > 0.0 {
            num_tray * (TRAY_ICON_SIZE + TRAY_ICON_SPACING) + TRAY_ICON_SPACING
        } else {
            0.0
        };
        let content = clock_text_width + BAR_PADDING_H * 2.0 + tray_width;
        content.max(MIN_RIGHT_WIDTH as f32)
    }

    /// Hit-test: return the tray item index at position x (in logical coords).
    pub fn tray_item_at(&self, x: f32) -> Option<usize> {
        let items = tray::current_items();
        if items.is_empty() {
            return None;
        }

        let font = typography::styles::BODY.font();
        let clock_text = &self.clock.text;
        let clock_width = font.measure_str(clock_text, None).0 + BAR_PADDING_H;

        let icon_size = TRAY_ICON_SIZE;
        let mut ix = self.width - clock_width - TRAY_ICON_SPACING;

        for (i, _item) in items.iter().rev().enumerate() {
            ix -= icon_size;
            if x >= ix && x <= ix + icon_size {
                return Some(items.len() - 1 - i);
            }
            ix -= TRAY_ICON_SPACING;
        }

        None
    }
}

/// Vertically center text using cap-height.
fn baseline_y(height: f32, font: &skia_safe::Font) -> f32 {
    let (_, metrics) = font.metrics();
    (height + metrics.cap_height) / 2.0
}

/// Create a Skia Image from a tray item's BGRA pixel data.
fn tray_icon_image(item: &tray::TrayItem) -> Option<Image> {
    // Try pixmap data first
    if let Some(data) = item.icon_data.as_ref() {
        if item.icon_width > 0 && item.icon_height > 0 {
            let info = ImageInfo::new(
                (item.icon_width, item.icon_height),
                ColorType::BGRA8888,
                skia_safe::AlphaType::Unpremul,
                None,
            );
            let row_bytes = item.icon_width as usize * 4;
            let skia_data = unsafe { Data::new_bytes(data) };
            if let Some(img) = Image::from_raster_data(&info, skia_data, row_bytes) {
                return Some(img);
            }
        }
    }

    // Fall back to icon file from theme
    if let Some(path) = item.icon_file.as_ref() {
        return otto_kit::icons::image_from_path(path, (TRAY_ICON_SIZE as i32, TRAY_ICON_SIZE as i32));
    }

    None
}
