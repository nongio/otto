use skia_safe::Color;

/// System color scheme preference, matching XDG `org.freedesktop.appearance color-scheme`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorScheme {
    #[default]
    NoPreference,
    Dark,
    Light,
}

impl ColorScheme {
    /// Construct from the XDG portal integer value.
    pub fn from_portal_value(v: u32) -> Self {
        match v {
            1 => Self::Dark,
            2 => Self::Light,
            _ => Self::NoPreference,
        }
    }
}

/// Application color theme based on Otto's design system
#[derive(Debug, Clone)]
pub struct Theme {
    // Accent colors
    pub accent_blue: Color,
    pub accent_gray: Color,

    // Fill colors (backgrounds)
    pub fill_primary: Color,
    pub fill_secondary: Color,
    pub fill_tertiary: Color,
    pub fill_quaternary: Color,

    // Text colors
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_tertiary: Color,

    // Material colors (surfaces)
    pub material_titlebar: Color,
    pub material_sidebar: Color,
    pub material_selection_focused: Color,

    // Shadow
    pub shadow: Color,
}

impl Theme {
    /// Light theme
    pub fn light() -> Self {
        Self {
            accent_blue: Color::from_argb(0xFF, 0x0A, 0x84, 0xFF),
            accent_gray: Color::from_argb(0xFF, 0x8E, 0x8E, 0x93),

            fill_primary: Color::from_argb(0x35, 0x00, 0x00, 0x00),
            fill_secondary: Color::from_argb(0x14, 0x00, 0x00, 0x00),
            fill_tertiary: Color::from_argb(0x0D, 0x00, 0x00, 0x00),
            fill_quaternary: Color::from_argb(0x08, 0x00, 0x00, 0x00),

            text_primary: Color::from_argb(0xD9, 0x00, 0x00, 0x00),
            text_secondary: Color::from_argb(0x80, 0x00, 0x00, 0x00),
            text_tertiary: Color::from_argb(0x40, 0x00, 0x00, 0x00),

            material_titlebar: Color::from_argb(0xCC, 0xEA, 0xEA, 0xEA),
            material_sidebar: Color::from_argb(0xAF, 0xEA, 0xEA, 0xEA),
            material_selection_focused: Color::from_argb(0xBF, 0x0A, 0x82, 0xFF),

            shadow: Color::from_argb(0x66, 0x1B, 0x1B, 0x1B),
        }
    }

    /// Dark theme
    pub fn dark() -> Self {
        Self {
            accent_blue: Color::from_argb(0xFF, 0x0A, 0x84, 0xFF),
            accent_gray: Color::from_argb(0xFF, 0x8E, 0x8E, 0x93),

            // Semi-transparent whites for layering on dark backgrounds
            fill_primary: Color::from_argb(0x40, 0xFF, 0xFF, 0xFF),
            fill_secondary: Color::from_argb(0x1A, 0xFF, 0xFF, 0xFF),
            fill_tertiary: Color::from_argb(0x0F, 0xFF, 0xFF, 0xFF),
            fill_quaternary: Color::from_argb(0x08, 0xFF, 0xFF, 0xFF),

            text_primary: Color::from_argb(0xF2, 0xFF, 0xFF, 0xFF),
            text_secondary: Color::from_argb(0x80, 0xFF, 0xFF, 0xFF),
            text_tertiary: Color::from_argb(0x40, 0xFF, 0xFF, 0xFF),

            // Dark translucent surfaces
            material_titlebar: Color::from_argb(0xBF, 0x28, 0x28, 0x28),
            material_sidebar: Color::from_argb(0xA8, 0x1E, 0x1E, 0x1E),
            material_selection_focused: Color::from_argb(0xBF, 0x0A, 0x82, 0xFF),

            shadow: Color::from_argb(0x99, 0x00, 0x00, 0x00),
        }
    }

    /// Return the appropriate theme for the given color scheme.
    /// Falls back to light for `NoPreference`.
    pub fn for_scheme(scheme: ColorScheme) -> Self {
        match scheme {
            ColorScheme::Dark => Self::dark(),
            _ => Self::light(),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::light()
    }
}
