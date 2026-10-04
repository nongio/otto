//! The Change Password sheet: a card over the dimmed window with the three
//! password fields and the two buttons that end it.
//!
//! Drawn on a subsurface of its own above the pane (see `main.rs`), because
//! the pane already scrolls in subsurfaces stacked over the window's buffer —
//! anything the window painted would sit underneath them. The subsurface
//! takes no input: the pointer and the keyboard stay on the window, whose
//! handlers send them here while the sheet is up, so the fields are edited by
//! the same editor every other text field in the app uses.
//!
//! Everything is laid out from the window's size alone, so the paint and the
//! hit test agree by construction.

use otto_kit::components::text_input::TextInput;
use otto_kit::prelude::*;
use otto_kit::typography::styles;
use skia_safe::{BlurStyle, ClipOp, Contains, MaskFilter, PaintStyle, Point, RRect};

use crate::panes::account::SheetView;
use crate::widgets;

const CARD_W: f32 = 440.0;
const PAD: f32 = 24.0;
const TITLE_H: f32 = 24.0;
const ROW_H: f32 = 36.0;
/// Room for the message line, two lines of footnote text.
const MESSAGE_H: f32 = 40.0;
const RADIUS: f32 = 12.0;
/// Width of each password field, which is also what the editor scrolls in.
pub const FIELD_W: f32 = widgets::TEXT_W;

/// Where a press on the sheet landed.
#[derive(Debug, PartialEq)]
pub enum SheetHit {
    /// One of the fields, and how far into its box.
    Field {
        id: &'static str,
        local_x: f32,
    },
    Cancel,
    Change,
    /// Somewhere else on the card.
    Card,
    /// The scrim: the window behind the sheet, which takes no press while it
    /// is up.
    Outside,
}

struct Layout {
    card: Rect,
    title_cy: f32,
    fields: [Rect; 3],
    labels_cy: [f32; 3],
    message: Rect,
    cancel: Rect,
    change: Rect,
}

fn cancel_label() -> &'static str {
    otto_kit::t!("common-cancel")
}

fn change_label() -> &'static str {
    otto_kit::t!("settings-account-change-password")
}

fn layout(width: f32, height: f32) -> Layout {
    let card_h = PAD + TITLE_H + 8.0 + ROW_H * 3.0 + MESSAGE_H + widgets::CONTROL_H + PAD;
    let card = Rect::from_xywh(
        (width - CARD_W) / 2.0,
        ((height - card_h) / 2.0).max(PAD),
        CARD_W,
        card_h,
    );
    let title_cy = card.top + PAD + TITLE_H / 2.0;
    let rows_top = card.top + PAD + TITLE_H + 8.0;
    let row_cy = |i: usize| rows_top + ROW_H * i as f32 + ROW_H / 2.0;
    let field = |i: usize| {
        Rect::from_xywh(
            card.right - PAD - FIELD_W,
            row_cy(i) - widgets::CONTROL_H / 2.0,
            FIELD_W,
            widgets::CONTROL_H,
        )
    };
    let message_top = rows_top + ROW_H * 3.0;
    let buttons_cy = card.bottom - PAD - widgets::CONTROL_H / 2.0;
    let buttons = widgets::button_rects(
        card.right - PAD,
        buttons_cy,
        &[cancel_label(), change_label()],
    );
    Layout {
        card,
        title_cy,
        fields: [field(0), field(1), field(2)],
        labels_cy: [row_cy(0), row_cy(1), row_cy(2)],
        message: Rect::from_xywh(card.left + PAD, message_top, CARD_W - PAD * 2.0, MESSAGE_H),
        cancel: buttons[0],
        change: buttons[1],
    }
}

/// What a press at (`x`, `y`) in window coordinates lands on.
pub fn hit(width: f32, height: f32, view: &SheetView, x: f32, y: f32) -> SheetHit {
    let layout = layout(width, height);
    let point = Point::new(x, y);
    if !layout.card.contains(point) {
        return SheetHit::Outside;
    }
    if layout.cancel.contains(point) {
        return SheetHit::Cancel;
    }
    if layout.change.contains(point) {
        return SheetHit::Change;
    }
    for (rect, (id, _, _)) in layout.fields.iter().zip(view.fields.iter()) {
        if rect.contains(point) {
            return SheetHit::Field {
                id,
                local_x: x - rect.left,
            };
        }
    }
    SheetHit::Card
}

fn fill(color: Color) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(color);
    paint
}

