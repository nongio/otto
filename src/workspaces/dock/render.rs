use layers::skia::PathEffect;
use layers::{prelude::*, types::Size};
use taffy::LengthPercentageAuto;

use crate::{
    config::Config,
    theme::{self, theme_colors},
    workspaces::{
        Application, utils::{FONT_CACHE, draw_balloon_rect}
    },
};

/// Draw a badge (red circle with white text), sized to fill the layer bounds.
pub fn draw_badge(text: String) -> ContentDrawFunction {
    let draw_fn = move |canvas: &layers::skia::Canvas, w: f32, h: f32| -> layers::skia::Rect {
        if text.is_empty() {
            return layers::skia::Rect::from_xywh(0.0, 0.0, w, h);
        }
        

        // White text centered
        let text_size = h * 0.55;
        let font_family = Config::with(|c| c.font_family.clone());
        let font_style = layers::skia::FontStyle::new(
            layers::skia::font_style::Weight::MEDIUM,
            layers::skia::font_style::Width::NORMAL,
            layers::skia::font_style::Slant::Upright,
        );
        let font = FONT_CACHE.with(|font_cache| {
            font_cache.make_font_with_fallback(font_family, font_style, text_size)
        });

        let mut text_paint =
            layers::skia::Paint::new(layers::skia::Color4f::new(1.0, 1.0, 1.0, 1.0), None);
        text_paint.set_anti_alias(true);

        let (_, text_bounds) = font.measure_str(&text, Some(&text_paint));
        let text_x = w/2.0 - text_bounds.width() / 2.0 - text_bounds.left;
        let text_y = h/2.0 - text_bounds.height() / 2.0 - text_bounds.top;

        canvas.draw_str(&text, (text_x, text_y), &font, &text_paint);

        layers::skia::Rect::from_xywh(0.0, 0.0, w, h)
    };
    draw_fn.into()
}

/// Draw a horizontal progress bar, sized to fill the layer bounds.
pub fn draw_progress(value: f64) -> ContentDrawFunction {
    let draw_fn = move |canvas: &layers::skia::Canvas, w: f32, h: f32| -> layers::skia::Rect {
        let value = value.clamp(0.0, 1.0) as f32;
        let corner_radius = h / 2.0;

        // Dark semi-transparent background track
        let mut bg_paint = layers::skia::Paint::new(
            layers::skia::Color4f::new(0.0, 0.0, 0.0, 0.30),
            None,
        );
        bg_paint.set_anti_alias(true);
        let bg_rect = layers::skia::Rect::from_xywh(0.0, 0.0, w, h);
        let bg_rrect =
            layers::skia::RRect::new_rect_xy(bg_rect, corner_radius, corner_radius);
        canvas.draw_rrect(bg_rrect, &bg_paint);

        // White fill proportional to progress
        if value > 0.0 {
            let fill_w = (w * value).max(h); // keep at least circle-width so it never looks empty
            let mut fill_paint = layers::skia::Paint::new(
                layers::skia::Color4f::new(1.0, 1.0, 1.0, 0.92),
                None,
            );
            fill_paint.set_anti_alias(true);
            let fill_rect = layers::skia::Rect::from_xywh(0.0, 0.0, fill_w.min(w), h);
            let fill_rrect =
                layers::skia::RRect::new_rect_xy(fill_rect, corner_radius, corner_radius);
            canvas.draw_rrect(fill_rrect, &fill_paint);
        }

        layers::skia::Rect::from_xywh(0.0, 0.0, w, h)
    };
    draw_fn.into()
}

