// Rust guideline compliant 2026-02-21

use skia_safe::{Canvas, ClipOp, Color, Contains, Paint, PaintStyle, Point, RRect, Rect};

use crate::theme::Theme;
use crate::typography::styles;

/// An item's content: a title and an optional line under it.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionListItem {
    pub title: String,
    /// Secondary line under the title, such as a role.
    pub subtitle: Option<String>,
}

impl SelectionListItem {
    /// An item with only a title.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
        }
    }

    /// The same item with `subtitle` under its title.
    #[must_use]
    pub fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }
}

/// What a press on the list landed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionListHit {
    /// The item at this index.
    Item(usize),
    /// The footer's add (+) button.
    Add,
    /// The footer's remove (−) button.
    Remove,
}

/// What the list shows besides its items: selection, enabled buttons, press.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SelectionListState {
    pub selected: Option<usize>,
    /// Whether the add button does anything; drawn dimmed when not.
    pub can_add: bool,
    /// Whether the remove button does anything; drawn dimmed when not.
    pub can_remove: bool,
    /// The part held down by the pointer, drawn pressed.
    pub pressed: Option<SelectionListHit>,
}

pub const ITEM_HEIGHT: f32 = 46.0;
/// Distance from one item's top to the next one's: a 2pt gap between them.
pub const ITEM_STEP: f32 = 48.0;
/// Inset of the items from the card's edges.
const ITEM_INSET: f32 = 6.0;
const ITEM_RADIUS: f32 = 8.0;
const CARD_RADIUS: f32 = 12.0;
/// Side of the square the leading closure paints into.
pub const LEADING_SIZE: f32 = 32.0;
/// Gap from the item's left edge to the leading square, and from the square
/// to the text.
const LEADING_GAP: f32 = 8.0;
pub const FOOTER_HEIGHT: f32 = 34.0;
const FOOTER_BUTTON_W: f32 = 30.0;
const FOOTER_BUTTON_H: f32 = 26.0;
/// Length of the strokes of the + and − glyphs.
const GLYPH: f32 = 11.0;

/// Geometry of the list, computed once and shared by [`draw`] and hit-testing.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionListLayout {
    /// The whole card, footer included.
    pub card: Rect,
    /// One rect per item. Items past the footer are laid out but neither
    /// drawn nor hit: the list does not scroll, so size the card for them.
    pub item_rects: Vec<Rect>,
    pub footer: Rect,
    pub add: Rect,
    pub remove: Rect,
}

impl SelectionListLayout {
    /// Lay out `count` items in a card filling `bounds`.
    pub fn compute(count: usize, bounds: Rect) -> Self {
        let footer = Rect::from_ltrb(
            bounds.left,
            bounds.bottom - FOOTER_HEIGHT,
            bounds.right,
            bounds.bottom,
        );
        let item_rects = (0..count)
            .map(|i| {
                Rect::from_xywh(
                    bounds.left + ITEM_INSET,
                    bounds.top + ITEM_INSET + i as f32 * ITEM_STEP,
                    bounds.width() - ITEM_INSET * 2.0,
                    ITEM_HEIGHT,
                )
            })
            .collect();
        let button = |i: f32| {
            Rect::from_xywh(
                footer.left + ITEM_INSET + i * (FOOTER_BUTTON_W + 2.0),
                footer.center_y() - FOOTER_BUTTON_H / 2.0,
                FOOTER_BUTTON_W,
                FOOTER_BUTTON_H,
            )
        };
        Self {
            card: bounds,
            item_rects,
            footer,
            add: button(0.0),
            remove: button(1.0),
        }
    }

    /// The height a card needs to show `count` items without clipping.
    pub fn height_for(count: usize) -> f32 {
        ITEM_INSET * 2.0 + count as f32 * ITEM_STEP + FOOTER_HEIGHT
    }

