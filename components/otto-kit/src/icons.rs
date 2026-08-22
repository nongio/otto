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

type IconCache = Arc<RwLock<HashMap<String, Option<skia::Image>>>>;

static ICON_CACHE: OnceLock<IconCache> = OnceLock::new();

fn icon_cache() -> IconCache {
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
    let icon =
        find_icon_any_size(icon_name, size, scale).and_then(|p| image_from_path(&p, (size, size)));

    ic.write().unwrap().insert(cache_key, icon.clone());
    icon
}

/// The directory sizes an XDG icon theme actually ships.
const STANDARD_ICON_SIZES: [i32; 10] = [16, 22, 24, 32, 48, 64, 96, 128, 256, 512];

/// Find `icon_name` for a request of `size`, falling back to the sizes a theme
/// is likely to have on disk.
///
/// A theme directory only answers for the sizes it declares, so an in-between
/// request misses icons that plainly exist: hicolor ships ghostty at 16 and 32,
/// and a request for 20 resolves to nothing at all. Callers scale the result
/// into their own rect anyway, so a neighbouring size is always better than no
/// icon. Larger neighbours come first — downscaling beats upscaling.
fn find_icon_any_size(icon_name: &str, size: i32, scale: i32) -> Option<String> {
    if let Some(path) = find_icon(icon_name, size, scale) {
        return Some(path);
    }
    let mut candidates: Vec<i32> = STANDARD_ICON_SIZES
        .into_iter()
        .filter(|&s| s != size)
        .collect();
    candidates.sort_by_key(|&s| (s < size, (s - size).abs()));
    candidates
        .into_iter()
        .find_map(|s| find_icon(icon_name, s, scale))
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

/// Look up the first icon in `names` that the theme actually has, at `size`.
///
/// Callers pass a most-specific-first chain — `["image-png",
/// "image-x-generic"]` — so a theme shipping only generic icons still
/// resolves. The whole chain is cached under one key, including the miss, so a
/// directory listing does not re-walk the theme for every row.
///
/// Unlike [`named_icon_sized`] this never reads `AppContext`, so it is callable
/// from a bare draw closure — which is what lets the compositor draw the same
/// icons server-side.
pub fn cached_icon_chain(names: &[&str], size: i32) -> Option<skia::Image> {
    cached_icon_chain_at(names, size, size)
}

/// The size at which a theme's icons are its full, colourful artwork.
///
/// Themes commonly ship monochrome outline art in their small fixed
/// directories (Fluent's `16/places/folder.svg` is a grey glyph) and the real
/// icon only in `scalable`, whose `MinSize` starts above those. Asking for
/// anything from this size up lands on the scalable tier.
pub const FULL_COLOUR_SIZE: i32 = 64;

/// [`cached_icon_chain`], with the theme lookup size separated from the size
/// the icon is rasterised at.
///
/// A 16px row still wants the colourful icon, just small — so it looks the
/// chain up at [`FULL_COLOUR_SIZE`] and renders that file at 16. Passing the
/// same value for both is the plain [`cached_icon_chain`] behaviour, which is
/// what a caller wants when it does want the theme's small-size art.
pub fn cached_icon_chain_at(names: &[&str], size: i32, lookup_size: i32) -> Option<skia::Image> {
    if names.is_empty() {
        return None;
    }
    let cache_key = format!("chain:{}@{size}/{lookup_size}", names.join("|"));
    let ic = icon_cache();

    {
        let cache = ic.read().unwrap();
        if let Some(entry) = cache.get(&cache_key) {
            return entry.clone();
        }
    }

    // Deliberately not `find_icon_in_theme`: that one substitutes a generic
    // icon when a name misses, so the first entry in a chain would always
    // "succeed" and the rest would never be consulted.
    let theme = crate::icon_theme::current_icon_theme();
    let icon = names
        .iter()
        .find_map(|name| exact_icon_in_theme(name, lookup_size, theme.as_deref()))
        .and_then(|path| image_from_path(&path, (size, size)));

    ic.write().unwrap().insert(cache_key, icon.clone());
    icon
}

// ---------------------------------------------------------------------------
// Icon theme lookup
// ---------------------------------------------------------------------------

/// Find an icon file path in the XDG icon theme directories.
///
/// Searches the desktop's configured icon theme first (from the portal), then
/// hicolor, then the other standard locations.
///
/// An `icon_name` that is already an absolute path — some desktop entries, and
/// most Waydroid ones, write one — is used as it stands.
pub fn find_icon(icon_name: &str, size: i32, scale: i32) -> Option<String> {
    if icon_name.starts_with('/') {
        return std::path::Path::new(icon_name)
            .is_file()
            .then(|| icon_name.to_string());
    }
    let theme = crate::icon_theme::current_icon_theme();
    find_icon_in_theme(icon_name, size, scale, theme.as_deref())
}

/// Look up exactly this icon name, with no generic substitution on a miss.
///
/// [`find_icon_in_theme`] deliberately falls back to a generic icon so a
/// single lookup never draws an empty square. That behaviour is wrong when the
/// caller has its own fallback chain, so this is the honest form: a miss is a
/// `None`.
pub fn exact_icon_in_theme(icon_name: &str, size: i32, theme_name: Option<&str>) -> Option<String> {
    if icon_name.starts_with('/') {
        return std::path::Path::new(icon_name)
            .is_file()
            .then(|| icon_name.to_string());
    }
    let mut lookup = freedesktop_icons::lookup(icon_name)
        .with_size(size.clamp(1, i32::from(u16::MAX)) as u16)
        .with_scale(1);
    if let Some(theme) = theme_name {
        lookup = lookup.with_theme(theme);
    }
    lookup
        .with_cache()
        .find()
        .map(|path| path.to_string_lossy().into_owned())
}

/// Find an icon in a specific theme (or the default one if `theme_name` is
/// `None`).
///
/// Backed by `freedesktop-icons`, which builds the theme index once and keeps
/// it. The obvious alternative, xdgkit, re-walks every icon directory on the
/// system for every single lookup — a third of a second each, and twice that
/// for a miss, which is unusable anywhere a screenful of icons is wanted at
/// once. It also missed icons that are plainly there.
pub fn find_icon_in_theme(
    icon_name: &str,
    size: i32,
    scale: i32,
    theme_name: Option<&str>,
) -> Option<String> {
    let lookup = |name: &str| {
        let mut lookup = freedesktop_icons::lookup(name)
            .with_size(size.clamp(1, i32::from(u16::MAX)) as u16)
            .with_scale(scale.clamp(1, i32::from(u16::MAX)) as u16);
        if let Some(theme) = theme_name {
            lookup = lookup.with_theme(theme);
        }
        lookup
            .with_cache()
            .find()
            .map(|path| path.to_string_lossy().into_owned())
    };

    lookup(icon_name).or_else(|| {
        // A generic icon is better than an empty square — but only for names
        // that are not already the generic ones, or a miss would recurse.
        if icon_name != "application-default-icon" && icon_name != "application-x-executable" {
            lookup("application-default-icon").or_else(|| lookup("application-x-executable"))
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
        let svg_data = std::fs::read(image_path).ok()?;
        svg_image_from_bytes(&svg_data, size.into())
    } else {
        let image_data = std::fs::read(image_path).ok()?;
        skia::Image::from_encoded(skia::Data::new_copy(&image_data))
    }
}

/// Rasterize SVG bytes at the given size using resvg.
///
/// Public so callers can rasterize icons embedded with `include_bytes!` —
/// bundling a fixed, small icon set at compile time sidesteps the runtime
/// resource-path and icon-theme lookups entirely, which matters for icons
/// that must always render (e.g. OSD glyphs) regardless of the compositor's
/// working directory or the desktop's installed icon theme.
pub fn svg_image_from_bytes(svg_data: &[u8], size: skia::ISize) -> Option<skia::Image> {
    let pixmap_size = resvg::tiny_skia::IntSize::from_wh(size.width as u32, size.height as u32)?;

    let options = usvg::Options {
        languages: vec!["en".to_string()],
        dpi: 1.0,
        default_size: usvg::Size::from_wh(pixmap_size.width() as f32, pixmap_size.height() as f32)?,
        ..Default::default()
    };
    let rtree = usvg::Tree::from_data(svg_data, &options).ok()?;
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

#[cfg(test)]
mod tests {
    /// The distinction `cached_icon_chain` depends on: an exact lookup must
    /// report a miss, where `find_icon_in_theme` deliberately substitutes a
    /// generic icon. Without this, the first name in a fallback chain always
    /// "succeeds" and the rest are never consulted.
    #[test]
    fn an_exact_lookup_reports_a_miss() {
        let missing = "otto-kit-no-such-icon-9f3a";
        assert_eq!(
            super::exact_icon_in_theme(missing, 16, None),
            None,
            "exact lookup must not substitute"
        );
    }

    /// A chain falls through to the entry the theme actually has. Skipped where
    /// no icon theme is installed, since that is an environment fact rather
    /// than a defect.
    #[test]
    fn a_chain_falls_through_to_what_exists() {
        let theme = crate::icon_theme::current_icon_theme();
        let Some(known) = ["folder", "text-x-generic", "application-x-executable"]
            .into_iter()
            .find(|n| super::exact_icon_in_theme(n, 16, theme.as_deref()).is_some())
        else {
            return;
        };
        assert!(
            super::cached_icon_chain(&["otto-kit-no-such-icon-9f3a", known], 16).is_some(),
            "the second entry in the chain should have resolved"
        );
    }
}