/// Paint the sheet over a transparent buffer the size of the window.
///
/// `editing` is the field that has the keyboard and its live editor, which is
/// drawn in place of that field's dots.
pub fn paint(
    canvas: &Canvas,
    width: f32,
    height: f32,
    dark: bool,
    view: &SheetView,
    editing: Option<(&'static str, &TextInput)>,
) {
    let theme = if dark { Theme::dark() } else { Theme::light() };
    let layout = layout(width, height);
    canvas.clear(Color::TRANSPARENT);

    // The window steps back behind the sheet.
    canvas.draw_rect(
        Rect::from_wh(width, height),
        &fill(Color::from_argb(if dark { 0x73 } else { 0x40 }, 0, 0, 0)),
    );

    let card = RRect::new_rect_xy(layout.card, RADIUS, RADIUS);
    let mut shadow = fill(Color::from_argb(0x55, 0, 0, 0));
    shadow.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 14.0, None));
    canvas.draw_rrect(card.with_offset((0.0, 6.0)), &shadow);
    canvas.draw_rrect(card, &fill(crate::view::pane_background(dark)));
    let mut hairline = fill(theme.fill_secondary);
    hairline.set_style(PaintStyle::Stroke);
    hairline.set_stroke_width(1.0);
    canvas.draw_rrect(card, &hairline);

    canvas.save();
    canvas.clip_rrect(card, ClipOp::Intersect, true);

    widgets::text_centered_y(
        canvas,
        change_label(),
        layout.card.left + PAD,
        layout.title_cy,
        styles::HEADLINE,
        theme.text_primary,
    );

    for (i, (id, label, count)) in view.fields.iter().enumerate() {
        widgets::text_centered_y(
            canvas,
            label,
            layout.card.left + PAD,
            layout.labels_cy[i],
            styles::BODY,
            theme.text_primary,
        );
        let rect = layout.fields[i];
        match editing.filter(|(editing, _)| editing == id) {
            Some((_, input)) => {
                canvas.save();
                canvas.translate((rect.left, rect.top));
                input.render_at(canvas, rect.width(), rect.height());
                canvas.restore();
            }
            // Dots for what is there, never the text — and no placeholder: an
            // empty password field is just empty.
            None => widgets::field_box(canvas, rect, &"\u{2022}".repeat(*count), "", &theme),
        }
    }

    if let Some((message, error)) = &view.message {
        let color = if *error {
            theme.accent_red
        } else {
            theme.text_secondary
        };
        let lines = widgets::wrap(message, styles::FOOTNOTE, layout.message.width());
        for (i, line) in lines.iter().take(2).enumerate() {
            widgets::text_centered_y(
                canvas,
                line,
                layout.message.left,
                layout.message.top + 10.0 + i as f32 * 17.0,
                styles::FOOTNOTE,
                color,
            );
        }
    }

    // Cancel is an ordinary button; Change Password is the default one, in
    // the accent, since Enter in the last field presses it.
    let enabled = !view.busy;
    let text = |enabled: bool, color: Color| {
        if enabled {
            color
        } else {
            theme.text_tertiary
        }
    };
    let button = |rect: Rect, label: &str, ground: Color, color: Color| {
        let rrect = RRect::new_rect_xy(rect, 6.0, 6.0);
        canvas.draw_rrect(rrect, &fill(ground));
        let style = styles::BODY_MEDIUM;
        let w = style.font().measure_str(label, None).0;
        widgets::text_centered_y(
            canvas,
            label,
            rect.center_x() - w / 2.0,
            rect.center_y(),
            style,
            color,
        );
    };
    button(
        layout.cancel,
        cancel_label(),
        theme.fill_tertiary,
        text(enabled, theme.text_primary),
    );
    button(
        layout.change,
        change_label(),
        if enabled {
            theme.accent
        } else {
            theme.fill_tertiary
        },
        text(enabled, Color::WHITE),
    );

    canvas.restore();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> SheetView {
        SheetView {
            fields: [("a", "A", 0), ("b", "B", 0), ("c", "C", 0)],
            message: None,
            busy: false,
        }
    }

    #[test]
    fn every_part_of_the_card_is_found_where_it_is_drawn() {
        let (w, h) = (900.0, 600.0);
        let l = layout(w, h);
        let at = |r: Rect| hit(w, h, &view(), r.center_x(), r.center_y());
        assert_eq!(at(l.cancel), SheetHit::Cancel);
        assert_eq!(at(l.change), SheetHit::Change);
        assert!(matches!(at(l.fields[1]), SheetHit::Field { id: "b", .. }));
        assert_eq!(hit(w, h, &view(), 2.0, 2.0), SheetHit::Outside);
        assert_eq!(
            hit(w, h, &view(), l.card.left + 4.0, l.card.top + 4.0),
            SheetHit::Card
        );
    }

    #[test]
    fn the_card_stays_on_a_window_shorter_than_it() {
        let l = layout(500.0, 100.0);
        assert!(l.card.top >= PAD);
    }
}