    /// What is under `(px, py)`, if anything that responds to a press.
    pub fn hit(&self, px: f32, py: f32) -> Option<SelectionListHit> {
        let point = Point::new(px, py);
        if self.add.contains(point) {
            return Some(SelectionListHit::Add);
        }
        if self.remove.contains(point) {
            return Some(SelectionListHit::Remove);
        }
        self.item_rects
            .iter()
            .position(|rect| rect.contains(point) && rect.bottom <= self.footer.top)
            .map(SelectionListHit::Item)
    }
}

/// The square the leading closure paints into, centered vertically in `item`.
pub fn leading_rect(item: Rect) -> Rect {
    Rect::from_xywh(
        item.left + LEADING_GAP,
        item.center_y() - LEADING_SIZE / 2.0,
        LEADING_SIZE,
        LEADING_SIZE,
    )
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
    paint.set_stroke_cap(skia_safe::paint::Cap::Round);
    paint
}

fn text(
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

/// `text` cut with an ellipsis to fit `room` points in `style`.
fn crop(text: &str, style: crate::typography::TextStyle, room: f32) -> String {
    let font = style.font();
    if font.measure_str(text, None).0 <= room {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + "…";
        if font.measure_str(&candidate, None).0 <= room {
            return candidate;
        }
    }
    String::new()
}

fn footer_button(
    canvas: &Canvas,
    rect: Rect,
    enabled: bool,
    pressed: bool,
    plus: bool,
    theme: &Theme,
) {
    if pressed && enabled {
        canvas.draw_rrect(
            RRect::new_rect_xy(rect, 6.0, 6.0),
            &fill(theme.fill_tertiary),
        );
    }
    let color = if enabled {
        theme.text_primary
    } else {
        theme.text_tertiary
    };
    let paint = stroke(color, 1.6);
    let (cx, cy) = (rect.center_x(), rect.center_y());
    canvas.draw_line((cx - GLYPH / 2.0, cy), (cx + GLYPH / 2.0, cy), &paint);
    if plus {
        canvas.draw_line((cx, cy - GLYPH / 2.0), (cx, cy + GLYPH / 2.0), &paint);
    }
}

/// Draw the card, its items and the add/remove footer.
///
/// `background` is the card's fill — pass the same one the surrounding
/// grouped lists use. `leading` is called once per visible item with the
/// square from [`leading_rect`]; the component owns no pictures, only where
/// they go.
pub fn draw(
    canvas: &Canvas,
    layout: &SelectionListLayout,
    items: &[SelectionListItem],
    state: &SelectionListState,
    theme: &Theme,
    background: Color,
    mut leading: impl FnMut(&Canvas, usize, Rect),
) {
    let card = RRect::new_rect_xy(layout.card, CARD_RADIUS, CARD_RADIUS);
    canvas.draw_rrect(card, &fill(background));

    canvas.save();
    canvas.clip_rrect(card, ClipOp::Intersect, true);

    for (i, (item, rect)) in items.iter().zip(&layout.item_rects).enumerate() {
        if rect.bottom > layout.footer.top {
            break;
        }
        let selected = state.selected == Some(i);
        let pressed = state.pressed == Some(SelectionListHit::Item(i));
        if selected || pressed {
            let alpha = if selected { 0x30 } else { 0x14 };
            let ground = if selected {
                theme.accent.with_a(alpha)
            } else {
                theme.text_primary.with_a(alpha)
            };
            canvas.draw_rrect(
                RRect::new_rect_xy(*rect, ITEM_RADIUS, ITEM_RADIUS),
                &fill(ground),
            );
        }

        let square = leading_rect(*rect);
        leading(canvas, i, square);

        let x = square.right + LEADING_GAP;
        let room = rect.right - LEADING_GAP - x;
        let cy = rect.center_y();
        let title_style = if selected {
            styles::SUBHEADLINE_EMPHASIZED
        } else {
            styles::SUBHEADLINE
        };
        match &item.subtitle {
            Some(subtitle) => {
                text(
                    canvas,
                    &crop(&item.title, title_style, room),
                    x,
                    cy - 8.0,
                    title_style,
                    theme.text_primary,
                );
                text(
                    canvas,
                    &crop(subtitle, styles::CAPTION_1, room),
                    x,
                    cy + 9.0,
                    styles::CAPTION_1,
                    theme.text_secondary,
                );
            }
            None => text(
                canvas,
                &crop(&item.title, title_style, room),
                x,
                cy,
                title_style,
                theme.text_primary,
            ),
        }
    }

    canvas.draw_rect(layout.footer, &fill(theme.fill_quaternary));
    canvas.draw_line(
        (layout.footer.left, layout.footer.top),
        (layout.footer.right, layout.footer.top),
        &stroke(theme.fill_tertiary, 1.0),
    );
    footer_button(
        canvas,
        layout.add,
        state.can_add,
        state.pressed == Some(SelectionListHit::Add),
        true,
        theme,
    );
    footer_button(
        canvas,
        layout.remove,
        state.can_remove,
        state.pressed == Some(SelectionListHit::Remove),
        false,
        theme,
    );
    canvas.restore();

    canvas.draw_rrect(card, &stroke(theme.fill_tertiary, 1.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(count: usize) -> SelectionListLayout {
        SelectionListLayout::compute(count, Rect::from_xywh(10.0, 20.0, 210.0, 300.0))
    }

    #[test]
    fn items_step_evenly_from_the_top() {
        let l = layout(3);
        assert_eq!(l.item_rects[0].top, 26.0);
        assert_eq!(l.item_rects[1].top - l.item_rects[0].top, ITEM_STEP);
        assert_eq!(l.footer.bottom, l.card.bottom);
    }

    #[test]
    fn every_part_is_hit_where_it_is_laid_out() {
        let l = layout(3);
        let at = |r: Rect| l.hit(r.center_x(), r.center_y());
        assert_eq!(at(l.item_rects[2]), Some(SelectionListHit::Item(2)));
        assert_eq!(at(l.add), Some(SelectionListHit::Add));
        assert_eq!(at(l.remove), Some(SelectionListHit::Remove));
        assert_eq!(l.hit(l.card.right - 4.0, l.footer.center_y()), None);
    }

    #[test]
    fn items_behind_the_footer_are_not_hit() {
        let l = SelectionListLayout::compute(10, Rect::from_xywh(0.0, 0.0, 200.0, 150.0));
        let hidden = l.item_rects[5];
        assert_eq!(l.hit(hidden.center_x(), hidden.center_y()), None);
    }

    #[test]
    fn height_for_fits_every_item_above_the_footer() {
        let count = 4;
        let bounds = Rect::from_xywh(0.0, 0.0, 200.0, SelectionListLayout::height_for(count));
        let l = SelectionListLayout::compute(count, bounds);
        assert!(l.item_rects[count - 1].bottom <= l.footer.top);
    }

    #[test]
    fn a_long_title_is_cropped() {
        let cropped = crop(&"W".repeat(80), styles::SUBHEADLINE, 60.0);
        assert!(cropped.ends_with('…'));
        assert!(styles::SUBHEADLINE.font().measure_str(&cropped, None).0 <= 60.0);
    }

    #[test]
    fn draw_smoke_test_does_not_panic() {
        let mut surface = skia_safe::surfaces::raster_n32_premul((240, 320)).unwrap();
        let items = vec![
            SelectionListItem::new("Riccardo").with_subtitle("Administrator"),
            SelectionListItem::new("Guest"),
        ];
        let state = SelectionListState {
            selected: Some(0),
            can_add: true,
            ..Default::default()
        };
        draw(
            surface.canvas(),
            &layout(2),
            &items,
            &state,
            &Theme::light(),
            Color::WHITE,
            |_, _, _| {},
        );
    }
}
