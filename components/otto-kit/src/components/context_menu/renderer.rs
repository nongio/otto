use super::{ContextMenuState, ContextMenuStyle};
use crate::components::menu_item::{MenuItem, MenuItemGroup, VisualState};
use skia_safe::{Canvas, Paint, RRect, Rect};

/// Pure rendering functions for ContextMenu
///
/// Stateless drawing - all data passed as parameters.
pub struct ContextMenuRenderer;

/// Height of the strip a scroll arrow sits in, at either end of a capped menu.
const ARROW_STRIP: f32 = 14.0;

impl ContextMenuRenderer {
    /// Calculate menu dimensions from items and style
    ///
    /// Returns (width, height) in logical pixels.
    pub fn measure(state: &ContextMenuState, style: &ContextMenuStyle) -> (f32, f32) {
        Self::measure_items(state.items(), style)
    }

    /// The height the items want, padding included, in logical points and
    /// before any [`ContextMenuStyle::max_height`] cap.
    pub fn content_height(items: &[MenuItem], style: &ContextMenuStyle) -> f32 {
        items
            .iter()
            .map(|item| style.item_height_of(item))
            .sum::<f32>()
            + style.vertical_padding * 2.0
    }

    /// How much taller the items are than the menu is allowed to be — what
    /// there is to scroll. Zero when everything fits.
    pub fn overflow(items: &[MenuItem], style: &ContextMenuStyle) -> f32 {
        match style.max_height {
            Some(max) => (Self::content_height(items, style) - max).max(0.0),
            None => 0.0,
        }
    }

    /// The scroll offset that brings the row at `index` inside the box a
    /// capped list is drawn in, given where it is scrolled now.
    ///
    /// Keyboard navigation needs this: the arrows move a highlight that the
    /// pointer never has to reach, so a selection walking past the last
    /// visible row would otherwise disappear under the menu's edge. Returns
    /// the offset unchanged when the row is already in view, and zero when
    /// there is nothing to scroll.
    ///
    /// The row is kept clear of [`ARROW_STRIP`] at either end so it does not
    /// come to rest under a scroll arrow — at the very ends of the list the
    /// clamp takes that margin back, which is exactly where no arrow is drawn.
    pub fn scroll_to_reveal(
        items: &[MenuItem],
        style: &ContextMenuStyle,
        index: usize,
        scroll: f32,
    ) -> f32 {
        let overflow = Self::overflow(items, style);
        if overflow <= 0.0 {
            return 0.0;
        }
        let box_height = Self::content_height(items, style) - overflow;

        let mut top = style.vertical_padding;
        for item in items.iter().take(index) {
            top += style.item_height_of(item);
        }
        let bottom = top + items.get(index).map_or(0.0, |i| style.item_height_of(i));

        let scroll = scroll.clamp(0.0, overflow);
        let target = if top - ARROW_STRIP < scroll {
            top - ARROW_STRIP
        } else if bottom + ARROW_STRIP > scroll + box_height {
            bottom + ARROW_STRIP - box_height
        } else {
            scroll
        };
        target.clamp(0.0, overflow)
    }

    /// Calculate dimensions for specific items (used for submenus)
    ///
    /// Returned dimensions are already multiplied by `style.draw_scale`.
    pub fn measure_items(items: &[MenuItem], style: &ContextMenuStyle) -> (f32, f32) {
        let s = style.draw_scale;

        // Calculate height from items (item heights are in logical pixels).
        // A list longer than the menu may be tall is capped here, not cut
        // short: the surplus scrolls (see `render_depth`).
        let height = Self::content_height(items, style);
        let height = match style.max_height {
            Some(max) => height.min(max),
            None => height,
        } * s;

        // Use provided width or compute from content, then scale
        let width = style
            .width
            .unwrap_or_else(|| Self::compute_optimal_width(items, style).max(style.min_width))
            * s;

        (width, height)
    }

