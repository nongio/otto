//! Selecting recognised words over a picture.
//!
//! A [`Word`] lives in decoded-pixel coordinates; the panel shows the picture
//! scaled and panned. Everything here converts between the two through the
//! [`PreviewLayout`] the host already has, so a selection made at one zoom is
//! still on the same words at another. The host owns the [`WordSelection`],
//! per the toolkit's draw/hit-test convention; this module answers what is
//! under a point, what a selection says, and where to paint it.

use std::ops::RangeInclusive;

use skia_safe::{Canvas, Color, Paint, RRect, Rect};

use super::{layout, Preview, PreviewLayout, Word, Zoom};
use crate::theme::Theme;

/// A run of words in reading order, from the one pressed to the one the
/// pointer is over. `anchor` and `head` are indices into `Pixels::words` and
/// may be in either order; [`WordSelection::range`] sorts them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WordSelection {
    pub anchor: usize,
    pub head: usize,
}

impl WordSelection {
    pub fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head }
    }

    /// A single word.
    pub fn word(index: usize) -> Self {
        Self {
            anchor: index,
            head: index,
        }
    }

    /// The selected indices, lowest first.
    pub fn range(&self) -> RangeInclusive<usize> {
        self.anchor.min(self.head)..=self.anchor.max(self.head)
    }
}

/// How far outside a word's box a press still counts as on it, in decoded
/// pixels. Recognisers crop boxes tight to the ink, and a pointer at the
/// edge of a glyph should not fall through.
const HIT_SLOP: f32 = 2.0;

/// Clear space around a highlighted word, in decoded pixels, so the tint does
/// not hug the ink.
const HIGHLIGHT_PAD: f32 = 1.0;

/// Corner radius of a highlight, in panel pixels.
const HIGHLIGHT_RADIUS: f32 = 2.0;

/// Alpha of the highlight over the theme's accent: the tint the toolkit's
/// text fields use for their own selection.
const HIGHLIGHT_ALPHA: u8 = 90;

/// What a selection is made on: the words, the width of the space they are
/// boxed in, and where that space is drawn.
///
/// Two kinds of preview have words, and they measure them in different
/// units — a picture's in its decoded pixels, a document's in its strip
/// coordinates — but the conversion is the same ratio either way, so
/// everything below works in "content units" and never asks which.
struct Words<'a> {
    words: &'a [Word],
    /// The content's own width in those units.
    width: f32,
    /// Where the content is drawn, in panel coordinates.
    content: Rect,
}

fn picture<'a>(layout: &PreviewLayout, preview: &'a Preview) -> Option<Words<'a>> {
    match preview {
        Preview::Pixels { pixels, .. } if pixels.width > 0 && pixels.height > 0 => Some(Words {
            words: &pixels.words,
            width: pixels.width as f32,
            content: layout.content,
        }),
        Preview::Pages { pages, words } if !pages.is_empty() => Some(Words {
            words,
            width: super::strip_size(pages).0,
            content: layout.content,
        }),
        _ => None,
    }
}

/// Panel pixels per content unit for the content drawn in `content`.
fn scale_of(width: f32, content: Rect) -> f32 {
    content.width() / width
}

/// A panel point in content coordinates.
fn to_picture(width: f32, content: Rect, x: f32, y: f32) -> (f32, f32) {
    let scale = scale_of(width, content);
    ((x - content.left) / scale, (y - content.top) / scale)
}

/// A content rect in panel coordinates.
fn to_panel(width: f32, content: Rect, rect: Rect) -> Rect {
    let scale = scale_of(width, content);
    Rect::from_ltrb(
        content.left + rect.left * scale,
        content.top + rect.top * scale,
        content.left + rect.right * scale,
        content.top + rect.bottom * scale,
    )
}

fn same_line(a: &Word, b: &Word) -> bool {
    a.block == b.block && a.paragraph == b.paragraph && a.line == b.line
}

