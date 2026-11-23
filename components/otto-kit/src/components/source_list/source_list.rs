// Rust guideline compliant 2026-02-21

use skia_safe::{Canvas, Color, Contains, Paint, Point, RRect, Rect};

use crate::theme::Theme;
use crate::typography::{ellipsize, styles, TextStyle};

/// One row's content: a title, an optional second line under it, and how many
/// small trailing glyphs sit at the row's right end.
///
/// The glyphs themselves are not here: the caller paints them, see [`draw`].
#[derive(Debug, Clone, Default)]
pub struct SourceListItem {
    pub label: String,
    /// A secondary line under the label, such as "2 in · 2 out". A row with
    /// one is a two-line row: taller, with a larger leading icon.
    pub subtitle: Option<String>,
    /// How many trailing glyphs the row carries, right-aligned and drawn
    /// right to left from [`GlyphSlot::Trailing`]`(0)`.
    pub trailing: usize,
}

impl SourceListItem {
    /// A one-line row titled `label`.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ..Self::default()
        }
    }

    /// Adds a secondary line under the label, making this a two-line row.
    #[must_use]
    pub fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Reserves `count` trailing glyph slots at the row's right end.
    #[must_use]
    pub fn with_trailing(mut self, count: usize) -> Self {
        self.trailing = count;
        self
    }

    /// Whether the row shows a subtitle under its label.
    pub fn is_two_line(&self) -> bool {
        self.subtitle.is_some()
    }
}

/// Which glyph of a row the draw callback is asked to paint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphSlot {
    /// The icon before the label.
    Leading,
    /// The `n`th trailing glyph, counted from the row's right end.
    Trailing(usize),
}

/// How large the list's rows are.
///
/// `Regular` is the compact sidebar; `Large` is for a window whose sidebar is
/// a handful of destinations, where taller rows and bigger glyphs read better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceListSize {
    #[default]
    Regular,
    Large,
}

/// Every measurement of a source list at one [`SourceListSize`].
#[derive(Debug, Clone, Copy)]
pub struct SourceListMetrics {
    /// A one-line row's height.
    pub item_height: f32,
    /// A two-line row's height.
    pub two_line_height: f32,
    /// Vertical space between rows.
    pub item_gap: f32,
    /// Horizontal inset of the rows from the list's own bounds.
    pub item_inset: f32,
    /// The selection pill's corner radius.
    pub corner_radius: f32,
    /// A one-line row's leading icon side.
    pub icon_size: f32,
    /// A two-line row's leading icon side.
    pub two_line_icon_size: f32,
    /// Space between the row's left edge and a one-line row's icon.
    pub leading_pad: f32,
    /// Space between the icon and the label.
    pub icon_label_gap: f32,
    /// A trailing glyph's side.
    pub trailing_size: f32,
    /// Space between two trailing glyphs.
    pub trailing_gap: f32,
    /// Space between the row's right edge and its last trailing glyph, and
    /// the least between a label and what follows it.
    pub trailing_pad: f32,
    /// The label, and the label of the selected row.
    pub title: TextStyle,
    pub title_selected: TextStyle,
    /// The subtitle of a two-line row.
    pub subtitle: TextStyle,
}

const REGULAR: SourceListMetrics = SourceListMetrics {
    item_height: 30.0,
    two_line_height: 42.0,
    item_gap: 2.0,
    item_inset: 8.0,
    corner_radius: 7.0,
    icon_size: 20.0,
    two_line_icon_size: 28.0,
    leading_pad: 7.0,
    icon_label_gap: 6.0,
    trailing_size: 12.0,
    trailing_gap: 4.0,
    trailing_pad: 8.0,
    title: styles::SUBHEADLINE,
    title_selected: styles::SUBHEADLINE_EMPHASIZED,
    subtitle: styles::FOOTNOTE,
};