    /// Compute the optimal menu width based on item label text measurements.
    ///
    /// Measures every label (and shortcut) with the actual font, adds padding
    /// for icons and submenu arrows, and returns the widest row in logical pixels.
    pub fn compute_optimal_width(items: &[MenuItem], style: &ContextMenuStyle) -> f32 {
        use crate::components::menu_item::MenuItemKind;

        let item_style = style.item_style();
        let font = item_style.font();
        let icon_size: f32 = 16.0;
        let icon_gap: f32 = 6.0;
        let submenu_arrow_space: f32 = 20.0;

        let mut max_width: f32 = 0.0;
        for item in items {
            let (label, shortcut, is_submenu) = match &item.kind {
                MenuItemKind::Action {
                    label, shortcut, ..
                } => (label.as_str(), shortcut.as_deref(), false),
                MenuItemKind::Submenu { label, .. } => (label.as_str(), None, true),
                MenuItemKind::Separator => continue,
            };

            let (label_w, _) = font.measure_str(label, None);
            let mut row_w = item_style.horizontal_padding * 2.0 + label_w;

            if item.icon.is_some() {
                row_w += icon_size + icon_gap;
            }
            if let Some(sc) = shortcut {
                let (sc_w, _) = font.measure_str(sc, None);
                row_w += sc_w + 20.0; // gap between label and shortcut
            }
            if is_submenu {
                row_w += submenu_arrow_space;
            }

            max_width = max_width.max(row_w);
        }

        // Add context menu horizontal padding (both sides)
        max_width + style.horizontal_padding * 2.0
    }

    /// Render items at a specific depth with specific selection
    ///
    /// `width` and `height` are the layer's actual pixel dimensions (already scaled).
    /// The canvas is scaled by `draw_scale` so all drawing uses unscaled logical coords.
    pub fn render_depth(
        canvas: &Canvas,
        items: &[MenuItem],
        selected: Option<usize>,
        style: &ContextMenuStyle,
        width: f32,
        height: f32,
        scroll: f32,
    ) {
        let s = style.draw_scale;
        let logical_w = width / s;
        let logical_h = height / s;

        canvas.save();
        canvas.scale((s, s));

        // Draw background and border at logical (unscaled) dimensions
        Self::draw_background(canvas, style, logical_w, logical_h);

        // The list scrolls inside the menu's box, so it is clipped to it and
        // slid up by the offset. The clip is the menu's own rounded shape, so
        // a row passing the top or bottom edge is cut by the corner rather
        // than spilling square out of it.
        let scroll = scroll.clamp(0.0, Self::overflow(items, style));
        canvas.save();
        if scroll > 0.0 || style.max_height.is_some() {
            canvas.clip_rrect(
                RRect::new_rect_xy(
                    Rect::from_xywh(0.0, 0.0, logical_w, logical_h),
                    style.corner_radius,
                    style.corner_radius,
                ),
                None,
                Some(true),
            );
        }
        canvas.translate((0.0, -scroll));

        // Draw menu items with states
        Self::draw_items_with_selection(
            canvas, items, selected, style, logical_w, scroll, logical_h,
        );

        canvas.restore();

        // The arrows go on last, over the list: they mark which way there is
        // more to see, and a row sliding under one has to disappear behind it
        // rather than through it.
        let overflow = Self::overflow(items, style);
        if overflow > 0.0 {
            if scroll > 0.5 {
                Self::draw_scroll_arrow(canvas, style, logical_w, 0.0, true);
            }
            if scroll < overflow - 0.5 {
                Self::draw_scroll_arrow(canvas, style, logical_w, logical_h - ARROW_STRIP, false);
            }
        }

        canvas.restore();
    }

