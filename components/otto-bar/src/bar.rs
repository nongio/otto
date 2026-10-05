use otto_kit::components::menu_bar::{MenuBarIcon, MenuBarRenderer, MenuBarState, MenuBarStyle};
use otto_kit::prelude::*;
use otto_kit::typography;
use skia_safe::{Canvas, Paint, TextBlob};

use crate::clock::Clock;
use crate::config::*;
use crate::tray;

/// Left panel: the Otto mark, then the app name and its menus.
pub struct LeftPanel {
    pub app_name: String,
    pub menu_state: MenuBarState,
    pub style: MenuBarStyle,
    /// The Otto menu is open: the mark wears the open-menu pill.
    pub logo_active: bool,
    pub width: f32,
    pub height: f32,
}

/// Where the right panel's three parts sit, in panel coordinates.
pub struct RightLayout {
    pub battery_x: f32,
    /// Zero when there is no battery, or it is configured off.
    pub battery_width: f32,
    pub keyboard_x: f32,
    /// Zero while the keyboard layout indicator is hidden.
    pub keyboard_width: f32,
    pub tray_x: f32,
    pub tray_width: f32,
}

/// Right panel: tray icons + battery + clock.
pub struct RightPanel {
    pub clock: Clock,
    pub tray_menu_state: MenuBarState,
    pub tray_style: MenuBarStyle,
    /// The power menu is open: the battery wears the same pill an open tray
    /// item does.
    pub battery_active: bool,
    /// The keyboard layout menu is open: the keycap wears the same pill.
    pub keyboard_active: bool,
    pub width: f32,
    pub height: f32,
}

/// How wide the Otto mark's pill is. The app menus start where it ends.
const LOGO_PILL_WIDTH: f32 = 30.0;
/// Radius of each of the mark's two dots.
const LOGO_DOT_RADIUS: f32 = 3.25;
/// Distance between the two dots' centres: the logo's proportions, where
/// the gap between the dots is about two thirds of a dot.
const LOGO_DOT_SPACING: f32 = 11.0;

/// How far the open-menu pill reaches past the battery glyph on each side.
/// Inside `TRAY_CLOCK_GAP`, so it never meets the tray's own pill.
const BATTERY_PILL_PADDING: f32 = 5.0;

/// Colours for a highlighted bar item (hover and active/open).
///
/// The bar sits over the wallpaper blur: a black scrim disappears against a
/// dark backdrop, so in dark mode the pill is a translucent white veil — light
/// enough to read as selected, sheer enough that the blur still shows through
/// and the label stays white.
struct HighlightColors {
    hover: skia_safe::Color,
    active: skia_safe::Color,
    on_active: skia_safe::Color,
}

fn highlight_colors() -> HighlightColors {
    let dark = matches!(
        otto_kit::color_scheme::current_color_scheme(),
        otto_kit::theme::ColorScheme::Dark
    );
    if dark {
        HighlightColors {
            hover: skia_safe::Color::from_argb(0x1A, 255, 255, 255),
            active: skia_safe::Color::from_argb(0x40, 255, 255, 255),
            on_active: skia_safe::Color::from_argb(0xF2, 255, 255, 255),
        }
    } else {
        HighlightColors {
            hover: skia_safe::Color::from_argb(30, 0, 0, 0),
            active: skia_safe::Color::from_argb(80, 0, 0, 0),
            on_active: skia_safe::Color::WHITE,
        }
    }
}

fn tray_menu_style() -> MenuBarStyle {
    let theme = AppContext::current_theme();
    let hl = highlight_colors();
    MenuBarStyle {
        height: BAR_HEIGHT as f32,
        item_padding_horizontal: 3.0,
        bar_padding_horizontal: 0.0,
        item_spacing: 0.0,
        icon_size: TRAY_ICON_SIZE,
        icon_text_gap: 0.0,
        background_color: skia_safe::Color::TRANSPARENT,
        text_color: theme.text_primary,
        text_active_color: hl.on_active,
        hover_color: hl.hover,
        active_color: hl.active,
        icon_tint: theme.text_primary,
        icon_active_tint: hl.on_active,
        font_size: 13.0,
        font_weight: skia_safe::font_style::Weight::SEMI_BOLD,
        first_item_font_weight: None,
        item_corner_radius: 4.0,
    }
}

