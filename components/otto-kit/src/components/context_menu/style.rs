use std::hash::Hash;

use crate::theme::Theme;

/// Visual styling for ContextMenuNext
///
/// Contains all visual configuration - colors, dimensions, spacing.
/// No logic or state.
#[derive(Clone, Debug)]
pub struct ContextMenuStyle {
    // === Dimensions ===
    /// Menu width (None = auto-calculate from items)
    pub width: Option<f32>,

    /// Minimum menu width
    pub min_width: f32,

    /// Horizontal padding inside menu
    pub horizontal_padding: f32,

    /// Vertical padding inside menu
    pub vertical_padding: f32,

    // === Shapes ===
    /// Corner radius for rounded corners
    pub corner_radius: f32,

    /// Border width
    pub border_width: f32,

    // === Animation Delays ===
    /// Delay before showing submenu on mouse hover
    pub show_delay_mouse: f32,

    /// Delay before showing submenu on keyboard navigation
    pub show_delay_keyboard: f32,

    /// Delay/duration for menu close fade-out
    pub close_delay: f32,

    // === Scale ===
    /// Display scale factor (e.g. screen_scale * 0.8)
    /// Applied to all dimensions: sizes, padding, fonts.
    pub draw_scale: f32,

    // === Theme ===
    /// Theme for colors
    pub theme: Theme,
}
impl Hash for ContextMenuStyle {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.width.map(|w| w.to_bits()).hash(state);
        self.min_width.to_bits().hash(state);
        self.horizontal_padding.to_bits().hash(state);
        self.vertical_padding.to_bits().hash(state);
        self.corner_radius.to_bits().hash(state);
        self.border_width.to_bits().hash(state);
        self.show_delay_mouse.to_bits().hash(state);
        self.show_delay_keyboard.to_bits().hash(state);
        self.close_delay.to_bits().hash(state);
        self.draw_scale.to_bits().hash(state);
        // For theme, we can hash the relevant colors
        // self.theme.material_titlebar.hash(state);
        // self.theme.fill_secondary.hash(state);
    }
}

impl Default for ContextMenuStyle {
    fn default() -> Self {
        Self {
            width: None,
            min_width: 220.0,
            horizontal_padding: 5.0,
            vertical_padding: 5.0,
            corner_radius: 6.0,
            border_width: 1.0,
            show_delay_mouse: 0.2,
            show_delay_keyboard: 0.0, // Instant on keyboard
            close_delay: 0.15,
            draw_scale: 1.0,
            theme: Theme::light(),
        }
    }
}

impl ContextMenuStyle {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn default_with_scale(scale: f32) -> Self {
        Self {
            draw_scale: scale,
            ..Self::default()
        }
    }
    // === Builder API ===

    pub fn with_width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn with_min_width(mut self, min_width: f32) -> Self {
        self.min_width = min_width;
        self
    }

    pub fn with_corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius;
        self
    }

    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    pub fn with_padding(mut self, horizontal: f32, vertical: f32) -> Self {
        self.horizontal_padding = horizontal;
        self.vertical_padding = vertical;
        self
    }

    // === Animation Delay Builders ===

    pub fn with_show_delay_mouse(mut self, delay: f32) -> Self {
        self.show_delay_mouse = delay;
        self
    }

    pub fn with_show_delay_keyboard(mut self, delay: f32) -> Self {
        self.show_delay_keyboard = delay;
        self
    }

    pub fn with_close_delay(mut self, delay: f32) -> Self {
        self.close_delay = delay;
        self
    }

    pub fn with_draw_scale(mut self, scale: f32) -> Self {
        self.draw_scale = scale;
        self
    }

    // === Utility Methods ===

    /// Scale a logical pixel value by draw_scale
    pub fn scale(&self, value: f32) -> f32 {
        value * self.draw_scale
    }

    /// Get the background color from theme
    pub fn background_color(&self) -> skia_safe::Color {
        self.theme.material_titlebar
    }

    /// Get the border color from theme
    pub fn border_color(&self) -> skia_safe::Color {
        self.theme.fill_secondary
    }
}