const LARGE: SourceListMetrics = SourceListMetrics {
    item_height: 38.0,
    two_line_height: 48.0,
    item_gap: 2.0,
    item_inset: 8.0,
    corner_radius: 9.0,
    icon_size: 20.0,
    two_line_icon_size: 32.0,
    leading_pad: 10.0,
    icon_label_gap: 10.0,
    trailing_size: 14.0,
    trailing_gap: 6.0,
    trailing_pad: 10.0,
    title: TextStyle {
        family: "Inter",
        weight: 500,
        size: 14.0,
    },
    title_selected: TextStyle {
        family: "Inter",
        weight: 600,
        size: 14.0,
    },
    subtitle: styles::SUBHEADLINE,
};

impl SourceListSize {
    /// The measurements rows of this size are laid out and drawn with.
    pub const fn metrics(self) -> &'static SourceListMetrics {
        match self {
            Self::Regular => &REGULAR,
            Self::Large => &LARGE,
        }
    }
}

impl SourceListMetrics {
    /// A row's height.
    pub fn row_height(&self, two_line: bool) -> f32 {
        if two_line {
            self.two_line_height
        } else {
            self.item_height
        }
    }

    /// The leading icon's square inside `item`, centred vertically.
    pub fn icon_rect(&self, item: Rect, two_line: bool) -> Rect {
        let side = if two_line {
            self.two_line_icon_size
        } else {
            self.icon_size
        };
        let left = item.left + self.icon_left_pad(two_line);
        Rect::from_xywh(left, item.center_y() - side / 2.0, side, side)
    }

    /// Where the label starts inside `item`.
    pub fn label_x(&self, item: Rect, two_line: bool) -> f32 {
        self.icon_rect(item, two_line).right + self.icon_label_gap
    }

    /// The square of trailing glyph `n`, counted from the right end of `item`.
    pub fn trailing_rect(&self, item: Rect, n: usize) -> Rect {
        let right =
            item.right - self.trailing_pad - n as f32 * (self.trailing_size + self.trailing_gap);
        let half = self.trailing_size / 2.0;
        Rect::from_ltrb(
            right - self.trailing_size,
            item.center_y() - half,
            right,
            item.center_y() + half,
        )
    }

    /// A two-line row's icon sits as far from the left as from the top, so
    /// the larger icon keeps even margins around it.
    fn icon_left_pad(&self, two_line: bool) -> f32 {
        if two_line {
            (self.two_line_height - self.two_line_icon_size) / 2.0
        } else {
            self.leading_pad
        }
    }
}

/// Geometry of the list, computed once and shared by [`draw`], hit-testing,
/// the keyboard and accessibility.
#[derive(Debug, Clone)]
pub struct SourceListLayout {
    pub item_rects: Vec<Rect>,
    /// Which rows are two-line, by index.
    pub two_line: Vec<bool>,
    pub size: SourceListSize,
}

impl SourceListLayout {
    /// Lay out `count` one-line items of the regular size in a list of
    /// `width`, top-left at `(x, y)`.
    pub fn compute(count: usize, x: f32, y: f32, width: f32) -> Self {
        Self::build(SourceListSize::Regular, vec![false; count], x, y, width)
    }

    /// Lay out `items` at `size` in a list of `width`, top-left at `(x, y)`.
    /// A row with a subtitle gets the two-line height.
    pub fn for_items(
        size: SourceListSize,
        items: &[SourceListItem],
        x: f32,
        y: f32,
        width: f32,
    ) -> Self {
        let two_line = items.iter().map(SourceListItem::is_two_line).collect();
        Self::build(size, two_line, x, y, width)
    }

    fn build(size: SourceListSize, two_line: Vec<bool>, x: f32, y: f32, width: f32) -> Self {
        let m = size.metrics();
        let mut top = y;
        let item_rects = two_line
            .iter()
            .map(|&two| {
                let h = m.row_height(two);
                let rect = Rect::from_xywh(x + m.item_inset, top, width - m.item_inset * 2.0, h);
                top += h + m.item_gap;
                rect
            })
            .collect();
        Self {
            item_rects,
            two_line,
            size,
        }
    }

