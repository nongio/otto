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

pub fn theme_colors() -> &'static Lazy<ThemeColors> {
    Config::with(|c| match c.theme_scheme {
        ThemeScheme::Light => &colors_light::COLORS,
        ThemeScheme::Dark => &colors_dark::COLORS,
    })
}

/// Get the configured accent color by name
pub fn accent_color() -> Color {
    let colors = theme_colors();
    Config::with(|c| {
        match c.accent_color.as_str() {
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
            _ => colors.accents_blue, // default fallback
        }
    })
}