fn left_menu_style() -> MenuBarStyle {
    let theme = AppContext::current_theme();
    let hl = highlight_colors();
    MenuBarStyle {
        height: BAR_HEIGHT as f32,
        item_padding_horizontal: 8.0,
        bar_padding_horizontal: 6.0,
        item_spacing: 0.0,
        icon_size: 16.0,
        icon_text_gap: 6.0,
        background_color: skia_safe::Color::TRANSPARENT,
        text_color: theme.text_primary,
        text_active_color: hl.on_active,
        hover_color: hl.hover,
        active_color: hl.active,
        icon_tint: theme.text_primary,
        icon_active_tint: hl.on_active,
        font_size: 13.0,
        // The menu titles; the application's name ahead of them is bold.
        font_weight: skia_safe::font_style::Weight::MEDIUM,
        first_item_font_weight: Some(skia_safe::font_style::Weight::BOLD),
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
            logo_active: false,
            width: LEFT_WIDTH as f32,
            height: BAR_HEIGHT as f32,
        }
    }

    /// Rebuild the style from the current theme — call on a color-scheme change.
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

    /// Where every menu item sits, in panel coordinates.
    ///
    /// Hit-testing, popup anchoring and what an assistive technology points at
    /// all come through here, so a click, the popup it opens and the rectangle
    /// a screen reader highlights cannot end up in three different places.
    pub fn menu_item_rects(&self) -> Vec<(f32, f32)> {
        let font = otto_kit::typography::get_font_with_fallback(
            "Inter",
            self.style.font_style(),
            self.style.font_size,
        );
        let first_font = self.style.first_item_font();
        let mut offset = LOGO_PILL_WIDTH + self.style.bar_padding_horizontal;
        self.menu_state
            .items()
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let font = match &first_font {
                    Some(first) if index == 0 => first,
                    _ => &font,
                };
                let width = self
                    .style
                    .item_width(self.style.item_content_width(item, font));
                let placed = (offset, width);
                offset += width + self.style.item_spacing;
                placed
            })
            .collect()
    }

    /// Hit-test: return the menu item index at position x (in panel coords).
    pub fn menu_item_at(&self, x: f32) -> Option<usize> {
        self.menu_item_rects()
            .into_iter()
            .position(|(left, width)| x >= left && x <= left + width)
    }

    /// Compute the x-offset of a menu item for popup positioning.
    pub fn item_anchor_x(&self, index: usize) -> f32 {
        let placed = self.menu_item_rects();
        match placed.get(index) {
            Some((left, _)) => *left,
            // Past the last item: the trailing edge, which is where a popup
            // for an item that is not there would have been anchored.
            None => placed
                .last()
                .map(|(left, width)| left + width + self.style.item_spacing)
                .unwrap_or(LOGO_PILL_WIDTH + self.style.bar_padding_horizontal),
        }
    }

    /// The Otto mark's pill, in panel coordinates: what a click on the mark
    /// lands on, and what the Otto menu hangs from.
    pub fn logo_rect(&self) -> (f32, f32, f32, f32) {
        (
            self.style.bar_padding_horizontal,
            0.0,
            LOGO_PILL_WIDTH,
            self.height,
        )
    }

    /// Whether `x` (in panel coords) is on the Otto mark. The bar's own
    /// padding before it counts, so the screen corner opens the menu.
    pub fn logo_at(&self, x: f32) -> bool {
        let (left, _, width, _) = self.logo_rect();
        x >= 0.0 && x <= left + width
    }

    pub fn draw(&self, canvas: &Canvas) {
        self.draw_logo(canvas);
        canvas.save();
        canvas.translate((LOGO_PILL_WIDTH, 0.0));
        MenuBarRenderer::render(
            canvas,
            &self.menu_state,
            &self.style,
            self.width - LOGO_PILL_WIDTH,
        );
        canvas.restore();
    }

    /// The Otto mark: the logo's two dots, in the bar's text colour.
    fn draw_logo(&self, canvas: &Canvas) {
        let (x, y, w, h) = self.logo_rect();
        let color = if self.logo_active {
            // The pill the app menus wear when open, so every open menu on
            // the bar looks the same.
            let hl = highlight_colors();
            let mut pill = Paint::default();
            pill.set_anti_alias(true);
            pill.set_color(hl.active);
            let radius = self.style.item_corner_radius;
            canvas.draw_round_rect(
                skia_safe::Rect::from_xywh(x, y, w, h),
                radius,
                radius,
                &pill,
            );
            hl.on_active
        } else {
            self.style.text_color
        };

        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(color);
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        for dx in [-LOGO_DOT_SPACING / 2.0, LOGO_DOT_SPACING / 2.0] {
            canvas.draw_circle((cx + dx, cy), LOGO_DOT_RADIUS, &paint);
        }
    }

    /// Compute the ideal panel width.
    pub fn target_width(&self) -> f32 {
        let w = MenuBarRenderer::measure_width(&self.menu_state, &self.style) + LOGO_PILL_WIDTH;
        w.max(LEFT_WIDTH as f32)
    }
}

impl RightPanel {
    pub fn new() -> Self {
        Self {
            clock: Clock::new(),
            tray_menu_state: MenuBarState::new(),
            tray_style: tray_menu_style(),
            battery_active: false,
            keyboard_active: false,
            width: RIGHT_WIDTH as f32,
            height: BAR_HEIGHT as f32,
        }
    }

