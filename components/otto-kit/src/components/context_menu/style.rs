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

    // === Typography ===
    /// Point size for item labels and shortcuts. `None` keeps the toolkit's
    /// menu default (13pt), which is what the bar's and the dock's menus use.
    /// A menu that is not a menu-bar menu — a pop-up button's, say — can ask
    /// for a larger one so its text matches the form it drops out of.
    pub item_font_size: Option<f32>,

    /// Height of a non-separator row. `None` keeps each item's own height.
    /// Set it alongside `item_font_size`: a larger font in a 22pt row reads
    /// as cramped rather than as bigger.
    pub item_height: Option<f32>,

    /// Tallest the menu may be drawn, in logical points. A list longer than
    /// this is not made shorter — the menu is capped at this height and the
    /// items scroll inside it. `None` lets the menu be as tall as its
    /// contents, which is right for the handful of rows a menu bar carries
    /// and wrong for a pop-up button listing every installed font.
    pub max_height: Option<f32>,

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
        self.item_font_size.map(|v| v.to_bits()).hash(state);
        self.item_height.map(|v| v.to_bits()).hash(state);
        self.max_height.map(|v| v.to_bits()).hash(state);
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
            corner_radius: 8.0,
            border_width: 1.0,
            show_delay_mouse: 0.2,
            show_delay_keyboard: 0.0, // Instant on keyboard
            close_delay: 0.15,
            item_font_size: None,
            item_height: None,
            max_height: None,
            draw_scale: 1.0,
            // Follow the system color scheme. A menu is a popup built on the
            // spot, so reading the watcher here is enough — by the time one
            // opens, the portal has long since answered.
            theme: crate::AppContext::current_theme(),
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

    /// Set the label point size and the row height together — see
    /// [`ContextMenuStyle::item_font_size`] for why they travel as a pair.
    /// Cap the menu's height, scrolling the items inside it beyond that.
    pub fn with_max_height(mut self, max_height: f32) -> Self {
        self.max_height = Some(max_height);
        self
    }

    pub fn with_item_metrics(mut self, font_size: f32, item_height: f32) -> Self {
        self.item_font_size = Some(font_size);
        self.item_height = Some(item_height);
        self
    }

    pub fn with_draw_scale(mut self, scale: f32) -> Self {
        self.draw_scale = scale;
        self
    }

    // === Utility Methods ===

    /// The item style this menu draws its rows with: the theme's own tones,
    /// plus whatever typography the menu asked for.
    pub fn item_style(&self) -> crate::components::menu_item::MenuItemStyle {
        let mut item_style = crate::components::menu_item::MenuItemStyle::from_theme(&self.theme);
        if let Some(size) = self.item_font_size {
            item_style.font_size = size;
            item_style.shortcut_font_size = size;
        }
        if let Some(height) = self.item_height {
            item_style.line_height = height;
        }
        item_style
    }

    /// How tall `item` is in this menu. Separators keep their own height —
    /// they are a rule, not a row, and stretching them with the text would
    /// only add space.
    pub fn item_height_of(&self, item: &crate::components::menu_item::MenuItem) -> f32 {
        match self.item_height {
            Some(height) if !item.is_separator() => height,
            _ => item.height,
        }
    }

    /// Scale a logical pixel value by draw_scale
    pub fn scale(&self, value: f32) -> f32 {
        value * self.draw_scale
    }

    /// Get the background color from theme
    pub fn background_color(&self) -> skia_safe::Color {
        self.theme.material_popup
    }

    /// Get the border color from theme — lighter than the background for definition
    pub fn border_color(&self) -> skia_safe::Color {
        self.theme.fill_primary
    }
}
