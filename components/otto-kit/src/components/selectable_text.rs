//! Read-only text that can be selected and copied.
//!
//! An info panel's facts — a name, a path, a size, a date — are worth copying
//! as often as they are worth reading, and a panel drawn as labels offers
//! neither the drag nor the Ctrl+C. This is the missing half: the panel lays
//! its text out as [`TextRun`]s, draws them through this module rather than as
//! bare labels, and hands its pointer events to one [`TextSelection`], which
//! answers what is selected and what to copy.
//!
//! Runs are given in reading order, and a selection can run from one to the
//! next — a drag from a label down through several values selects all of it,
//! the way it does on a web page. It is a model only: which text is where
//! comes from the runs, so what is drawn and what is hit cannot disagree.
//!
//! One click places the start of a drag, two select a word, three the whole
//! run. The clicks are counted here, from their times and places, so a host
//! only has to report the presses.

use std::ops::Range;
use std::time::{Duration, Instant};

use skia_safe::{Canvas, Color, Contains, Paint, Point, RRect, Rect};

use crate::common::Renderable;
use crate::components::label::Label;
use crate::typography::{measure_runs, TextStyle};

/// One piece of text drawn on one line: a label, a value, a name.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    /// What is drawn, which may be shortened with an ellipsis to fit.
    pub text: String,
    /// What copying the whole run gives: the full value behind a shortened
    /// one. Equal to `text` unless [`Self::with_full`] said otherwise.
    pub full: String,
    pub style: TextStyle,
    pub color: Color,
    /// Left edge of the text, and the line's vertical centre — the same
    /// placement [`Label::centered_on`] takes.
    pub left: f32,
    pub cy: f32,
}

impl TextRun {
    /// `text` with its left edge at `left`, centred vertically on `cy`.
    pub fn at(text: impl Into<String>, style: TextStyle, color: Color, left: f32, cy: f32) -> Self {
        let text = text.into();
        Self {
            full: text.clone(),
            text,
            style,
            color,
            left,
            cy,
        }
    }

    /// `text` centred horizontally on `cx`, the way [`Label::centered_at`]
    /// places it.
    pub fn centered(
        text: impl Into<String>,
        style: TextStyle,
        color: Color,
        cx: f32,
        cy: f32,
    ) -> Self {
        let mut run = Self::at(text, style, color, cx, cy);
        run.left = cx - run.width() / 2.0;
        run
    }

    /// Copy `full` when the whole run is selected, rather than the shortened
    /// text on screen.
    pub fn with_full(mut self, full: impl Into<String>) -> Self {
        self.full = full.into();
        self
    }

    pub fn width(&self) -> f32 {
        measure_runs(&self.style.font(), &self.text)
    }

    fn height(&self) -> f32 {
        let (_, metrics) = self.style.font().metrics();
        (metrics.descent - metrics.ascent).max(self.style.size)
    }

    /// The box the run's text occupies — what a press has to land in.
    pub fn rect(&self) -> Rect {
        let h = self.height();
        Rect::from_xywh(self.left, self.cy - h / 2.0, self.width(), h)
    }

    /// Where on the line the character boundary `offset` falls.
    pub fn x_at(&self, offset: usize) -> f32 {
        let offset = floor_boundary(&self.text, offset);
        self.left + measure_runs(&self.style.font(), &self.text[..offset])
    }

    /// The character boundary nearest `x`.
    pub fn offset_at(&self, x: f32) -> usize {
        let mut best = (0, (x - self.left).abs());
        for (offset, _) in self.text.char_indices().skip(1) {
            let distance = (x - self.x_at(offset)).abs();
            if distance < best.1 {
                best = (offset, distance);
            }
        }
        let end = self.text.len();
        if (x - self.x_at(end)).abs() < best.1 {
            best = (end, 0.0);
        }
        best.0
    }

    /// Draw the text.
    pub fn draw(&self, canvas: &Canvas) {
        Label::new(self.text.as_str())
            .with_style(self.style)
            .with_color(self.color)
            .centered_on(self.left, self.cy)
            .render(canvas);
    }
}