/// Configure a badge overlay layer (initially hidden; caller must call set_opacity to show it).
/// The layer is positioned to float at the top-right corner of the icon content area.
pub fn setup_badge_layer(layer: &Layer, icon_width: f32) {
    let badge_size = icon_width * 0.4;
    let tree = LayerTreeBuilder::default()
        .key("badge")
        .layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        })
        .size(Size {
            width: taffy::Dimension::Length(badge_size),
            height: taffy::Dimension::Length(badge_size),
        })
        .anchor_point(Point { x: 0.5, y: 0.5 })
        .background_color(theme_colors().accents_red.opacity(0.9))
        .border_corner_radius(BorderRadius::new_single(badge_size/2.0))
        .opacity((0.0, None))
        .shadow_color(theme_colors().shadow_color.opacity(0.4))
        .shadow_offset(((0.0, 0.0).into(), None))
        .shadow_radius((10.0, None))
        .shadow_spread((3.0, None))
        .pointer_events(false)
        .build()
        .unwrap();
    layer.build_layer_tree(&tree);
    // Hang off the top-right corner of the icon (icon starts at x = icon_width * 0.025)
    let pos_x = icon_width * 0.90;// - badge_size * 0.55;
    let pos_y = icon_width * 0.05;
    layer.set_position(Point { x: pos_x, y: pos_y }, None);
}

/// Configure a progress-bar overlay layer (initially hidden; caller must set_opacity to show it).
/// Positioned near the bottom of the square icon_stack (overlays the lower part of the icon).
pub fn setup_progress_layer(layer: &Layer, icon_width: f32) {
    let bar_width = icon_width * 0.78;
    let bar_height = icon_width * 0.062;
    // 3% margin from the bottom edge of the square icon_stack.
    let pos_y = icon_width - bar_height + (icon_width * 0.01);
    let pos_x = (icon_width - bar_width) / 2.0;
    let tree = LayerTreeBuilder::default()
        .key("progress")
        .layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        })
        .size(Size {
            width: taffy::Dimension::Length(bar_width),
            height: taffy::Dimension::Length(bar_height),
        })
        .opacity((0.0, None))
        .pointer_events(false)
        .build()
        .unwrap();
    layer.build_layer_tree(&tree);
    layer.set_position(Point { x: pos_x, y: pos_y }, None);
}

pub fn setup_app_icon(
    layer: &Layer,
    icon_layer: &Layer,
    application: Application,
    icon_width: f32,
    running: bool,
) {
    let app_name = application
        .desktop_name()
        .clone()
        .unwrap_or(application.identifier.clone());

    // `draw_app_icon` no longer draws the running indicator; pass `running` here only
    // to keep the public signature stable — running indicator is a separate layer.
    let _ = running;
    let draw_picture = Some(draw_app_icon(&application));
    let height_padding = icon_width * 0.20;
    let container_tree = LayerTreeBuilder::default()
        .key(app_name)
        .layout_style(taffy::Style {
            display: taffy::Display::Flex,
            position: taffy::Position::Relative,
            overflow: taffy::geometry::Point {
                x: taffy::style::Overflow::Visible,
                y: taffy::style::Overflow::Visible,
            },
            ..Default::default()
        })
        .size((
            Size {
                width: taffy::Dimension::Length(icon_width),
                // Outer container keeps height_padding for the running indicator dot.
                height: taffy::Dimension::Length(icon_width + height_padding),
            },
            Some(Transition::ease_in_quad(0.2)),
        ))
        
        .build()
        .unwrap();
    layer.build_layer_tree(&container_tree);

    let icon_tree = LayerTreeBuilder::default()
        .key("icon")
        .layout_style(taffy::Style {
            display: taffy::Display::Block,
            position: taffy::Position::Relative,
            ..Default::default()
        })
        .size((
            Size {
                width: taffy::Dimension::Percent(1.0),
                height: taffy::Dimension::Percent(1.0),
            },
            None, // None
        ))
        .pointer_events(false)
        .image_cache(true)
        .background_color(Color::new_rgba(1.0, 0.0, 0.0, 0.0))
        .content(draw_picture)
        .build()
        .unwrap();
    icon_layer.build_layer_tree(&icon_tree);
}