fn same_paragraph(a: &Word, b: &Word) -> bool {
    a.block == b.block && a.paragraph == b.paragraph
}

/// The word under a panel point, if the point is on one.
pub fn word_at(layout: &PreviewLayout, preview: &Preview, x: f32, y: f32) -> Option<usize> {
    let found = picture(layout, preview)?;
    let (px, py) = to_picture(found.width, found.content, x, y);
    found.words.iter().position(|word| {
        let rect = word.rect().with_outset((HIT_SLOP, HIT_SLOP));
        px >= rect.left && px <= rect.right && py >= rect.top && py <= rect.bottom
    })
}

/// The word a drag has reached: the one under the point, or failing that
/// the nearest in reading order — the word on the closest line, then the
/// closest along it. A pointer past the last word answers the last word, so
/// dragging off the end selects to the end.
pub fn word_near(layout: &PreviewLayout, preview: &Preview, x: f32, y: f32) -> Option<usize> {
    if let Some(index) = word_at(layout, preview, x, y) {
        return Some(index);
    }
    let found = picture(layout, preview)?;
    let words = found.words;
    if words.is_empty() {
        return None;
    }
    let (px, py) = to_picture(found.width, found.content, x, y);

    // The closest line by vertical distance to its box; ties go to the
    // earlier line, which is where a drag straight up from a gap lands.
    let mut best_line: Option<(RangeInclusive<usize>, f32)> = None;
    let mut start = 0;
    while start < words.len() {
        let line = line_of(words, start);
        let end = *line.end();
        let (top, bottom) = words[line.clone()].iter().fold(
            (f32::INFINITY, f32::NEG_INFINITY),
            |(top, bottom), word| {
                let rect = word.rect();
                (top.min(rect.top), bottom.max(rect.bottom))
            },
        );
        let distance = if py < top {
            top - py
        } else if py > bottom {
            py - bottom
        } else {
            0.0
        };
        if best_line.as_ref().is_none_or(|(_, best)| distance < *best) {
            best_line = Some((line, distance));
        }
        start = end + 1;
    }
    let (line, _) = best_line?;

    // Along the line, the closest word by horizontal distance to its box;
    // a point before every word on it is the first, past every word the
    // last.
    let mut best_word: Option<(usize, f32)> = None;
    for index in line {
        let rect = words[index].rect();
        let distance = if px < rect.left {
            rect.left - px
        } else if px > rect.right {
            px - rect.right
        } else {
            0.0
        };
        if best_word.is_none_or(|(_, best)| distance < best) {
            best_word = Some((index, distance));
        }
    }
    best_word.map(|(index, _)| index)
}

/// The indices of every word on the same line as `index`. Words arrive in
/// reading order, so a line is a contiguous run.
pub(crate) fn line_of(words: &[Word], index: usize) -> RangeInclusive<usize> {
    let Some(word) = words.get(index) else {
        return index..=index;
    };
    let mut first = index;
    while first > 0 && same_line(&words[first - 1], word) {
        first -= 1;
    }
    let mut last = index;
    while last + 1 < words.len() && same_line(&words[last + 1], word) {
        last += 1;
    }
    first..=last
}

/// The selected words as text: a space between words, a line break between
/// lines, a blank line between paragraphs or blocks.
pub fn selection_text(words: &[Word], selection: WordSelection) -> String {
    let mut text = String::new();
    let mut previous: Option<&Word> = None;
    for index in selection.range() {
        let Some(word) = words.get(index) else {
            break;
        };
        if let Some(prev) = previous {
            if !same_paragraph(prev, word) {
                text.push_str("\n\n");
            } else if !same_line(prev, word) {
                text.push('\n');
            } else {
                text.push(' ');
            }
        }
        text.push_str(&word.text);
        previous = Some(word);
    }
    text
}

