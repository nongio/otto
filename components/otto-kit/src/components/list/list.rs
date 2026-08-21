use skia_safe::{Canvas, Color, Contains, Paint, PaintStyle, Point, RRect, Rect};

use crate::theme::Theme;
use crate::typography::styles;

/// A single row's content. Layout only needs to know whether there is a
/// detail line to pick a height; drawing paints both lines.
#[derive(Debug, Clone)]
pub struct ListRow {
    pub label: String,
    /// Secondary line under the label. `None` keeps the row single-height.
    pub detail: Option<String>,
}

impl ListRow {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

pub const ROW_HEIGHT: f32 = 42.0;
pub const ROW_HEIGHT_DETAIL: f32 = 56.0;
pub const CORNER_RADIUS: f32 = 9.0;
/// Leading padding for labels and separators. Separators are inset from the
/// leading edge by this much rather than running full width.
pub const LEADING_INSET: f32 = 14.0;
/// Trailing padding a control sits inset from the card's right edge.
pub const TRAILING_INSET: f32 = 14.0;
/// Height reserved above the card when a section title is drawn.
pub const TITLE_HEIGHT: f32 = 24.0;

pub fn row_height(row: &ListRow) -> f32 {
    if row.detail.is_some() {
        ROW_HEIGHT_DETAIL
    } else {
        ROW_HEIGHT
    }
}

/// Geometry of a card, computed once and shared by [`draw`] and hit-testing —
/// the discipline `sidebar_item_rect`/`pane_at` established in the settings
/// scaffold, formalised here.
#[derive(Debug, Clone)]
pub struct ListLayout {
    /// Section title rect, if one was reserved. `None` when there is no
    /// title, in which case the card starts at the layout's own origin.
    pub title_rect: Option<Rect>,
    pub card_rect: Rect,
    pub row_rects: Vec<Rect>,
}

impl ListLayout {
    /// Lay out `rows` in a card of `width`, top-left at `(x, y)`. Pass
    /// `has_title` when a section title will be drawn above the card — it
    /// reserves [`TITLE_HEIGHT`] without needing the title text itself.
    pub fn compute(rows: &[ListRow], has_title: bool, x: f32, y: f32, width: f32) -> Self {
        let title_rect = has_title.then(|| Rect::from_xywh(x, y, width, TITLE_HEIGHT));
        let card_top = if has_title { y + TITLE_HEIGHT } else { y };

        let mut row_rects = Vec::with_capacity(rows.len());
        let mut row_y = card_top;
        for row in rows {
            let h = row_height(row);
            row_rects.push(Rect::from_xywh(x, row_y, width, h));
            row_y += h;
        }

        Self {
            title_rect,
            card_rect: Rect::from_ltrb(x, card_top, x + width, row_y),
            row_rects,
        }
    }

    /// Total height from the layout's origin to the card's bottom, title
    /// included — what a caller adds to `y` to place whatever comes next.
    pub fn total_height(&self) -> f32 {
        self.card_rect.bottom - self.title_rect.map_or(self.card_rect.top, |t| t.top)
    }

    /// Row index under `(px, py)`, if any.
    pub fn row_at(&self, px: f32, py: f32) -> Option<usize> {
        self.row_rects
            .iter()
            .position(|r| r.contains(Point::new(px, py)))
    }
}

/// Trailing-edge rect inside `row`, `width` wide, inset from the right edge
/// and spanning the row's full height — where a toggle, slider, or other
/// control paints itself. The caller centres its control within it.
pub fn trailing_rect(row: Rect, width: f32) -> Rect {
    Rect::from_xywh(
        row.right - TRAILING_INSET - width,
        row.top,
        width,
        row.height(),
    )
}

/// The card fill used by the settings scaffold: near-opaque white in light
/// mode, a faint white wash over the dark material in dark mode. A starting
/// point, not a mandate — callers with their own material may pass their own
/// background to [`draw`] instead.
pub fn default_card_background(dark: bool) -> Color {
    if dark {
        Color::from_argb(0x14, 0xFF, 0xFF, 0xFF)
    } else {
        Color::WHITE
    }
}

fn fill(color: Color) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(color);
    paint
}

fn stroke(color: Color, width: f32) -> Paint {
    let mut paint = fill(color);
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(width);
    paint
}

/// Text drawn so its optical centre sits on `cy`.
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

