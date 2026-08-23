//! Controls used by the settings rows.
//!
//! Each one draws itself into a rect and reports nothing back — there is no
//! hit-testing or state here yet. They exist so the panes can be looked at
//! before any of them is wired to the compositor.

use otto_kit::prelude::*;
use skia_safe::{BlurStyle, ClipOp, MaskFilter, PaintStyle, PathBuilder, PathEffect, Point, RRect};

/// Trailing-edge control width, so labels can be laid out against it.
pub const TOGGLE_W: f32 = 40.0;
pub const TOGGLE_H: f32 = 24.0;
pub const SLIDER_W: f32 = 160.0;
/// Width of a row's pop-up button. The control itself is
/// `otto_kit::components::dropdown::field`; only the width is the pane's
/// choice.
pub const SELECT_W: f32 = 176.0;
pub const CONTROL_H: f32 = 24.0;

/// One size for every control's own text — a readout, a field's value, a
/// keycap, a button's label.
///
/// The same size as the pop-up button's label, the menu it drops, and the row
/// label beside it: a control's value is content, not an annotation of it, and
/// a row of mixed controls has to read as one line rather than as four
/// typographic accidents. `styles::SUBHEADLINE` is what the detail line under
/// a label uses — a control set in it looks like a caption of itself.
pub const CONTROL_TEXT: TextStyle = styles::BODY;

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
pub fn text_centered_y(
    canvas: &Canvas,
    text: &str,
    x: f32,
    cy: f32,
    style: TextStyle,
    color: Color,
) {
    Label::new(text)
        .with_style(style)
        .with_color(color)
        .centered_on(x, cy)
        .render(canvas);
}

pub fn text_right(
    canvas: &Canvas,
    text: &str,
    right: f32,
    cy: f32,
    style: TextStyle,
    color: Color,
) {
    let width = style.font().measure_str(text, None).0;
    text_centered_y(canvas, text, right - width, cy, style, color);
}

/// iOS-style switch, drawn by the toolkit's own control.
///
/// `knob_fraction` is where the knob sits: 0.0 off, 1.0 on, in between while
/// a flip is animating. The track colour follows it, so the pane gets the
/// slide and the colour change from one number.
pub fn toggle(canvas: &Canvas, x: f32, cy: f32, knob_fraction: f32, theme: &Theme) {
    let rect = Rect::from_xywh(x, cy - TOGGLE_H / 2.0, TOGGLE_W, TOGGLE_H);
    toggle::draw(
        canvas,
        rect,
        knob_fraction,
        ToggleInteraction::Normal,
        theme,
    );
}

/// Track, filled portion, and knob. The readout sits to the right of the track.
pub fn slider(
    canvas: &Canvas,
    x: f32,
    cy: f32,
    value: f32,
    min: f32,
    max: f32,
    readout: &str,
    theme: &Theme,
) {
    let t = ((value - min) / (max - min)).clamp(0.0, 1.0);
    let track = Rect::from_xywh(x, cy - 2.0, SLIDER_W, 4.0);
    canvas.draw_rrect(
        RRect::new_rect_xy(track, 2.0, 2.0),
        &fill(theme.fill_secondary),
    );
    let filled = Rect::from_xywh(x, cy - 2.0, SLIDER_W * t, 4.0);
    canvas.draw_rrect(RRect::new_rect_xy(filled, 2.0, 2.0), &fill(theme.accent));

    let knob_x = x + SLIDER_W * t;
    let mut shadow = fill(Color::from_argb(0x38, 0, 0, 0));
    shadow.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 2.0, false));
    canvas.draw_circle(Point::new(knob_x, cy + 1.0), 8.0, &shadow);
    canvas.draw_circle(Point::new(knob_x, cy), 8.0, &fill(Color::WHITE));
    canvas.draw_circle(
        Point::new(knob_x, cy),
        8.0,
        &stroke(theme.fill_tertiary, 0.5),
    );

    text_centered_y(
        canvas,
        readout,
        x + SLIDER_W + 12.0,
        cy,
        CONTROL_TEXT,
        theme.text_secondary,
    );
}

/// A text row's field width. The rect itself comes from `view::text_rect`.
pub const TEXT_W: f32 = 220.0;

/// A text field at rest — what a row draws when it does *not* have the
/// keyboard. The focused one is drawn by `otto_kit`'s own `TextInput`, so this
/// only has to match its resting looks.
pub fn text_field(canvas: &Canvas, rect: Rect, value: &str, theme: &Theme) {
    let rrect = RRect::new_rect_xy(rect, 6.0, 6.0);
    canvas.draw_rrect(rrect, &fill(theme.fill_quaternary));
    canvas.draw_rrect(rrect, &stroke(theme.fill_secondary, 1.0));
    canvas.save();
    canvas.clip_rrect(rrect, ClipOp::Intersect, true);
    let style = CONTROL_TEXT;
    let (text, color) = if value.is_empty() {
        ("Not set".to_string(), theme.text_tertiary)
    } else {
        (
            elide_tail(value, style, rect.width() - 18.0),
            theme.text_primary,
        )
    };
    text_centered_y(
        canvas,
        &text,
        rect.left + 9.0,
        rect.center_y(),
        style,
        color,
    );
    canvas.restore();
}