pub fn setup_miniwindow_icon(layer: &Layer, inner_layer: &Layer, _icon_width: f32) {
    let container_tree = LayerTreeBuilder::default()
        .key("miniwindow")
        .layout_style(taffy::Style {
            display: taffy::Display::Flex,
            ..Default::default()
        })
        .size((
            Size {
                width: taffy::Dimension::Length(0.0),
                height: taffy::Dimension::Percent(1.0),
            },
            Some(Transition::ease_in_quad(0.2)),
        ))
        .background_color(Color::new_rgba(1.0, 0.0, 0.0, 0.0))
        .build()
        .unwrap();
    layer.build_layer_tree(&container_tree);

    let inner_tree = LayerTreeBuilder::default()
        .key("mini_window_content")
        .layout_style(taffy::Style {
            position: taffy::Position::Relative,
            ..Default::default()
        })
        .position(Point::default())
        .size((
            Size {
                width: taffy::Dimension::Percent(1.0),
                height: taffy::Dimension::Percent(1.0),
            },
            None,
        ))
        // fixme
        .image_cache(true)
        .pointer_events(false)
        // .background_color(Color::new_rgba(0.0, 0.5, 0.0, 0.5))
        .build()
        .unwrap();
    inner_layer.build_layer_tree(&inner_tree);
}

pub fn setup_label(new_layer: &Layer, label_text: String) {
    let _draw_scale = Config::with(|config| config.screen_scale as f32);
    let text_size = 26.0;
    let font_family = Config::with(|config| config.font_family.clone());
    let font = FONT_CACHE.with(|font_cache| {
        font_cache.make_font_with_fallback(
            font_family,
            layers::skia::FontStyle::default(),
            text_size,
        )
    });

    let text = label_text.clone();
    let paint = layers::skia::Paint::default();
    let text_bounds = font.measure_str(label_text, Some(&paint));

    let text_bounds = text_bounds.1;
    let arrow_height = 20.0;
    let text_padding_h = 30.0;
    let text_padding_v = 14.0;
    let safe_margin = 100.0;
    let label_size_width = text_bounds.width() + text_padding_h * 2.0 + safe_margin * 2.0;
    // Fixed height based on font size, not measured text bounds
    let label_size_height = text_size + arrow_height + text_padding_v * 2.0 + safe_margin * 2.0;

    let rect_corner_radius = 10.0;
    let arrow_width = 25.0;
    let arrow_corner_radius = 3.0;
    // Calculate tooltip dimensions
    let tooltip_width = label_size_width - safe_margin * 2.0;
    let tooltip_height = label_size_height - safe_margin * 2.0;

    let arrow_path = draw_balloon_rect(
        safe_margin,
        safe_margin,
        tooltip_width,
        tooltip_height,
        rect_corner_radius,
        arrow_width,
        arrow_height,
        0.5,
        arrow_corner_radius,
    );

    let draw_label = move |canvas: &layers::skia::Canvas, w: f32, h: f32| -> layers::skia::Rect {
        // Tooltip parameters

        let text = text.clone();
        let tooltip_height = h - safe_margin * 2.0;

        // Paint for the tooltip background
        let mut paint = layers::skia::Paint::default();

        // choose colors according to theme scheme so tooltip looks correct in dark mode
        let (bg_col, text_col) = Config::with(|c| match c.theme_scheme {
            crate::theme::ThemeScheme::Light => (
                layers::skia::Color4f::new(157.0 / 255.0, 157.0 / 255.0, 157.0 / 255.0, 1.0),
                theme_colors().text_primary.c4f(),
            ),
            crate::theme::ThemeScheme::Dark => (
                layers::skia::Color4f::new(157.0 / 255.0, 157.0 / 255.0, 157.0 / 255.0, 1.0),
                theme_colors().text_primary.c4f(),
            ),
        });
        paint.set_color4f(bg_col, None);
        paint.set_anti_alias(true);

        // // Paint for the text
        let mut text_paint = layers::skia::Paint::default();
        text_paint.set_color4f(text_col, None);
        text_paint.set_anti_alias(true);

        // // Draw the text inside the tooltip
        let text_x = safe_margin + text_padding_h;
        // Position text baseline at 68% of content area (excluding arrow)
        let text_y = safe_margin + (tooltip_height - arrow_height) * 0.68;
        canvas.draw_str(text.as_str(), (text_x, text_y), &font, &text_paint);
        layers::skia::Rect::from_xywh(0.0, 0.0, w, h)
    };
    let label_tree = LayerTreeBuilder::default()
        .key(format!("{}_label", new_layer.key()))
        .shape(layers::prelude::Shape::from_path(&arrow_path))
        .blend_mode(layers::prelude::BlendMode::BackgroundBlur)
        .layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            max_size: taffy::geometry::Size {
                width: taffy::style::Dimension::Length(label_size_width),
                height: taffy::style::Dimension::Length(label_size_height),
            },
            inset: taffy::geometry::Rect::<LengthPercentageAuto> {
                top: LengthPercentageAuto::Auto,
                right: LengthPercentageAuto::Auto,
                bottom: LengthPercentageAuto::Auto,
                left: LengthPercentageAuto::Percent(0.5),
            },
            ..Default::default()
        })
        .size(Size {
            width: taffy::Dimension::Length(label_size_width),
            height: taffy::Dimension::Length(label_size_height),
        })
        .background_color(theme_colors().materials_ultrathick)
        .position(Point {
            x: -label_size_width / 2.0,
            y: -label_size_height - 10.0 + safe_margin,
        })
        .shadow_color(theme_colors().shadow_color)
        .shadow_offset(((0.0, 0.0).into(), None))
        .shadow_radius((20.0, None))
        .opacity((0.0, None))
        .pointer_events(false)
        .content(Some(draw_label))
        .build()
        .unwrap();

    new_layer.build_layer_tree(&label_tree);
}