    /// One end's scroll affordance: a strip in the menu's own colour with a
    /// chevron in it, hiding the row that runs under it and pointing at the
    /// rows past the edge.
    fn draw_scroll_arrow(
        canvas: &Canvas,
        style: &ContextMenuStyle,
        width: f32,
        top: f32,
        up: bool,
    ) {
        let strip = Rect::from_xywh(0.0, top, width, ARROW_STRIP);
        let mut fill = Paint::default();
        fill.set_color(style.background_color());
        fill.set_anti_alias(true);
        canvas.draw_rect(strip, &fill);

        let mut chevron = Paint::default();
        chevron.set_color(style.theme.text_secondary);
        chevron.set_anti_alias(true);
        chevron.set_style(skia_safe::paint::Style::Stroke);
        chevron.set_stroke_width(1.4);
        chevron.set_stroke_cap(skia_safe::PaintCap::Round);
        chevron.set_stroke_join(skia_safe::PaintJoin::Round);

        let cx = width / 2.0;
        let cy = top + ARROW_STRIP / 2.0;
        let (half_w, half_h) = (4.5, 2.5);
        let tip_y = if up { cy - half_h } else { cy + half_h };
        let base_y = if up { cy + half_h } else { cy - half_h };
        let mut path = skia_safe::PathBuilder::new();
        path.move_to(skia_safe::Point::new(cx - half_w, base_y));
        path.line_to(skia_safe::Point::new(cx, tip_y));
        path.line_to(skia_safe::Point::new(cx + half_w, base_y));
        canvas.draw_path(&path.detach(), &chevron);
    }

    /// Draw menu background and border
    fn draw_background(canvas: &Canvas, style: &ContextMenuStyle, width: f32, height: f32) {
        let popup_rect = RRect::new_rect_xy(
            Rect::from_xywh(0.0, 0.0, width, height),
            style.corner_radius,
            style.corner_radius,
        );

        // Draw background
        let mut bg_paint = Paint::default();
        bg_paint.set_color(style.background_color());
        bg_paint.set_anti_alias(true);
        canvas.draw_rrect(popup_rect, &bg_paint);

        // Draw border
        let mut border_paint = Paint::default();
        border_paint.set_color(style.border_color());
        border_paint.set_style(skia_safe::paint::Style::Stroke);
        border_paint.set_stroke_width(style.border_width);
        border_paint.set_anti_alias(true);
        canvas.draw_rrect(popup_rect, &border_paint);
    }

    /// Draw items with explicit selection (for depth-specific rendering)
    /// Draw the rows, hover state applied, clipped to what the box can show.
    ///
    /// `scroll` is how far the list has slid up — the canvas is already
    /// translated by it — and `visible_height` the box it slides inside.
    /// Rows outside that band are not built at all: a menu listing every
    /// installed font is well over a thousand rows while a dozen are on
    /// screen, and each row costs a clone and a text layout whether or not
    /// the clip goes on to throw it away. Culling here is what keeps such a
    /// menu openable rather than a stall on every frame it is up.
    fn draw_items_with_selection(
        canvas: &Canvas,
        items: &[MenuItem],
        selected: Option<usize>,
        style: &ContextMenuStyle,
        width: f32,
        scroll: f32,
        visible_height: f32,
    ) {
        // Save canvas state and translate for padding
        canvas.save();
        canvas.translate((style.horizontal_padding, style.vertical_padding));

        let (offset, visible) = Self::visible_items(items, selected, style, scroll, visible_height);

        // Render using MenuItemGroup, offset to where the first drawn row
        // actually sits in the list rather than at the list's own origin.
        MenuItemGroup::new()
            .at(0.0, offset)
            .with_width(width - style.horizontal_padding * 2.0)
            .with_style(style.item_style())
            .items(visible)
            .render(canvas);

        // Restore canvas state
        canvas.restore();
    }

