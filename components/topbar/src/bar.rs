use otto_kit::components::menu_bar::{MenuBarIcon, MenuBarRenderer, MenuBarState, MenuBarStyle};
use otto_kit::prelude::*;
use otto_kit::typography;
use skia_safe::{Canvas, Paint, TextBlob};

use crate::clock::Clock;
use crate::config::*;
use crate::tray;

/// Left panel: app name + menus.
pub struct LeftPanel {
    pub app_name: String,
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
        text_active_color: skia_safe::Color::WHITE,
        hover_color: skia_safe::Color::from_argb(30, 0, 0, 0),
        active_color: skia_safe::Color::from_argb(80, 0, 0, 0),
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
        text_active_color: skia_safe::Color::WHITE,
        hover_color: skia_safe::Color::from_argb(30, 0, 0, 0),
        active_color: skia_safe::Color::from_argb(80, 0, 0, 0),
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
            app_name: "Otto".to_string(),
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
        // Preserve any existing menu items after the app name
        let had_menu = self.menu_state.items().len() > 1;
        self.app_name = name.to_string();
        if !had_menu {
            self.menu_state = MenuBarState::new();
            self.menu_state.add_item(name);
        }
    }

    /// Set the app menu items from a fetched dbusmenu layout.
    /// Keeps the app name as item 0, adds top-level menu labels after it.
    pub fn set_app_menu(&mut self, menu: Option<&crate::appmenu::AppMenu>) {
        let mut state = MenuBarState::new();
        state.add_item(&self.app_name);

        if let Some(menu) = menu {
            for item in &menu.layout.items {
                if item.visible && !item.label.is_empty() {
                    let label = item.label.replace('_', "");
                    state.add_item(&label);
                }
            }
        }

        self.menu_state = state;
    }

    /// Hit-test: return the menu item index at position x (in panel coords).
    pub fn menu_item_at(&self, x: f32) -> Option<usize> {
        if self.menu_state.items().is_empty() {
            return None;
        }

        let font = otto_kit::typography::get_font_with_fallback(
            "Inter",
            self.style.font_style(),
            self.style.font_size,
        );
        let mut offset = self.style.bar_padding_horizontal;
        for (i, item) in self.menu_state.items().iter().enumerate() {
            let cw = self.style.item_content_width(item, &font);
            let iw = self.style.item_width(cw);
            if x >= offset && x <= offset + iw {
                return Some(i);
            }
            offset += iw + self.style.item_spacing;
        }
        None
    }

    /// Compute the x-offset of a menu item for popup positioning.
    pub fn item_anchor_x(&self, index: usize) -> f32 {
        let font = otto_kit::typography::get_font_with_fallback(
            "Inter",
            self.style.font_style(),
            self.style.font_size,
        );
        let mut offset = self.style.bar_padding_horizontal;
        for (i, item) in self.menu_state.items().iter().enumerate() {
            if i == index {
                return offset;
            }
            let cw = self.style.item_content_width(item, &font);
            let iw = self.style.item_width(cw);
            offset += iw + self.style.item_spacing;
        }
        offset
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
        let old_count = self.tray_menu_state.items().len();
        let active = self.tray_menu_state.active_index();
        self.tray_menu_state = build_tray_menu_state();
        // Only preserve active highlight if item count is unchanged —
        // if an item was added or removed the index is no longer valid.
        if self.tray_menu_state.items().len() == old_count {
            self.tray_menu_state.set_active(active);
        }
    }

    pub fn draw(&self, canvas: &Canvas) {
        let theme = AppContext::current_theme();

        // Clock on the right edge
        let clock_width = self.draw_clock(canvas, &theme);

        // Tray icons to the left of the clock, with a gap
        let tray_width = MenuBarRenderer::measure_width(&self.tray_menu_state, &self.tray_style);
        let gap = if tray_width > 0.0 { TRAY_CLOCK_GAP } else { 0.0 };
        let tray_x = self.width - clock_width - gap - tray_width;

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
        let font = typography::styles::BODY_MEDIUM.font();
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
        let font = typography::styles::BODY_MEDIUM.font();
        let clock_text_width = font.measure_str(&self.clock.text, None).0;
        let tray_width = MenuBarRenderer::measure_width(&self.tray_menu_state, &self.tray_style);
        let gap = if tray_width > 0.0 { TRAY_CLOCK_GAP } else { 0.0 };
        let content = clock_text_width + BAR_PADDING_H * 2.0 + gap + tray_width;
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

    /// Return the bounding rect (x, y, w, h) in surface-local coords for a tray icon.
    pub fn tray_item_rect(&self, index: usize) -> Option<(f32, f32, f32, f32)> {
        let items = self.tray_menu_state.items();
        if index >= items.len() {
            return None;
        }
        let font = otto_kit::typography::get_font_with_fallback(
            "Inter",
            self.tray_style.font_style(),
            self.tray_style.font_size,
        );
        let clock_width = {
            let cfont = typography::styles::BODY.font();
            cfont.measure_str(&self.clock.text, None).0 + BAR_PADDING_H
        };
        let tray_width = MenuBarRenderer::measure_width(&self.tray_menu_state, &self.tray_style);
        let gap = if tray_width > 0.0 { TRAY_CLOCK_GAP } else { 0.0 };
        let tray_x = self.width - clock_width - gap - tray_width;

        let mut offset = self.tray_style.bar_padding_horizontal;
        for (i, item) in items.iter().enumerate() {
            let cw = self.tray_style.item_content_width(item, &font);
            let iw = self.tray_style.item_width(cw);
            if i == index {
                let x = tray_x + offset;
                return Some((x, 0.0, iw, self.tray_style.height));
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
