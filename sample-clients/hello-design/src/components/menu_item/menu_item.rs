use skia_safe::{Canvas, Color, Font, Paint, Point, RRect, Rect};

use crate::common::Renderable;
use crate::typography::TextStyle;

/// Visual state of a menu item
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItemState {
    Normal,
    Hovered,
    Disabled,
}

/// Type of menu item
#[derive(Debug, Clone)]
pub enum MenuItemKind {
    Action {
        label: String,
        shortcut: Option<String>,
    },
    Separator,
}

#[derive(Debug, Clone)]
pub struct MenuItem {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub kind: MenuItemKind,
    pub state: MenuItemState,
    pub font: Font,
    pub shortcut_font: Font,
}

impl MenuItem {
    pub fn action(label: impl Into<String>) -> Self {
        let style = crate::typography::styles::BODY; // 13pt
        // Old used 13pt for shortcuts - nearly same as main text
        let shortcut_style = crate::typography::styles::BODY; 
        
        Self {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 24.0, // Match old: tighter spacing
            kind: MenuItemKind::Action {
                label: label.into(),
                shortcut: None,
            },
            state: MenuItemState::Normal,
            font: style.font(),
            shortcut_font: shortcut_style.font(),
        }
    }

    pub fn separator() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 9.0,
            kind: MenuItemKind::Separator,
            state: MenuItemState::Normal,
            font: Font::default(),
            shortcut_font: Font::default(),
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

    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        if let MenuItemKind::Action { label, .. } = self.kind {
            self.kind = MenuItemKind::Action {
                label,
                shortcut: Some(shortcut.into()),
            };
        }
        self
    }

    pub fn with_state(mut self, state: MenuItemState) -> Self {
        self.state = state;
        self
    }

    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.font = style.font();
        self
    }

    pub fn build(self) -> Self {
        self
    }
}

impl Renderable for MenuItem {
    fn render(&self, canvas: &Canvas) {
        match &self.kind {
            MenuItemKind::Separator => {
                let mut paint = Paint::default();
                paint.set_color(Color::from_argb(26, 0, 0, 0)); // 10% black
                paint.set_anti_alias(true);
                paint.set_stroke_width(1.0);
                
                let y = self.y + self.height / 2.0;
                canvas.draw_line(
                    Point::new(self.x + 8.0, y),
                    Point::new(self.x + self.width - 8.0, y),
                    &paint,
                );
            }
            MenuItemKind::Action { label, shortcut } => {
                // Draw background if hovered and not disabled
                // Old code: no vertical padding, full height highlight
                if self.state == MenuItemState::Hovered {
                    let mut bg_paint = Paint::default();
                    // Old: [0.039, 0.51, 1.0, 0.75] = rgba(10, 130, 255, 0.75)
                    bg_paint.set_color(Color::from_argb(191, 10, 130, 255));
                    bg_paint.set_anti_alias(true);
                    
                    let bg_rect = RRect::new_rect_xy(
                        Rect::from_xywh(self.x + 6.0, self.y, self.width - 12.0, self.height),
                        5.0,
                        5.0,
                    );
                    canvas.draw_rrect(&bg_rect, &bg_paint);
                }

                // Choose text color based on state
                let text_color = match self.state {
                    MenuItemState::Normal => Color::from_argb(217, 0, 0, 0), // 85% black
                    MenuItemState::Hovered => Color::WHITE,
                    MenuItemState::Disabled => Color::from_argb(64, 0, 0, 0), // 25% black
                };

                // Draw main label
                let mut paint = Paint::default();
                paint.set_color(text_color);
                paint.set_anti_alias(true);

                let baseline_y = self.y + self.height * 0.68;
                canvas.draw_str(
                    label,
                    Point::new(self.x + 12.0, baseline_y),
                    &self.font,
                    &paint,
                );

                // Draw shortcut if present
                if let Some(shortcut_text) = shortcut {
                    // Old: shortcuts always use disabled_text_color in normal state (lighter)
                    let shortcut_color = match self.state {
                        MenuItemState::Hovered => Color::WHITE,
                        MenuItemState::Disabled => Color::from_argb(64, 0, 0, 0), // 25% black
                        MenuItemState::Normal => Color::from_argb(64, 0, 0, 0), // Same as disabled for subtle look
                    };

                    paint.set_color(shortcut_color);
                    let (shortcut_width, _) = self.shortcut_font.measure_str(shortcut_text, None);
                    canvas.draw_str(
                        shortcut_text,
                        Point::new(self.x + self.width - shortcut_width - 12.0, baseline_y),
                        &self.shortcut_font,
                        &paint,
                    );
                }
            }
        }
    }
}
