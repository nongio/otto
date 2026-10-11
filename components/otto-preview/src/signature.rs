//! The person's signature: drawn once on a pad, kept, and placed on any
//! document after that as strokes of ink.
//!
//! Kept as the strokes themselves rather than a picture, in
//! `$XDG_DATA_HOME/otto/signature.json`, so it is as sharp at any size it is
//! placed at and goes into the document as an ink annotation every reader
//! draws.

// Rust guideline compliant 2026-02-21

use std::path::PathBuf;

use otto_kit::components::label::Label;
use otto_kit::prelude::*;
use otto_kit::skia::{Contains, PaintCap, PaintJoin, PaintStyle, PathBuilder, Point, RRect};
use otto_kit::typography::styles;
use serde::{Deserialize, Serialize};

/// The drawing area on the pad, in points.
const PAD_W: f32 = 440.0;
const PAD_H: f32 = 170.0;
/// Room around the card's edge.
const MARGIN: f32 = 20.0;
const TITLE_H: f32 = 30.0;
const BUTTON_W: f32 = 84.0;
const BUTTON_H: f32 = 28.0;
/// How far a stroke's points must be apart to be kept, in points.
const STEP: f32 = 1.5;

/// A signature: strokes in the pad's points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Signature {
    pub strokes: Vec<Vec<(f32, f32)>>,
}

impl Signature {
    /// The box the strokes take up: left, top, right, bottom.
    fn bounds(&self) -> Option<(f32, f32, f32, f32)> {
        let mut points = self.strokes.iter().flatten();
        let first = points.next()?;
        Some(points.fold(
            (first.0, first.1, first.0, first.1),
            |(l, t, r, b), (x, y)| (l.min(*x), t.min(*y), r.max(*x), b.max(*y)),
        ))
    }

    /// Width over height of the strokes.
    pub fn aspect(&self) -> f32 {
        self.bounds()
            .map(|(l, t, r, b)| (r - l).max(1.0) / (b - t).max(1.0))
            .unwrap_or(1.0)
    }

    /// The strokes scaled to fit `rect` and centred in it, keeping their
    /// shape.
    pub fn fitted(&self, rect: otto_pdf::Rect) -> Vec<Vec<(f32, f32)>> {
        let Some((l, t, r, b)) = self.bounds() else {
            return Vec::new();
        };
        let (w, h) = ((r - l).max(1.0), (b - t).max(1.0));
        let scale = (rect.width() / w).min(rect.height() / h);
        let left = rect.left + (rect.width() - w * scale) / 2.0;
        let top = rect.top + (rect.height() - h * scale) / 2.0;
        self.strokes
            .iter()
            .map(|stroke| {
                stroke
                    .iter()
                    .map(|(x, y)| (left + (x - l) * scale, top + (y - t) * scale))
                    .collect()
            })
            .collect()
    }
}

/// Where the signature is kept.
fn path() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
        })?;
    Some(data.join("otto/signature.json"))
}

/// The person's signature, if they have drawn one.
pub fn load() -> Option<Signature> {
    let text = std::fs::read_to_string(path()?).ok()?;
    let signature: Signature = serde_json::from_str(&text).ok()?;
    (!signature.strokes.is_empty()).then_some(signature)
}

