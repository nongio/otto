//! The Solar glyphs Otto's own interface draws with.
//!
//! Toolbar, menu and control glyphs come from a vendored subset of the Solar
//! icon set (`resources/icons/solar`, installed to
//! `/usr/share/otto/icons/solar`) rather than from the person's icon theme,
//! so they look the same whatever theme is in force. App icons, file-type
//! icons and tray icons still come from the theme.
//!
//! Each glyph is a 24×24 monochrome SVG drawn in `currentColor`. It is
//! rasterised with that colour set to the one the caller draws with, so the
//! image comes out already tinted; callers that tint with `SrcIn` afterwards,
//! as they do for a theme's symbolic art, get the same result.

// Rust guideline compliant 2026-02-21

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use skia_safe as skia;

/// The weight a Solar glyph is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SolarStyle {
    /// Outlined strokes: a control at rest.
    #[default]
    Linear,
    /// Filled shapes: a control that is pressed or switched on.
    Bold,
}

impl SolarStyle {
    /// The set's directory for this style.
    fn directory(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Bold => "bold",
        }
    }
}

/// Names in the set that are also file-type icons.
///
/// `folder` is the sidebar's generic place and a listing's directory alike.
/// The generic lookups leave it to the theme, so a listing keeps the theme's
/// full-colour folder; [`super::cached_glyph_chain`] and [`solar_icon`] still
/// draw it as a glyph where the caller asks for one.
const TYPE_ICON_NAMES: &[&str] = &["folder"];

/// The colour a glyph is drawn in when the caller tints it afterwards.
///
/// Opaque black, the ink a theme's symbolic art arrives in; `SrcIn` keeps
/// only its coverage.
pub(super) const INK: skia::Color = skia::Color::BLACK;

/// The folder the Solar set lives in, found once per process.
///
/// `$OTTO_SOLAR_ICONS` first, then `share/otto/icons/solar` beside the
/// running binary's prefix, then the system install, then the repository
/// itself so a development build finds the set without installing it.
fn solar_dir() -> Option<&'static Path> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let from_env = std::env::var_os("OTTO_SOLAR_ICONS").map(PathBuf::from);
        let beside_exe = std::env::current_exe().ok().and_then(|exe| {
            Some(
                exe.parent()?
                    .parent()?
                    .join("share")
                    .join("otto")
                    .join("icons")
                    .join("solar"),
            )
        });
        let system = Some(PathBuf::from("/usr/share/otto/icons/solar"));
        let repository = Some(PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../resources/icons/solar"
        )));
        [from_env, beside_exe, system, repository]
            .into_iter()
            .flatten()
            .find(|dir| dir.join(SolarStyle::Linear.directory()).is_dir())
    })
    .as_deref()
}

/// The file for `name` in `style`, if the set has it.
///
/// A glyph the set keeps only in linear answers a bold request with its
/// linear file: a pressed control still shows its glyph.
pub fn solar_icon_path(name: &str, style: SolarStyle) -> Option<PathBuf> {
    // A name with a path in it is not a glyph name.
    if name.is_empty() || name.contains('/') {
        return None;
    }
    let dir = solar_dir()?;
    let file = format!("{name}.svg");
    [style, SolarStyle::Linear]
        .into_iter()
        .map(|style| dir.join(style.directory()).join(&file))
        .find(|path| path.is_file())
}

/// Whether the generic named-icon lookups draw `name` from the set.
pub(super) fn is_glyph_name(name: &str) -> bool {
    !TYPE_ICON_NAMES.contains(&name) && solar_icon_path(name, SolarStyle::Linear).is_some()
}

/// `svg` with its `currentColor` resolved to `color`.
///
/// The colour goes on the root element, where `currentColor` inherits it
/// from, and its alpha becomes the root's group opacity so overlapping
/// strokes do not darken where they cross.
fn with_current_color(svg: &str, color: skia::Color) -> Option<String> {
    let at = svg.find("<svg")? + "<svg".len();
    let opacity = f32::from(color.a()) / 255.0;
    let attributes = format!(
        " color=\"#{:02x}{:02x}{:02x}\" opacity=\"{opacity}\"",
        color.r(),
        color.g(),
        color.b()
    );
    let mut tinted = String::with_capacity(svg.len() + attributes.len());
    tinted.push_str(&svg[..at]);
    tinted.push_str(&attributes);
    tinted.push_str(&svg[at..]);
    Some(tinted)
}

/// `color` as `aarrggbb`, for a cache key.
pub(super) fn color_key(color: skia::Color) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}",
        color.a(),
        color.r(),
        color.g(),
        color.b()
    )
}