/// Trim characters off the END of `text` until it fits `width`, marking the
/// cut with a trailing ellipsis.
pub fn elide_tail(text: &str, style: otto_kit::typography::TextStyle, width: f32) -> String {
    let font = style.font();
    if font.measure_str(text, None).0 <= width {
        return text.to_string();
    }
    let mut end = text.len();
    while end > 0 {
        end -= 1;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        let candidate = format!("{}…", &text[..end]);
        if font.measure_str(&candidate, None).0 <= width {
            return candidate;
        }
    }
    "…".to_string()
}

/// Trim characters off the FRONT of `text` until it fits `width`, marking the
/// cut with a leading ellipsis. Returns `text` unchanged when it already fits.
fn elide_head(text: &str, style: otto_kit::typography::TextStyle, width: f32) -> String {
    let font = style.font();
    if font.measure_str(text, None).0 <= width {
        return text.to_string();
    }
    // Walk char boundaries from the front; the first tail that fits with the
    // ellipsis in front of it is the answer.
    for (i, _) in text.char_indices() {
        let candidate = format!("…{}", &text[i..]);
        if font.measure_str(&candidate, None).0 <= width {
            return candidate;
        }
    }
    "…".to_string()
}

/// The "Choose…" button's width, and the gap between it and the path field.
pub const CHOOSE_W: f32 = 84.0;
const CHOOSE_GAP: f32 = 8.0;
/// The path field beside it.
const FILE_FIELD_W: f32 = 196.0;

/// Where the "Choose…" button sits, for drawing and for hit-testing.
pub fn choose_rect(right: f32, cy: f32) -> Rect {
    Rect::from_xywh(right - CHOOSE_W, cy - CONTROL_H / 2.0, CHOOSE_W, CONTROL_H)
}

/// A file setting: the chosen path, and the button that changes it.
///
/// The path reads from the left like any other field's text. When it does not
/// fit, the *head* is elided — the file's own name is what identifies it, and
/// truncating that away to keep `/home/user/` would leave the one part nobody
/// needs.
pub fn file_field(canvas: &Canvas, right: f32, cy: f32, value: &str, theme: &Theme) {
    let button = choose_rect(right, cy);
    let field = Rect::from_xywh(
        button.left - CHOOSE_GAP - FILE_FIELD_W,
        cy - CONTROL_H / 2.0,
        FILE_FIELD_W,
        CONTROL_H,
    );

    let rrect = RRect::new_rect_xy(field, 6.0, 6.0);
    canvas.draw_rrect(rrect, &fill(theme.fill_quaternary));
    canvas.draw_rrect(rrect, &stroke(theme.fill_secondary, 1.0));
    canvas.save();
    canvas.clip_rrect(rrect, ClipOp::Intersect, true);
    let style = CONTROL_TEXT;
    let (text, color) = if value.is_empty() {
        ("No file chosen".to_string(), theme.text_tertiary)
    } else {
        (value.to_string(), theme.text_primary)
    };
    // Left aligned, like every other field's text. The file's own name is what
    // identifies it, so what a too-long path loses is its leading directories:
    // the head is elided rather than the tail truncated.
    let inner = FILE_FIELD_W - 18.0;
    let text = elide_head(&text, style, inner);
    text_centered_y(canvas, &text, field.left + 9.0, cy, style, color);
    canvas.restore();

    let brrect = RRect::new_rect_xy(button, 6.0, 6.0);
    canvas.draw_rrect(brrect, &fill(theme.fill_tertiary));
    canvas.draw_rrect(brrect, &stroke(theme.fill_secondary, 1.0));
    let label = "Choose…";
    let label_w = style.font().measure_str(label, None).0;
    text_centered_y(
        canvas,
        label,
        button.center_x() - label_w / 2.0,
        cy,
        style,
        theme.text_primary,
    );
}

/// Editable-looking field drawn into a rect the caller has measured, for
/// controls that share a row and so cannot each claim a fixed width.
///
/// `placeholder` stands in while the value is empty, so a freshly added
/// shortcut says what is missing rather than showing a blank box.
pub fn field_box(canvas: &Canvas, rect: Rect, value: &str, placeholder: &str, theme: &Theme) {
    let rrect = RRect::new_rect_xy(rect, 6.0, 6.0);
    canvas.draw_rrect(rrect, &fill(theme.fill_quaternary));
    canvas.draw_rrect(rrect, &stroke(theme.fill_secondary, 1.0));
    canvas.save();
    canvas.clip_rrect(rrect, ClipOp::Intersect, true);
    let (text, color) = if value.is_empty() {
        (placeholder, theme.text_tertiary)
    } else {
        (value, theme.text_primary)
    };
    text_centered_y(
        canvas,
        text,
        rect.left + 9.0,
        rect.center_y(),
        styles::SUBHEADLINE,
        color,
    );
    canvas.restore();
}