/// Where to paint the selection: one panel rect per line it touches, the
/// union of that line's selected words, clipped to the panel's inner box.
/// Lines scrolled or panned entirely out of the box are left out.
pub(crate) fn selection_rects(
    layout: &PreviewLayout,
    preview: &Preview,
    selection: WordSelection,
) -> Vec<Rect> {
    let Some(found) = picture(layout, preview) else {
        return Vec::new();
    };
    let words = found.words;
    let mut rects: Vec<Rect> = Vec::new();
    let mut current: Option<(&Word, Rect)> = None;
    for index in selection.range() {
        let Some(word) = words.get(index) else {
            break;
        };
        let rect = word.rect();
        match current.as_mut() {
            Some((first, union)) if same_line(first, word) => union.join(rect),
            _ => {
                if let Some((_, union)) = current.take() {
                    rects.push(union);
                }
                current = Some((word, rect));
            }
        }
    }
    if let Some((_, union)) = current {
        rects.push(union);
    }
    rects
        .into_iter()
        .map(|rect| rect.with_outset((HIGHLIGHT_PAD, HIGHLIGHT_PAD)))
        .map(|rect| to_panel(found.width, found.content, rect))
        .filter_map(|rect| {
            let mut clipped = rect;
            clipped.intersect(layout.inner).then_some(clipped)
        })
        .collect()
}

