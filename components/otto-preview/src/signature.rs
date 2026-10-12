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
use otto_kit::components::push_button::{self, PushButton, Role};
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
    let text = serde_json::to_string(signature).map_err(std::io::Error::other)?;
    write_private(&path, text.as_bytes())
}

/// Write `bytes` to `path` for the person alone to read: into a new file
/// beside it, readable by its owner only from the moment it exists, synced,
/// then renamed over `path`, so nobody else ever reads it and nobody reads
/// half of it.
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::hash::{BuildHasher, Hasher};
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("no folder to keep it in"))?;
    std::fs::create_dir_all(dir)?;
    // A name nobody can guess ahead of time: the standard library's random
    // hash keys over the clock and the process.
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    hasher.write_u128(now.as_nanos());
    hasher.write_u32(std::process::id());
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = dir.join(format!(".{name}.{:016x}", hasher.finish()));
    // `create_new` never opens a file, or a link, already there.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    let written = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .and_then(|()| std::fs::rename(&temp, path));
    drop(file);
    if let Err(err) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(err);
    }
    if let Ok(dir) = std::fs::File::open(dir) {
        let _ = dir.sync_all();
    }
    Ok(())
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

/// The pad's buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Clear,
    Cancel,
    Done,
}

impl Control {
    fn label(self) -> String {
        match self {
            Self::Clear => otto_kit::t_owned!("studio-signature-clear"),
            Self::Cancel => otto_kit::t_owned!("studio-signature-cancel"),
            Self::Done => otto_kit::t_owned!("studio-signature-done"),
        }
    }
}

/// The pad a signature is drawn on: a card over the window.
#[derive(Debug, Clone, Default)]
pub struct Pad {
    strokes: Vec<Vec<(f32, f32)>>,
    drawing: bool,
    /// The button a press began on, and whether the pointer is still on it:
    /// a button acts on release, over the one it was pressed on.
    held: Option<(Control, bool)>,
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
    let card_h = TITLE_H + PAD_H + push_button::HEIGHT + 3.0 * MARGIN;
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
    let (y, h) = (area.bottom + MARGIN, push_button::HEIGHT);
    let wide = |control: Control| push_button::width(&control.label());
    let done_w = wide(Control::Done);
    let done = Rect::from_xywh(card.right - MARGIN - done_w, y, done_w, h);
    let cancel_w = wide(Control::Cancel);
    let cancel = Rect::from_xywh(done.left - 8.0 - cancel_w, y, cancel_w, h);
    let clear = Rect::from_xywh(card.left + MARGIN, y, wide(Control::Clear), h);
    PadLayout {
        card,
        area,
        clear,
        cancel,
        done,
    }
}

impl PadLayout {
    fn control_at(&self, at: Point) -> Option<Control> {
        [
            (Control::Clear, self.clear),
            (Control::Cancel, self.cancel),
            (Control::Done, self.done),
        ]
        .into_iter()
        .find(|(_, rect)| rect.contains(at))
        .map(|(control, _)| control)
    }

    fn rect(&self, control: Control) -> Rect {
        match control {
            Control::Clear => self.clear,
            Control::Cancel => self.cancel,
            Control::Done => self.done,
        }
    }
}

impl Pad {
    /// The signature drawn so far, if anything has been.
    pub fn signature(&self) -> Option<Signature> {
        (!self.strokes.is_empty()).then(|| Signature {
            strokes: self.strokes.clone(),
        })
    }

    /// A press anywhere in a window of `size`: a stroke on the paper, a
    /// button held, or a cancel outside the card.
    pub fn press(&mut self, size: (f32, f32), at: Point) -> PadAction {
        let layout = layout(size);
        if layout.area.contains(at) {
            self.strokes
                .push(vec![(at.x - layout.area.left, at.y - layout.area.top)]);
            self.drawing = true;
            return PadAction::None;
        }
        if let Some(control) = layout.control_at(at) {
            self.held = Some((control, true));
            return PadAction::None;
        }
        if !layout.card.contains(at) {
            return PadAction::Cancel;
        }
        PadAction::None
    }

    /// The pointer moved. Returns whether the pad changed.
    pub fn motion(&mut self, size: (f32, f32), at: Point) -> bool {
        if let Some((control, over)) = &mut self.held {
            let now = layout(size).control_at(at) == Some(*control);
            return std::mem::replace(over, now) != now;
        }
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

    /// The button went up at `at`: a stroke ends, or the held button acts
    /// if the pointer is still on it.
    pub fn release(&mut self, size: (f32, f32), at: Point) -> PadAction {
        if std::mem::take(&mut self.drawing) {
            // A click is a dot.
            if let Some(stroke) = self.strokes.last_mut() {
                if stroke.len() == 1 {
                    let (x, y) = stroke[0];
                    stroke.push((x + 0.5, y + 0.5));
                }
            }
            return PadAction::None;
        }
        let Some((control, _)) = self.held.take() else {
            return PadAction::None;
        };
        if layout(size).control_at(at) != Some(control) {
            return PadAction::None;
        }
        match control {
            Control::Clear => {
                self.strokes.clear();
                PadAction::None
            }
            Control::Cancel => PadAction::Cancel,
            Control::Done if self.strokes.is_empty() => PadAction::None,
            Control::Done => PadAction::Done,
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

        let drawn = !self.strokes.is_empty();
        let held = self
            .held
            .and_then(|(control, over)| over.then_some(control));
        for (control, role, enabled) in [
            (Control::Clear, Role::Normal, drawn),
            (Control::Cancel, Role::Normal, true),
            (Control::Done, Role::Default, drawn),
        ] {
            push_button::draw(
                canvas,
                layout.rect(control),
                &control.label(),
                PushButton::new(role)
                    .pressed(held == Some(control))
                    .enabled(enabled),
                theme,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_signature_is_written_for_its_owner_alone_and_whole() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("otto/signature.json");
        write_private(&path, b"first").unwrap();
        write_private(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("signature.json")]);
    }

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
        let on_done = Point::new(done.center_x(), done.center_y());
        let click = |pad: &mut Pad, at: Point| {
            pad.press(size, at);
            pad.release(size, at)
        };
        assert_eq!(click(&mut pad, on_done), PadAction::None);
        let start = Point::new(area.left + 10.0, area.top + 10.0);
        assert_eq!(pad.press(size, start), PadAction::None);
        let end = Point::new(area.left + 40.0, area.top + 30.0);
        assert!(pad.motion(size, end));
        pad.release(size, end);
        assert_eq!(pad.signature().unwrap().strokes[0].len(), 2);
        // A press on Done let go elsewhere takes it back.
        pad.press(size, on_done);
        assert!(pad.motion(size, Point::new(2.0, 2.0)));
        assert_eq!(pad.release(size, Point::new(2.0, 2.0)), PadAction::None);
        assert_eq!(click(&mut pad, on_done), PadAction::Done);
        let clear = layout(size).clear;
        click(&mut pad, Point::new(clear.center_x(), clear.center_y()));
        assert!(pad.signature().is_none());
        // Outside the card is a cancel.
        assert_eq!(pad.press(size, Point::new(2.0, 2.0)), PadAction::Cancel);
    }
}
