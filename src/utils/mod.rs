use layers::{
    prelude::{ContentDrawFunction, Layer, PointerHandlerFunction, Transition},
    skia::{self},
};

use crate::{config::Config, workspaces::utils::FONT_CACHE};
pub mod natural_layout;

/// Parse a hex color string (e.g., "#1a1a2e" or "1a1a2e") into a Skia Color4f
pub fn parse_hex_color(hex: &str) -> skia::Color4f {
    let hex = hex.trim_start_matches('#');

    // Default to a dark color if parsing fails
    let default_color = skia::Color4f::new(0.1, 0.1, 0.18, 1.0);

    if hex.len() != 6 {
        tracing::warn!("Invalid hex color format: {}, using default", hex);
        return default_color;
    }

    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(26) as f32 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(26) as f32 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(46) as f32 / 255.0;

    skia::Color4f::new(r, g, b, 1.0)
}

// Delegate icon functions to otto-kit
pub use otto_kit::icons::{image_from_path, named_icon};

/// Find an icon using the configured theme or auto-detection.
///
/// Reads the theme name from otto's Config and delegates to otto-kit.
pub fn find_icon_with_theme(icon_name: &str, size: i32, scale: i32) -> Option<String> {
    Config::with(|config| {
        otto_kit::icons::find_icon_in_theme(icon_name, size, scale, config.icon_theme.as_deref())
    })
}

/// Load an image from resources directory, falling back to system locations and theme icons
///
/// Search order:
/// 1. `./resources/{path}`
/// 2. `/etc/otto/share/{path}`
/// 3. Theme icon with `alternative_theme_icon_name`
pub fn resource_image(
    path: &str,
    alternative_theme_icon_name: &str,
) -> Option<layers::skia::Image> {
    // Try local resources directory first
    let local_path = format!("resources/{}", path);
    if let Some(image) = image_from_path(&local_path, (512, 512)) {
        return Some(image);
    }

    // Try system-wide installation directory
    let system_path = format!("/etc/otto/share/{}", path);
    if let Some(image) = image_from_path(&system_path, (512, 512)) {
        return Some(image);
    }

    // Fall back to theme icon
    named_icon(alternative_theme_icon_name)
}

/// Find a resource file path
///
/// Search order:
/// 1. `./resources/{path}`
/// 2. `/etc/otto/share/{path}`
pub fn resource_path(path: &str) -> Option<std::path::PathBuf> {
    use std::path::PathBuf;

    // Try local resources directory first
    let local_path = PathBuf::from(format!("resources/{}", path));
    if local_path.exists() {
        return Some(local_path);
    }

    // Try system-wide installation directory
    let system_path = PathBuf::from(format!("/etc/otto/share/{}", path));
    if system_path.exists() {
        return Some(system_path);
    }

    None
}
pub fn draw_named_icon(icon_name: &str) -> Option<ContentDrawFunction> {
    let icon = named_icon(icon_name);
    icon.as_ref().map(|icon| {
        let icon = icon.clone();
        let resampler = skia::CubicResampler::catmull_rom();

        let draw_function = move |canvas: &skia::Canvas, w: f32, h: f32| -> layers::skia::Rect {
            let paint = skia::Paint::new(skia::Color4f::new(1.0, 1.0, 1.0, 1.0), None);
            canvas.draw_image_rect_with_sampling_options(
                &icon,
                None,
                skia::Rect::from_xywh(0.0, 0.0, w, h),
                resampler,
                &paint,
            );
            skia::Rect::from_xywh(0.0, 0.0, w, h)
        };
        draw_function.into()
    })
}

pub fn notify_observers<T>(observers: &Vec<std::sync::Weak<dyn Observer<T>>>, event: &T) {
    for observer in observers {
        if let Some(observer) = observer.upgrade() {
            observer.notify(event);
        }
    }
}

pub trait Observable<T> {
    fn add_listener(&mut self, observer: std::sync::Arc<dyn Observer<T>>);
    fn observers<'a>(&'a self) -> Box<dyn Iterator<Item = std::sync::Weak<dyn Observer<T>>> + 'a>;
    fn notify_observers(&self, event: &T) {
        for observer in self.observers() {
            if let Some(observer) = observer.upgrade() {
                observer.notify(event);
            }
        }
    }
}