/// The largest character boundary of `text` at or below `offset`.
fn floor_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// A place in a list of runs: which run, and which character boundary in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextPos {
    pub run: usize,
    pub offset: usize,
}

/// Two presses closer together than this, and nearer each other than
/// [`MULTI_CLICK_SLOP`], count as one double or triple click.
const MULTI_CLICK: Duration = Duration::from_millis(400);
const MULTI_CLICK_SLOP: f32 = 4.0;

/// What is selected across a panel's runs, and the drag making it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextSelection {
    anchor: Option<TextPos>,
    focus: Option<TextPos>,
    dragging: bool,
    /// The last press, for counting clicks: when, where and how many.
    last_press: Option<(Instant, Point, u32)>,
}

impl TextSelection {
    pub fn new() -> Self {
        Self::default()
    }

    /// The run under `(x, y)`, and the boundary in it nearest `x`.
    pub fn hit(runs: &[TextRun], x: f32, y: f32) -> Option<TextPos> {
        let point = Point::new(x, y);
        runs.iter()
            .position(|run| {
                !run.text.is_empty() && run.rect().with_outset((2.0, 1.0)).contains(point)
            })
            .map(|run| TextPos {
                run,
                offset: runs[run].offset_at(x),
            })
    }

    /// Whether `(x, y)` is over any run: where the pointer should be an I-beam.
    pub fn is_over_text(runs: &[TextRun], x: f32, y: f32) -> bool {
        Self::hit(runs, x, y).is_some()
    }

    /// A press at `(x, y)`, at `now`. Returns whether it landed on text and
    /// was taken; a press anywhere else clears the selection and is left for
    /// the host.
    pub fn press(&mut self, runs: &[TextRun], x: f32, y: f32, now: Instant) -> bool {
        let Some(pos) = Self::hit(runs, x, y) else {
            self.clear();
            self.last_press = None;
            return false;
        };
        let point = Point::new(x, y);
        let clicks = match self.last_press {
            Some((at, last, count))
                if now.duration_since(at) <= MULTI_CLICK
                    && (last - point).length() <= MULTI_CLICK_SLOP =>
            {
                count % 3 + 1
            }
            _ => 1,
        };
        self.last_press = Some((now, point, clicks));
        let text = &runs[pos.run].text;
        let (start, end) = match clicks {
            2 => word_at(text, pos.offset),
            3 => (0, text.len()),
            _ => (pos.offset, pos.offset),
        };
        self.anchor = Some(TextPos {
            run: pos.run,
            offset: start,
        });
        self.focus = Some(TextPos {
            run: pos.run,
            offset: end,
        });
        // A double or triple click selects and is done; a single one starts
        // a drag.
        self.dragging = clicks == 1;
        true
    }

    /// The pointer moved to `(x, y)` with the button down. Returns whether
    /// the selection changed.
    pub fn drag(&mut self, runs: &[TextRun], x: f32, y: f32) -> bool {
        if !self.dragging || runs.is_empty() {
            return false;
        }
        let pos = Self::hit(runs, x, y).unwrap_or_else(|| nearest(runs, x, y));
        if self.focus == Some(pos) {
            return false;
        }
        self.focus = Some(pos);
        true
    }

