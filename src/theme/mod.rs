use layers::skia::{
    font_style::{Slant, Width},
    textlayout::TextStyle,
    FontStyle,
};
use layers::types::Color;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

use crate::config::Config;

pub fn text_style_with_size_and_weight(
    size: f32,
    weight: layers::skia::font_style::Weight,
) -> layers::skia::textlayout::TextStyle {
    let scale = Config::with(|c| c.screen_scale);
    let mut ts = TextStyle::new();
    ts.set_font_size(size * scale as f32);
    let fs = FontStyle::new(weight, Width::NORMAL, Slant::Upright);
    ts.set_font_style(fs);
    ts
}

macro_rules! define_text_styles {
    ({ $($name:ident => ($weight:expr, $size:expr)),* $(,)? }) => {
        use layers::skia::font_style::Weight;
        use layers::skia::textlayout::TextStyle;
        use crate::theme::text_style_with_size_and_weight;

        $(#[allow(dead_code)]
        pub fn $name() -> TextStyle {text_style_with_size_and_weight($size, $weight)})*
    };
}

/// The colours the compositor's own chrome paints with: otto-kit's palette,
/// converted to the scene graph's colour type.
///
/// otto-kit's [`otto_kit::theme::Theme`] is the only palette table. The
/// compositor draws next to otto-kit-drawn windows and menus, so a second
/// table here is how the dock and the window beside it ended up in two
/// different greys. Field names follow otto-kit's.
pub struct ThemeColors {
    pub accent_blue: Color,
    pub accent_red: Color,
    pub fill_primary: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_tertiary: Color,
    pub material_medium: Color,
    pub material_thin: Color,
    pub material_ultrathick: Color,
    pub material_tooltip: Color,
    pub shadow: Color,
    /// The line that closes the edge of a floating surface — the dock bar, a
    /// label balloon, a menu. otto-kit's `Theme::hairline`, so a window the
    /// compositor frames and the dock beside it draw the same line.
    pub hairline: Color,
    /// The palette itself, for what is looked up by name (the accents).
    kit: otto_kit::theme::Theme,
}

impl ThemeColors {
    fn from_kit(kit: otto_kit::theme::Theme) -> Self {
        Self {
            accent_blue: layers_color(kit.accent_blue),
            accent_red: layers_color(kit.accent_red),
            fill_primary: layers_color(kit.fill_primary),
            text_primary: layers_color(kit.text_primary),
            text_secondary: layers_color(kit.text_secondary),
            text_tertiary: layers_color(kit.text_tertiary),
            material_medium: layers_color(kit.material_medium),
            material_thin: layers_color(kit.material_thin),
            material_ultrathick: layers_color(kit.material_ultrathick),
            material_tooltip: layers_color(kit.material_tooltip),
            shadow: layers_color(kit.shadow),
            hairline: layers_color(kit.hairline),
            kit,
        }
    }

    /// The palette's colour for an accent name; see [`ACCENT_NAMES`].
    pub fn named_accent(&self, name: &str) -> Option<Color> {
        self.kit.named_accent(name).map(layers_color)
    }
}

/// An otto-kit (skia) colour as the scene graph's.
fn layers_color(c: otto_kit::skia::Color) -> Color {
    Color::new_rgba255(c.r(), c.g(), c.b(), c.a())
}

// The palettes as designed, not `Theme::light()`/`dark()`: those fold in the
// portal's accent and solid materials for clients without blur, and the
// compositor decides both itself (`accent_color`, `chrome_material`).
static LIGHT: LazyLock<ThemeColors> =
    LazyLock::new(|| ThemeColors::from_kit(otto_kit::theme::Theme::light_palette()));
static DARK: LazyLock<ThemeColors> =
    LazyLock::new(|| ThemeColors::from_kit(otto_kit::theme::Theme::dark_palette()));

pub mod text_styles;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ThemeScheme {
    Light,
    Dark,
}

/// The otto-kit palette matching the compositor's configured scheme.
///
/// Toolkit widgets the compositor draws itself — dock menus, the workspace
/// rename field — take their colors from a `Theme`, and otto-kit's own default
/// follows the XDG portal, which only client apps watch. Inside the compositor
/// the config is the source of truth, so hand it over explicitly.
pub fn kit_theme() -> otto_kit::theme::Theme {
    match Config::with(|c| c.theme_scheme.clone()) {
        ThemeScheme::Dark => otto_kit::theme::Theme::dark(),
        ThemeScheme::Light => otto_kit::theme::Theme::light(),
    }
}

pub fn theme_colors() -> &'static ThemeColors {
    Config::with(|c| match c.theme_scheme {
        ThemeScheme::Light => &LIGHT,
        ThemeScheme::Dark => &DARK,
    })
}

/// Every accent colour a user can choose, in the order they are offered.
///
/// The settings schema serves this list as the choices for `accent_color`, so
/// a name that is not here cannot be set. otto-kit owns both the list and the
/// colours, so `accent_by_name` and the schema cannot drift apart.
pub use otto_kit::theme::ACCENT_NAMES;

/// Resolve an accent name against the current scheme's palette.
pub fn accent_by_name(name: &str) -> Option<Color> {
    theme_colors().named_accent(name)
}

