use layers::skia::PathEffect;
use layers::{prelude::*, types::Size};
use taffy::LengthPercentageAuto;

use crate::{
    config::{Config, DockPosition},
    theme::theme_colors,
    utils::parse_hex_color,
    workspaces::{
        utils::{draw_balloon_rect, BalloonArrow, FONT_CACHE},
        Application,
    },
};

/// The colour filter that tints app icons, or `None` when `dock.colorize_icons`
/// is off.
///
/// Every icon on screen is a mirror of the one source layer owned by
/// `AppIconsManager`, so the tint can't live on the source: it is applied to
/// each mirror instead, and both the dock and the app switcher take it from
/// here so a themed desktop is tinted consistently.
///
/// The matrix maps the icon to its luminance re-tinted in `colorize_color`,
/// then blends that back toward the original by `colorize_intensity`.
pub fn icon_color_filter() -> Option<layers::skia::ColorFilter> {
    use layers::skia;

    let dock_config = Config::with(|c| c.dock.clone());
    if !dock_config.colorize_icons {
        return None;
    }
    let color = parse_hex_color(&dock_config.colorize_color);
    let intensity = dock_config.colorize_intensity.clamp(0.0, 1.0) as f32;
    let (r, g, b) = (color.r, color.g, color.b);
    let (lr, lg, lb) = (0.2126_f32, 0.7152_f32, 0.0722_f32);
    let inv = 1.0 - intensity;
    let matrix = skia::ColorMatrix::new(
        inv + intensity * lr * r,
        intensity * lg * r,
        intensity * lb * r,
        0.0,
        0.0,
        intensity * lr * g,
        inv + intensity * lg * g,
        intensity * lb * g,
        0.0,
        0.0,
        intensity * lr * b,
        intensity * lg * b,
        inv + intensity * lb * b,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
    );
    Some(skia::color_filters::matrix(&matrix, None))
}

/// The badge circle's diameter for an icon `icon_width` wide.
pub fn badge_size(icon_width: f32) -> f32 {
    icon_width * 0.4
}

/// Draw a badge (red circle with white text), sized to fill the layer bounds.
///
/// `size` is what the layer will measure once laid out. A badge whose text is
/// set before its first layout pass is handed a zero size here, and the digit
/// drawn into that recording would be invisible for as long as the recording
/// is reused — leaving a red circle with nothing in it.
pub fn draw_badge(text: String, size: f32) -> ContentDrawFunction {
    let draw_fn = move |canvas: &layers::skia::Canvas, w: f32, h: f32| -> layers::skia::Rect {
        let (w, h) = (
            if w > 0.0 { w } else { size },
            if h > 0.0 { h } else { size },
        );
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
        let text_x = w / 2.0 - text_bounds.width() / 2.0 - text_bounds.left;
        let text_y = h / 2.0 - text_bounds.height() / 2.0 - text_bounds.top;

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
        let mut bg_paint =
            layers::skia::Paint::new(layers::skia::Color4f::new(0.0, 0.0, 0.0, 0.30), None);
        bg_paint.set_anti_alias(true);
        let bg_rect = layers::skia::Rect::from_xywh(0.0, 0.0, w, h);
        let bg_rrect = layers::skia::RRect::new_rect_xy(bg_rect, corner_radius, corner_radius);
        canvas.draw_rrect(bg_rrect, &bg_paint);

        // White fill proportional to progress
        if value > 0.0 {
            let fill_w = (w * value).max(h); // keep at least circle-width so it never looks empty
            let mut fill_paint =
                layers::skia::Paint::new(layers::skia::Color4f::new(1.0, 1.0, 1.0, 0.92), None);
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
    let badge_size = badge_size(icon_width);
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
        .border_corner_radius(BorderRadius::new_single(badge_size / 2.0))
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
    let pos_x = icon_width * 0.90; // - badge_size * 0.55;
    let pos_y = icon_width * 0.05;
    layer.set_position(Point { x: pos_x, y: pos_y }, None);
}

/// Configure a progress-bar overlay layer (initially hidden; caller must set_opacity to show it).
/// Positioned near the bottom of the square icon_stack (overlays the lower part of the icon).
pub fn setup_progress_layer(layer: &Layer, icon_width: f32) {
    let bar_width = icon_width * 0.78;
    let bar_height = icon_width * 0.062;
    // 3% margin from the bottom edge of the square icon_stack.
    let pos_y = icon_width - bar_height;
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
    let _height_padding = icon_width * 0.20;
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
                height: taffy::Dimension::Percent(1.0),
            },
            None,
        ))
        .picture_cached(false)
        .image_cache(false)
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
        // EXPERIMENT P4: enable picture+image caching on dock icon layers (icons are static)
        .picture_cached(true)
        .image_cache(true)
        .background_color(Color::new_rgba(1.0, 0.0, 0.0, 0.0))
        .content(draw_picture)
        .build()
        .unwrap();
    icon_layer.build_layer_tree(&icon_tree);
}