/// Draw the card: background, hairline border, section title (if any),
/// labels, detail lines, and inset separators between rows. `background`
/// lets the caller pick its own material — see [`default_card_background`]
/// for the settings scaffold's choice.
///
/// `paint_trailing` is called once per row with that row's full-height rect;
/// the caller derives its own control rect from it, typically via
/// [`trailing_rect`], and paints whatever belongs there. Rows carry no
/// notion of "the" control on purpose — this is how a toggle and a slider
/// coexist in the same list without the component knowing either exists.
pub fn draw(
    canvas: &Canvas,
    layout: &ListLayout,
    rows: &[ListRow],
    title: Option<&str>,
    theme: &Theme,
    background: Color,
    mut paint_trailing: impl FnMut(&Canvas, usize, Rect),
) {
    if let (Some(title), Some(rect)) = (title, layout.title_rect) {
        text_centered_y(
            canvas,
            title,
            rect.left + 2.0,
            rect.top + 9.0,
            styles::FOOTNOTE_EMPHASIZED,
            theme.text_secondary,
        );
    }

    let rrect = RRect::new_rect_xy(layout.card_rect, CORNER_RADIUS, CORNER_RADIUS);
    canvas.draw_rrect(rrect, &fill(background));
    canvas.draw_rrect(rrect, &stroke(theme.fill_tertiary, 1.0));

    for (i, (row, rect)) in rows.iter().zip(&layout.row_rects).enumerate() {
        let cy = rect.center_y();
        let label_x = rect.left + LEADING_INSET;

        match &row.detail {
            Some(detail) => {
                text_centered_y(
                    canvas,
                    &row.label,
                    label_x,
                    cy - 9.0,
                    styles::SUBHEADLINE,
                    theme.text_primary,
                );
                text_centered_y(
                    canvas,
                    detail,
                    label_x,
                    cy + 9.0,
                    styles::CAPTION_1,
                    theme.text_secondary,
                );
            }
            None => text_centered_y(
                canvas,
                &row.label,
                label_x,
                cy,
                styles::SUBHEADLINE,
                theme.text_primary,
            ),
        }

        paint_trailing(canvas, i, *rect);

        if i + 1 < rows.len() {
            canvas.draw_line(
                Point::new(rect.left + LEADING_INSET, rect.bottom),
                Point::new(rect.right, rect.bottom),
                &stroke(theme.fill_tertiary, 1.0),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<ListRow> {
        vec![
            ListRow::new("Wi-Fi"),
            ListRow::new("Bluetooth").with_detail("Connected to Magic Mouse"),
            ListRow::new("VPN"),
        ]
    }

    #[test]
    fn detail_rows_are_taller() {
        assert_eq!(row_height(&ListRow::new("a")), ROW_HEIGHT);
        assert_eq!(
            row_height(&ListRow::new("a").with_detail("b")),
            ROW_HEIGHT_DETAIL
        );
    }

    #[test]
    fn layout_stacks_rows_without_gaps() {
        let rows = rows();
        let layout = ListLayout::compute(&rows, false, 10.0, 20.0, 300.0);
        assert_eq!(layout.row_rects.len(), 3);
        assert_eq!(layout.row_rects[0].top, 20.0);
        assert_eq!(layout.row_rects[1].top, layout.row_rects[0].bottom);
        assert_eq!(layout.row_rects[2].bottom, layout.card_rect.bottom);
        assert_eq!(layout.card_rect.height(), layout.total_height());
    }

    #[test]
    fn title_reserves_space_above_the_card() {
        let rows = rows();
        let layout = ListLayout::compute(&rows, true, 10.0, 20.0, 300.0);
        let title = layout.title_rect.expect("title rect");
        assert_eq!(title.top, 20.0);
        assert_eq!(layout.card_rect.top, 20.0 + TITLE_HEIGHT);
        assert_eq!(
            layout.total_height(),
            TITLE_HEIGHT + layout.card_rect.height()
        );
    }

    #[test]
    fn row_at_matches_row_rects() {
        let rows = rows();
        let layout = ListLayout::compute(&rows, false, 0.0, 0.0, 300.0);
        for (i, rect) in layout.row_rects.iter().enumerate() {
            assert_eq!(layout.row_at(rect.left + 5.0, rect.center_y()), Some(i));
        }
        assert_eq!(layout.row_at(-5.0, 5.0), None);
        assert_eq!(layout.row_at(5.0, layout.card_rect.bottom + 5.0), None);
    }

    #[test]
    fn trailing_rect_sits_inset_from_the_right_edge() {
        let row = Rect::from_xywh(0.0, 0.0, 300.0, ROW_HEIGHT);
        let slot = trailing_rect(row, 40.0);
        assert_eq!(slot.width(), 40.0);
        assert_eq!(slot.right, row.right - TRAILING_INSET);
        assert_eq!(slot.height(), row.height());
    }
}
