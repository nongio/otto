//! XDG icon theme lookup, image loading, and caching.
//!
//! Provides `named_icon()` to look up and cache icons by name from the
//! system icon theme, with support for SVG and raster formats.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use skia_safe as skia;

// ---------------------------------------------------------------------------
// Icon cache
// ---------------------------------------------------------------------------

static ICON_CACHE: OnceLock<Arc<RwLock<HashMap<String, Option<skia::Image>>>>> = OnceLock::new();

fn icon_cache() -> Arc<RwLock<HashMap<String, Option<skia::Image>>>> {
    ICON_CACHE
        .get_or_init(|| Arc::new(RwLock::new(HashMap::new())))
        .clone()
}

/// Look up an icon by name from the icon theme, with caching.
///
/// Searches the system icon theme directories for the given icon name,
/// loads it (SVG or raster), caches the result, and returns the image.
pub fn named_icon(icon_name: &str) -> Option<skia::Image> {
    let ic = icon_cache();

    // Check cache (includes negative lookups)
    {
        let cache = ic.read().unwrap();
        if let Some(entry) = cache.get(icon_name) {
            return entry.clone();
        }
    }

    // Cache miss — look up and load
    let icon = find_icon(icon_name, 512, 1).and_then(|p| image_from_path(&p, (512, 512)));

    ic.write()
        .unwrap()
        .insert(icon_name.to_string(), icon.clone());
    icon
}

/// Look up an icon by name with a specific size, with caching.
///
/// The cache key includes the size to allow different resolutions.
pub fn named_icon_sized(icon_name: &str, size: i32) -> Option<skia::Image> {
    let cache_key = format!("{icon_name}@{size}");
    let ic = icon_cache();

    {
        let cache = ic.read().unwrap();
        if let Some(entry) = cache.get(&cache_key) {
            return entry.clone();
        }
    }

    let scale = crate::app_runner::context::AppContext::scale_factor().max(1);
    let icon = find_icon(icon_name, size, scale).and_then(|p| image_from_path(&p, (size, size)));

    ic.write().unwrap().insert(cache_key, icon.clone());
    icon
}

/// Load an icon from a file path with caching.
pub fn cached_file_icon(path: &str, size: i32) -> Option<skia::Image> {
    let cache_key = format!("file:{path}@{size}");
    let ic = icon_cache();

    {
        let cache = ic.read().unwrap();
        if let Some(entry) = cache.get(&cache_key) {
            return entry.clone();
        }
    }

    let icon = image_from_path(path, (size, size));

    ic.write().unwrap().insert(cache_key, icon.clone());
    icon
}

// ---------------------------------------------------------------------------
// Icon theme lookup
// ---------------------------------------------------------------------------

/// Find an icon file path using XDG icon theme directories.
///
/// Searches the compositor's configured icon theme (from the portal) first,
/// then falls back to auto-detection.
pub fn find_icon(icon_name: &str, size: i32, scale: i32) -> Option<String> {
    let theme = crate::icon_theme::current_icon_theme();
    let result = find_icon_in_theme(icon_name, size, scale, theme.as_deref());

    // xdgkit may return a fallback icon (e.g. application-default-icon) even
    // when the requested icon doesn't exist.  Reject results whose filename
    // doesn't match what we asked for.
    result.filter(|path| {
        std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|stem| stem.starts_with(icon_name))
            .unwrap_or(false)
    })
}

/// Find an icon in a specific theme (or auto-detect if `theme_name` is None).
pub fn find_icon_in_theme(
    icon_name: &str,
    size: i32,
    scale: i32,
    theme_name: Option<&str>,
) -> Option<String> {
    let dir_list = xdgkit::icon_finder::generate_dir_list();

    let result = if let Some(name) = theme_name {
        // Try specified theme first
        let theme_dir = dir_list.iter().find(|dir| dir.theme == name).cloned();

        if let Some(theme_dir) = theme_dir {
            let theme = xdgkit::icon_theme::IconTheme::from_pathbuff(theme_dir.index());
            xdgkit::icon_finder::multiple_find_icon(
                icon_name.to_string(),
                size,
                scale,
                dir_list.clone(),
                theme,
            )
            .map(|p| p.to_string_lossy().into_owned())
        } else {
            tracing::warn!("Icon theme '{name}' not found, falling back to auto-detection");
            xdgkit::icon_finder::find_icon(icon_name.to_string(), size, scale)
                .map(|p| p.to_string_lossy().into_owned())
        }
    } else {
        xdgkit::icon_finder::find_icon(icon_name.to_string(), size, scale)
            .map(|p| p.to_string_lossy().into_owned())
    };

    // Fallbacks
    result.or_else(|| {
        if icon_name != "application-default-icon" && icon_name != "application-x-executable" {
            find_icon("application-default-icon", size, scale)
                .or_else(|| find_icon("application-x-executable", size, scale))
        } else {
            None
        }
    })
}

// ---------------------------------------------------------------------------
// Image loading
// ---------------------------------------------------------------------------

/// Load an image from a file path, supporting both SVG and raster formats.
///
/// SVGs are rasterized at the given size using resvg. Raster images are loaded as-is.
pub fn image_from_path(path: &str, size: impl Into<skia::ISize>) -> Option<skia::Image> {
    let image_path = std::path::Path::new(path);

    if image_path.extension().and_then(std::ffi::OsStr::to_str) == Some("svg") {
        load_svg_image(path, size.into())
    } else {
        let image_data = std::fs::read(image_path).ok()?;
        skia::Image::from_encoded(skia::Data::new_copy(&image_data))
    }
}

/// Rasterize an SVG file at the given size using resvg.
fn load_svg_image(path: &str, size: skia::ISize) -> Option<skia::Image> {
    let svg_data = std::fs::read(path).ok()?;

    let pixmap_size = resvg::tiny_skia::IntSize::from_wh(size.width as u32, size.height as u32)?;

    let options = usvg::Options {
        languages: vec!["en".to_string()],
        dpi: 1.0,
        default_size: usvg::Size::from_wh(pixmap_size.width() as f32, pixmap_size.height() as f32)?,
        ..Default::default()
    };
    let rtree = usvg::Tree::from_data(&svg_data, &options).ok()?;
    let svg_size = rtree.size().to_int_size();

    let mut pixmap = resvg::tiny_skia::Pixmap::new(pixmap_size.width(), pixmap_size.height())?;
    let transform = resvg::tiny_skia::Transform::from_scale(
        pixmap_size.width() as f32 / svg_size.width() as f32,
        pixmap_size.height() as f32 / svg_size.height() as f32,
    );
    resvg::render(&rtree, transform, &mut pixmap.as_mut());

    let info = skia::ImageInfo::new(
        (pixmap_size.width() as i32, pixmap_size.height() as i32),
        skia::ColorType::RGBA8888,
        skia::AlphaType::Premul,
        None,
    );
    skia::images::raster_from_data(
        &info,
        skia::Data::new_copy(pixmap.data()),
        pixmap_size.width() as usize * 4,
    )
}
