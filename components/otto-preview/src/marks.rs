//! Marks on the document: what the person draws to point at something, and
//! what the agent draws to point back. Plan 0016, *Marks*.
//!
//! Both kinds live in one list and one coordinate space, the document's own:
//! a picture's pixels, or a page's PDF points with the page's index. That is
//! what keeps a mark on the thing it marks through zoom, pan and a reload,
//! and what an agent's tools can act on directly (a crop, a mask). The
//! window only converts to and from the screen to draw and to take strokes.
//!
//! The person's marks are numbered as they are drawn, and the numbers are
//! drawn beside them, so "brighten 1, remove 2" works in the chat. Marks not
//! yet sent go with the next message (see [`Marks::export`]); sent ones stay,
//! fainter. The agent's marks come in layers it names, so it can redraw or
//! take back one set at a time.

// Rust guideline compliant 2026-02-21

use std::path::{Path, PathBuf};

use otto_kit::prelude::*;
use otto_kit::preview::{Preview, PreviewLayout};
use otto_kit::skia::{Contains, PaintCap, PaintJoin, PaintStyle, PathBuilder, Point};
use serde_json::{json, Value};

/// The stroke a mark is drawn with on screen, in points.
const STROKE: f32 = 3.0;
/// The number badge's radius.
const BADGE: f32 = 9.0;
/// How far a freehand stroke's points must be apart to be kept, in points.
const STEP: f32 = 2.0;

/// Who drew a mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Author {
    Person,
    Agent,
}

/// A mark's shape, in document coordinates.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// A freehand stroke.
    Path(Vec<(f32, f32)>),
    /// A box, by two opposite corners.
    Rect((f32, f32), (f32, f32)),
    /// An ellipse in the box of two opposite corners.
    Ellipse((f32, f32), (f32, f32)),
    /// An arrow from the first point to the second.
    Arrow((f32, f32), (f32, f32)),
}

impl Shape {
    fn points(&self) -> Vec<(f32, f32)> {
        match self {
            Self::Path(points) => points.clone(),
            Self::Rect(a, b) | Self::Ellipse(a, b) | Self::Arrow(a, b) => vec![*a, *b],
        }
    }

    /// The box around the shape, as `[x, y, width, height]`.
    pub fn bounds(&self) -> [f32; 4] {
        let points = self.points();
        let (mut left, mut top) = (f32::MAX, f32::MAX);
        let (mut right, mut bottom) = (f32::MIN, f32::MIN);
        for (x, y) in &points {
            left = left.min(*x);
            top = top.min(*y);
            right = right.max(*x);
            bottom = bottom.max(*y);
        }
        if points.is_empty() {
            return [0.0; 4];
        }
        [left, top, right - left, bottom - top]
    }

    fn to_json(&self) -> Value {
        let round = |(x, y): (f32, f32)| json!([x.round(), y.round()]);
        match self {
            Self::Path(points) => json!({
                "kind": "path",
                "points": points.iter().copied().map(round).collect::<Vec<_>>(),
            }),
            Self::Rect(a, b) => json!({ "kind": "rect", "from": round(*a), "to": round(*b) }),
            Self::Ellipse(a, b) => json!({ "kind": "ellipse", "from": round(*a), "to": round(*b) }),
            Self::Arrow(a, b) => json!({ "kind": "arrow", "from": round(*a), "to": round(*b) }),
        }
    }

    /// A shape from the agent's JSON, as [`Self::to_json`] writes it.
    fn from_json(value: &Value) -> Option<Self> {
        let point = |value: &Value| -> Option<(f32, f32)> {
            let pair = value.as_array()?;
            Some((
                pair.first()?.as_f64()? as f32,
                pair.get(1)?.as_f64()? as f32,
            ))
        };
        let ends = || Some((point(value.get("from")?)?, point(value.get("to")?)?));
        match value.get("kind")?.as_str()? {
            "path" => {
                let points: Option<Vec<_>> =
                    value.get("points")?.as_array()?.iter().map(point).collect();
                points.filter(|points| points.len() >= 2).map(Self::Path)
            }
            "rect" => ends().map(|(a, b)| Self::Rect(a, b)),
            "ellipse" => ends().map(|(a, b)| Self::Ellipse(a, b)),
            "arrow" => ends().map(|(a, b)| Self::Arrow(a, b)),
            _ => None,
        }
    }
}

