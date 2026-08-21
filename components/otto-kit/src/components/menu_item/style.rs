use skia_safe::Color;

/// Visual styling for MenuItem
#[derive(Clone, Debug)]
pub struct MenuItemStyle {
    // === Dimensions ===
    pub horizontal_padding: f32,
    pub line_height: f32,
    pub separator_height: f32,
    pub border_radius: f32,

    // === Colors ===
    pub text_color_normal: Color,
    pub text_color_hovered: Color,
    pub text_color_disabled: Color,
    pub shortcut_color_normal: Color,
    pub shortcut_color_hovered: Color,
    pub bg_color_hovered: Color,
    pub separator_color: Color,

    // === Typography ===
    pub font_size: f32,
    pub shortcut_font_size: f32,
}

impl Default for MenuItemStyle {
    fn default() -> Self {
        Self {
            horizontal_padding: 10.0,
            line_height: 22.0,
            separator_height: 9.0,
            border_radius: 5.0,

            text_color_normal: Color::from_argb(217, 0, 0, 0), // 85% black
            text_color_hovered: Color::WHITE,
            text_color_disabled: Color::from_argb(64, 0, 0, 0), // 25% black
            shortcut_color_normal: Color::from_argb(64, 0, 0, 0),
            shortcut_color_hovered: Color::WHITE,
            bg_color_hovered: Color::from_argb(191, 10, 130, 255),
            separator_color: Color::from_argb(26, 0, 0, 0), // 10% black

            font_size: 13.0,
            shortcut_font_size: 13.0,
        }
    }
}

impl MenuItemStyle {
    /// The font labels are drawn with: the toolkit's menu face at this
    /// style's own point size, so a menu that asked for larger text gets it
    /// everywhere it is measured as well as everywhere it is drawn.
    pub fn font(&self) -> skia_safe::Font {
        crate::typography::TextStyle {
            size: self.font_size,
            ..crate::typography::styles::BODY_MEDIUM
        }
        .font()
    }

    /// The font shortcuts are drawn with.
    pub fn shortcut_font(&self) -> skia_safe::Font {
        crate::typography::TextStyle {
            size: self.shortcut_font_size,
            ..crate::typography::styles::BODY_MEDIUM
        }
        .font()
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// A style using the theme's own text and fill tones, so menu items stay
    /// legible in dark mode — `default()` is fixed to a light appearance and
    /// draws near-invisible dark text on a dark menu background.
    pub fn from_theme(theme: &crate::theme::Theme) -> Self {
        Self {
            text_color_normal: theme.text_primary,
            text_color_disabled: theme.text_tertiary,
            shortcut_color_normal: theme.text_tertiary,
            separator_color: theme.fill_secondary,
            ..Self::default()
        }
    }

    // === Builder API ===

    pub fn with_padding(mut self, padding: f32) -> Self {
        self.horizontal_padding = padding;
        self
    }

    pub fn with_line_height(mut self, height: f32) -> Self {
        self.line_height = height;
        self
    }

    pub fn with_hover_color(mut self, color: Color) -> Self {
        self.bg_color_hovered = color;
        self
    }

    pub fn with_text_colors(mut self, normal: Color, hovered: Color, disabled: Color) -> Self {
        self.text_color_normal = normal;
        self.text_color_hovered = hovered;
        self.text_color_disabled = disabled;
        self
    }
}