pub trait Observer<T>: Sync + Send {
    fn notify(&self, event: &T);
}

pub fn draw_text_content(
    text: impl Into<String>,
    text_style: skia::textlayout::TextStyle,
    text_align: skia::textlayout::TextAlign,
) -> Option<ContentDrawFunction> {
    let text = text.into();
    let foreground_paint =
        layers::skia::Paint::new(layers::skia::Color4f::new(0.0, 0.0, 0.0, 0.5), None);
    let mut text_style = text_style.clone();
    text_style.set_foreground_paint(&foreground_paint);
    let ff = Config::with(|c| c.font_family.clone());
    text_style.set_font_families(&[ff]);

    let mut paragraph_style = layers::skia::textlayout::ParagraphStyle::new();
    paragraph_style.set_text_direction(layers::skia::textlayout::TextDirection::LTR);
    paragraph_style.set_text_style(&text_style.clone());
    paragraph_style.set_text_align(text_align);
    paragraph_style.set_max_lines(1);
    paragraph_style.set_ellipsis("…");
    // println!("FS: {}", text_style.font_size());

    let draw_function = move |canvas: &skia::Canvas, w: f32, h: f32| -> layers::skia::Rect {
        // let paint = skia::Paint::new(skia::Color4f::new(1.0, 1.0, 1.0, 1.0), None);

        let mut builder = FONT_CACHE.with(|font_cache| {
            layers::skia::textlayout::ParagraphBuilder::new(
                &paragraph_style,
                font_cache.font_collection.clone(),
            )
        });
        let mut paragraph = builder.add_text(&text).build();
        paragraph.layout(w);
        paragraph.paint(canvas, (0.0, (h - paragraph.height()) / 2.0));

        skia::Rect::from_xywh(0.0, 0.0, w, h)
    };
    Some(draw_function.into())
}

pub fn button_press_filter() -> PointerHandlerFunction {
    let darken_color = skia::Color::from_argb(100, 100, 100, 100);
    let add = skia::Color::from_argb(0, 0, 0, 0);
    let filter = skia::color_filters::lighting(darken_color, add);

    let f = move |layer: &Layer, _x: f32, _y: f32| {
        layer.set_color_filter(filter.clone());
    };
    f.into()
}

pub fn button_press_scale(s: f32) -> PointerHandlerFunction {
    let f = move |layer: &Layer, _x: f32, _y: f32| {
        layer.set_scale(
            layers::types::Point::new(s, s),
            Transition::spring(0.3, 0.1),
        );
    };
    f.into()
}

pub fn button_release_filter() -> PointerHandlerFunction {
    let f = |layer: &Layer, _x: f32, _y: f32| {
        layer.set_color_filter(None);
    };
    f.into()
}

pub fn button_release_scale() -> PointerHandlerFunction {
    let f = |layer: &Layer, _x: f32, _y: f32| {
        layer.set_scale(
            layers::types::Point::new(1.0, 1.0),
            Transition::spring(0.3, 0.1),
        );
    };
    f.into()
}

/// Determines if a connector name indicates a laptop's internal panel
///
/// Laptop panels use specific connector types:
/// - eDP (embedded DisplayPort) - most modern laptops
/// - LVDS (Low-Voltage Differential Signaling) - older laptops
/// - DSI (Display Serial Interface) - some ARM-based devices
/// Split a string into shell words, handling double and single quotes.
/// Simplified replacement for the `shell-words` crate — covers the quoting
/// found in .desktop Exec= fields.
pub fn shell_split(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut chars = s.chars();
    let mut in_single = false;
    let mut in_double = false;

    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '\\' if in_double => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            c if c.is_ascii_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

pub fn is_laptop_panel(connector_name: &str) -> bool {
    connector_name.starts_with("eDP-")
        || connector_name.starts_with("LVDS-")
        || connector_name.starts_with("DSI-")
}