/// One mark.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    /// The number shown beside it: the person's count up from 1, the agent's
    /// are its own labels or none.
    pub n: Option<u32>,
    pub by: Author,
    /// The page it is on, for a document of pages.
    pub page: Option<usize>,
    pub shape: Shape,
    /// What the agent says about it, drawn beside it.
    pub label: Option<String>,
    /// The agent's layer, which it redraws or clears as one.
    pub layer: Option<String>,
    /// Gone with a message: drawn fainter, and not sent again.
    pub sent: bool,
}

/// Where the document's own coordinates land on the screen: the picture, or
/// one page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub page: Option<usize>,
    /// Where it is drawn, in window points.
    pub screen: Rect,
    /// Its own size: a picture's pixels, a page's PDF points.
    pub size: (f32, f32),
}

impl Frame {
    pub fn to_screen(self, (x, y): (f32, f32)) -> Point {
        Point::new(
            self.screen.left + x / self.size.0 * self.screen.width(),
            self.screen.top + y / self.size.1 * self.screen.height(),
        )
    }

    pub fn to_document(self, at: Point) -> (f32, f32) {
        (
            ((at.x - self.screen.left) / self.screen.width() * self.size.0).clamp(0.0, self.size.0),
            ((at.y - self.screen.top) / self.screen.height() * self.size.1).clamp(0.0, self.size.1),
        )
    }
}

