use std::hash::Hash;

use skia_safe::Color;

use crate::components::label::TextAlign;
use crate::theme::Theme;
use crate::typography::{styles, TextStyle};

/// Visual styling for a [`TextInput`](super::TextInput). Colors and dimensions
/// only — no state, no logic.
///
/// Dimensions are in unscaled points; `draw_scale` multiplies all of them (and
/// the font size) at draw time, the same way [`ContextMenuStyle`] works.
///
/// [`ContextMenuStyle`]: crate::components::context_menu::ContextMenuStyle
#[derive(Clone, Debug)]
pub struct TextInputStyle {
    /// Font of the text and placeholder.
    pub text_style: TextStyle,
    /// Horizontal alignment of the text inside the box.
    pub align: TextAlign,

    // === Dimensions ===
    pub horizontal_padding: f32,
    pub corner_radius: f32,
    pub caret_width: f32,
    pub focus_ring_width: f32,
    /// Display scale, applied to every dimension above and to the font size.
    pub draw_scale: f32,

    // === Colors ===
    pub background: Color,
    pub text_color: Color,
    pub placeholder_color: Color,
    pub selection_color: Color,
    /// Text color inside the selection.
    pub selected_text_color: Color,
    pub caret_color: Color,
    pub focus_ring_color: Color,
}

impl Hash for TextInputStyle {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.text_style.family.hash(state);
        self.text_style.weight.hash(state);
        self.text_style.size.to_bits().hash(state);
        self.align.hash(state);
        self.horizontal_padding.to_bits().hash(state);
        self.corner_radius.to_bits().hash(state);
        self.caret_width.to_bits().hash(state);
        self.focus_ring_width.to_bits().hash(state);
        self.draw_scale.to_bits().hash(state);
        for color in [
            self.background,
            self.text_color,
            self.placeholder_color,
            self.selection_color,
            self.selected_text_color,
            self.caret_color,
            self.focus_ring_color,
        ] {
            (color.a(), color.r(), color.g(), color.b()).hash(state);
        }
    }
}

impl Default for TextInputStyle {
    fn default() -> Self {
        Self::with_theme(Theme::light())
    }
}

impl TextInputStyle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Derive the palette from a theme: accent for selection and focus ring,
    /// the theme's text colors for the glyphs.
    pub fn with_theme(theme: Theme) -> Self {
        Self {
            text_style: styles::BODY,
            align: TextAlign::Left,
            horizontal_padding: 6.0,
            corner_radius: 6.0,
            caret_width: 1.5,
            focus_ring_width: 2.0,
            draw_scale: 1.0,
            background: theme.fill_quaternary,
            text_color: theme.text_primary,
            placeholder_color: theme.text_tertiary,
            selection_color: with_alpha(theme.accent, 90),
            selected_text_color: theme.text_primary,
            caret_color: theme.accent,
            focus_ring_color: with_alpha(theme.accent, 200),
        }
    }

    pub fn default_with_scale(scale: f32) -> Self {
        Self {
            draw_scale: scale,
            ..Self::default()
        }
    }

    // === Builder API ===

    pub fn with_text_style(mut self, text_style: TextStyle) -> Self {
        self.text_style = text_style;
        self
    }

    pub fn with_align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    pub fn with_scale(mut self, draw_scale: f32) -> Self {
        self.draw_scale = draw_scale;
        self
    }

    pub fn with_background(mut self, background: Color) -> Self {
        self.background = background;
        self
    }

    pub fn with_text_color(mut self, text_color: Color) -> Self {
        self.text_color = text_color;
        self
    }

    pub fn with_selection_color(mut self, selection_color: Color) -> Self {
        self.selection_color = selection_color;
        self
    }

    pub fn with_caret_color(mut self, caret_color: Color) -> Self {
        self.caret_color = caret_color;
        self
    }

    // === Scaled dimensions ===

    pub fn scaled_horizontal_padding(&self) -> f32 {
        self.horizontal_padding * self.draw_scale
    }

    pub fn scaled_corner_radius(&self) -> f32 {
        self.corner_radius * self.draw_scale
    }

    pub fn scaled_caret_width(&self) -> f32 {
        (self.caret_width * self.draw_scale).max(1.0)
    }

    pub fn scaled_focus_ring_width(&self) -> f32 {
        self.focus_ring_width * self.draw_scale
    }

    pub fn font(&self) -> skia_safe::Font {
        self.text_style.font_scaled(self.draw_scale)
    }
}

fn with_alpha(color: Color, alpha: u8) -> Color {
    Color::from_argb(alpha, color.r(), color.g(), color.b())
}
