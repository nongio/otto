use skia_safe::Color;

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
    /// Light theme (based on Otto's light theme)
    pub fn light() -> Self {
        Self {
            // Accents
            accent_blue: Color::from_argb(0xFF, 0x0A, 0x84, 0xFF), // #0A84FF
            accent_gray: Color::from_argb(0xFF, 0x8E, 0x8E, 0x93), // #8E8E93

            // Fills (semi-transparent blacks for layering)
            fill_primary: Color::from_argb(0x35, 0x00, 0x00, 0x00), // #00000035
            fill_secondary: Color::from_argb(0x14, 0x00, 0x00, 0x00), // #00000014
            fill_tertiary: Color::from_argb(0x0D, 0x00, 0x00, 0x00), // #0000000D
            fill_quaternary: Color::from_argb(0x08, 0x00, 0x00, 0x00), // #00000008

            // Text
            text_primary: Color::from_argb(0xD9, 0x00, 0x00, 0x00), // #000000D9
            text_secondary: Color::from_argb(0x80, 0x00, 0x00, 0x00), // #00000080
            text_tertiary: Color::from_argb(0x40, 0x00, 0x00, 0x00), // #00000040

            // Materials
            material_titlebar: Color::from_argb(0xCC, 0xEA, 0xEA, 0xEA), // #EAEAEACC
            material_sidebar: Color::from_argb(0xAF, 0xEA, 0xEA, 0xEA),  // #eaeaeaaf
            material_selection_focused: Color::from_argb(0xBF, 0x0A, 0x82, 0xFF), // #0A82FFBF

            // Shadow
            shadow: Color::from_argb(0x66, 0x1B, 0x1B, 0x1B), // #1b1b1b66
        }
    }

    /// Dark theme (based on Otto's dark theme)
    pub fn dark() -> Self {
        // For now, return light theme - can be expanded later
        Self::light()
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::light()
    }
}