/// The frames a preview lays out into: one for a picture, one per page.
/// Nothing else can be marked: text and listings reflow.
pub fn frames(preview: &Preview, layout: &PreviewLayout) -> Vec<Frame> {
    match preview {
        Preview::Pixels { pixels, .. } => vec![Frame {
            page: None,
            screen: layout.content,
            size: (
                pixels.intrinsic_width as f32,
                pixels.intrinsic_height as f32,
            ),
        }],
        Preview::Pages { pages, .. } => pages
            .iter()
            .zip(&layout.page_rects)
            .enumerate()
            .map(|(index, (page, rect))| Frame {
                page: Some(index),
                screen: *rect,
                size: (page.width, page.height),
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// A stroke being drawn.
#[derive(Debug, Clone)]
struct Stroke {
    frame: Frame,
    /// A box rather than a freehand line: Shift was held.
    boxed: bool,
    points: Vec<(f32, f32)>,
    /// The last point taken, on screen, to space the points out.
    last: Point,
}

/// Every mark on the document, and the stroke in progress.
#[derive(Debug, Default)]
pub struct Marks {
    pub list: Vec<Mark>,
    /// The person's next number.
    next: u32,
    stroke: Option<Stroke>,
    /// The mark whose badge the pointer is over: the badge shows a cross,
    /// and a click there deletes the mark.
    pub hovered: Option<usize>,
}

impl Marks {
    /// Start a stroke at `at`, on the frame under it. `boxed` draws a box.
    pub fn begin(&mut self, frames: &[Frame], at: Point, boxed: bool) -> bool {
        let Some(frame) = frames
            .iter()
            .find(|frame| frame.screen.contains(at))
            .copied()
        else {
            return false;
        };
        self.stroke = Some(Stroke {
            frame,
            boxed,
            points: vec![frame.to_document(at)],
            last: at,
        });
        true
    }

    pub fn drawing(&self) -> bool {
        self.stroke.is_some()
    }

    /// Carry the stroke on to `at`. Returns whether it moved.
    pub fn extend(&mut self, at: Point) -> bool {
        let Some(stroke) = &mut self.stroke else {
            return false;
        };
        if stroke.boxed {
            let point = stroke.frame.to_document(at);
            stroke.points.truncate(1);
            stroke.points.push(point);
            return true;
        }
        if (at.x - stroke.last.x).hypot(at.y - stroke.last.y) < STEP {
            return false;
        }
        stroke.last = at;
        stroke.points.push(stroke.frame.to_document(at));
        true
    }

    /// End the stroke, keeping it as the person's next numbered mark unless
    /// it was only a click.
    pub fn finish(&mut self) -> bool {
        let Some(stroke) = self.stroke.take() else {
            return false;
        };
        let shape = match (stroke.boxed, stroke.points.as_slice()) {
            (true, [a, b]) => Shape::Rect(*a, *b),
            (false, points) if points.len() >= 2 => Shape::Path(points.to_vec()),
            _ => return true,
        };
        let [_, _, width, height] = shape.bounds();
        if width < 1.0 && height < 1.0 {
            return true;
        }
        self.next += 1;
        self.list.push(Mark {
            n: Some(self.next),
            by: Author::Person,
            page: stroke.frame.page,
            shape,
            label: None,
            layer: None,
            sent: false,
        });
        true
    }

    /// The mark whose badge is under `at`, the last drawn first.
    pub fn badge_at(&self, frames: &[Frame], at: Point) -> Option<usize> {
        self.list
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, mark)| {
                let frame = frames.iter().find(|frame| frame.page == mark.page)?;
                let (rect, _) = badge(frame, mark, 1.0, false)?;
                rect.with_outset((2.0, 2.0)).contains(at).then_some(index)
            })
    }

    /// Follow the pointer over the badges. Returns whether the one under it
    /// changed.
    pub fn hover(&mut self, frames: &[Frame], at: Option<Point>) -> bool {
        let hovered = at.and_then(|at| self.badge_at(frames, at));
        std::mem::replace(&mut self.hovered, hovered) != hovered
    }

    /// Delete the mark at `index`, the person's or the agent's.
    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.list.len() {
            return false;
        }
        self.list.remove(index);
        self.hovered = None;
        self.renumber();
        true
    }

    /// The person's next number follows the highest still there.
    fn renumber(&mut self) {
        self.next = self
            .list
            .iter()
            .filter(|mark| mark.by == Author::Person)
            .filter_map(|mark| mark.n)
            .max()
            .unwrap_or(0);
    }

    /// Take back the person's last mark not yet sent.
    pub fn undo(&mut self) -> bool {
        let Some(index) = self
            .list
            .iter()
            .rposition(|mark| mark.by == Author::Person && !mark.sent)
        else {
            return false;
        };
        self.list.remove(index);
        self.hovered = None;
        self.renumber();
        true
    }

    /// The numbers of the person's marks that go with the next message.
    pub fn pending(&self) -> Vec<u32> {
        self.list
            .iter()
            .filter(|mark| mark.by == Author::Person && !mark.sent)
            .filter_map(|mark| mark.n)
            .collect()
    }

    /// Replace the agent's layer `layer` with `marks`, given as JSON: a list
    /// of `{ "shape": {...}, "label"?, "page"? }`. Returns how many were
    /// taken, or why none could be.
    pub fn draw_agent(&mut self, layer: &str, marks: &Value) -> Result<usize, String> {
        let list = marks
            .as_array()
            .ok_or("marks must be a list of { shape, label?, page? }")?;
        let mut taken = Vec::new();
        for (index, mark) in list.iter().enumerate() {
            let shape = mark
                .get("shape")
                .and_then(Shape::from_json)
                .ok_or_else(|| format!("mark {index}: a shape is {{\"kind\": \"rect\"|\"ellipse\"|\"arrow\", \"from\": [x, y], \"to\": [x, y]}} or {{\"kind\": \"path\", \"points\": [[x, y], ...]}}"))?;
            taken.push(Mark {
                n: None,
                by: Author::Agent,
                page: mark
                    .get("page")
                    .and_then(Value::as_u64)
                    .map(|page| page as usize),
                shape,
                label: mark.get("label").and_then(Value::as_str).map(str::to_owned),
                layer: Some(layer.to_owned()),
                sent: false,
            });
        }
        self.clear_layer(layer);
        let count = taken.len();
        self.list.extend(taken);
        Ok(count)
    }

    /// Take away the agent's layer `layer`, or every agent mark for `None`.
    pub fn clear_layer(&mut self, layer: &str) -> usize {
        let before = self.list.len();
        self.list.retain(|mark| {
            !(mark.by == Author::Agent
                && (layer.is_empty() || mark.layer.as_deref() == Some(layer)))
        });
        before - self.list.len()
    }

    /// Take away the person's marks.
    pub fn clear_person(&mut self) -> usize {
        let before = self.list.len();
        self.list.retain(|mark| mark.by != Author::Person);
        self.next = 0;
        before - self.list.len()
    }

    /// Every mark as JSON, for the agent.
    pub fn to_json(&self, file: &Path, size: Option<(f32, f32)>, only_pending: bool) -> Value {
        let marks: Vec<Value> = self
            .list
            .iter()
            .filter(|mark| !only_pending || (mark.by == Author::Person && !mark.sent))
            .map(|mark| {
                let mut value = json!({
                    "by": match mark.by { Author::Person => "person", Author::Agent => "agent" },
                    "shape": mark.shape.to_json(),
                    "bounds": mark.shape.bounds().map(f32::round),
                });
                if let Some(n) = mark.n {
                    value["n"] = json!(n);
                }
                if let Some(page) = mark.page {
                    value["page"] = json!(page);
                }
                if let Some(label) = &mark.label {
                    value["label"] = json!(label);
                }
                if let Some(layer) = &mark.layer {
                    value["layer"] = json!(layer);
                }
                if mark.by == Author::Person {
                    value["sent"] = json!(mark.sent);
                }
                value
            })
            .collect();
        let mut out = json!({
            "file": file,
            "units": if size.is_some() { "the picture's pixels" } else { "PDF points on the page named by \"page\" (0-based)" },
            "marks": marks,
        });
        if let Some((width, height)) = size {
            out["size"] = json!([width, height]);
        }
        out
    }

    /// Write the marks going with the next message where an agent can read
    /// them: a JSON file, and for a picture the picture with the marks drawn
    /// and numbered. Marks them sent. Returns the files, none when nothing is
    /// pending.
    pub fn export(&mut self, file: &Path, preview: &Preview, dir: &Path) -> Vec<PathBuf> {
        if self.pending().is_empty() {
            return Vec::new();
        }
        if let Err(err) = std::fs::create_dir_all(dir) {
            tracing::warn!(%err, dir = %dir.display(), "could not write the marks");
            return Vec::new();
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|time| time.as_millis())
            .unwrap_or_default();
        let size = picture_size(preview);
        let mut files = Vec::new();
        let picture =
            render(preview, &self.list).map(|png| (dir.join(format!("marks-{stamp}.png")), png));
        let mut json = self.to_json(file, size, true);
        json["about"] = json!(
            "Marks the person drew on the file in Otto's Preview to point at what they mean, \
             numbered as on screen. Coordinates are in the units named; bounds are [x, y, w, h]."
        );
        if let Some((path, _)) = &picture {
            json["picture"] = json!(path);
        }
        let json_path = dir.join(format!("marks-{stamp}.json"));
        match serde_json::to_vec_pretty(&json).map(|bytes| std::fs::write(&json_path, bytes)) {
            Ok(Ok(())) => files.push(json_path),
            _ => tracing::warn!(path = %json_path.display(), "could not write the marks"),
        }
        if let Some((path, png)) = picture {
            match std::fs::write(&path, png) {
                Ok(()) => files.push(path),
                Err(err) => tracing::warn!(%err, "could not write the marked picture"),
            }
        }
        for mark in &mut self.list {
            if mark.by == Author::Person {
                mark.sent = true;
            }
        }
        files
    }
}

/// Where the marks going with a message, and the views an agent asks for,
/// are written for the agent to read.
pub fn dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("otto-preview")
}