pub fn setup_miniwindow_icon(layer: &Layer, inner_layer: &Layer, icon_width: f32) {
    let container_tree = LayerTreeBuilder::default()
        .key("miniwindow")
        .layout_style(taffy::Style {
            display: taffy::Display::Flex,
            ..Default::default()
        })
        .size((
            Size {
                width: taffy::Dimension::Length(0.0),
                height: taffy::Dimension::Length(icon_width),
            },
            None,
        ))
        .background_color(Color::new_rgba(1.0, 0.0, 0.0, 0.0))
        // .image_cache(true)
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

/// Widest a tooltip balloon may get, in logical points, arrow excluded.
const MAX_LABEL_BODY_WIDTH: f32 = 280.0;

/// Shorten `text` until it fits `max_width`, appending an ellipsis.
///
/// Measured with the same font and paint the balloon draws with, so what fits
/// here is exactly what fits on screen.
fn elide_to_width(
    text: &str,
    font: &layers::skia::Font,
    paint: &layers::skia::Paint,
    max_width: f32,
) -> String {
    if max_width <= 0.0 || font.measure_str(text, Some(paint)).1.width() <= max_width {
        return text.to_string();
    }
    // Char boundaries, so a multi-byte title never splits mid-codepoint.
    let cuts: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    // Largest prefix whose elided form still fits; 0 chars always qualifies.
    let mut best = 0;
    let (mut lo, mut hi) = (0, cuts.len().saturating_sub(1));
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let candidate = format!("{}\u{2026}", &text[..cuts[mid]]);
        if font.measure_str(&candidate, Some(paint)).1.width() <= max_width {
            best = mid;
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    format!("{}\u{2026}", &text[..cuts[best]])
}

/// Build the tooltip balloon shown while hovering a dock element.
///
/// The balloon always points at the element it belongs to, so its arrow — and
/// with it the whole layout — follows the edge the dock is docked to: a bottom
/// dock puts the tooltip above the icon, a side dock beside it.
pub fn setup_label(new_layer: &Layer, label_text: String, position: DockPosition) {
    // The tooltip is drawn straight into the scene, so every measurement below
    // is in physical pixels: keep the design in logical points and scale once.
    let scale = Config::with(|config| config.screen_scale as f32);
    let text_size = 13.0 * scale;
    let font_family = Config::with(|config| config.font_family.clone());
    let font = FONT_CACHE.with(|font_cache| {
        font_cache.make_font_with_fallback(
            font_family,
            layers::skia::FontStyle::default(),
            text_size,
        )
    });

    let paint = layers::skia::Paint::default();
    let text_padding_h = 15.0 * scale;
    // Long window titles would otherwise grow the balloon until it runs past
    // the screen edge and gets clipped mid-word: elide instead.
    let max_text_width = MAX_LABEL_BODY_WIDTH * scale - text_padding_h * 2.0;
    let label_text = elide_to_width(&label_text, &font, &paint, max_text_width);

    let text = label_text.clone();
    let text_bounds = font.measure_str(&label_text, Some(&paint));

    let text_bounds = text_bounds.1;
    let arrow_height = 10.0 * scale;
    let text_padding_v = 7.0 * scale;
    let safe_margin = 50.0 * scale;
    // The text box, without the arrow that sticks out of one of its edges.
    let body_width = text_bounds.width() + text_padding_h * 2.0;
    let body_height = text_size + text_padding_v * 2.0;
    let arrow = match position {
        DockPosition::Bottom => BalloonArrow::Bottom,
        // A dock on the left edge puts the tooltip to the icon's right, so the
        // arrow points back at the dock — and vice versa.
        DockPosition::Left => BalloonArrow::Left,
        DockPosition::Right => BalloonArrow::Right,
    };
    let (tooltip_width, tooltip_height) = if position.is_vertical() {
        (body_width + arrow_height, body_height)
    } else {
        (body_width, body_height + arrow_height)
    };
    let label_size_width = tooltip_width + safe_margin * 2.0;
    let label_size_height = tooltip_height + safe_margin * 2.0;

    let rect_corner_radius = 5.0 * scale;
    let arrow_width = 12.5 * scale;
    let arrow_corner_radius = 1.5 * scale;

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
        arrow,
    );

    // Where the text box starts inside the layer: the arrow eats into the
    // tooltip on the side it points to.
    let body_x = safe_margin
        + if arrow == BalloonArrow::Left {
            arrow_height
        } else {
            0.0
        };

    let draw_label = move |canvas: &layers::skia::Canvas, w: f32, h: f32| -> layers::skia::Rect {
        // Tooltip parameters

        let text = text.clone();

        // The balloon body is the layer's own background (see
        // `background_color` below); all this draw has to add is the label.
        // Both used to be picked here from a `theme_scheme` match whose two
        // arms held the same hardcoded #9d9d9d — the *light* tooltip material,
        // copied verbatim into the dark arm. It never reached the screen
        // either, the layer background painted over it.
        let mut text_paint = layers::skia::Paint::default();
        text_paint.set_color4f(theme_colors().text_primary.c4f(), None);
        text_paint.set_anti_alias(true);

        // // Draw the text inside the tooltip
        let text_x = body_x + text_padding_h;
        // Position text baseline at 68% of the text box
        let text_y = safe_margin + body_height * 0.68;
        canvas.draw_str(text.as_str(), (text_x, text_y), &font, &text_paint);
        layers::skia::Rect::from_xywh(0.0, 0.0, w, h)
    };
    let label_tree = LayerTreeBuilder::default()
        // A stable key: the tooltip is rebuilt in place every time the dock
        // moves, and deriving the key from the layer's own key grew it by a
        // suffix on every rebuild.
        .key("dock_label")
        .shape(layers::prelude::Shape::from_path(&arrow_path))
        .blend_mode(layers::prelude::BlendMode::BackgroundBlur)
        .layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            max_size: taffy::geometry::Size {
                width: taffy::style::Dimension::Length(label_size_width),
                height: taffy::style::Dimension::Length(label_size_height),
            },
            // Anchored to the middle of the icon's far edge; the offset below
            // then places the balloon so its arrow tip lands on the icon.
            inset: match arrow {
                BalloonArrow::Bottom => taffy::geometry::Rect::<LengthPercentageAuto> {
                    top: LengthPercentageAuto::Auto,
                    right: LengthPercentageAuto::Auto,
                    bottom: LengthPercentageAuto::Auto,
                    left: LengthPercentageAuto::Percent(0.5),
                },
                BalloonArrow::Left => taffy::geometry::Rect::<LengthPercentageAuto> {
                    top: LengthPercentageAuto::Percent(0.5),
                    right: LengthPercentageAuto::Auto,
                    bottom: LengthPercentageAuto::Auto,
                    left: LengthPercentageAuto::Percent(1.0),
                },
                BalloonArrow::Right => taffy::geometry::Rect::<LengthPercentageAuto> {
                    top: LengthPercentageAuto::Percent(0.5),
                    right: LengthPercentageAuto::Auto,
                    bottom: LengthPercentageAuto::Auto,
                    left: LengthPercentageAuto::Percent(0.0),
                },
            },
            ..Default::default()
        })
        .size(Size {
            width: taffy::Dimension::Length(label_size_width),
            height: taffy::Dimension::Length(label_size_height),
        })
        // The palette's tooltip material, so the balloon follows the scheme
        // like the rest of the chrome instead of sitting on one fixed grey.
        .background_color(theme_colors().materials_controls_tooltip)
        .position(match arrow {
            BalloonArrow::Bottom => Point {
                x: -label_size_width / 2.0,
                y: -label_size_height - 5.0 * scale + safe_margin,
            },
            BalloonArrow::Left => Point {
                x: -safe_margin + 5.0 * scale,
                y: -label_size_height / 2.0,
            },
            BalloonArrow::Right => Point {
                x: -label_size_width + safe_margin - 5.0 * scale,
                y: -label_size_height / 2.0,
            },
        })
        .shadow_color(theme_colors().shadow_color)
        .shadow_offset(((0.0, 0.0).into(), None))
        .shadow_radius((10.0 * scale, None))
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