/// Side of the square "+"/"−" buttons that add and remove a line.
pub const LINE_BUTTON: f32 = 24.0;

/// A "+" or "−" button. `plus` picks which stroke it gets — the two are the
/// same control, and drawing them from one function keeps them identical.
pub fn line_button(canvas: &Canvas, rect: Rect, plus: bool, theme: &Theme) {
    let rrect = RRect::new_rect_xy(rect, 6.0, 6.0);
    canvas.draw_rrect(rrect, &fill(theme.fill_tertiary));
    canvas.draw_rrect(rrect, &stroke(theme.fill_secondary, 1.0));

    let mut paint = stroke(theme.text_primary, 1.6);
    paint.set_stroke_cap(skia_safe::paint::Cap::Round);
    let (cx, cy) = (rect.center_x(), rect.center_y());
    let arm = 5.0;
    canvas.draw_line(Point::new(cx - arm, cy), Point::new(cx + arm, cy), &paint);
    if plus {
        canvas.draw_line(Point::new(cx, cy - arm), Point::new(cx, cy + arm), &paint);
    }
}

/// Key combination shown as individual keycaps.
pub fn key_combo(canvas: &Canvas, right: f32, cy: f32, combo: &str, theme: &Theme) {
    let keys: Vec<&str> = combo.split('+').map(|k| k.trim()).collect();
    let style = CONTROL_TEXT;
    let gap = 4.0;
    let pad = 7.0;

    let widths: Vec<f32> = keys
        .iter()
        .map(|k| style.font().measure_str(k, None).0 + pad * 2.0)
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + gap * (keys.len() as f32 - 1.0);

    let mut x = right - total;
    for (key, width) in keys.iter().zip(&widths) {
        let rect = Rect::from_xywh(x, cy - 11.0, *width, 22.0);
        let rrect = RRect::new_rect_xy(rect, 5.0, 5.0);
        canvas.draw_rrect(rrect, &fill(theme.fill_tertiary));
        canvas.draw_rrect(rrect, &stroke(theme.fill_secondary, 1.0));
        text_centered_y(canvas, key, x + pad, cy, style, theme.text_primary);
        x += width + gap;
    }
}

/// Small circular arrow marking a row that overrides its inherited value.
pub fn revert_badge(canvas: &Canvas, cx: f32, cy: f32, theme: &Theme) {
    let mut paint = stroke(theme.accent, 1.4);
    paint.set_stroke_cap(skia_safe::paint::Cap::Round);
    let r = 5.5;
    let arc = Rect::from_xywh(cx - r, cy - r, r * 2.0, r * 2.0);
    canvas.draw_arc(arc, 40.0, 280.0, false, &paint);

    let mut head = PathBuilder::new();
    head.move_to(Point::new(cx + r - 1.0, cy - r + 1.5));
    head.line_to(Point::new(cx + r + 2.5, cy - r + 2.0));
    head.line_to(Point::new(cx + r + 0.5, cy - r + 5.0));
    head.close();
    canvas.draw_path(&head.detach(), &fill(theme.accent));
}

/// "Restart required" pill. Amber regardless of theme — it is a status, not
/// a surface.
pub fn restart_pill(canvas: &Canvas, x: f32, cy: f32) {
    let text = "Restart required";
    let style = styles::CAPTION_2;
    let width = style.font().measure_str(text, None).0 + 14.0;
    let rect = Rect::from_xywh(x, cy - 9.0, width, 18.0);
    canvas.draw_rrect(
        RRect::new_rect_xy(rect, 9.0, 9.0),
        &fill(Color::from_argb(0x24, 0xFF, 0x9F, 0x0A)),
    );
    text_centered_y(
        canvas,
        text,
        x + 7.0,
        cy,
        style,
        Color::from_argb(0xE0, 0xB2, 0x6A, 0x00),
    );
}

/// Hairline between rows, inset from the leading edge like a grouped list.
pub fn separator(canvas: &Canvas, x0: f32, x1: f32, y: f32, theme: &Theme) {
    canvas.draw_line(
        Point::new(x0, y),
        Point::new(x1, y),
        &stroke(theme.fill_tertiary, 1.0),
    );
}

/// Dashed rectangle used by the displays canvas for the desktop bounds.
pub fn dashed_rect(canvas: &Canvas, rect: Rect, color: Color) {
    let mut paint = stroke(color, 1.0);
    paint.set_path_effect(PathEffect::dash(&[4.0, 4.0], 0.0));
    canvas.draw_rect(rect, &paint);
}
