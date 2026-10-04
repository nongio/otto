//! The users pane's sheets: a card over the dimmed window with a title, a
//! paragraph or a few fields, and the two buttons that end it — Change
//! Password, Reset Password, Add User and Delete User all take this shape.
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
/// Line height of the body paragraph.
const BODY_LINE_H: f32 = 19.0;
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
    /// The default button.
    Action,
    /// Somewhere else on the card.
    Card,
    /// The scrim: the window behind the sheet, which takes no press while it
    /// is up.
    Outside,
}

struct Layout {
    card: Rect,
    title_cy: f32,
    /// The body paragraph's wrapped lines, and the top of the first.
    body: Vec<String>,
    body_top: f32,
    fields: Vec<Rect>,
    labels_cy: Vec<f32>,
    message: Rect,
    cancel: Rect,
    action: Rect,
}

fn cancel_label() -> &'static str {
    otto_kit::t!("common-cancel")
}

fn layout(width: f32, height: f32, view: &SheetView) -> Layout {
    let body = view
        .body
        .as_deref()
        .map(|text| widgets::wrap(text, styles::BODY, CARD_W - PAD * 2.0))
        .unwrap_or_default();
    let body_h = if body.is_empty() {
        0.0
    } else {
        body.len() as f32 * BODY_LINE_H + 8.0
    };
    let count = view.fields.len();
    let card_h =
        PAD + TITLE_H + 8.0 + body_h + ROW_H * count as f32 + MESSAGE_H + widgets::CONTROL_H + PAD;
    let card = Rect::from_xywh(
        (width - CARD_W) / 2.0,
        ((height - card_h) / 2.0).max(PAD),
        CARD_W,
        card_h,
    );
    let title_cy = card.top + PAD + TITLE_H / 2.0;
    let body_top = card.top + PAD + TITLE_H + 8.0;
    let rows_top = body_top + body_h;
    let row_cy = |i: usize| rows_top + ROW_H * i as f32 + ROW_H / 2.0;
    let field = |i: usize| {
        Rect::from_xywh(
            card.right - PAD - FIELD_W,
            row_cy(i) - widgets::CONTROL_H / 2.0,
            FIELD_W,
            widgets::CONTROL_H,
        )
    };
    let message_top = rows_top + ROW_H * count as f32;
    let buttons_cy = card.bottom - PAD - widgets::CONTROL_H / 2.0;
    let buttons =
        widgets::button_rects(card.right - PAD, buttons_cy, &[cancel_label(), view.action]);
    Layout {
        card,
        title_cy,
        body,
        body_top,
        fields: (0..count).map(field).collect(),
        labels_cy: (0..count).map(row_cy).collect(),
        message: Rect::from_xywh(card.left + PAD, message_top, CARD_W - PAD * 2.0, MESSAGE_H),
        cancel: buttons[0],
        action: buttons[1],
    }
}

/// What a press at (`x`, `y`) in window coordinates lands on.
pub fn hit(width: f32, height: f32, view: &SheetView, x: f32, y: f32) -> SheetHit {
    let layout = layout(width, height, view);
    let point = Point::new(x, y);
    if !layout.card.contains(point) {
        return SheetHit::Outside;
    }
    if layout.cancel.contains(point) {
        return SheetHit::Cancel;
    }
    if layout.action.contains(point) {
        return SheetHit::Action;
    }
    for (rect, field) in layout.fields.iter().zip(view.fields.iter()) {
        if rect.contains(point) {
            return SheetHit::Field {
                id: field.id,
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
/// drawn in place of what that field shows at rest.
pub fn paint(
    canvas: &Canvas,
    width: f32,
    height: f32,
    dark: bool,
    view: &SheetView,
    editing: Option<(&'static str, &TextInput)>,
) {
    let theme = if dark { Theme::dark() } else { Theme::light() };
    let layout = layout(width, height, view);
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

    let title = widgets::elide_tail(&view.title, styles::HEADLINE, CARD_W - PAD * 2.0);
    widgets::text_centered_y(
        canvas,
        &title,
        layout.card.left + PAD,
        layout.title_cy,
        styles::HEADLINE,
        theme.text_primary,
    );

    for (i, line) in layout.body.iter().enumerate() {
        widgets::text_centered_y(
            canvas,
            line,
            layout.card.left + PAD,
            layout.body_top + i as f32 * BODY_LINE_H + BODY_LINE_H / 2.0,
            styles::BODY,
            theme.text_secondary,
        );
    }

    for (i, field) in view.fields.iter().enumerate() {
        widgets::text_centered_y(
            canvas,
            field.label,
            layout.card.left + PAD,
            layout.labels_cy[i],
            styles::BODY,
            theme.text_primary,
        );
        let rect = layout.fields[i];
        match editing.filter(|(editing, _)| *editing == field.id) {
            Some((_, input)) => {
                canvas.save();
                canvas.translate((rect.left, rect.top));
                input.render_at(canvas, rect.width(), rect.height());
                canvas.restore();
            }
            // A password field shows dots for what is there, never the text
            // — the view has already made them — and no placeholder: an
            // empty field is just empty.
            None => widgets::field_box(canvas, rect, &field.shown, "", &theme),
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

    // Cancel is an ordinary button; the action is the default one, in the
    // accent — or in red when it destroys something — since Enter presses it.
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
        layout.action,
        view.action,
        if !enabled {
            theme.fill_tertiary
        } else if view.destructive {
            theme.accent_red
        } else {
            theme.accent
        },
        text(enabled, Color::WHITE),
    );

    canvas.restore();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panes::account::SheetField;

    fn view(fields: usize, body: Option<&str>) -> SheetView {
        let ids = ["a", "b", "c", "d"];
        SheetView {
            title: "Title".into(),
            body: body.map(str::to_string),
            fields: ids[..fields]
                .iter()
                .map(|id| SheetField {
                    id,
                    label: "Label",
                    shown: String::new(),
                })
                .collect(),
            action: "Do It",
            destructive: false,
            message: None,
            busy: false,
        }
    }

    #[test]
    fn every_part_of_the_card_is_found_where_it_is_drawn() {
        let (w, h) = (900.0, 600.0);
        let v = view(3, None);
        let l = layout(w, h, &v);
        let at = |r: Rect| hit(w, h, &v, r.center_x(), r.center_y());
        assert_eq!(at(l.cancel), SheetHit::Cancel);
        assert_eq!(at(l.action), SheetHit::Action);
        assert!(matches!(at(l.fields[1]), SheetHit::Field { id: "b", .. }));
        assert_eq!(hit(w, h, &v, 2.0, 2.0), SheetHit::Outside);
        assert_eq!(
            hit(w, h, &v, l.card.left + 4.0, l.card.top + 4.0),
            SheetHit::Card
        );
    }

    #[test]
    fn a_body_pushes_the_fields_down_and_no_fields_shrinks_the_card() {
        let plain = layout(900.0, 600.0, &view(2, None));
        let with_body = layout(900.0, 600.0, &view(2, Some("A paragraph.")));
        assert!(with_body.fields[0].top > plain.fields[0].top);
        let empty = layout(900.0, 600.0, &view(0, Some("Sure?")));
        assert!(empty.card.height() < plain.card.height());
    }

    #[test]
    fn the_card_stays_on_a_window_shorter_than_it() {
        let l = layout(500.0, 100.0, &view(4, None));
        assert!(l.card.top >= PAD);
    }
}