#[cfg(test)]
mod tests {
    use super::*;
    use layers::engine::Engine;
    use serial_test::serial;

    const SLOT: f32 = 100.0;

    /// The tint is opt-in: with `colorize_icons` off every icon mirror — the
    /// dock's and the app switcher's — must be handed no filter at all, not an
    /// identity matrix that would still cost a layer pass.
    #[test]
    #[serial]
    fn icon_tint_is_absent_unless_colorize_is_enabled() {
        let _ = Config::update(|c| c.dock.colorize_icons = false);
        assert!(icon_color_filter().is_none());

        let _ = Config::update(|c| {
            c.dock.colorize_icons = true;
            c.dock.colorize_color = "#ff8800".to_string();
            c.dock.colorize_intensity = 1.0;
        });
        assert!(icon_color_filter().is_some());

        let _ = Config::update(|c| c.dock.colorize_icons = false);
    }

    /// A slot layer with a label sublayer, laid out once so the render bounds
    /// are meaningful. Returns (engine, slot, label).
    fn label_scene(position: DockPosition) -> (std::sync::Arc<Engine>, Layer, Layer) {
        let engine = Engine::create(1000.0, 1000.0);
        let slot = engine.new_layer();
        slot.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        slot.set_size(Size::points(SLOT, SLOT), None);
        slot.set_position(layers::types::Point::new(400.0, 400.0), None);
        let _ = engine.add_layer(&slot);

        let label = engine.new_layer();
        let _ = slot.add_sublayer(&label);
        setup_label(&label, "Calculator".to_string(), position);
        engine.update(0.0);
        (engine, slot, label)
    }