/// A picture's own size, in pixels.
pub fn picture_size(preview: &Preview) -> Option<(f32, f32)> {
    match preview {
        Preview::Pixels { pixels, .. } => Some((
            pixels.intrinsic_width as f32,
            pixels.intrinsic_height as f32,
        )),
        _ => None,
    }
}

/// The picture as decoded, with `marks` drawn on it and numbered, as PNG.
/// `None` for anything but a picture.
pub fn render(preview: &Preview, marks: &[Mark]) -> Option<Vec<u8>> {
    let Preview::Pixels { pixels, .. } = preview else {
        return None;
    };
    let image = pixels.to_image()?;
    let (width, height) = (image.width(), image.height());
    let mut surface = otto_kit::skia::surfaces::raster_n32_premul((width, height))?;
    let canvas = surface.canvas();
    canvas.draw_image(&image, (0, 0), None);
    let frame = Frame {
        page: None,
        screen: Rect::from_wh(width as f32, height as f32),
        size: (
            pixels.intrinsic_width as f32,
            pixels.intrinsic_height as f32,
        ),
    };
    // Thicker than on screen: the picture is seen scaled down.
    let scale = (width.max(height) as f32 / 800.0).max(1.0);
    for mark in marks {
        draw_mark(canvas, &frame, mark, scale, false, false);
    }
    let encoded =
        surface
            .image_snapshot()
            .encode(None, otto_kit::skia::EncodedImageFormat::PNG, None)?;
    Some(encoded.as_bytes().to_vec())
}

