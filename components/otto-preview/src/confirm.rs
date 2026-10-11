//! The question a window with unsaved changes asks before it closes: save
//! them, close without them, or stay open.

// Rust guideline compliant 2026-02-21

use otto_kit::components::label::Label;
use otto_kit::prelude::*;
use otto_kit::skia::{Contains, Point, RRect};
use otto_kit::typography::styles;

/// The card's width, and the room around its edge.
const CARD_W: f32 = 380.0;
const MARGIN: f32 = 20.0;
const TITLE_H: f32 = 26.0;
const BODY_H: f32 = 36.0;
const BUTTON_W: f32 = 96.0;
const BUTTON_H: f32 = 28.0;

/// What the person picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Save,
    Discard,
    Cancel,
}

/// Where the card's parts are for a window of `size`.
struct Layout {
    card: Rect,
    save: Rect,
    cancel: Rect,
    discard: Rect,
}

fn layout((width, height): (f32, f32)) -> Layout {
    let card_w = CARD_W.min(width - 2.0 * MARGIN);
    let card_h = TITLE_H + BODY_H + BUTTON_H + 3.0 * MARGIN;
    let card = Rect::from_xywh(
        (width - card_w) / 2.0,
        ((height - card_h) / 2.0).max(MARGIN),
        card_w,
        card_h,
    );
    let y = card.bottom - MARGIN - BUTTON_H;
    let save = Rect::from_xywh(card.right - MARGIN - BUTTON_W, y, BUTTON_W, BUTTON_H);
    let cancel = Rect::from_xywh(save.left - 8.0 - BUTTON_W, y, BUTTON_W, BUTTON_H);
    let discard = Rect::from_xywh(card.left + MARGIN, y, BUTTON_W, BUTTON_H);
    Layout {
        card,
        save,
        cancel,
        discard,
    }
}

/// What a press at `at` answers, in a window of `size`. A press outside the
/// card is a cancel; one on the card between its buttons is nothing.
pub fn press(size: (f32, f32), at: Point) -> Option<Answer> {
    let layout = layout(size);
    if layout.save.contains(at) {
        Some(Answer::Save)
    } else if layout.discard.contains(at) {
        Some(Answer::Discard)
    } else if layout.cancel.contains(at) || !layout.card.contains(at) {
        Some(Answer::Cancel)
    } else {
        None
    }
}

/// Paint the question over a window of `size`, about the file `name`.
pub fn draw(canvas: &Canvas, size: (f32, f32), name: &str, theme: &Theme) {
    let layout = layout(size);
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(Color::from_argb(0x55, 0, 0, 0));
    canvas.draw_rect(Rect::from_wh(size.0, size.1), &paint);
    paint.set_color(theme.card_material());
    canvas.draw_rrect(RRect::new_rect_xy(layout.card, 14.0, 14.0), &paint);

    let text_w = layout.card.width() - 2.0 * MARGIN;
    Label::new(otto_kit::t_owned!(
        "studio-close-title",
        name = name.to_owned()
    ))
    .with_style(styles::HEADLINE)
    .with_color(theme.text_primary)
    .with_width(text_w)
    .centered_on(
        layout.card.left + MARGIN,
        layout.card.top + MARGIN + TITLE_H / 2.0,
    )
    .render(canvas);
    Label::new(otto_kit::t_owned!("studio-close-body"))
        .with_style(styles::BODY)
        .with_color(theme.text_secondary)
        .with_width(text_w)
        .centered_on(
            layout.card.left + MARGIN,
            layout.card.top + MARGIN + TITLE_H + BODY_H / 2.0,
        )
        .render(canvas);

    let button = |rect: Rect, label: String, primary: bool| {
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(if primary {
            theme.accent
        } else {
            theme.fill_tertiary
        });
        canvas.draw_rrect(RRect::new_rect_xy(rect, 7.0, 7.0), &paint);
        Label::new(label)
            .with_style(styles::BODY)
            .with_color(if primary {
                Color::WHITE
            } else {
                theme.text_primary
            })
            .centered_at(rect.center_x(), rect.center_y())
            .render(canvas);
    };
    button(
        layout.discard,
        otto_kit::t_owned!("studio-close-discard"),
        false,
    );
    button(
        layout.cancel,
        otto_kit::t_owned!("studio-signature-cancel"),
        false,
    );
    button(layout.save, otto_kit::t_owned!("studio-save"), true);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_button_answers_and_outside_is_a_cancel() {
        let size = (800.0, 600.0);
        let layout = layout(size);
        let middle = |rect: Rect| Point::new(rect.center_x(), rect.center_y());
        assert_eq!(press(size, middle(layout.save)), Some(Answer::Save));
        assert_eq!(press(size, middle(layout.discard)), Some(Answer::Discard));
        assert_eq!(press(size, middle(layout.cancel)), Some(Answer::Cancel));
        assert_eq!(press(size, Point::new(2.0, 2.0)), Some(Answer::Cancel));
        assert_eq!(
            press(
                size,
                Point::new(layout.card.center_x(), layout.card.top + 30.0)
            ),
            None
        );
    }
}
