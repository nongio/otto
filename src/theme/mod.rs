use layers::skia::{
    font_style::{Slant, Width},
    textlayout::TextStyle,
    FontStyle,
};
use layers::types::Color;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::config::Config;

// Macro to define a Lazy group of colors
macro_rules! define_colors {
    ($init_name:ident, { $($name:ident => $hex:expr),* $(,)? }) => {
        use layers::types::Color;
        use once_cell::sync::Lazy;
        use crate::theme::ThemeColors;
        // Lazy static initialization of the group
        pub static $init_name: Lazy<ThemeColors> = Lazy::new(|| ThemeColors {
            $($name: Color::new_hex($hex)),*
        });
    };
}

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

        paste::paste! {
        $(#[allow(dead_code)]
        pub fn [<$name>]() -> TextStyle {text_style_with_size_and_weight($size, $weight)})*
        }
    };
}
#[allow(unused)]
pub struct ThemeColors {
    pub accents_red: Color,
    pub accents_orange: Color,
    pub accents_yellow: Color,
    pub accents_green: Color,
    pub accents_mint: Color,
    pub accents_teal: Color,
    pub accents_cyan: Color,
    pub accents_blue: Color,
    pub accents_indigo: Color,
    pub accents_purple: Color,
    pub accents_pink: Color,
    pub accents_gray: Color,
    pub accents_brown: Color,
    pub accents_vibrant_red: Color,
    pub accents_vibrant_orange: Color,
    pub accents_vibrant_yellow: Color,
    pub accents_vibrant_green: Color,
    pub accents_vibrant_mint: Color,
    pub accents_vibrant_teal: Color,
    pub accents_vibrant_cyan: Color,
    pub accents_vibrant_blue: Color,
    pub accents_vibrant_indigo: Color,
    pub accents_vibrant_purple: Color,
    pub accents_vibrant_pink: Color,
    pub accents_vibrant_brown: Color,
    pub accents_vibrant_gray: Color,
    pub fills_primary: Color,
    pub fills_secondary: Color,
    pub fills_tertiary: Color,
    pub fills_quaternary: Color,
    pub fills_quinary: Color,
    pub fills_vibrant_primary: Color,
    pub fills_vibrant_secondary: Color,
    pub fills_vibrant_tertiary: Color,
    pub fills_vibrant_quaternary: Color,
    pub fills_vibrant_quinary: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_tertiary: Color,
    pub text_quaternary: Color,
    pub text_quinary: Color,
    pub text_vibrant_primary: Color,
    pub text_vibrant_secondary: Color,
    pub text_vibrant_tertiary: Color,
    pub text_vibrant_quaternary: Color,
    pub text_vibrant_quinary: Color,
    pub materials_ultrathick: Color,
    pub materials_thick: Color,
    pub materials_medium: Color,
    pub materials_thin: Color,
    pub materials_ultrathin: Color,
    pub materials_highlight: Color,
    pub materials_controls_menu: Color,
    pub materials_controls_popover: Color,
    pub materials_controls_title_bar: Color,
    pub materials_controls_sidebar: Color,
    pub materials_controls_selection_focused: Color,
    pub materials_controls_selection_unfocused: Color,
    pub materials_controls_header_view: Color,
    pub materials_controls_tooltip: Color,
    pub materials_controls_under_window_background: Color,
    pub materials_controls_fullscreen: Color,
    pub materials_controls_hud: Color,
    pub shadow_color: Color,
}

mod colors_dark;
mod colors_light;
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

pub fn theme_colors() -> &'static Lazy<ThemeColors> {
    Config::with(|c| match c.theme_scheme {
        ThemeScheme::Light => &colors_light::COLORS,
        ThemeScheme::Dark => &colors_dark::COLORS,
    })
}

/// Every accent colour a user can choose, in the order they are offered.
///
/// The settings schema serves this list as the choices for `accent_color`, so
/// a name that is not here cannot be set — `accent_by_name` and the schema
/// cannot drift apart.
pub const ACCENT_NAMES: &[&str] = &[
    "blue", "purple", "pink", "red", "orange", "yellow", "green", "mint", "teal", "cyan", "indigo",
    "brown", "gray",
];

/// Resolve an accent name against the current scheme's palette.
pub fn accent_by_name(name: &str) -> Option<Color> {
    let colors = theme_colors();
    Some(match name {
        "red" => colors.accents_red,
        "orange" => colors.accents_orange,
        "yellow" => colors.accents_yellow,
        "green" => colors.accents_green,
        "mint" => colors.accents_mint,
        "teal" => colors.accents_teal,
        "cyan" => colors.accents_cyan,
        "blue" => colors.accents_blue,
        "indigo" => colors.accents_indigo,
        "purple" => colors.accents_purple,
        "pink" => colors.accents_pink,
        "gray" => colors.accents_gray,
        "brown" => colors.accents_brown,
        _ => return None,
    })
}

/// Read a `#RGB` or `#RRGGBB` literal.
///
/// The leading `#` is required: it is what tells a colour apart from a palette
/// name, and without it `abc` would be a valid colour as well as an unknown
/// name. Three digits expand the way CSS expands them, each digit doubled, so
/// `#f0a` and `#ff00aa` are the same colour.
pub fn parse_hex(text: &str) -> Option<Color> {
    let digits = text.trim().strip_prefix('#')?;
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |s: &str| u8::from_str_radix(s, 16).ok();
    let (r, g, b) = match digits.len() {
        3 => {
            let d = |i: usize| byte(&digits[i..i + 1]).map(|v| v * 17);
            (d(0)?, d(1)?, d(2)?)
        }
        6 => (
            byte(&digits[0..2])?,
            byte(&digits[2..4])?,
            byte(&digits[4..6])?,
        ),
        _ => return None,
    };
    Some(Color::new_rgba255(r, g, b, 255))
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
        Some(color) => Color::new_rgba255(color.r(), color.g(), color.b(), color.a()),
        None => publish_accent(),
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
    let color = accent_from(&name).unwrap_or_else(|| theme_colors().accents_blue);
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

    /// Three digits expand the way CSS expands them, so the short form of a
    /// colour is the same colour.
    #[test]
    fn short_hex_expands_by_doubling_each_digit() {
        assert_eq!(rgb255(parse_hex("#f0a").unwrap()), (0xFF, 0x00, 0xAA));
        assert_eq!(parse_hex("#f0a"), parse_hex("#ff00aa"));
        assert_eq!(rgb255(parse_hex("#FF00AA").unwrap()), (0xFF, 0x00, 0xAA));
    }

    /// The `#` is what tells a colour apart from a palette name; without it a
    /// name of six hex letters would parse as a colour.
    #[test]
    fn only_a_hash_prefixed_literal_parses() {
        for text in ["", "#", "ff00aa", "#ff00a", "#ff00aaa", "#gggggg", "blue"] {
            assert!(parse_hex(text).is_none(), "`{text}` parsed as a colour");
        }
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