/// Rasterise the Solar glyph `name` at `size` pixels, drawn in `color`.
///
/// Cached by name, size, style and colour. `None` when the set is not
/// installed or does not have the glyph, so the caller can fall back to the
/// theme or to drawing its own.
pub fn solar_icon(
    name: &str,
    size: i32,
    style: SolarStyle,
    color: skia::Color,
) -> Option<skia::Image> {
    if size <= 0 {
        return None;
    }
    let cache_key = format!(
        "solar:{}/{name}@{size}#{}",
        style.directory(),
        color_key(color)
    );
    let cache = super::icon_cache();
    if let Some(entry) = cache.read().unwrap().get(&cache_key) {
        return entry.clone();
    }

    let icon = solar_icon_path(name, style)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|svg| with_current_color(&svg, color))
        .and_then(|svg| super::svg_image_from_bytes(svg.as_bytes(), (size, size).into()));

    cache.write().unwrap().insert(cache_key, icon.clone());
    icon
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The most opaque pixel of `image`, unpremultiplied.
    fn darkest_pixel(image: &skia::Image) -> (u8, u8, u8, u8) {
        let info = skia::ImageInfo::new(
            image.dimensions(),
            skia::ColorType::RGBA8888,
            skia::AlphaType::Unpremul,
            None,
        );
        let row = image.width() as usize * 4;
        let mut pixels = vec![0u8; row * image.height() as usize];
        assert!(image.read_pixels(
            &info,
            &mut pixels,
            row,
            (0, 0),
            skia::image::CachingHint::Allow
        ));
        pixels
            .chunks_exact(4)
            .map(|p| (p[0], p[1], p[2], p[3]))
            .max_by_key(|p| p.3)
            .expect("an image has pixels")
    }

    /// Channels equal up to the rounding of a premultiplied round trip.
    fn assert_near(got: (u8, u8, u8), want: (u8, u8, u8)) {
        let close = |a: u8, b: u8| a.abs_diff(b) <= 2;
        assert!(
            close(got.0, want.0) && close(got.1, want.1) && close(got.2, want.2),
            "{got:?} is not {want:?}"
        );
    }

    #[test]
    fn a_solar_name_resolves_in_both_styles() {
        let linear = solar_icon_path("zoom-in-symbolic", SolarStyle::Linear)
            .expect("the repository's set is found in a development build");
        assert!(linear.ends_with("linear/zoom-in-symbolic.svg"));
        let bold = solar_icon_path("otto-pen-symbolic", SolarStyle::Bold).unwrap();
        assert!(bold.ends_with("bold/otto-pen-symbolic.svg"));
        // Kept only in linear: a bold request still finds the glyph.
        let delete = solar_icon_path("otto-delete-symbolic", SolarStyle::Bold).unwrap();
        assert!(delete.ends_with("linear/otto-delete-symbolic.svg"));
        assert!(solar_icon_path("otto-kit-no-such-glyph", SolarStyle::Linear).is_none());
    }

    #[test]
    fn current_color_takes_the_requested_colour() {
        let red = skia::Color::from_argb(255, 220, 30, 40);
        let image = solar_icon("otto-pen-symbolic", 32, SolarStyle::Linear, red)
            .expect("the pen glyph rasterises");
        assert_eq!(image.dimensions(), skia::ISize::new(32, 32));
        let (r, g, b, a) = darkest_pixel(&image);
        assert_eq!(a, 255, "the stroke is opaque somewhere");
        assert_near((r, g, b), (220, 30, 40));

        let faint = skia::Color::from_argb(128, 255, 255, 255);
        let image = solar_icon("otto-pen-symbolic", 32, SolarStyle::Bold, faint).unwrap();
        let (r, g, b, a) = darkest_pixel(&image);
        assert_near((r, g, b), (255, 255, 255));
        assert!((127..=129).contains(&a), "alpha is the colour's: {a}");
    }

    #[test]
    fn current_color_goes_on_the_root_element() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><path stroke="currentColor"/></svg>"#;
        let tinted = with_current_color(svg, skia::Color::from_rgb(1, 2, 255)).unwrap();
        assert!(tinted.starts_with(r##"<svg color="#0102ff" opacity="1" xmlns="##));
    }

    #[test]
    fn folder_stays_with_the_theme_in_generic_lookups() {
        assert!(!is_glyph_name("folder"));
        assert!(is_glyph_name("user-home"));
        assert!(solar_icon_path("folder", SolarStyle::Linear).is_some());
    }
}