    /// The rows intersecting the visible band, and the y the first of them
    /// sits at in the list's own coordinates.
    ///
    /// Split out from the drawing so the culling can be tested without a
    /// canvas: the arithmetic is where an off-by-one shows up as a row that
    /// blinks out at the edge of a scroll.
    fn visible_items(
        items: &[MenuItem],
        selected: Option<usize>,
        style: &ContextMenuStyle,
        scroll: f32,
        visible_height: f32,
    ) -> (f32, Vec<MenuItem>) {
        let top = scroll;
        let bottom = scroll + visible_height;

        let mut y = 0.0_f32;
        let mut offset = 0.0_f32;
        let mut visible: Vec<MenuItem> = Vec::new();
        for (i, item_data) in items.iter().enumerate() {
            let height = style.item_height_of(item_data);
            if y + height >= top && y <= bottom {
                if visible.is_empty() {
                    offset = y;
                }
                let mut data = item_data.clone();
                data.height = height;
                if Some(i) == selected {
                    data.set_visual_state(VisualState::Hovered);
                }
                visible.push(data);
            }
            y += height;
        }
        (offset, visible)
    }

    /// Hit test to determine which menu item is at the given position
    ///
    /// Returns the index of the item at (x, y), or None if outside menu bounds.
    /// Considers padding and returns None for separators.
    pub fn hit_test(
        state: &ContextMenuState,
        style: &ContextMenuStyle,
        x: f32,
        y: f32,
    ) -> Option<usize> {
        Self::hit_test_items(state.items(), style, x, y, state.scroll())
    }

