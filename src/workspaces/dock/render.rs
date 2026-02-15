use layers::skia::PathEffect;
use layers::{prelude::*, types::Size};
use taffy::LengthPercentageAuto;

use crate::{
    config::Config,
    theme::theme_colors,
    workspaces::{
        utils::{draw_balloon_rect, FONT_CACHE},
        Application,
    },
};

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

    let draw_picture = Some(draw_app_icon(&application, running));
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
                height: taffy::Dimension::Length(icon_width + height_padding),
            },
            Some(Transition::ease_in_quad(0.2)), // None
        ))
        .background_color(Color::new_rgba(1.0, 0.0, 0.0, 0.0))
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

/// Setup app container for protocol-provided layer (wlr layer shell)
/// The icon layer is the wlr layer itself, we just draw the running indicator
pub fn setup_app_container_for_protocol_layer(
    layer: &Layer,
    indicator_layer: &Layer,
    app_name: String,
    icon_width: f32,
    running: bool,
) {
    let draw_indicator = Some(draw_running_indicator_only(running));
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
                height: taffy::Dimension::Length(icon_width + height_padding),
            },
            Some(Transition::ease_in_quad(0.2)),
        ))
        .background_color(Color::new_rgba(1.0, 0.0, 0.0, 0.0))
        .build()
        .unwrap();
    layer.build_layer_tree(&container_tree);

    // Indicator layer only draws the running circle
    let indicator_tree = LayerTreeBuilder::default()
        .key("indicator")
        .layout_style(taffy::Style {
            display: taffy::Display::Block,
            position: taffy::Position::Absolute,
            ..Default::default()
        })
        .size((
            Size {
                width: taffy::Dimension::Percent(1.0),
                height: taffy::Dimension::Percent(1.0),
            },
            None,
        ))
        .pointer_events(false)
        .background_color(Color::new_rgba(1.0, 0.0, 0.0, 0.0))
        .content(draw_indicator)
        .build()
        .unwrap();
    indicator_layer.build_layer_tree(&indicator_tree);
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

pub fn draw_app_icon(application: &Application, running: bool) -> ContentDrawFunction {
    let application = application.clone();
    let draw_picture = move |canvas: &layers::skia::Canvas, w: f32, h: f32| -> layers::skia::Rect {
        let icon_size = (w * 0.95).max(0.0);
        // Scale indicator with icon size (base ratio from 95.0)
        let circle_radius = icon_size * 0.025;
        let icon_y = h / 2.0 - icon_size / 2.0 - icon_size * 0.04;
        let icon_x = (w - icon_size) / 2.0;
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
        if running {
            // use primary text color for the running indicator (dark on light theme)
            let mut color = theme_colors().text_primary.c4f();
            color.a = 0.9;
            let mut paint = layers::skia::Paint::new(color, None);
            paint.set_anti_alias(true);
            paint.set_style(layers::skia::paint::Style::Fill);
            let indicator_y_offset = icon_size * 0.04;
            canvas.draw_circle(
                (w / 2.0, h - (indicator_y_offset + circle_radius)),
                circle_radius,
                &paint,
            );
        }

        layers::skia::Rect::from_xywh(0.0, 0.0, w, h)
    };

    draw_picture.into()
}

/// Draw only the running indicator (no icon) for protocol-provided layers
pub fn draw_running_indicator_only(running: bool) -> ContentDrawFunction {
    let draw_picture = move |canvas: &layers::skia::Canvas, w: f32, h: f32| -> layers::skia::Rect {
        let icon_size = (w * 0.95).max(0.0);
        let circle_radius = icon_size * 0.025;
        
        if running {
            // use primary text color for the running indicator (dark on light theme)
            let mut color = theme_colors().text_primary.c4f();
            color.a = 0.9;
            let mut paint = layers::skia::Paint::new(color, None);
            paint.set_anti_alias(true);
            paint.set_style(layers::skia::paint::Style::Fill);
            let indicator_y_offset = icon_size * 0.04;
            canvas.draw_circle(
                (w / 2.0, h - (indicator_y_offset + circle_radius)),
                circle_radius,
                &paint,
            );
        }

        layers::skia::Rect::from_xywh(0.0, 0.0, w, h)
    };

    draw_picture.into()
}