    /// The button came up.
    pub fn release(&mut self) {
        self.dragging = false;
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// Drop the selection. Returns whether there was one to drop.
    pub fn clear(&mut self) -> bool {
        let had = self.has_selection();
        self.anchor = None;
        self.focus = None;
        self.dragging = false;
        had
    }

    /// Whether anything is selected: a range, not a caret between two
    /// characters.
    pub fn has_selection(&self) -> bool {
        matches!((self.anchor, self.focus), (Some(a), Some(f)) if a != f)
    }

    fn ordered(&self) -> Option<(TextPos, TextPos)> {
        let (a, f) = (self.anchor?, self.focus?);
        (a != f).then(|| if a <= f { (a, f) } else { (f, a) })
    }

    /// The selected part of each run it touches, as byte ranges, clamped to
    /// the runs as they are now.
    pub fn ranges(&self, runs: &[TextRun]) -> Vec<(usize, Range<usize>)> {
        let Some((start, end)) = self.ordered() else {
            return Vec::new();
        };
        (start.run..=end.run.min(runs.len().saturating_sub(1)))
            .filter_map(|index| {
                let text = &runs.get(index)?.text;
                let from = if index == start.run { start.offset } else { 0 };
                let to = if index == end.run {
                    end.offset
                } else {
                    text.len()
                };
                let (from, to) = (floor_boundary(text, from), floor_boundary(text, to));
                (from < to).then_some((index, from..to))
            })
            .collect()
    }

    /// The selected text, ready for the clipboard. A run selected whole
    /// gives its full value; runs on one line are joined with a space and
    /// lines with a newline.
    pub fn selected_text(&self, runs: &[TextRun]) -> Option<String> {
        let ranges = self.ranges(runs);
        let mut out = String::new();
        let mut last_cy: Option<f32> = None;
        for (index, range) in ranges {
            let run = &runs[index];
            if let Some(cy) = last_cy {
                out.push(if (cy - run.cy).abs() < 2.0 { ' ' } else { '\n' });
            }
            if range == (0..run.text.len()) {
                out.push_str(&run.full);
            } else {
                out.push_str(&run.text[range]);
            }
            last_cy = Some(run.cy);
        }
        (!out.is_empty()).then_some(out)
    }

    /// Draw the selection's highlight behind the runs, in `color`. Call it
    /// before drawing the runs themselves.
    pub fn draw_highlight(&self, canvas: &Canvas, runs: &[TextRun], color: Color) {
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(color);
        for (index, range) in self.ranges(runs) {
            let run = &runs[index];
            let rect = run.rect();
            let band = Rect::from_ltrb(
                run.x_at(range.start),
                rect.top,
                run.x_at(range.end),
                rect.bottom,
            );
            canvas.draw_rrect(RRect::new_rect_xy(band, 2.0, 2.0), &paint);
        }
    }

    /// A key for what the highlight looks like, for hosts that cache what
    /// they draw and need to know when to draw again.
    pub fn key(&self) -> Option<(TextPos, TextPos)> {
        self.ordered()
    }
}

/// The run nearest `(x, y)` — the closest line, then the closest place on
/// it — for a drag that has left the text.
fn nearest(runs: &[TextRun], x: f32, y: f32) -> TextPos {
    let distance = |run: &TextRun| {
        let rect = run.rect();
        let dy = if y < rect.top {
            rect.top - y
        } else if y > rect.bottom {
            y - rect.bottom
        } else {
            0.0
        };
        let dx = if x < rect.left {
            rect.left - x
        } else if x > rect.right {
            x - rect.right
        } else {
            0.0
        };
        dy * 1000.0 + dx
    };
    let run = (0..runs.len())
        .min_by(|&a, &b| distance(&runs[a]).total_cmp(&distance(&runs[b])))
        .unwrap_or(0);
    TextPos {
        run,
        offset: runs[run].offset_at(x),
    }
}

/// The word around byte `offset` of `text`: a run of letters and digits, or
/// of anything else that is not a space, or the space itself.
fn word_at(text: &str, offset: usize) -> (usize, usize) {
    let offset = floor_boundary(text, offset);
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            0
        } else if c.is_whitespace() {
            1
        } else {
            2
        }
    };
    let Some(here) = text[offset..]
        .chars()
        .next()
        .or_else(|| text[..offset].chars().next_back())
    else {
        return (0, 0);
    };
    let kind = class(here);
    let start = text[..offset]
        .char_indices()
        .rev()
        .take_while(|(_, c)| class(*c) == kind)
        .last()
        .map_or(offset, |(i, _)| i);
    let end = text[offset..]
        .char_indices()
        .find(|(_, c)| class(*c) != kind)
        .map_or(text.len(), |(i, _)| offset + i);
    (start, end.max(start))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typography::styles;

    fn runs() -> Vec<TextRun> {
        vec![
            TextRun::at("Where", styles::BODY, Color::BLACK, 10.0, 20.0),
            TextRun::at("~/Pictures/Trip", styles::BODY, Color::BLACK, 100.0, 20.0)
                .with_full("/home/me/Pictures/Trip"),
            TextRun::at("Modified", styles::BODY, Color::BLACK, 10.0, 50.0),
            TextRun::at("25 Sep 2026", styles::BODY, Color::BLACK, 100.0, 50.0),
        ]
    }

    fn centre(run: &TextRun) -> (f32, f32) {
        let rect = run.rect();
        (rect.center_x(), rect.center_y())
    }

    #[test]
    fn offsets_and_positions_agree() {
        let run = &runs()[3];
        for offset in [0, 3, 6, run.text.len()] {
            assert_eq!(run.offset_at(run.x_at(offset)), offset);
        }
        assert_eq!(run.offset_at(run.left - 50.0), 0);
        assert_eq!(run.offset_at(run.rect().right + 50.0), run.text.len());
    }

    #[test]
    fn a_drag_selects_across_runs_and_copies_by_line() {
        let runs = runs();
        let now = Instant::now();
        let mut selection = TextSelection::new();
        let start = runs[0].x_at(0);
        assert!(selection.press(&runs, start + 0.1, 20.0, now));
        assert!(!selection.has_selection(), "a press alone is a caret");
        let end = runs[3].x_at(2);
        assert!(selection.drag(&runs, end, 50.0));
        selection.release();
        assert_eq!(
            selection.selected_text(&runs).as_deref(),
            Some("Where /home/me/Pictures/Trip\nModified 25")
        );
    }

    #[test]
    fn a_double_click_takes_the_word_and_a_triple_the_run() {
        let runs = runs();
        let now = Instant::now();
        let mut selection = TextSelection::new();
        let x = runs[3].x_at(4);
        selection.press(&runs, x, 50.0, now);
        selection.press(&runs, x, 50.0, now + Duration::from_millis(100));
        assert_eq!(selection.selected_text(&runs).as_deref(), Some("Sep"));
        assert!(!selection.is_dragging());
        selection.press(&runs, x, 50.0, now + Duration::from_millis(200));
        assert_eq!(
            selection.selected_text(&runs).as_deref(),
            Some("25 Sep 2026")
        );

        // A shortened value copies whole.
        let (px, py) = centre(&runs[1]);
        let later = now + Duration::from_secs(2);
        for step in 0..3 {
            selection.press(&runs, px, py, later + Duration::from_millis(step * 100));
        }
        assert_eq!(
            selection.selected_text(&runs).as_deref(),
            Some("/home/me/Pictures/Trip")
        );
    }

    #[test]
    fn a_press_off_the_text_clears_and_is_not_taken() {
        let runs = runs();
        let now = Instant::now();
        let mut selection = TextSelection::new();
        let (x, y) = centre(&runs[3]);
        for step in 0..3 {
            selection.press(&runs, x, y, now + Duration::from_millis(step * 50));
        }
        assert!(selection.has_selection());
        assert!(!selection.press(&runs, 5.0, 200.0, now + Duration::from_secs(1)));
        assert!(!selection.has_selection());
        assert!(selection.selected_text(&runs).is_none());
    }

    #[test]
    fn a_selection_outliving_its_text_is_clamped() {
        let mut runs = runs();
        let now = Instant::now();
        let mut selection = TextSelection::new();
        let (x, y) = centre(&runs[1]);
        for step in 0..3 {
            selection.press(&runs, x, y, now + Duration::from_millis(step * 50));
        }
        runs[1].text = "~/P".into();
        assert_eq!(selection.ranges(&runs), vec![(1, 0..3)]);
        runs.truncate(1);
        assert!(selection.ranges(&runs).is_empty());
    }

    #[test]
    fn words_break_at_spaces_and_punctuation() {
        assert_eq!(word_at("25 Sep 2026", 4), (3, 6));
        assert_eq!(word_at("a/b.jpg", 0), (0, 1));
        assert_eq!(word_at("a/b.jpg", 4), (4, 7));
        assert_eq!(word_at("x", 1), (0, 1));
        assert_eq!(word_at("", 0), (0, 0));
    }
}