    /// The balloon itself, in the same coordinate space as the slot bounds:
    /// the layer is padded by a safe margin all around, so its own bounds say
    /// nothing about where the tooltip is drawn.
    fn balloon_rect(label: &Layer) -> layers::skia::Rect {
        label.render_layer().global_shape_bounds
    }

    /// A very long window title must not grow the balloon without bound: it is
    /// elided so the tooltip stays inside its cap instead of being clipped.
    #[test]
    #[serial]
    fn long_titles_are_elided_to_the_max_width() {
        let _ = Config::update(|c| c.dock.position = DockPosition::Bottom);
        let scale = Config::with(|c| c.screen_scale as f32);
        let (_engine, _slot, label) = label_scene(DockPosition::Bottom);
        let short = balloon_rect(&label).width();

        let long = "The Otto Scene Graph - Layer Tree and KMS Planes - a very long title";
        setup_label(&label, long.to_string(), DockPosition::Bottom);
        _engine.update(0.0);
        let wide = balloon_rect(&label).width();

        assert!(
            wide > short,
            "a longer title should still widen the balloon ({wide} vs {short})"
        );
        assert!(
            wide <= MAX_LABEL_BODY_WIDTH * scale + 1.0,
            "balloon {wide} wider than the {} cap",
            MAX_LABEL_BODY_WIDTH * scale
        );
    }

