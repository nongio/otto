use skia_safe::{Canvas, Color, Paint, RRect, Rect};

use crate::common::Renderable;
use crate::components::icon::Icon;
use crate::components::label::{Label, TextAlign};
use crate::typography::TextStyle;

/// Button state for visual feedback
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    Normal,
    Hovered,
    Pressed,
    Disabled,
}

/// Icon position relative to the label
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconPosition {
    Left,
    Right,
    Top,
    Bottom,
}

/// Button variant styles
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonVariant {
    Primary,
    Secondary,
    Outline,
    Ghost,
    Danger,
}

/// A flexible button component that can contain a label, icon, or both
pub struct Button {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,

    // Content
    label: Option<String>,
    icon: Option<String>,
    icon_position: IconPosition,

    // Styling
    variant: ButtonVariant,
    state: ButtonState,
    background_color: Option<Color>,
    text_color: Option<Color>,
    border_color: Option<Color>,
    corner_radius: f32,
    padding_horizontal: f32,
    padding_vertical: f32,
    icon_size: f32,
    icon_spacing: f32,
    text_style: TextStyle,
}

impl Button {
    /// Create a new button with a label
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 0.0,  // Auto-calculated
            height: 0.0, // Auto-calculated
            label: Some(label.into()),
            icon: None,
            icon_position: IconPosition::Left,
            variant: ButtonVariant::Primary,
            state: ButtonState::Normal,
            background_color: None,
            text_color: None,
            border_color: None,
            corner_radius: 8.0,
            padding_horizontal: 16.0,
            padding_vertical: 8.0,
            icon_size: 20.0,
            icon_spacing: 8.0,
            text_style: crate::typography::styles::BODY,
        }
    }

    /// Create a button with only an icon
    pub fn icon(icon_name: impl Into<String>) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            label: None,
            icon: Some(icon_name.into()),
            icon_position: IconPosition::Left,
            variant: ButtonVariant::Primary,
            state: ButtonState::Normal,
            background_color: None,
            text_color: None,
            border_color: None,
            corner_radius: 8.0,
            padding_horizontal: 8.0,
            padding_vertical: 8.0,
            icon_size: 20.0,
            icon_spacing: 8.0,
            text_style: crate::typography::styles::BODY,
        }
    }

    pub fn at(mut self, x: f32, y: f32) -> Self {
        self.x = x;
        self.y = y;
        self
    }

    pub fn with_size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    pub fn with_icon(mut self, icon_name: impl Into<String>) -> Self {
        self.icon = Some(icon_name.into());
        self
    }

    pub fn with_icon_position(mut self, position: IconPosition) -> Self {
        self.icon_position = position;
        self
    }

    /// Get the button's calculated width
    pub fn width(&self) -> f32 {
        let (width, _) = self.calculate_dimensions();
        width
    }

    /// Get the button's calculated height
    pub fn height(&self) -> f32 {
        let (_, height) = self.calculate_dimensions();
        height
    }

    pub fn with_variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn with_state(mut self, state: ButtonState) -> Self {
        self.state = state;
        self
    }

    pub fn with_background(mut self, color: Color) -> Self {
        self.background_color = Some(color);
        self
    }

    pub fn with_text_color(mut self, color: Color) -> Self {
        self.text_color = Some(color);
        self
    }

    pub fn with_border_color(mut self, color: Color) -> Self {
        self.border_color = Some(color);
        self
    }

    pub fn with_corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius;
        self
    }

    pub fn with_padding(mut self, horizontal: f32, vertical: f32) -> Self {
        self.padding_horizontal = horizontal;
        self.padding_vertical = vertical;
        self
    }

    pub fn with_icon_size(mut self, size: f32) -> Self {
        self.icon_size = size;
        self
    }

    pub fn with_text_style(mut self, style: TextStyle) -> Self {
        self.text_style = style;
        self
    }

    pub fn primary(mut self) -> Self {
        self.variant = ButtonVariant::Primary;
        self
    }

    pub fn secondary(mut self) -> Self {
        self.variant = ButtonVariant::Secondary;
        self
    }

    pub fn outline(mut self) -> Self {
        self.variant = ButtonVariant::Outline;
        self
    }

    pub fn ghost(mut self) -> Self {
        self.variant = ButtonVariant::Ghost;
        self
    }

    pub fn danger(mut self) -> Self {
        self.variant = ButtonVariant::Danger;
        self
    }

    pub fn disabled(mut self) -> Self {
        self.state = ButtonState::Disabled;
        self
    }

    pub fn build(self) -> Self {
        self
    }

    /// Get the default colors for the current variant and state
    fn get_colors(&self) -> (Color, Color, Option<Color>) {
        // Returns: (background, text, border)
        match self.variant {
            ButtonVariant::Primary => match self.state {
                ButtonState::Normal => (
                    Color::from_rgb(59, 130, 246), // Blue
                    Color::WHITE,
                    None,
                ),
                ButtonState::Hovered => (
                    Color::from_rgb(37, 99, 235), // Darker blue
                    Color::WHITE,
                    None,
                ),
                ButtonState::Pressed => (
                    Color::from_rgb(29, 78, 216), // Even darker blue
                    Color::WHITE,
                    None,
                ),
                ButtonState::Disabled => (
                    Color::from_rgb(156, 163, 175), // Gray
                    Color::from_rgb(209, 213, 219),
                    None,
                ),
            },
            ButtonVariant::Secondary => match self.state {
                ButtonState::Normal => (
                    Color::from_rgb(229, 231, 235),
                    Color::from_rgb(55, 65, 81),
                    None,
                ),
                ButtonState::Hovered => (
                    Color::from_rgb(209, 213, 219),
                    Color::from_rgb(31, 41, 55),
                    None,
                ),
                ButtonState::Pressed => (
                    Color::from_rgb(156, 163, 175),
                    Color::from_rgb(17, 24, 39),
                    None,
                ),
                ButtonState::Disabled => (
                    Color::from_rgb(243, 244, 246),
                    Color::from_rgb(156, 163, 175),
                    None,
                ),
            },
            ButtonVariant::Outline => match self.state {
                ButtonState::Normal => (
                    Color::TRANSPARENT,
                    Color::from_rgb(55, 65, 81),
                    Some(Color::from_rgb(209, 213, 219)),
                ),
                ButtonState::Hovered => (
                    Color::from_rgb(249, 250, 251),
                    Color::from_rgb(31, 41, 55),
                    Some(Color::from_rgb(156, 163, 175)),
                ),
                ButtonState::Pressed => (
                    Color::from_rgb(243, 244, 246),
                    Color::from_rgb(17, 24, 39),
                    Some(Color::from_rgb(107, 114, 128)),
                ),
                ButtonState::Disabled => (
                    Color::TRANSPARENT,
                    Color::from_rgb(156, 163, 175),
                    Some(Color::from_rgb(229, 231, 235)),
                ),
            },
            ButtonVariant::Ghost => match self.state {
                ButtonState::Normal => (
                    Color::TRANSPARENT,
                    Color::from_argb(0x80, 0x00, 0x00, 0x00), // text_secondary - 50% opacity black
                    None,
                ),
                ButtonState::Hovered => (
                    Color::from_rgb(249, 250, 251),
                    Color::from_argb(0xD9, 0x00, 0x00, 0x00), // text_primary on hover
                    None,
                ),
                ButtonState::Pressed => (
                    Color::from_rgb(243, 244, 246),
                    Color::from_argb(0xD9, 0x00, 0x00, 0x00), // text_primary when pressed
                    None,
                ),
                ButtonState::Disabled => (
                    Color::TRANSPARENT,
                    Color::from_argb(0x40, 0x00, 0x00, 0x00), // text_tertiary - 25% opacity
                    None,
                ),
            },
            ButtonVariant::Danger => match self.state {
                ButtonState::Normal => (
                    Color::from_rgb(239, 68, 68), // Red
                    Color::WHITE,
                    None,
                ),
                ButtonState::Hovered => (
                    Color::from_rgb(220, 38, 38), // Darker red
                    Color::WHITE,
                    None,
                ),
                ButtonState::Pressed => (
                    Color::from_rgb(185, 28, 28), // Even darker red
                    Color::WHITE,
                    None,
                ),
                ButtonState::Disabled => (
                    Color::from_rgb(156, 163, 175),
                    Color::from_rgb(209, 213, 219),
                    None,
                ),
            },
        }
    }

    /// Calculate the button dimensions if not explicitly set
    fn calculate_dimensions(&self) -> (f32, f32) {
        if self.width > 0.0 && self.height > 0.0 {
            return (self.width, self.height);
        }

        let font = self.text_style.font();
        let mut content_width: f32 = 0.0;
        let mut content_height: f32 = 0.0;

        // Calculate text dimensions if present
        if let Some(ref label) = self.label {
            let (text_width, _) = font.measure_str(label, None);
            content_width += text_width;
            content_height = content_height.max(font.size());
        }

        // Add icon dimensions if present
        if self.icon.is_some() {
            match self.icon_position {
                IconPosition::Left | IconPosition::Right => {
                    if self.label.is_some() {
                        content_width += self.icon_spacing;
                    }
                    content_width += self.icon_size;
                    content_height = content_height.max(self.icon_size);
                }
                IconPosition::Top | IconPosition::Bottom => {
                    if self.label.is_some() {
                        content_height += self.icon_spacing;
                    }
                    content_height += self.icon_size;
                    content_width = content_width.max(self.icon_size);
                }
            }
        }

        // Add padding
        let width = if self.width > 0.0 {
            self.width
        } else {
            content_width + self.padding_horizontal * 2.0
        };

        let height = if self.height > 0.0 {
            self.height
        } else {
            content_height + self.padding_vertical * 2.0
        };

        (width, height)
    }
}

