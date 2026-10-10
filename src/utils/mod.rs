use layers::{
    prelude::{ContentDrawFunction, Layer, PointerHandlerFunction, Transition},
    skia::{self},
};

use crate::{config::Config, workspaces::utils::FONT_CACHE};
pub mod natural_layout;

/// Parse a hex colour (`#1a1a2e`, `1a1a2e`, `#RGB`, `#RRGGBBAA`) into a Skia
/// `Color4f`, falling back to a dark blue for anything unreadable.
pub fn parse_hex_color(hex: &str) -> skia::Color4f {
    otto_kit::color::parse_hex_lenient(hex)
        .map(skia::Color4f::from)
        .unwrap_or_else(|| {
            tracing::warn!("Invalid hex color format: {}, using default", hex);
            skia::Color4f::new(0.1, 0.1, 0.18, 1.0)
        })
}

// Delegate icon functions to otto-kit
pub use otto_kit::icons::image_from_path;

/// Find an icon using the configured theme or auto-detection.
///
/// Reads the theme name from otto's Config and delegates to otto-kit.
///
/// With no theme configured — and Otto's own not installed — the desktop's is
/// used, as otto-kit apps do, rather than hicolor alone.
pub fn find_icon_with_theme(icon_name: &str, size: i32, scale: i32) -> Option<String> {
    static DESKTOP_THEME: std::sync::LazyLock<Option<String>> =
        std::sync::LazyLock::new(otto_kit::icon_theme::desktop_theme);
    let theme = Config::with(|config| config.icon_theme.clone()).or_else(|| DESKTOP_THEME.clone());
    otto_kit::icons::find_icon_in_theme(icon_name, size, scale, theme.as_deref())
}

/// Look up a themed icon for compositor chrome.
///
/// Unlike [`named_icon`], this resolves against the icon theme from otto's
/// config: otto-kit's own theme state is fed by the Settings portal, which the
/// compositor serves but never consumes, so in-process lookups would otherwise
/// fall back to hicolor and find nothing.
///
/// The lookup is strict — a generic `application-default-icon` substituted for
/// a missing UI icon is rejected, so the caller can try the next candidate name.
fn themed_chrome_icon(icon_name: &str) -> Option<layers::skia::Image> {
    let path = find_icon_with_theme(icon_name, 512, 1)?;
    let matches = std::path::Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|stem| stem.starts_with(icon_name));
    matches.then(|| otto_kit::icons::cached_file_icon(&path, 512))?
}

/// Draw the first of `icon_names` that the current icon theme provides.
///
/// Themes disagree on names for the same glyph (`close-symbolic` vs
/// `window-close-symbolic`, `plus-symbolic` vs `list-add-symbolic`), so chrome
/// passes every spelling it accepts.
pub fn draw_named_icon_any(icon_names: &[&str]) -> Option<ContentDrawFunction> {
    icon_names
        .iter()
        .find_map(|name| themed_chrome_icon(name))
        .map(icon_draw_function)
}

fn icon_draw_function(icon: layers::skia::Image) -> ContentDrawFunction {
    let resampler = skia::CubicResampler::catmull_rom();

    let draw_function = move |canvas: &skia::Canvas, w: f32, h: f32| -> layers::skia::Rect {
        // Symbolic icons ship as dark glyphs, so tint them with the theme's
        // text colour — otherwise they vanish against a dark background.
        let mut paint = skia::Paint::new(skia::Color4f::new(1.0, 1.0, 1.0, 1.0), None);
        paint.set_anti_alias(true);
        paint.set_color_filter(skia::color_filters::blend(
            crate::theme::theme_colors().text_primary.c4f().to_color(),
            skia::BlendMode::SrcIn,
        ));
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
        layers::skia::Paint::new(crate::theme::theme_colors().text_primary.c4f(), None);
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

pub fn button_press_scale(s: f32) -> PointerHandlerFunction {
    let f = move |layer: &Layer, _x: f32, _y: f32| {
        layer.set_scale(
            layers::types::Point::new(s, s),
            Transition::spring(0.3, 0.1),
        );
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
pub fn is_laptop_panel(connector_name: &str) -> bool {
    connector_name.starts_with("eDP-")
        || connector_name.starts_with("LVDS-")
        || connector_name.starts_with("DSI-")
}