/// Paint a selection over a preview already drawn with [`super::draw`] into
/// the same `bounds` at the same `zoom`. Draws nothing for a preview without
/// words, so a host can call it unconditionally.
pub fn draw_selection(
    canvas: &Canvas,
    bounds: Rect,
    preview: &Preview,
    theme: &Theme,
    zoom: Zoom,
    selection: WordSelection,
) {
    let geometry = layout(bounds, preview, 0, zoom);
    let rects = selection_rects(&geometry, preview, selection);
    if rects.is_empty() {
        return;
    }
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(Color::from_argb(
        HIGHLIGHT_ALPHA,
        theme.accent.r(),
        theme.accent.g(),
        theme.accent.b(),
    ));
    canvas.save();
    canvas.clip_rect(geometry.inner, None, true);
    for rect in rects {
        canvas.draw_rrect(
            RRect::new_rect_xy(rect, HIGHLIGHT_RADIUS, HIGHLIGHT_RADIUS),
            &paint,
        );
    }
    canvas.restore();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::{Page, Pixels};

    fn word(text: &str, left: u32, top: u32, width: u32, height: u32, line: u32) -> Word {
        Word {
            text: text.to_string(),
            left,
            top,
            width,
            height,
            confidence: 90,
            block: 1,
            paragraph: 1,
            line,
        }
    }

    fn words() -> Vec<Word> {
        vec![
            word("Hello", 21, 41, 54, 19, 1),
            word("Otto", 82, 44, 45, 16, 1),
            word("42", 134, 44, 23, 17, 1),
            word("Next", 21, 80, 60, 18, 2),
        ]
    }

    fn preview() -> Preview {
        Preview::Pixels {
            pixels: Pixels {
                width: 400,
                height: 120,
                intrinsic_width: 400,
                intrinsic_height: 120,
                data: vec![0; 400 * 120 * 4],
                frame_delays: Vec::new(),
                words: words(),
            },
            pages: 1,
            page: 1,
        }
    }

    fn bounds() -> Rect {
        Rect::from_xywh(0.0, 0.0, 800.0, 240.0)
    }

    fn fitted() -> (Preview, PreviewLayout) {
        let preview = preview();
        let layout = layout(bounds(), &preview, 0, Zoom::FIT);
        (preview, layout)
    }

    /// A decoded-pixel point in panel coordinates, for the fitted layout.
    fn panel_point(layout: &PreviewLayout, x: f32, y: f32) -> (f32, f32) {
        let scale = layout.content.width() / 400.0;
        (
            layout.content.left + x * scale,
            layout.content.top + y * scale,
        )
    }

    #[test]
    fn word_at_hits_a_word_and_misses_a_gap() {
        let (preview, layout) = fitted();
        let (x, y) = panel_point(&layout, 100.0, 50.0);
        assert_eq!(word_at(&layout, &preview, x, y), Some(1));
        // Between "Hello" (ends 75) and "Otto" (starts 82), past the slop.
        let (x, y) = panel_point(&layout, 78.5, 50.0);
        assert_eq!(word_at(&layout, &preview, x, y), None);
        // Nowhere near a line.
        let (x, y) = panel_point(&layout, 100.0, 5.0);
        assert_eq!(word_at(&layout, &preview, x, y), None);
    }

    #[test]
    fn word_near_past_the_right_edge_is_the_last_word_on_that_line() {
        let (preview, layout) = fitted();
        let (x, y) = panel_point(&layout, 390.0, 50.0);
        assert_eq!(word_near(&layout, &preview, x, y), Some(2));
        // Past the last line altogether: the last word of the picture.
        let (x, y) = panel_point(&layout, 390.0, 115.0);
        assert_eq!(word_near(&layout, &preview, x, y), Some(3));
        // Before everything: the first word.
        let (x, y) = panel_point(&layout, 2.0, 2.0);
        assert_eq!(word_near(&layout, &preview, x, y), Some(0));
    }

    #[test]
    fn line_of_spans_the_contiguous_run() {
        let words = words();
        assert_eq!(line_of(&words, 1), 0..=2);
        assert_eq!(line_of(&words, 3), 3..=3);
    }

    #[test]
    fn selection_text_breaks_between_lines() {
        let words = words();
        assert_eq!(
            selection_text(&words, WordSelection::new(3, 1)),
            "Otto 42\nNext"
        );
        assert_eq!(selection_text(&words, WordSelection::word(0)), "Hello");
    }

    #[test]
    fn selection_text_puts_a_blank_line_between_paragraphs() {
        let mut words = words();
        words[3].paragraph = 2;
        assert_eq!(
            selection_text(&words, WordSelection::new(2, 3)),
            "42\n\nNext"
        );
    }

    #[test]
    fn selection_rects_are_one_per_line() {
        let (preview, layout) = fitted();
        let rects = selection_rects(&layout, &preview, WordSelection::new(0, 3));
        assert_eq!(rects.len(), 2);
        // The first line's rect covers "Hello" through "42", padded by one
        // decoded pixel, at the panel's scale.
        let (left, top) = panel_point(&layout, 20.0, 40.0);
        let (right, bottom) = panel_point(&layout, 158.0, 62.0);
        assert!((rects[0].left - left).abs() < 0.01);
        assert!((rects[0].top - top).abs() < 0.01);
        assert!((rects[0].right - right).abs() < 0.01);
        assert!((rects[0].bottom - bottom).abs() < 0.01);
        assert!(rects[1].top > rects[0].bottom);
    }

    #[test]
    fn selection_rects_are_clipped_to_the_inner_box() {
        let preview = preview();
        let zoom = Zoom {
            scale: 4.0,
            offset: (-600.0, 0.0),
            band: (0.0, 0.0),
        };
        let layout = layout(bounds(), &preview, 0, zoom);
        for rect in selection_rects(&layout, &preview, WordSelection::new(0, 3)) {
            assert!(rect.left >= layout.inner.left && rect.right <= layout.inner.right);
        }
    }

    #[test]
    fn draw_selection_paints_nothing_without_words() {
        let mut surface = skia_safe::surfaces::raster_n32_premul((800, 240)).expect("surface");
        let preview = Preview::Pixels {
            pixels: Pixels {
                width: 400,
                height: 120,
                intrinsic_width: 400,
                intrinsic_height: 120,
                data: vec![0; 400 * 120 * 4],
                frame_delays: Vec::new(),
                words: Vec::new(),
            },
            pages: 1,
            page: 1,
        };
        draw_selection(
            surface.canvas(),
            bounds(),
            &preview,
            &Theme::light_palette(),
            Zoom::FIT,
            WordSelection::word(0),
        );
    }
}
