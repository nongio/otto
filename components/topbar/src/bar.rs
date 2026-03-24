use otto_kit::components::menu_bar::{MenuBarIcon, MenuBarRenderer, MenuBarState, MenuBarStyle};
use otto_kit::prelude::*;
use otto_kit::typography;
use skia_safe::{Canvas, Paint, TextBlob};

use crate::clock::Clock;
use crate::config::*;
use crate::tray;

/// Left panel: app name + menus.
pub struct LeftPanel {
    pub menu_state: MenuBarState,
    pub style: MenuBarStyle,
    pub width: f32,
    pub height: f32,
}

/// Right panel: tray icons + clock.
pub struct RightPanel {
    pub clock: Clock,
    pub tray_menu_state: MenuBarState,
    pub tray_style: MenuBarStyle,
    pub width: f32,
    pub height: f32,
}

fn tray_menu_style() -> MenuBarStyle {
    let theme = AppContext::current_theme();
    MenuBarStyle {
        height: BAR_HEIGHT as f32,
        item_padding_horizontal: 3.0,
        bar_padding_horizontal: 0.0,
        item_spacing: 0.0,
        icon_size: TRAY_ICON_SIZE,
        icon_text_gap: 0.0,
        background_color: skia_safe::Color::TRANSPARENT,
        text_color: theme.text_primary,
        hover_color: skia_safe::Color::from_argb(20, 255, 255, 255),
        active_color: skia_safe::Color::from_argb(40, 255, 255, 255),
        icon_tint: theme.text_primary,
        icon_active_tint: skia_safe::Color::WHITE,
        font_size: 13.0,
        font_weight: skia_safe::font_style::Weight::SEMI_BOLD,
        item_corner_radius: 4.0,
    }
}

fn left_menu_style() -> MenuBarStyle {
    let theme = AppContext::current_theme();
    MenuBarStyle {
        height: BAR_HEIGHT as f32,
        item_padding_horizontal: 8.0,
        bar_padding_horizontal: 6.0,
        item_spacing: 0.0,
        icon_size: 16.0,
        icon_text_gap: 6.0,
        background_color: skia_safe::Color::TRANSPARENT,
        text_color: theme.text_primary,
        hover_color: skia_safe::Color::from_argb(20, 255, 255, 255),
        active_color: skia_safe::Color::from_argb(40, 255, 255, 255),
        icon_tint: theme.text_primary,
        icon_active_tint: skia_safe::Color::WHITE,
        font_size: 13.0,
        font_weight: skia_safe::font_style::Weight::BOLD,
        item_corner_radius: 4.0,
    }
}

/// Build a MenuBarState from current tray items.
pub fn build_tray_menu_state() -> MenuBarState {
    let items = tray::current_items();
    let mut state = MenuBarState::new();
    for item in &items {
        // Prefer pixmap data, then pre-resolved icon file, then icon name
        if let Some(data) = item.icon_data.as_ref() {
            if item.icon_width > 0 && item.icon_height > 0 {
                state.add_icon_item(MenuBarIcon::Pixmap {
                    data: data.clone(),
                    width: item.icon_width,
                    height: item.icon_height,
                });
                continue;
            }
        }
        if let Some(path) = item.icon_file.as_ref() {
            state.add_icon_item(MenuBarIcon::File(path.clone()));
        } else if let Some(name) = item.icon_name.as_ref() {
            state.add_icon_item(MenuBarIcon::Named(name.clone()));
        } else {
            state.add_icon_item(MenuBarIcon::Named("application-default-icon".into()));
        }
    }
    state
}

fn build_left_menu_state() -> MenuBarState {
    let mut state = MenuBarState::new();
    state.add_item("Otto");
    state
}

impl LeftPanel {
    pub fn new() -> Self {
        Self {
            menu_state: build_left_menu_state(),
            style: left_menu_style(),
            width: LEFT_WIDTH as f32,
            height: BAR_HEIGHT as f32,
        }
    }

    pub fn update_style(&mut self) {
        self.style = left_menu_style();
    }

    /// Set the app name shown in the left panel.
    pub fn set_app_name(&mut self, name: &str) {
        self.menu_state = MenuBarState::new();
        self.menu_state.add_item(name);
    }