/// Paint every mark over the content, and the stroke in progress.
pub fn draw(canvas: &Canvas, frames: &[Frame], marks: &Marks) {
    for (index, mark) in marks.list.iter().enumerate() {
        if let Some(frame) = frames.iter().find(|frame| frame.page == mark.page) {
            draw_mark(
                canvas,
                frame,
                mark,
                1.0,
                mark.sent,
                marks.hovered == Some(index),
            );
        }
    }
    if let Some(stroke) = &marks.stroke {
        let shape = match (stroke.boxed, stroke.points.as_slice()) {
            (true, [a, b]) => Shape::Rect(*a, *b),
            (_, points) => Shape::Path(points.to_vec()),
        };
        let mark = Mark {
            n: None,
            by: Author::Person,
            page: stroke.frame.page,
            shape,
            label: None,
            layer: None,
            sent: false,
        };
        draw_mark(canvas, &stroke.frame, &mark, 1.0, false, false);
    }
}

/// Where a mark's badge starts: its first point, or its box's corner.
fn anchor(frame: &Frame, mark: &Mark) -> Option<Point> {
    match &mark.shape {
        Shape::Path(points) => points.first().map(|point| frame.to_screen(*point)),
        Shape::Rect(a, b) | Shape::Ellipse(a, b) => {
            let (a, b) = (frame.to_screen(*a), frame.to_screen(*b));
            Some(Point::new(a.x.min(b.x), a.y.min(b.y)))
        }
        Shape::Arrow(a, _) => Some(frame.to_screen(*a)),
    }
}

/// The badge beside a mark and what it says: the person's number, the
/// agent's label or a dot, and a cross while the pointer is over it.
fn badge(frame: &Frame, mark: &Mark, scale: f32, hovered: bool) -> Option<(Rect, String)> {
    let anchor = anchor(frame, mark)?;
    let text = match (hovered, mark.n, &mark.label) {
        (true, ..) => "\u{2715}".to_owned(),
        (false, Some(n), _) => n.to_string(),
        (false, None, Some(label)) => label.clone(),
        (false, None, None) => "\u{2022}".to_owned(),
    };
    let radius = BADGE * scale;
    let (text_w, _) = badge_style(scale).font().measure_str(&text, None);
    // A label keeps its width while it shows the cross, so the pointer
    // stays on it.
    let shown = match (hovered, mark.n, &mark.label) {
        (true, None, Some(label)) => badge_style(scale).font().measure_str(label, None).0,
        _ => text_w,
    };
    let width = (shown + radius).max(2.0 * radius);
    let rect = Rect::from_xywh(
        anchor.x - radius,
        anchor.y - 2.0 * radius - 2.0,
        width,
        2.0 * radius,
    );
    Some((rect, text))
}

fn badge_style(scale: f32) -> TextStyle {
    TextStyle {
        size: 12.0 * scale,
        ..styles::FOOTNOTE_EMPHASIZED
    }
}

fn colour(by: Author) -> Color {
    match by {
        Author::Person => Color::from_rgb(255, 59, 48),
        Author::Agent => Color::from_rgb(10, 132, 255),
    }
}

