use layers::prelude::*;

use crate::{config::Config, theme::theme_colors, workspaces::utils::FONT_CACHE};

use super::model::AppSwitcherModel;

/// Compute the layout metrics for a given model.
/// Returns `(component_width, component_height, available_icon_size, icon_padding, gap, padding_h, padding_v)`.
pub fn layout_metrics(state: &AppSwitcherModel) -> (f32, f32, f32, f32, f32, f32, f32) {
    let draw_scale = Config::with(|config| config.screen_scale) as f32 * 0.8;
    let available_width = state.width as f32 - 20.0 * draw_scale;
    let icon_size: f32 = 160.0 * draw_scale;
    let icon_padding: f32 = available_width * 0.006 * draw_scale;
    let gap: f32 = icon_padding / 2.0;
    let apps_len = state.apps.len().max(1) as f32;
    let total_gaps = (apps_len - 1.0) * gap;
    let total_padding = apps_len * icon_padding * 2.0 + total_gaps;

    let mut padding_h: f32 = ((available_width - total_padding) * 0.03 * draw_scale).max(0.0);
    if padding_h > 15.0 * draw_scale {
        padding_h = 15.0 * draw_scale;
    }
    let mut padding_v: f32 = ((available_width - total_padding) * 0.014 * draw_scale).max(0.0);
    if padding_v > 50.0 * draw_scale {
        padding_v = 50.0 * draw_scale;
    }

    let available_icon_size = ((available_width - total_padding - padding_h * 2.0)
        / state.apps.len().max(1) as f32)
        .min(icon_size);
    // Reserve enough vertical room for the app label under the selected icon.
    // This keeps the panel from looking too tight on 1.0 scale outputs.
    let min_padding_v = (available_icon_size / 8.0) * 1.6;
    padding_v = padding_v.max(min_padding_v);

    let component_width = apps_len * available_icon_size + total_padding + padding_h * 2.0;
    let component_height = available_icon_size + icon_padding * 2.0 + padding_v * 2.0;

    (
        component_width,
        component_height,
        available_icon_size,
        icon_padding,
        gap,
        padding_h,
        padding_v,
    )
}

/// Returns a draw function that renders the selection highlight + app name overlay
/// onto the container layer.  Icon rendering is done by mirror layers (not here).
pub fn draw_appswitcher_overlay(state: &AppSwitcherModel) -> ContentDrawFunction {
    let (_, _, available_icon_size, icon_padding, gap, padding_h, _) = layout_metrics(state);
    let has_apps = !state.apps.is_empty();
    let current_app = state.current_app as f32;
    let font_size: f32 = available_icon_size / 8.0;
    let app_name = if has_apps && state.current_app < state.apps.len() {
        state.apps[state.current_app]
            .desktop_name()
            .clone()
            .unwrap_or_default()
    } else {
        String::new()
    };

    let draw = move |canvas: &layers::skia::Canvas, w: f32, h: f32| -> layers::skia::Rect {
        let slot_width = available_icon_size + icon_padding * 2.0;
        let selection_x = padding_h + current_app * (slot_width + gap);
        let selection_y = h / 2.0 - slot_width / 2.0;

        if has_apps {
            // Selection highlight box
            let selection_bg = theme_colors().fills_primary.c4f();
            let mut paint = layers::skia::Paint::new(selection_bg, None);
            paint.set_anti_alias(true);
            let rrect = layers::skia::RRect::new_rect_xy(
                layers::skia::Rect::from_xywh(selection_x, selection_y, slot_width, slot_width),
                slot_width / 10.0,
                slot_width / 10.0,
            );
            canvas.draw_rrect(rrect, &paint);

            // App name text
            let font_family = Config::with(|c| c.font_family.clone());
            let font_style = layers::skia::FontStyle::new(
                layers::skia::font_style::Weight::MEDIUM,
                layers::skia::font_style::Width::CONDENSED,
                layers::skia::font_style::Slant::Upright,
            );
            let font = FONT_CACHE
                .with(|fc| fc.make_font_with_fallback(font_family, font_style, font_size));
            let mut text_paint =
                layers::skia::Paint::new(theme_colors().text_secondary.c4f(), None);
            text_paint.set_anti_alias(true);
            let text_bounds = font.measure_str(&app_name, Some(&text_paint)).1;
            let text_x = selection_x + (slot_width - text_bounds.width()) / 2.0;
            let text_y = selection_y + slot_width + font_size * 1.2;
            canvas.draw_str(&app_name, (text_x, text_y), &font, &text_paint);
        }

        layers::skia::Rect::from_xywh(0.0, 0.0, w, h)
    };
    draw.into()
}