    pub fn draw(&self, canvas: &Canvas) {
        MenuBarRenderer::render(canvas, &self.menu_state, &self.style, self.width);
    }

    /// Compute the ideal panel width.
    pub fn target_width(&self) -> f32 {
        let w = MenuBarRenderer::measure_width(&self.menu_state, &self.style);
        w.max(LEFT_WIDTH as f32)
    }
}

impl RightPanel {
    pub fn new() -> Self {
        Self {
            clock: Clock::new(),
            tray_menu_state: MenuBarState::new(),
            tray_style: tray_menu_style(),
            width: RIGHT_WIDTH as f32,
            height: BAR_HEIGHT as f32,
        }
    }

    pub fn update_style(&mut self) {
        self.tray_style = tray_menu_style();
    }

    /// Rebuild the tray MenuBarState from current tray items.
    pub fn sync_tray_items(&mut self) {
        self.tray_menu_state = build_tray_menu_state();
    }

    pub fn draw(&self, canvas: &Canvas) {
        let theme = AppContext::current_theme();

        // Clock on the right edge
        let clock_width = self.draw_clock(canvas, &theme);

        // Tray icons to the left of the clock, rendered via MenuBar
        let tray_width = MenuBarRenderer::measure_width(&self.tray_menu_state, &self.tray_style);
        let tray_x = self.width - clock_width - tray_width;

        canvas.save();
        canvas.translate((tray_x, 0.0));
        MenuBarRenderer::render(
            canvas,
            &self.tray_menu_state,
            &self.tray_style,
            tray_width,
        );
        canvas.restore();
    }

    fn draw_clock(&self, canvas: &Canvas, theme: &Theme) -> f32 {
        let font = typography::styles::BODY.font();
        let text = &self.clock.text;
        let text_width = font.measure_str(text, None).0;

        let x = self.width - text_width - BAR_PADDING_H;
        let y = baseline_y(self.height, &font);

        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(theme.text_primary);

        if let Some(blob) = TextBlob::new(text, &font) {
            canvas.draw_text_blob(&blob, (x, y), &paint);
        }

        text_width + BAR_PADDING_H
    }

    /// Compute the ideal panel width based on current clock text and tray icon count.
    pub fn target_width(&self) -> f32 {
        let font = typography::styles::BODY.font();
        let clock_text_width = font.measure_str(&self.clock.text, None).0;
        let tray_width = MenuBarRenderer::measure_width(&self.tray_menu_state, &self.tray_style);
        let content = clock_text_width + BAR_PADDING_H * 2.0 + tray_width;
        content.max(MIN_RIGHT_WIDTH as f32)
    }

    /// Hit-test: return the tray item index at position x (in panel coords).
    pub fn tray_item_at(&self, x: f32) -> Option<usize> {
        if self.tray_menu_state.items().is_empty() {
            return None;
        }

        let font = typography::styles::BODY.font();
        let clock_width = font.measure_str(&self.clock.text, None).0 + BAR_PADDING_H;
        let tray_width =
            MenuBarRenderer::measure_width(&self.tray_menu_state, &self.tray_style);
        let tray_x = self.width - clock_width - tray_width;

        let local_x = x - tray_x;
        if local_x < 0.0 || local_x > tray_width {
            return None;
        }

        // Walk items to find hit
        let font = otto_kit::typography::get_font_with_fallback(
            "Inter",
            self.tray_style.font_style(),
            self.tray_style.font_size,
        );
        let mut offset = self.tray_style.bar_padding_horizontal;
        for (i, item) in self.tray_menu_state.items().iter().enumerate() {
            let cw = self.tray_style.item_content_width(item, &font);
            let iw = self.tray_style.item_width(cw);
            if local_x >= offset && local_x <= offset + iw {
                return Some(i);
            }
            offset += iw + self.tray_style.item_spacing;
        }

        None
    }
}

/// Vertically center text using cap-height.
fn baseline_y(height: f32, font: &skia_safe::Font) -> f32 {
    let (_, metrics) = font.metrics();
    (height + metrics.cap_height) / 2.0
}