fn draw_mark(canvas: &Canvas, frame: &Frame, mark: &Mark, scale: f32, faint: bool, hovered: bool) {
    let mut colour = colour(mark.by);
    if faint {
        colour = colour.with_a(110);
    }
    let mut stroke = Paint::default();
    stroke.set_anti_alias(true);
    stroke.set_style(PaintStyle::Stroke);
    stroke.set_stroke_width(STROKE * scale);
    stroke.set_stroke_cap(PaintCap::Round);
    stroke.set_stroke_join(PaintJoin::Round);
    stroke.set_color(colour);

    let corners = |a, b| {
        let (a, b): (Point, Point) = (frame.to_screen(a), frame.to_screen(b));
        Rect::from_ltrb(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y))
    };
    match &mark.shape {
        Shape::Path(points) => {
            let mut path = PathBuilder::new();
            for (index, point) in points.iter().enumerate() {
                let at = frame.to_screen(*point);
                if index == 0 {
                    path.move_to(at);
                } else {
                    path.line_to(at);
                }
            }
            canvas.draw_path(&path.detach(), &stroke);
        }
        Shape::Rect(a, b) => {
            let rect = corners(*a, *b);
            canvas.draw_rect(rect, &stroke);
        }
        Shape::Ellipse(a, b) => {
            let rect = corners(*a, *b);
            canvas.draw_oval(rect, &stroke);
        }
        Shape::Arrow(a, b) => {
            let (from, to) = (frame.to_screen(*a), frame.to_screen(*b));
            canvas.draw_line(from, to, &stroke);
            let angle = (to.y - from.y).atan2(to.x - from.x);
            let head = 12.0 * scale;
            for side in [-0.5_f32, 0.5] {
                let back = angle + std::f32::consts::PI + side;
                canvas.draw_line(
                    to,
                    Point::new(to.x + head * back.cos(), to.y + head * back.sin()),
                    &stroke,
                );
            }
        }
    }
    // The number, label or dot in a filled pill.
    let Some((pill, text)) = badge(frame, mark, scale, hovered) else {
        return;
    };
    let radius = BADGE * scale;
    let mut fill = Paint::default();
    fill.set_anti_alias(true);
    fill.set_color(colour);
    canvas.draw_round_rect(pill, radius, radius, &fill);
    Label::new(text)
        .with_style(badge_style(scale))
        .with_color(Color::WHITE)
        .centered_at(pill.center_x(), pill.center_y())
        .render(canvas);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture_frame() -> Frame {
        Frame {
            page: None,
            screen: Rect::from_xywh(100.0, 50.0, 400.0, 300.0),
            size: (4000.0, 3000.0),
        }
    }

    #[test]
    fn a_stroke_is_kept_in_the_pictures_pixels() {
        let frame = picture_frame();
        let mut marks = Marks::default();
        assert!(marks.begin(&[frame], Point::new(100.0, 50.0), false));
        marks.extend(Point::new(300.0, 200.0));
        assert!(marks.finish());
        let mark = &marks.list[0];
        assert_eq!(mark.n, Some(1));
        assert_eq!(mark.shape, Shape::Path(vec![(0.0, 0.0), (2000.0, 1500.0)]));
        assert_eq!(marks.pending(), vec![1]);
    }

    #[test]
    fn a_click_is_not_a_mark_and_a_box_takes_two_corners() {
        let frame = picture_frame();
        let mut marks = Marks::default();
        marks.begin(&[frame], Point::new(200.0, 100.0), false);
        marks.finish();
        assert!(marks.list.is_empty());
        marks.begin(&[frame], Point::new(200.0, 100.0), true);
        marks.extend(Point::new(220.0, 110.0));
        marks.extend(Point::new(300.0, 150.0));
        marks.finish();
        assert_eq!(
            marks.list[0].shape,
            Shape::Rect((1000.0, 500.0), (2000.0, 1000.0))
        );
    }

    #[test]
    fn the_agent_redraws_its_layers_and_the_person_keeps_theirs() {
        let mut marks = Marks::default();
        marks.begin(&[picture_frame()], Point::new(100.0, 50.0), true);
        marks.extend(Point::new(200.0, 100.0));
        marks.finish();
        let drawn = json!([
            { "shape": { "kind": "rect", "from": [10, 10], "to": [50, 50] }, "label": "dust" },
            { "shape": { "kind": "arrow", "from": [0, 0], "to": [9, 9] } }
        ]);
        assert_eq!(marks.draw_agent("spots", &drawn), Ok(2));
        assert_eq!(marks.draw_agent("spots", &json!([drawn[0].clone()])), Ok(1));
        assert_eq!(
            marks.list.len(),
            2,
            "the layer was replaced, the person's mark kept"
        );
        assert!(marks
            .draw_agent("bad", &json!([{ "shape": { "kind": "star" } }]))
            .is_err());
        assert_eq!(marks.clear_layer(""), 1);
        assert_eq!(marks.list.len(), 1);
    }

    #[test]
    fn a_click_on_a_badge_finds_its_mark() {
        let frame = picture_frame();
        let mut marks = Marks::default();
        marks.begin(&[frame], Point::new(200.0, 100.0), true);
        marks.extend(Point::new(300.0, 200.0));
        marks.finish();
        // The badge sits just above the mark's top-left corner.
        let on_badge = Point::new(200.0 + 2.0, 100.0 - BADGE - 2.0);
        assert_eq!(marks.badge_at(&[frame], on_badge), Some(0));
        assert_eq!(marks.badge_at(&[frame], Point::new(250.0, 150.0)), None);
        assert!(marks.hover(&[frame], Some(on_badge)));
        assert!(marks.remove(0));
        assert!(marks.list.is_empty());
    }

    #[test]
    fn undo_takes_back_only_what_is_not_sent() {
        let mut marks = Marks::default();
        for _ in 0..2 {
            marks.begin(&[picture_frame()], Point::new(100.0, 50.0), true);
            marks.extend(Point::new(200.0, 100.0));
            marks.finish();
        }
        marks.list[0].sent = true;
        assert!(marks.undo());
        assert!(!marks.undo(), "the sent mark stays");
        assert_eq!(marks.pending(), Vec::<u32>::new());
    }
}