    /// Rebuild the style from the current theme — call on a color-scheme change.
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

    /// Where everything in the right panel sits.
    ///
    /// Drawing, hit-testing and the rectangles handed to assistive
    /// technologies all come through here, so a click, the menu it opens and
    /// the box a screen reader highlights cannot drift apart.
    pub fn layout(&self) -> RightLayout {
        let battery_width = crate::battery::width();
        let keyboard_width = crate::keyboard_layout::width();
        let tray_width = MenuBarRenderer::measure_width(&self.tray_menu_state, &self.tray_style);

        let [_, battery_x, keyboard_x, tray_x] = pack_right_to_left(
            self.width - BAR_PADDING_H,
            [
                self.clock_text_width(),
                battery_width,
                keyboard_width,
                tray_width,
            ],
        );

        RightLayout {
            battery_x,
            battery_width,
            keyboard_x,
            keyboard_width,
            tray_x,
            tray_width,
        }
    }

    pub fn draw(&self, canvas: &Canvas) {
        let theme = AppContext::current_theme();
        let layout = self.layout();

        // Right to left: clock on the edge, then the battery, then the tray.
        self.draw_clock(canvas, &theme);

        if self.battery_active && layout.battery_width > 0.0 {
            // The tray's pill, down to the colours and the corner: an open
            // menu looks the same whichever item it belongs to.
            let hl = highlight_colors();
            let mut paint = Paint::default();
            paint.set_anti_alias(true);
            paint.set_color(hl.active);
            let (px, py, pw, ph) = self.battery_pill_rect().unwrap_or_default();
            let pill = skia_safe::Rect::from_xywh(px, py, pw, ph);
            let radius = self.tray_style.item_corner_radius;
            canvas.draw_round_rect(pill, radius, radius, &paint);

            let mut active_theme = theme.clone();
            active_theme.text_primary = hl.on_active;
            crate::battery::draw(canvas, layout.battery_x, self.height, &active_theme);
        } else {
            crate::battery::draw(canvas, layout.battery_x, self.height, &theme);
        }

        if layout.keyboard_width > 0.0 {
            let color = if self.keyboard_active {
                let hl = highlight_colors();
                let mut paint = Paint::default();
                paint.set_anti_alias(true);
                paint.set_color(hl.active);
                let (px, py, pw, ph) = self.keyboard_pill_rect().unwrap_or_default();
                let radius = self.tray_style.item_corner_radius;
                canvas.draw_round_rect(
                    skia_safe::Rect::from_xywh(px, py, pw, ph),
                    radius,
                    radius,
                    &paint,
                );
                hl.on_active
            } else {
                theme.text_primary
            };
            crate::keyboard_layout::draw(canvas, layout.keyboard_x, self.height, color);
        }

        canvas.save();
        canvas.translate((layout.tray_x, 0.0));
        MenuBarRenderer::render(
            canvas,
            &self.tray_menu_state,
            &self.tray_style,
            layout.tray_width,
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

    /// Compute the ideal panel width from the clock text, the battery and the
    /// tray icon count.
    pub fn target_width(&self) -> f32 {
        let content = packed_width([
            self.clock_text_width(),
            crate::battery::width(),
            crate::keyboard_layout::width(),
            MenuBarRenderer::measure_width(&self.tray_menu_state, &self.tray_style),
        ]) + BAR_PADDING_H * 2.0;
        content.max(MIN_RIGHT_WIDTH as f32)
    }

    /// The clock's text alone. Zero while the clock is hidden, whose text is
    /// then empty.
    fn clock_text_width(&self) -> f32 {
        if self.clock.text.is_empty() {
            return 0.0;
        }
        typography::styles::BODY_MEDIUM
            .font()
            .measure_str(&self.clock.text, None)
            .0
    }

    /// Hit-test: return the tray item index at position x (in panel coords).
    /// How wide the clock is, which is also where the tray ends.
    ///
    /// Measured the same way the panel draws and hit-tests it, so the
    /// rectangle a screen reader is given is the one on screen.
    pub fn clock_width(&self) -> f32 {
        self.clock_text_width() + BAR_PADDING_H
    }

    /// Whether `x` (in panel coords) is on the battery indicator.
    ///
    /// The hit box is the drawn glyph plus half the gap on either side: the
    /// glyph is 26 points of a 30-point bar, and a click that lands a pixel
    /// above it is still a click on the battery.
    pub fn battery_at(&self, x: f32) -> bool {
        let layout = self.layout();
        if layout.battery_width <= 0.0 {
            return false;
        }
        let slack = TRAY_CLOCK_GAP / 2.0;
        x >= layout.battery_x - slack && x <= layout.battery_x + layout.battery_width + slack
    }

    /// The battery indicator's bounding box in surface-local coords.
    pub fn battery_rect(&self) -> Option<(f32, f32, f32, f32)> {
        let layout = self.layout();
        if layout.battery_width <= 0.0 {
            return None;
        }
        Some((layout.battery_x, 0.0, layout.battery_width, self.height))
    }

    /// The pill an open power menu draws behind the battery.
    ///
    /// The menu hangs from this rect, as a tray menu hangs from its item's
    /// pill, so both line up with the highlight above them.
    pub fn battery_pill_rect(&self) -> Option<(f32, f32, f32, f32)> {
        self.battery_rect().map(|(x, y, w, h)| {
            (
                x - BATTERY_PILL_PADDING,
                y,
                w + BATTERY_PILL_PADDING * 2.0,
                h,
            )
        })
    }

    /// Whether `x` (in panel coords) is on the keyboard layout indicator,
    /// with the same slack either side the battery has.
    pub fn keyboard_at(&self, x: f32) -> bool {
        let layout = self.layout();
        if layout.keyboard_width <= 0.0 {
            return false;
        }
        let slack = TRAY_CLOCK_GAP / 2.0;
        x >= layout.keyboard_x - slack && x <= layout.keyboard_x + layout.keyboard_width + slack
    }

    /// The keyboard layout indicator's bounding box in surface-local coords.
    pub fn keyboard_rect(&self) -> Option<(f32, f32, f32, f32)> {
        let layout = self.layout();
        if layout.keyboard_width <= 0.0 {
            return None;
        }
        Some((layout.keyboard_x, 0.0, layout.keyboard_width, self.height))
    }

    /// The pill an open layout menu draws behind the keycap, and hangs from.
    pub fn keyboard_pill_rect(&self) -> Option<(f32, f32, f32, f32)> {
        self.keyboard_rect().map(|(x, y, w, h)| {
            (
                x - BATTERY_PILL_PADDING,
                y,
                w + BATTERY_PILL_PADDING * 2.0,
                h,
            )
        })
    }

    pub fn tray_item_at(&self, x: f32) -> Option<usize> {
        if self.tray_menu_state.items().is_empty() {
            return None;
        }

        let layout = self.layout();
        let local_x = x - layout.tray_x;
        if local_x < 0.0 || local_x > layout.tray_width {
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
        let tray_x = self.layout().tray_x;

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

/// Where each of `widths` starts when laid right to left from `right_edge`,
/// with `TRAY_CLOCK_GAP` between neighbours that are there at all.
///
/// A zero width is an item that is not shown: it takes no room and no gap,
/// and its x is where it would start, the left edge of what is placed so far.
fn pack_right_to_left<const N: usize>(right_edge: f32, widths: [f32; N]) -> [f32; N] {
    let mut edge = right_edge;
    let mut placed_any = false;
    widths.map(|width| {
        if width > 0.0 {
            if placed_any {
                edge -= TRAY_CLOCK_GAP;
            }
            edge -= width;
            placed_any = true;
        }
        edge
    })
}

/// How much room [`pack_right_to_left`] takes for `widths`.
fn packed_width<const N: usize>(widths: [f32; N]) -> f32 {
    let shown = widths.iter().filter(|w| **w > 0.0).count();
    widths.iter().sum::<f32>() + TRAY_CLOCK_GAP * shown.saturating_sub(1) as f32
}

/// Vertically center text using cap-height.
fn baseline_y(height: f32, font: &skia_safe::Font) -> f32 {
    let (_, metrics) = font.metrics();
    (height + metrics.cap_height) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hidden_clock_leaves_no_gap_at_the_edge() {
        // Clock, battery, keyboard, tray.
        let shown = pack_right_to_left(100.0, [40.0, 20.0, 0.0, 10.0]);
        assert_eq!(
            shown,
            [
                60.0,
                60.0 - TRAY_CLOCK_GAP - 20.0,
                28.0,
                28.0 - TRAY_CLOCK_GAP - 10.0
            ]
        );

        // Without the clock the battery takes its place against the edge.
        let hidden = pack_right_to_left(100.0, [0.0, 20.0, 0.0, 10.0]);
        assert_eq!(hidden[1], 80.0);
        assert_eq!(hidden[3], 80.0 - TRAY_CLOCK_GAP - 10.0);
    }

    #[test]
    fn the_packed_width_counts_gaps_only_between_shown_items() {
        assert_eq!(
            packed_width([40.0, 20.0, 0.0, 10.0]),
            70.0 + 2.0 * TRAY_CLOCK_GAP
        );
        assert_eq!(packed_width([0.0, 20.0, 0.0, 0.0]), 20.0);
        assert_eq!(packed_width([0.0, 0.0, 0.0, 0.0]), 0.0);
    }
}