/// Read a `#RGB`, `#RRGGBB` or `#RRGGBBAA` literal; see
/// [`otto_kit::color::parse_hex`].
pub fn parse_hex(text: &str) -> Option<Color> {
    otto_kit::color::parse_hex(text).map(layers_color)
}

/// Resolve whatever the `accent_color` setting holds: a palette name, or a
/// hex literal for a colour the palette has no name for.
///
/// Names come first. They are the values the settings app offers and the ones
/// that follow the light and dark palettes, and none of them can be mistaken
/// for a literal anyway.
pub fn accent_from(text: &str) -> Option<Color> {
    accent_by_name(text).or_else(|| parse_hex(text))
}

/// The accent colour everything paints with.
///
/// The value lives in otto-kit's `accent` store rather than being resolved
/// here on every call. otto-kit draws Otto's window decorations and reads the
/// accent from that store, so keeping a second copy on this side is what left
/// the titlebar controls painting otto-kit's blue fallback while the workspace
/// selector painted the configured accent. One store, read by both.
///
/// Seeds the store on first read, so a caller that runs before
/// [`publish_accent`] still gets the configured colour rather than the
/// fallback.
pub fn accent_color() -> Color {
    match otto_kit::accent::current_accent() {
        Some(color) => layers_color(color),
        None => publish_accent(),
    }
}

/// Whether the desktop's own chrome is frosted — see `Config::frosting`.
/// Read through otto-kit's store so this process and the components agree.
pub fn frosting() -> bool {
    otto_kit::frosting::enabled()
}

/// The blend mode a piece of frosted chrome draws with: the backdrop blur
/// while frosting is on, plain compositing when it is off.
pub fn chrome_blend_mode() -> layers::types::BlendMode {
    if frosting() {
        layers::types::BlendMode::BackgroundBlur
    } else {
        layers::types::BlendMode::Normal
    }
}

/// The least opaque a piece of chrome gets without its frost: a hint of what
/// is behind it, no more — a see-through panel with nothing blurred behind
/// it reads as a glitch.
pub const UNFROSTED_MIN_ALPHA: f32 = 0.92;

/// `color` as chrome wears it: the translucent material while frosting is
/// on, the same hue taken up to at least [`UNFROSTED_MIN_ALPHA`] when it is
/// off.
pub fn chrome_material(color: Color) -> Color {
    material_at_least(color, UNFROSTED_MIN_ALPHA)
}

/// [`chrome_material`] for the dock bar: its floor is otto-kit's
/// [`otto_kit::frosting::BAR_UNFROSTED_MIN_ALPHA`], the one the top bar uses,
/// lower than the rest of the chrome's because only the wallpaper passes
/// under a bar.
pub fn bar_material(color: Color) -> Color {
    material_at_least(
        color,
        otto_kit::frosting::BAR_UNFROSTED_MIN_ALPHA as f32 / 255.0,
    )
}

fn material_at_least(color: Color, floor: f32) -> Color {
    if frosting() {
        color
    } else {
        Color {
            alpha: color.alpha.max(floor),
            ..color
        }
    }
}

/// Resolve the accent from the configuration and publish it to the store.
///
/// Call after anything that changes what the accent resolves to — the
/// `accent_color` setting, or the colour scheme whose palette it names.
pub fn publish_accent() -> Color {
    // The name is copied out before resolving it: `accent_by_name` reads the
    // configuration again for the palette, and `Config::with` is not
    // re-entrant.
    let name = Config::with(|c| c.accent_color.clone());
    let color = accent_from(&name).unwrap_or_else(|| theme_colors().accent_blue);
    otto_kit::accent::set_accent(color.c4f().to_color());
    color
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb255(color: Color) -> (u8, u8, u8) {
        let c = color.c4f();
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        (byte(c.r), byte(c.g), byte(c.b))
    }

    /// A name wins over the parser, and a colour the palette cannot name is
    /// still resolved.
    #[test]
    fn the_accent_resolves_from_a_name_or_a_literal() {
        assert_eq!(
            rgb255(accent_from("blue").unwrap()),
            rgb255(accent_by_name("blue").unwrap())
        );
        assert_eq!(rgb255(accent_from("#123456").unwrap()), (0x12, 0x34, 0x56));
        assert!(accent_from("chartreuse").is_none());
    }

    /// The accent survives the trip through otto-kit's store, so a caller on
    /// Otto's side and a draw routine on otto-kit's side see the same colour.
    #[test]
    fn accent_round_trips_through_the_shared_store() {
        for name in ACCENT_NAMES {
            let expected = accent_by_name(name).expect("named accent resolves");
            otto_kit::accent::set_accent(expected.c4f().to_color());

            let kit = otto_kit::accent::current_accent().expect("store holds the accent");
            assert_eq!(
                (kit.r(), kit.g(), kit.b()),
                rgb255(expected),
                "otto-kit sees a different `{name}` than Otto published"
            );
            assert_eq!(
                rgb255(accent_color()),
                rgb255(expected),
                "reading `{name}` back through the store changed it"
            );
        }
    }
}