    /// Hit test specific items (for depth-specific testing)
    ///
    /// `scroll` is how far the list is scrolled inside the menu's box: the
    /// pointer arrives in the box's coordinates, and the items have moved up
    /// by that much underneath it.
    pub fn hit_test_items(
        items: &[MenuItem],
        style: &ContextMenuStyle,
        x: f32,
        y: f32,
        scroll: f32,
    ) -> Option<usize> {
        // Use the same width logic as measure_items
        let total_width = style
            .width
            .unwrap_or_else(|| Self::compute_optimal_width(items, style).max(style.min_width));
        if x < style.horizontal_padding || x > total_width - style.horizontal_padding {
            return None;
        }

        // The pointer must be inside the *box*, which a capped menu ends
        // before its items do.
        let content_height = Self::content_height(items, style);
        let box_height = match style.max_height {
            Some(max) => content_height.min(max),
            None => content_height,
        };
        if y < style.vertical_padding || y > box_height - style.vertical_padding {
            return None;
        }

        // Into the list's own coordinates, which have slid up by `scroll`.
        let y = y + scroll.clamp(0.0, Self::overflow(items, style));

        // Calculate position relative to first item
        let mut current_y = style.vertical_padding;

        for (i, item) in items.iter().enumerate() {
            let item_bottom = current_y + style.item_height_of(item);

            if y >= current_y && y < item_bottom {
                return if item.is_separator() { None } else { Some(i) };
            }

            current_y = item_bottom;
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_state() -> ContextMenuState {
        use crate::components::menu_item::{MenuItem, MenuItemKind};

        ContextMenuState::new(vec![
            MenuItem::new(MenuItemKind::Action {
                label: "Item 1".to_string(),
                shortcut: None,
                action_id: None,
            }),
            MenuItem::new(MenuItemKind::Separator),
            MenuItem::new(MenuItemKind::Action {
                label: "Item 2".to_string(),
                shortcut: None,
                action_id: None,
            }),
        ])
    }

    #[test]
    fn test_measure() {
        let state = create_test_state();
        let style = ContextMenuStyle::default();

        let (width, height) = ContextMenuRenderer::measure(&state, &style);

        assert!(width >= style.min_width);
        assert!(height > 0.0);
    }

    /// Twenty rows of 20pt in a box capped at 100pt: only a few fit, so the
    /// arrows walking the selection have to drag the list along.
    fn scrolling_menu() -> (Vec<MenuItem>, ContextMenuStyle) {
        use crate::components::menu_item::MenuItemKind;

        let items: Vec<MenuItem> = (0..20)
            .map(|i| {
                MenuItem::new(MenuItemKind::Action {
                    label: format!("Item {i}"),
                    shortcut: None,
                    action_id: None,
                })
            })
            .collect();
        let style = ContextMenuStyle::default()
            .with_item_metrics(13.0, 20.0)
            .with_max_height(100.0);
        (items, style)
    }

    #[test]
    fn only_the_rows_in_the_box_are_built() {
        let (items, style) = scrolling_menu();

        // Twenty 20pt rows in a 100pt box: five fit, plus the ones straddling
        // the edges. Nothing near a list's worth.
        let (offset, visible) =
            ContextMenuRenderer::visible_items(&items, None, &style, 0.0, 100.0);
        assert_eq!(offset, 0.0);
        assert!(visible.len() < items.len());
        assert!(visible.len() >= 5, "the box should be full");

        // Scrolled down, the drawn rows start further down the list and are
        // offset to where they actually sit, so they land under the pointer
        // where the hit test says they are. The row straddling the top edge
        // is drawn too — half of it is on screen.
        let (offset, scrolled) =
            ContextMenuRenderer::visible_items(&items, None, &style, 200.0, 100.0);
        assert_eq!(offset, 180.0);
        assert!(scrolled.len() <= visible.len() + 1);
        assert!(scrolled.len() >= 5);
    }

    #[test]
    fn an_uncapped_menu_still_draws_all_of_itself() {
        let (items, _) = scrolling_menu();
        let style = ContextMenuStyle::default().with_item_metrics(13.0, 20.0);
        let height = ContextMenuRenderer::content_height(&items, &style);
        let (offset, visible) =
            ContextMenuRenderer::visible_items(&items, None, &style, 0.0, height);
        assert_eq!(offset, 0.0);
        assert_eq!(visible.len(), items.len());
    }

    #[test]
    fn reveal_leaves_a_visible_row_alone() {
        let (items, style) = scrolling_menu();
        // Row 2 sits inside the box while it is at the top.
        assert_eq!(
            ContextMenuRenderer::scroll_to_reveal(&items, &style, 2, 0.0),
            0.0
        );
    }

    #[test]
    fn reveal_follows_the_selection_down_and_back_up() {
        let (items, style) = scrolling_menu();

        // Walking off the bottom scrolls just far enough to bring the row in.
        let down = ContextMenuRenderer::scroll_to_reveal(&items, &style, 10, 0.0);
        assert!(down > 0.0);
        assert!(
            ContextMenuRenderer::hit_test_items(&items, &style, 20.0, 50.0, down).is_some(),
            "the box should be showing rows once scrolled"
        );
        // ...and the row is now inside it, so a second call is a no-op.
        assert_eq!(
            ContextMenuRenderer::scroll_to_reveal(&items, &style, 10, down),
            down
        );

        // Coming back up scrolls the other way, never past the top.
        let up = ContextMenuRenderer::scroll_to_reveal(&items, &style, 1, down);
        assert!(up < down);
        assert_eq!(
            ContextMenuRenderer::scroll_to_reveal(&items, &style, 0, up),
            0.0
        );
    }

    #[test]
    fn reveal_stays_within_what_there_is_to_scroll() {
        let (items, style) = scrolling_menu();
        let overflow = ContextMenuRenderer::overflow(&items, &style);

        assert_eq!(
            ContextMenuRenderer::scroll_to_reveal(&items, &style, 19, 0.0),
            overflow
        );

        // A list that fits has nothing to scroll, whatever is selected.
        let short = ContextMenuStyle::default().with_item_metrics(13.0, 20.0);
        assert_eq!(
            ContextMenuRenderer::scroll_to_reveal(&items, &short, 19, 0.0),
            0.0
        );
    }

    #[test]
    fn test_measure_with_custom_width() {
        let state = create_test_state();
        let style = ContextMenuStyle::default().with_width(300.0);

        let (width, _height) = ContextMenuRenderer::measure(&state, &style);

        assert_eq!(width, 300.0);
    }
}