    #[test]
    #[serial]
    fn bottom_dock_puts_the_balloon_above_the_slot() {
        let _ = Config::update(|c| c.dock.position = DockPosition::Bottom);
        let (_engine, slot, label) = label_scene(DockPosition::Bottom);
        let slot = slot.render_bounds_transformed();
        let balloon = balloon_rect(&label);

        assert!(
            balloon.bottom <= slot.y() + 1.0,
            "balloon {balloon:?} should sit above the slot {slot:?}"
        );
        assert!(
            (balloon.center_x() - slot.center_x()).abs() < 1.0,
            "balloon {balloon:?} should be centred on the slot {slot:?}"
        );
    }

    #[test]
    #[serial]
    fn left_dock_puts_the_balloon_right_of_the_slot() {
        let _ = Config::update(|c| c.dock.position = DockPosition::Left);
        let (_engine, slot, label) = label_scene(DockPosition::Left);
        let slot = slot.render_bounds_transformed();
        let balloon = balloon_rect(&label);

        assert!(
            balloon.x() >= slot.right - 1.0,
            "balloon {balloon:?} should sit right of the slot {slot:?}"
        );
        assert!(
            (balloon.center_y() - slot.center_y()).abs() < 1.0,
            "balloon {balloon:?} should be centred on the slot {slot:?}"
        );
    }

    #[test]
    #[serial]
    fn right_dock_puts_the_balloon_left_of_the_slot() {
        let _ = Config::update(|c| c.dock.position = DockPosition::Right);
        let (_engine, slot, label) = label_scene(DockPosition::Right);
        let slot = slot.render_bounds_transformed();
        let balloon = balloon_rect(&label);

        assert!(
            balloon.right <= slot.x() + 1.0,
            "balloon {balloon:?} should sit left of the slot {slot:?}"
        );
        assert!(
            (balloon.center_y() - slot.center_y()).abs() < 1.0,
            "balloon {balloon:?} should be centred on the slot {slot:?}"
        );
    }

    /// Moving the dock rebuilds the tooltips in place: the same layer, rebuilt
    /// for the new edge, has to end up exactly where a freshly built one does.
    fn rebuild_matches_fresh(from: DockPosition, to: DockPosition) {
        let (_engine, slot, label) = label_scene(from);
        let engine = _engine;
        setup_label(&label, "Calculator".to_string(), to);
        engine.update(0.0);
        let rebuilt = balloon_rect(&label);
        let rebuilt_slot = slot.render_bounds_transformed();

        let (_engine2, slot2, label2) = label_scene(to);
        let fresh = balloon_rect(&label2);
        let fresh_slot = slot2.render_bounds_transformed();

        // Compare relative to each slot: the two scenes are laid out apart.
        let rel = |b: layers::skia::Rect, s: layers::skia::Rect| {
            (b.x() - s.x(), b.y() - s.y(), b.width(), b.height())
        };
        let (rx, ry, rw, rh) = rel(rebuilt, rebuilt_slot);
        let (fx, fy, fw, fh) = rel(fresh, fresh_slot);
        assert!(
            (rx - fx).abs() < 1.0 && (ry - fy).abs() < 1.0,
            "{from:?} -> {to:?}: rebuilt balloon at ({rx}, {ry}), fresh one at ({fx}, {fy})"
        );
        assert!(
            (rw - fw).abs() < 1.0 && (rh - fh).abs() < 1.0,
            "{from:?} -> {to:?}: rebuilt balloon is {rw}x{rh}, fresh one is {fw}x{fh}"
        );
    }

    #[test]
    #[serial]
    fn moving_the_dock_rebuilds_the_balloon() {
        for (from, to) in [
            (DockPosition::Bottom, DockPosition::Right),
            (DockPosition::Bottom, DockPosition::Left),
            (DockPosition::Right, DockPosition::Bottom),
            (DockPosition::Left, DockPosition::Bottom),
            (DockPosition::Left, DockPosition::Right),
        ] {
            let _ = Config::update(|c| c.dock.position = to);
            rebuild_matches_fresh(from, to);
        }
    }
}