/// Draw the app icon image only (no running indicator — that is a separate layer).
pub fn draw_app_icon(application: &Application) -> ContentDrawFunction {
    let application = application.clone();
    let draw_picture = move |canvas: &layers::skia::Canvas, w: f32, h: f32| -> layers::skia::Rect {
        // Fill the entire layer with the icon.
        let icon_size = w;
        let icon_y = (h - icon_size) / 2.0;
        let icon_x = 0.0;
        if let Some(image) = &application.icon.clone() {
            let mut paint =
                layers::skia::Paint::new(layers::skia::Color4f::new(1.0, 1.0, 1.0, 1.0), None);

            paint.set_style(layers::skia::paint::Style::Fill);
            // draw image with shadow
            let shadow_color = layers::skia::Color4f::new(0.0, 0.0, 0.0, 0.5);

            let mut shadow_paint = layers::skia::Paint::new(shadow_color, None);
            let shadow_offset = layers::skia::Vector::new(5.0, 5.0);
            let shadow_color = layers::skia::Color::from_argb(128, 0, 0, 0); // semi-transparent black
            let shadow_blur_radius = 5.0;

            let shadow_filter = layers::skia::image_filters::drop_shadow_only(
                (shadow_offset.x, shadow_offset.y),
                (shadow_blur_radius, shadow_blur_radius),
                shadow_color,
                None,
                None,
                layers::skia::image_filters::CropRect::default(),
            );
            shadow_paint.set_image_filter(shadow_filter);

            canvas.draw_image_rect(
                image,
                None,
                layers::skia::Rect::from_xywh(icon_x, icon_y, icon_size, icon_size),
                &shadow_paint,
            );
            let resampler = layers::skia::CubicResampler::catmull_rom();

            canvas.draw_image_rect_with_sampling_options(
                image,
                None,
                layers::skia::Rect::from_xywh(icon_x, icon_y, icon_size, icon_size),
                layers::skia::SamplingOptions::from(resampler),
                &paint,
            );
        } else {
            let mut rect = layers::skia::Rect::from_xywh(0.0, 0.0, icon_size, icon_size);
            rect.inset((10.0, 10.0));
            let rrect = layers::skia::RRect::new_rect_xy(rect, 10.0, 10.0);
            let mut paint =
                layers::skia::Paint::new(layers::skia::Color4f::new(1.0, 1.0, 1.0, 0.2), None);
            canvas.draw_rrect(rrect, &paint);

            paint.set_stroke(true);
            paint.set_stroke_width(6.0);
            paint.set_color4f(layers::skia::Color4f::new(0.0, 0.0, 0.0, 1.0), None);
            let intervals = [12.0, 6.0]; // Length of the dash and the gap
            let path_effect = PathEffect::dash(&intervals, 0.0);
            paint.set_path_effect(path_effect);
            canvas.draw_rrect(rrect, &paint);

            if let Some(picure) = &application.picture {
                // let mut paint = layers::skia::Paint::default();
                canvas.draw_picture(picure, None, None);
            }
        }
        layers::skia::Rect::from_xywh(0.0, 0.0, w, h)
    };

    draw_picture.into()
}