    /// The measurements this layout was made with.
    pub fn metrics(&self) -> &'static SourceListMetrics {
        self.size.metrics()
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

    /// The leading icon's square for row `index`.
    pub fn icon_rect(&self, index: usize) -> Option<Rect> {
        let item = *self.item_rects.get(index)?;
        Some(self.metrics().icon_rect(item, self.is_two_line(index)))
    }

    /// Draws the keyboard focus ring around row `index`, following the
    /// selection pill's shape.
    pub fn draw_focus_ring(&self, canvas: &Canvas, index: usize) {
        if let Some(item) = self.item_rects.get(index) {
            crate::focus::draw_focus_ring(canvas, *item, self.metrics().corner_radius);
        }
    }

    fn is_two_line(&self, index: usize) -> bool {
        self.two_line.get(index).copied().unwrap_or(false)
    }
}

/// Square the leading icon of a regular one-line row paints into, centred
/// vertically in `item`.
pub fn icon_rect(item: Rect) -> Rect {
    REGULAR.icon_rect(item, false)
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

/// The leading icon's colour: the accent, so the sidebar's glyphs carry the
/// user's colour, or white on the selected fill.
pub fn icon_tint(theme: &Theme, selected: bool) -> Color {
    if selected {
        Color::WHITE
    } else {
        theme.accent
    }
}

/// The subtitle's colour: secondary text, or a softened white on the
/// selected fill.
fn subtitle_tint(theme: &Theme, selected: bool) -> Color {
    if selected {
        Color::from_argb(0xC8, 0xFF, 0xFF, 0xFF)
    } else {
        theme.text_secondary
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
    max_w: f32,
    style: TextStyle,
    color: Color,
) {
    use crate::common::Renderable;
    let text = ellipsize(&style.font(), text, max_w.max(0.0));
    crate::components::label::Label::new(text)
        .with_style(style)
        .with_color(color)
        .centered_on(x, cy)
        .render(canvas);
}

/// Draw the list: the selected row's filled highlight, then every item's
/// glyphs, label and subtitle tinted to contrast against it.
///
/// `glyph` is called for each row's leading icon and each of its trailing
/// glyphs, with the square to paint into and its tint: [`icon_tint`] for the
/// leading icon, [`item_tint`] for trailing glyphs. The
/// component owns no glyph set, only where each goes and what colour it is.
pub fn draw(
    canvas: &Canvas,
    layout: &SourceListLayout,
    items: &[SourceListItem],
    selected: Option<usize>,
    theme: &Theme,
    mut glyph: impl FnMut(&Canvas, usize, GlyphSlot, Rect, Color),
) {
    let m = layout.metrics();
    for (i, (item, rect)) in items.iter().zip(&layout.item_rects).enumerate() {
        let is_selected = selected == Some(i);
        if is_selected {
            canvas.draw_rrect(
                RRect::new_rect_xy(*rect, m.corner_radius, m.corner_radius),
                &fill(theme.material_selection_focused),
            );
        }

        let two_line = item.is_two_line();
        let tint = item_tint(theme, is_selected);
        glyph(
            canvas,
            i,
            GlyphSlot::Leading,
            m.icon_rect(*rect, two_line),
            icon_tint(theme, is_selected),
        );
        for n in 0..item.trailing {
            glyph(
                canvas,
                i,
                GlyphSlot::Trailing(n),
                m.trailing_rect(*rect, n),
                tint,
            );
        }

        let x = m.label_x(*rect, two_line);
        let text_right = if item.trailing > 0 {
            m.trailing_rect(*rect, item.trailing - 1).left - m.trailing_gap
        } else {
            rect.right - m.trailing_pad
        };
        let title = if is_selected {
            m.title_selected
        } else {
            m.title
        };
        match &item.subtitle {
            None => text_centered_y(
                canvas,
                &item.label,
                x,
                rect.center_y(),
                text_right - x,
                title,
                tint,
            ),
            Some(subtitle) => {
                // The two lines, at 1.2 line height and 1pt apart, centred
                // as a block in the row.
                let block = 1.2 * (title.size + m.subtitle.size) + 1.0;
                let top = rect.center_y() - block / 2.0;
                let bottom = rect.center_y() + block / 2.0;
                text_centered_y(
                    canvas,
                    &item.label,
                    x,
                    top + 0.6 * title.size,
                    text_right - x,
                    title,
                    tint,
                );
                text_centered_y(
                    canvas,
                    subtitle,
                    x,
                    bottom - 0.6 * m.subtitle.size,
                    text_right - x,
                    m.subtitle,
                    subtitle_tint(theme, is_selected),
                );
            }
        }
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
    fn regular_layout_keeps_its_rows() {
        let layout = SourceListLayout::compute(3, 0.0, 10.0, 200.0);
        assert_eq!(layout.item_rects.len(), 3);
        assert_eq!(layout.item_rects[0].top, 10.0);
        assert_eq!(layout.item_rects[0].height(), 30.0);
        assert_eq!(layout.item_rects[1].top - layout.item_rects[0].top, 32.0);
        assert_eq!(layout.item_rects[0].left, 8.0);
        assert_eq!(layout.height(), layout.item_rects[2].bottom);
    }

    #[test]
    fn large_rows_are_taller_and_two_line_rows_taller_still() {
        let items = vec![
            SourceListItem::new("Graph"),
            SourceListItem::new("Speakers").with_subtitle("2 in · 2 out"),
        ];
        let layout = SourceListLayout::for_items(SourceListSize::Large, &items, 0.0, 0.0, 200.0);
        let m = SourceListSize::Large.metrics();
        assert_eq!(layout.item_rects[0].height(), m.item_height);
        assert_eq!(layout.item_rects[1].height(), m.two_line_height);
        assert_eq!(layout.item_rects[1].top, m.item_height + m.item_gap);
        assert_eq!(layout.icon_rect(1).unwrap().width(), m.two_line_icon_size);
        assert_eq!(layout.icon_rect(0).unwrap().width(), m.icon_size);
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
        let item = Rect::from_xywh(0.0, 0.0, 200.0, 30.0);
        let icon = icon_rect(item);
        assert_eq!(icon.width(), 20.0);
        assert_eq!(icon.height(), 20.0);
        assert_eq!(icon.center_y(), item.center_y());
        assert_eq!(icon.center_x(), 17.0);
    }

    #[test]
    fn trailing_glyphs_run_right_to_left_inside_the_row() {
        let m = SourceListSize::Large.metrics();
        let item = Rect::from_xywh(0.0, 0.0, 200.0, m.item_height);
        let first = m.trailing_rect(item, 0);
        let second = m.trailing_rect(item, 1);
        assert_eq!(first.right, item.right - m.trailing_pad);
        assert!(second.right <= first.left);
        assert_eq!(first.center_y(), item.center_y());
    }

    #[test]
    fn selected_tint_is_white() {
        let theme = Theme::light();
        assert_eq!(item_tint(&theme, true), Color::WHITE);
        assert_eq!(item_tint(&theme, false), theme.text_primary);
        assert_eq!(icon_tint(&theme, true), Color::WHITE);
        assert_eq!(icon_tint(&theme, false), theme.accent);
    }

    #[test]
    fn draw_smoke_test_does_not_panic() {
        let mut surface = skia_safe::surfaces::raster_n32_premul((200, 200)).unwrap();
        let mut items = items();
        items.push(
            SourceListItem::new("Mic")
                .with_subtitle("1 in")
                .with_trailing(2),
        );
        for size in [SourceListSize::Regular, SourceListSize::Large] {
            let layout = SourceListLayout::for_items(size, &items, 0.0, 0.0, 200.0);
            draw(
                surface.canvas(),
                &layout,
                &items,
                Some(1),
                &Theme::light(),
                |_, _, _, _, _| {},
            );
        }
    }
}