impl Renderable for Button {
    fn render(&self, canvas: &Canvas) {
        let (width, height) = self.calculate_dimensions();
        let (bg_color, text_color, border_color) = self.get_colors();

        // Use custom colors if provided
        let bg_color = self.background_color.unwrap_or(bg_color);
        let text_color = self.text_color.unwrap_or(text_color);
        let border_color = self.border_color.or(border_color);

        // Draw background
        let rect = Rect::from_xywh(self.x, self.y, width, height);
        let rrect = RRect::new_rect_xy(rect, self.corner_radius, self.corner_radius);

        let mut bg_paint = Paint::default();
        bg_paint.set_color(bg_color);
        bg_paint.set_anti_alias(true);
        canvas.draw_rrect(rrect, &bg_paint);

        // Draw border if present
        if let Some(border_color) = border_color {
            let mut border_paint = Paint::default();
            border_paint.set_color(border_color);
            border_paint.set_style(skia_safe::PaintStyle::Stroke);
            border_paint.set_stroke_width(1.0);
            border_paint.set_anti_alias(true);
            canvas.draw_rrect(rrect, &border_paint);
        }

        // Calculate content layout
        let font = self.text_style.font();
        let content_x = self.x + self.padding_horizontal;
        let content_y = self.y + self.padding_vertical;
        let content_width = width - self.padding_horizontal * 2.0;
        let content_height = height - self.padding_vertical * 2.0;

        match (&self.label, &self.icon) {
            // Both label and icon
            (Some(label), Some(icon_name)) => {
                let (text_width, _) = font.measure_str(label, None);

                match self.icon_position {
                    IconPosition::Left => {
                        let total_width = self.icon_size + self.icon_spacing + text_width;
                        let start_x = content_x + (content_width - total_width) / 2.0;

                        // Icon
                        Icon::new(icon_name)
                            .at(start_x, content_y + (content_height - self.icon_size) / 2.0)
                            .with_size(self.icon_size)
                            .with_color(text_color)
                            .render(canvas);

                        // Label
                        Label::new(label)
                            .at(
                                start_x + self.icon_size + self.icon_spacing,
                                content_y + (content_height - font.size()) / 2.0,
                            )
                            .with_style(self.text_style)
                            .with_color(text_color)
                            .render(canvas);
                    }
                    IconPosition::Right => {
                        let total_width = text_width + self.icon_spacing + self.icon_size;
                        let start_x = content_x + (content_width - total_width) / 2.0;

                        // Label
                        Label::new(label)
                            .at(start_x, content_y + (content_height - font.size()) / 2.0)
                            .with_style(self.text_style)
                            .with_color(text_color)
                            .render(canvas);

                        // Icon
                        Icon::new(icon_name)
                            .at(
                                start_x + text_width + self.icon_spacing,
                                content_y + (content_height - self.icon_size) / 2.0,
                            )
                            .with_size(self.icon_size)
                            .with_color(text_color)
                            .render(canvas);
                    }
                    IconPosition::Top => {
                        let total_height = self.icon_size + self.icon_spacing + font.size();
                        let start_y = content_y + (content_height - total_height) / 2.0;

                        // Icon
                        Icon::new(icon_name)
                            .at(content_x + (content_width - self.icon_size) / 2.0, start_y)
                            .with_size(self.icon_size)
                            .with_color(text_color)
                            .render(canvas);

                        // Label
                        Label::new(label)
                            .at(content_x, start_y + self.icon_size + self.icon_spacing)
                            .with_width(content_width)
                            .with_align(TextAlign::Center)
                            .with_style(self.text_style)
                            .with_color(text_color)
                            .render(canvas);
                    }
                    IconPosition::Bottom => {
                        let total_height = font.size() + self.icon_spacing + self.icon_size;
                        let start_y = content_y + (content_height - total_height) / 2.0;

                        // Label
                        Label::new(label)
                            .at(content_x, start_y)
                            .with_width(content_width)
                            .with_align(TextAlign::Center)
                            .with_style(self.text_style)
                            .with_color(text_color)
                            .render(canvas);

                        // Icon
                        Icon::new(icon_name)
                            .at(
                                content_x + (content_width - self.icon_size) / 2.0,
                                start_y + font.size() + self.icon_spacing,
                            )
                            .with_size(self.icon_size)
                            .with_color(text_color)
                            .render(canvas);
                    }
                }
            }
            // Only label
            (Some(label), None) => {
                Label::new(label)
                    .at(content_x, content_y + (content_height - font.size()) / 2.0)
                    .with_width(content_width)
                    .with_align(TextAlign::Center)
                    .with_style(self.text_style)
                    .with_color(text_color)
                    .render(canvas);
            }
            // Only icon
            (None, Some(icon_name)) => {
                Icon::new(icon_name)
                    .at(
                        content_x + (content_width - self.icon_size) / 2.0,
                        content_y + (content_height - self.icon_size) / 2.0,
                    )
                    .with_size(self.icon_size)
                    .with_color(text_color)
                    .render(canvas);
            }
            // Neither (shouldn't happen)
            (None, None) => {}
        }
    }

    fn intrinsic_size(&self) -> Option<(f32, f32)> {
        Some(self.calculate_dimensions())
    }
}
