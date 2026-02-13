use std::{
    collections::HashMap,
    sync::{Arc, OnceLock, RwLock},
};

use layers::{
    prelude::{ContentDrawFunction, Layer, PointerHandlerFunction, Transition},
    skia::{self},
    utils::load_svg_image,
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

static ICON_CACHE: OnceLock<Arc<RwLock<HashMap<String, skia::Image>>>> = OnceLock::new();

fn icon_cache() -> Arc<RwLock<HashMap<String, skia::Image>>> {
    ICON_CACHE
        .get_or_init(|| Arc::new(RwLock::new(HashMap::new())))
        .clone()
}

// FIXME check why skia_safe svg is broken
// pub fn image_from_svg(
//     image_data: &[u8],
//     ctx: Option<skia::gpu::DirectContext>,
// ) -> layers::skia::Image {
//     let options = usvg::Options::default();
//     let mut rtree = usvg::Tree::from_data(image_data, &options).unwrap();
//     rtree.size = usvg::Size::from_wh(512.0, 512.0).unwrap();
//     let xml_options = usvg::XmlOptions::default();
//     let xml = usvg::TreeWriting::to_string(&rtree, &xml_options);
//     let font_mgr = layers::skia::FontMgr::new();
//     let svg = layers::skia::svg::Dom::from_bytes(xml.as_bytes(), font_mgr).unwrap();

//     let mut surface = {
//         if let Some(mut ctx) = ctx {
//             let image_info = skia::ImageInfo::new(
//                 (512, 512),
//                 skia::ColorType::RGBA8888,
//                 skia::AlphaType::Premul,
//                 None,
//             );
//             skia::gpu::surfaces::render_target(
//                 &mut ctx,
//                 skia::gpu::Budgeted::No,
//                 &image_info,
//                 None,
//                 skia::gpu::SurfaceOrigin::TopLeft,
//                 None,
//                 false,
//                 false,
//             )
//             .unwrap()
//         } else {
//             skia::surfaces::raster_n32_premul((512, 512)).unwrap()
//         }
//     };

//     let canvas = surface.canvas();
//     svg.render(canvas);
//     surface.image_snapshot()
// }

pub fn image_from_path(path: &str, size: impl Into<skia::ISize>) -> Option<layers::skia::Image> {
    let image_path = std::path::Path::new(path);

    let image = if image_path.extension().and_then(std::ffi::OsStr::to_str) == Some("svg") {
        load_svg_image(path, size).ok()?
    } else {
        let image_data = std::fs::read(image_path).ok()?;
        layers::skia::Image::from_encoded(layers::skia::Data::new_copy(image_data.as_slice()))
            .unwrap()
    };

    Some(image)
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

pub fn named_icon(icon_name: &str) -> Option<layers::skia::Image> {
    let ic = icon_cache();
    let mut ic = ic.write().unwrap();
    if let Some(icon) = ic.get(icon_name) {
        return Some(icon.clone());
    }
    // not found
    let icon_path = find_icon_with_theme(icon_name, 512, 1);
    let icon = icon_path
        .as_ref()
        .and_then(|icon_path| image_from_path(icon_path, (512, 512)));
    if let Some(i) = icon.as_ref() {
        ic.insert(icon_name.to_string(), i.clone());
    }
    icon
}

/// Find an icon using the configured theme or auto-detection
pub fn find_icon_with_theme(icon_name: &str, size: i32, scale: i32) -> Option<String> {
    Config::with(|config| {
        if let Some(theme_name) = &config.icon_theme {
            // Use specified theme
            let dir_list_vector = xdgkit::icon_finder::generate_dir_list();

            // Try to find the theme by name
            let theme_dir = dir_list_vector
                .iter()
                .find(|dir| &dir.theme == theme_name)
                .cloned();

            if let Some(theme_dir) = theme_dir {
                // Load the IconTheme from the found directory
                let theme = xdgkit::icon_theme::IconTheme::from_pathbuff(theme_dir.index());

                xdgkit::icon_finder::multiple_find_icon(
                    icon_name.to_string(),
                    size,
                    scale,
                    dir_list_vector,
                    theme,
                )
                .map(|p| p.to_str().unwrap().to_string())
            } else {
                tracing::warn!(
                    "Icon theme '{}' not found, falling back to auto-detection",
                    theme_name
                );
                xdgkit::icon_finder::find_icon(icon_name.to_string(), size, scale)
                    .map(|p| p.to_str().unwrap().to_string())
            }
        } else {
            // Use auto-detection (None or omitted)
            xdgkit::icon_finder::find_icon(icon_name.to_string(), size, scale)
                .map(|p| p.to_str().unwrap().to_string())
        }
    })
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
pub fn is_laptop_panel(connector_name: &str) -> bool {
    connector_name.starts_with("eDP-")
        || connector_name.starts_with("LVDS-")
        || connector_name.starts_with("DSI-")
}
