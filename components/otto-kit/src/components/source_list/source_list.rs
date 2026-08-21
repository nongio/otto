use skia_safe::{Canvas, Color, Contains, Paint, Point, RRect, Rect};

use crate::theme::Theme;
use crate::typography::styles;

/// One row's content. The icon itself is not here — it is caller-supplied,
/// see [`draw`] — only the label the component knows how to lay out and
/// paint.
#[derive(Debug, Clone)]
pub struct SourceListItem {
    pub label: String,
}

impl SourceListItem {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
        }
    }
}

pub const ITEM_HEIGHT: f32 = 30.0;
pub const ITEM_STEP: f32 = 32.0;
/// Horizontal and top/bottom inset of the row from the list's own bounds.
pub const ITEM_INSET: f32 = 8.0;
pub const CORNER_RADIUS: f32 = 7.0;
/// Icon box side length, centred inside the row.
pub const ICON_SIZE: f32 = 20.0;
/// Distance from the icon box's centre to the row's left edge.
const ICON_CENTER_X: f32 = 17.0;
/// Left edge of the label, past the icon.
pub const LABEL_INSET: f32 = 33.0;

/// Geometry of the list, computed once and shared by [`draw`] and
/// hit-testing.
#[derive(Debug, Clone)]
pub struct SourceListLayout {
    pub item_rects: Vec<Rect>,
}

impl SourceListLayout {
    /// Lay out `count` items in a list of `width`, top-left at `(x, y)`.
    pub fn compute(count: usize, x: f32, y: f32, width: f32) -> Self {
        let item_rects = (0..count)
            .map(|i| {
                Rect::from_xywh(
                    x + ITEM_INSET,
                    y + i as f32 * ITEM_STEP,
                    width - ITEM_INSET * 2.0,
                    ITEM_HEIGHT,
                )
            })
            .collect();
        Self { item_rects }
    }

    /// Total height from the layout's origin to the last item's bottom.
    pub fn height(&self) -> f32 {
        self.item_rects.last().map_or(0.0, |r| r.bottom)
    }

    /// Item index under `(px, py)`, if any.
    pub fn item_at(&self, px: f32, py: f32) -> Option<usize> {
        self.item_rects
            .iter()
            .position(|r| r.contains(Point::new(px, py)))
    }
}

/// Square the icon closure paints into, centred vertically in `item` at
/// [`ICON_CENTER_X`] from its left edge.
pub fn icon_rect(item: Rect) -> Rect {
    let half = ICON_SIZE / 2.0;
    let cy = item.center_y();
    Rect::from_ltrb(
        item.left + ICON_CENTER_X - half,
        cy - half,
        item.left + ICON_CENTER_X + half,
        cy + half,
    )
}

/// Icon and label colour for a row: white on the selected fill, the theme's
/// primary text otherwise.
pub fn item_tint(theme: &Theme, selected: bool) -> Color {
    if selected {
        Color::WHITE
    } else {
        theme.text_primary
    }
}

fn fill(color: Color) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(color);
    paint
}

fn text_centered_y(
    canvas: &Canvas,
    text: &str,
    x: f32,
    cy: f32,
    style: crate::typography::TextStyle,
    color: Color,
) {
    use crate::common::Renderable;
    crate::components::label::Label::new(text)
        .with_style(style)
        .with_color(color)
        .centered_on(x, cy)
        .render(canvas);
}

/// Draw the list: the selected row's filled highlight, then every item's
/// icon and label tinted to contrast against it.
///
/// `icon` is called once per item with the square from [`icon_rect`] and the
/// tint from [`item_tint`] — the component owns none of the glyph set (see
/// `otto-settings`'s own `glyphs` module), only where it goes and what
/// colour it should be.
pub fn draw(
    canvas: &Canvas,
    layout: &SourceListLayout,
    items: &[SourceListItem],
    selected: Option<usize>,
    theme: &Theme,
    mut icon: impl FnMut(&Canvas, usize, Rect, Color),
) {
    for (i, (item, rect)) in items.iter().zip(&layout.item_rects).enumerate() {
        let is_selected = selected == Some(i);
        if is_selected {
            canvas.draw_rrect(
                RRect::new_rect_xy(*rect, CORNER_RADIUS, CORNER_RADIUS),
                &fill(theme.material_selection_focused),
            );
        }

        let tint = item_tint(theme, is_selected);
        icon(canvas, i, icon_rect(*rect), tint);

        text_centered_y(
            canvas,
            &item.label,
            rect.left + LABEL_INSET,
            rect.center_y(),
            if is_selected {
                styles::SUBHEADLINE_EMPHASIZED
            } else {
                styles::SUBHEADLINE
            },
            tint,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<SourceListItem> {
        vec![
            SourceListItem::new("General"),
            SourceListItem::new("Displays"),
            SourceListItem::new("Dock"),
        ]
    }

    #[test]
    fn layout_steps_items_evenly() {
        let layout = SourceListLayout::compute(3, 0.0, 10.0, 200.0);
        assert_eq!(layout.item_rects.len(), 3);
        assert_eq!(layout.item_rects[0].top, 10.0);
        assert_eq!(
            layout.item_rects[1].top - layout.item_rects[0].top,
            ITEM_STEP
        );
        assert_eq!(layout.height(), layout.item_rects[2].bottom);
    }

    #[test]
    fn item_at_matches_item_rects() {
        let layout = SourceListLayout::compute(3, 0.0, 0.0, 200.0);
        for (i, rect) in layout.item_rects.iter().enumerate() {
            assert_eq!(layout.item_at(rect.center_x(), rect.center_y()), Some(i));
        }
        assert_eq!(layout.item_at(-5.0, 5.0), None);
    }

    #[test]
    fn icon_rect_is_centred_in_the_item() {
        let item = Rect::from_xywh(0.0, 0.0, 200.0, ITEM_HEIGHT);
        let icon = icon_rect(item);
        assert_eq!(icon.width(), ICON_SIZE);
        assert_eq!(icon.height(), ICON_SIZE);
        assert_eq!(icon.center_y(), item.center_y());
    }

    #[test]
    fn selected_tint_is_white() {
        let theme = Theme::light();
        assert_eq!(item_tint(&theme, true), Color::WHITE);
        assert_eq!(item_tint(&theme, false), theme.text_primary);
    }

    #[test]
    fn draw_smoke_test_does_not_panic() {
        let mut surface = skia_safe::surfaces::raster_n32_premul((100, 100)).unwrap();
        let items = items();
        let layout = SourceListLayout::compute(items.len(), 0.0, 0.0, 100.0);
        draw(
            surface.canvas(),
            &layout,
            &items,
            Some(1),
            &Theme::light(),
            |_, _, _, _| {},
        );
    }
}