/// Keep `signature` as the person's.
pub fn save(signature: &Signature) -> std::io::Result<()> {
    let path = path().ok_or_else(|| std::io::Error::other("no data folder"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string(signature).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

/// What a press on the pad asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadAction {
    /// Nothing beyond drawing.
    None,
    /// Put the pad away without keeping anything.
    Cancel,
    /// Keep what was drawn as the signature.
    Done,
}

/// The pad a signature is drawn on: a card over the window.
#[derive(Debug, Clone, Default)]
pub struct Pad {
    strokes: Vec<Vec<(f32, f32)>>,
    drawing: bool,
}

/// Where the pad's parts are for a window of `size`.
struct PadLayout {
    card: Rect,
    area: Rect,
    clear: Rect,
    cancel: Rect,
    done: Rect,
}

fn layout((width, height): (f32, f32)) -> PadLayout {
    let pad_w = PAD_W.min(width - 4.0 * MARGIN).max(160.0);
    let card_w = pad_w + 2.0 * MARGIN;
    let card_h = TITLE_H + PAD_H + BUTTON_H + 3.0 * MARGIN;
    let card = Rect::from_xywh(
        (width - card_w) / 2.0,
        ((height - card_h) / 2.0).max(MARGIN),
        card_w,
        card_h,
    );
    let area = Rect::from_xywh(
        card.left + MARGIN,
        card.top + MARGIN + TITLE_H,
        pad_w,
        PAD_H,
    );
    let y = area.bottom + MARGIN;
    let done = Rect::from_xywh(card.right - MARGIN - BUTTON_W, y, BUTTON_W, BUTTON_H);
    let cancel = Rect::from_xywh(done.left - 8.0 - BUTTON_W, y, BUTTON_W, BUTTON_H);
    let clear = Rect::from_xywh(card.left + MARGIN, y, BUTTON_W, BUTTON_H);
    PadLayout {
        card,
        area,
        clear,
        cancel,
        done,
    }
}

impl Pad {
    /// The signature drawn so far, if anything has been.
    pub fn signature(&self) -> Option<Signature> {
        (!self.strokes.is_empty()).then(|| Signature {
            strokes: self.strokes.clone(),
        })
    }

    /// A press anywhere in a window of `size`.
    pub fn press(&mut self, size: (f32, f32), at: Point) -> PadAction {
        let layout = layout(size);
        if layout.area.contains(at) {
            self.strokes
                .push(vec![(at.x - layout.area.left, at.y - layout.area.top)]);
            self.drawing = true;
            return PadAction::None;
        }
        if layout.clear.contains(at) {
            self.strokes.clear();
            return PadAction::None;
        }
        if layout.cancel.contains(at) || !layout.card.contains(at) {
            return PadAction::Cancel;
        }
        if layout.done.contains(at) && !self.strokes.is_empty() {
            return PadAction::Done;
        }
        PadAction::None
    }

    /// The pointer moved. Returns whether the pad changed.
    pub fn motion(&mut self, size: (f32, f32), at: Point) -> bool {
        if !self.drawing {
            return false;
        }
        let area = layout(size).area;
        let point = (
            (at.x - area.left).clamp(0.0, area.width()),
            (at.y - area.top).clamp(0.0, area.height()),
        );
        let Some(stroke) = self.strokes.last_mut() else {
            return false;
        };
        let last = stroke.last().copied().unwrap_or(point);
        if (point.0 - last.0).hypot(point.1 - last.1) < STEP {
            return false;
        }
        stroke.push(point);
        true
    }

    pub fn release(&mut self) {
        self.drawing = false;
        // A click is a dot.
        if let Some(stroke) = self.strokes.last_mut() {
            if stroke.len() == 1 {
                let (x, y) = stroke[0];
                stroke.push((x + 0.5, y + 0.5));
            }
        }
    }

    /// Paint the pad over a window of `size`.
    pub fn draw(&self, canvas: &Canvas, size: (f32, f32), theme: &Theme) {
        let layout = layout(size);
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(Color::from_argb(0x55, 0, 0, 0));
        canvas.draw_rect(Rect::from_wh(size.0, size.1), &paint);
        paint.set_color(theme.card_material());
        canvas.draw_rrect(RRect::new_rect_xy(layout.card, 14.0, 14.0), &paint);

        Label::new(otto_kit::t_owned!("studio-signature-title"))
            .with_style(styles::HEADLINE)
            .with_color(theme.text_primary)
            .with_width(layout.area.width())
            .centered_on(
                layout.area.left,
                layout.card.top + MARGIN + TITLE_H / 2.0 - 4.0,
            )
            .render(canvas);

        // The paper, with a line to sign on.
        paint.set_color(Color::WHITE);
        canvas.draw_rrect(RRect::new_rect_xy(layout.area, 8.0, 8.0), &paint);
        let line_y = layout.area.bottom - 36.0;
        paint.set_style(PaintStyle::Stroke);
        paint.set_stroke_width(1.0);
        paint.set_color(Color::from_argb(70, 0, 0, 0));
        canvas.draw_line(
            (layout.area.left + 24.0, line_y),
            (layout.area.right - 24.0, line_y),
            &paint,
        );
        if self.strokes.is_empty() {
            Label::new(otto_kit::t_owned!("studio-signature-hint"))
                .with_style(styles::FOOTNOTE)
                .with_color(Color::from_argb(110, 0, 0, 0))
                .with_width(layout.area.width())
                .centered_on(layout.area.left + 24.0, line_y + 14.0)
                .render(canvas);
        }

        let mut ink = Paint::default();
        ink.set_anti_alias(true);
        ink.set_style(PaintStyle::Stroke);
        ink.set_stroke_width(2.2);
        ink.set_stroke_cap(PaintCap::Round);
        ink.set_stroke_join(PaintJoin::Round);
        ink.set_color(Color::from_rgb(0x12, 0x1F, 0x5A));
        for stroke in &self.strokes {
            let mut path = PathBuilder::new();
            for (index, (x, y)) in stroke.iter().enumerate() {
                let at = (layout.area.left + x, layout.area.top + y);
                if index == 0 {
                    path.move_to(at);
                } else {
                    path.line_to(at);
                }
            }
            canvas.draw_path(&path.detach(), &ink);
        }

        let button = |rect: Rect, label: String, primary: bool, enabled: bool| {
            let mut paint = Paint::default();
            paint.set_anti_alias(true);
            paint.set_color(if primary && enabled {
                theme.accent
            } else {
                theme.fill_tertiary
            });
            canvas.draw_rrect(RRect::new_rect_xy(rect, 7.0, 7.0), &paint);
            let color = match (primary, enabled) {
                (_, false) => theme.text_tertiary,
                (true, true) => Color::WHITE,
                (false, true) => theme.text_primary,
            };
            Label::new(label)
                .with_style(styles::BODY)
                .with_color(color)
                .centered_at(rect.center_x(), rect.center_y())
                .render(canvas);
        };
        let drawn = !self.strokes.is_empty();
        button(
            layout.clear,
            otto_kit::t_owned!("studio-signature-clear"),
            false,
            drawn,
        );
        button(
            layout.cancel,
            otto_kit::t_owned!("studio-signature-cancel"),
            false,
            true,
        );
        button(
            layout.done,
            otto_kit::t_owned!("studio-signature-done"),
            true,
            drawn,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signature() -> Signature {
        Signature {
            strokes: vec![vec![(10.0, 20.0), (110.0, 70.0)]],
        }
    }

    #[test]
    fn a_signature_fits_a_box_keeping_its_shape() {
        let fitted = signature().fitted(otto_pdf::Rect::new(0.0, 0.0, 100.0, 100.0));
        // 100 × 50 strokes in a 100 × 100 box: full width, centred down it.
        assert_eq!(fitted, vec![vec![(0.0, 25.0), (100.0, 75.0)]]);
        assert!((signature().aspect() - 2.0).abs() < 1e-4);
    }

    #[test]
    fn the_pad_draws_clears_and_is_done() {
        let size = (900.0, 700.0);
        let area = layout(size).area;
        let mut pad = Pad::default();
        // Done does nothing with nothing drawn.
        let done = layout(size).done;
        assert_eq!(
            pad.press(size, Point::new(done.center_x(), done.center_y())),
            PadAction::None
        );
        assert_eq!(
            pad.press(size, Point::new(area.left + 10.0, area.top + 10.0)),
            PadAction::None
        );
        assert!(pad.motion(size, Point::new(area.left + 40.0, area.top + 30.0)));
        pad.release();
        assert_eq!(pad.signature().unwrap().strokes[0].len(), 2);
        assert_eq!(
            pad.press(size, Point::new(done.center_x(), done.center_y())),
            PadAction::Done
        );
        let clear = layout(size).clear;
        pad.press(size, Point::new(clear.center_x(), clear.center_y()));
        assert!(pad.signature().is_none());
        // Outside the card is a cancel.
        assert_eq!(pad.press(size, Point::new(2.0, 2.0)), PadAction::Cancel);
    }
}
