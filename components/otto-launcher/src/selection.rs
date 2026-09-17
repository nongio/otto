//! Selecting text in the ask log.
//!
//! The log is painted, not laid out as widgets, so there is nothing in it to
//! select by itself. This module gives the painted lines a second, flatter
//! shape — a [`Span`] per piece of text, in reading order, with the rectangle
//! it was drawn in — and everything selection needs is expressed against that:
//! where a press lands, what a drag covers, which rectangles to highlight, and
//! what ends up on the clipboard.
//!
//! Spans are rebuilt whenever the log is laid out again, which while an answer
//! is arriving is once a pass. Earlier spans keep their index — an answer
//! grows at the end — so a selection made halfway through a reply survives the
//! rest of it.
//!
//! Measuring inside a span is done on demand rather than cached: only the span
//! under the pointer and the two ends of the selection are ever measured, so
//! laying the log out again stays as cheap as it was before any of this.

use otto_kit::typography::{measure_runs, offset_at};
use skia_safe::{Font, Rect};

/// One run of text in the log, as it was painted.
#[derive(Clone, Debug)]
pub struct Span {
    /// Where the text sits, in the log pane's content coordinates.
    pub rect: Rect,
    pub text: String,
    /// The face it was drawn in, so a caret can be placed inside it.
    pub font: Font,
    /// Which visual line it belongs to. Copied text breaks where this
    /// changes, so runs sharing a line of an answer come out as one line.
    pub line: usize,
}

impl Span {
    /// How far into the text `byte` sits, from the span's left edge.
    fn advance(&self, byte: usize) -> f32 {
        let byte = byte.min(self.text.len());
        if byte == 0 {
            0.0
        } else if byte == self.text.len() {
            self.rect.width()
        } else {
            measure_runs(&self.font, &self.text[..byte])
        }
    }

    /// The character boundary nearest `x`, in content coordinates.
    fn offset_at(&self, x: f32) -> usize {
        offset_at(&self.font, &self.text, x - self.rect.left)
    }
}

/// A place in the log's text: a span, and how far into it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Caret {
    pub span: usize,
    pub byte: usize,
}

/// What is selected: where the drag started, and where it is now. The focus
/// may come before the anchor — selections are made in both directions.
#[derive(Clone, Copy, Debug)]
pub struct Selection {
    pub anchor: Caret,
    pub focus: Caret,
}

impl Selection {
    /// A selection of nothing, at `caret`: what a press makes before the
    /// pointer has moved anywhere.
    pub fn at(caret: Caret) -> Self {
        Self {
            anchor: caret,
            focus: caret,
        }
    }

    /// The two ends in reading order.
    pub fn range(&self) -> (Caret, Caret) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.focus
    }

    /// The same selection extended to `focus`, keeping its anchor.
    pub fn extended_to(&self, focus: Caret) -> Self {
        Self {
            anchor: self.anchor,
            focus,
        }
    }

    /// Whether both ends still name a span that exists, after the log has
    /// been laid out again.
    pub fn fits(&self, spans: &[Span]) -> bool {
        [self.anchor, self.focus].iter().all(|caret| {
            spans
                .get(caret.span)
                .is_some_and(|span| caret.byte <= span.text.len())
        })
    }
}

/// Where in the text `point` is, when it is on some text.
///
/// Exact on purpose: this is what a press asks, and a press on the log's
/// background is not a selection — it takes hold of the card instead.
pub fn caret_at(spans: &[Span], point: (f32, f32)) -> Option<Caret> {
    let (x, y) = point;
    let span = spans.iter().position(|span| {
        (span.rect.top..span.rect.bottom).contains(&y)
            && (span.rect.left..span.rect.right).contains(&x)
    })?;
    Some(Caret {
        span,
        byte: spans[span].offset_at(x),
    })
}

/// Where in the text `point` is, or the nearest place to it: what a drag
/// asks, because the pointer leaves the text it started in and the selection
/// still has to follow.
pub fn nearest_caret(spans: &[Span], point: (f32, f32)) -> Option<Caret> {
    let (x, y) = point;
    let distance = |span: &Span| {
        let dy = (span.rect.top - y).max(y - span.rect.bottom).max(0.0);
        let dx = (span.rect.left - x).max(x - span.rect.right).max(0.0);
        // Lines count for more than columns: the nearest text to a pointer
        // out in the margin is the line beside it, not a line above with
        // something drawn further across.
        dy * 1000.0 + dx
    };
    let span = (0..spans.len()).min_by(|&a, &b| {
        distance(&spans[a])
            .partial_cmp(&distance(&spans[b]))
            .unwrap_or(std::cmp::Ordering::Equal)
    })?;
    Some(Caret {
        span,
        byte: spans[span].offset_at(x),
    })
}

/// The word around `caret`, for a double press. A caret in a run of spaces
/// takes the spaces, so a double press never selects nothing.
pub fn word_at(spans: &[Span], caret: Caret) -> Selection {
    let Some(span) = spans.get(caret.span) else {
        return Selection::at(caret);
    };
    let text = span.text.as_str();
    if text.is_empty() {
        return Selection::at(caret);
    }
    let word = |c: char| c.is_alphanumeric() || c == '_';
    // Which character the caret means. On the boundary between a word and
    // what is not one, the word wins: a press just after the last letter of
    // "cargo" means "cargo", not the space behind it.
    let byte = caret.byte.min(text.len());
    let before = text[..byte].chars().next_back();
    let after = text[byte..].chars().next();
    let at = match (before, after) {
        (Some(before), _) if word(before) => before,
        (_, Some(after)) => after,
        (Some(before), None) => before,
        (None, None) => return Selection::at(caret),
    };
    let same = |c: char| word(c) == word(at) && c.is_whitespace() == at.is_whitespace();
    let start = text[..byte]
        .char_indices()
        .rev()
        .take_while(|(_, c)| same(*c))
        .map(|(index, _)| index)
        .last()
        .unwrap_or(byte);
    let end = text[byte..]
        .char_indices()
        .take_while(|(_, c)| same(*c))
        .map(|(index, c)| byte + index + c.len_utf8())
        .last()
        .unwrap_or(byte);
    Selection {
        anchor: Caret {
            span: caret.span,
            byte: start,
        },
        focus: Caret {
            span: caret.span,
            byte: end,
        },
    }
}

/// The whole visual line `caret` is on, for a third press: every span sharing
/// its line, end to end.
pub fn line_at(spans: &[Span], caret: Caret) -> Selection {
    let Some(line) = spans.get(caret.span).map(|span| span.line) else {
        return Selection::at(caret);
    };
    let on_line = |index: &usize| spans[*index].line == line;
    let first = (0..spans.len()).filter(on_line).min().unwrap_or(caret.span);
    let last = (0..spans.len()).filter(on_line).max().unwrap_or(caret.span);
    Selection {
        anchor: Caret {
            span: first,
            byte: 0,
        },
        focus: Caret {
            span: last,
            byte: spans[last].text.len(),
        },
    }
}

/// Everything in the log, for select-all.
pub fn everything(spans: &[Span]) -> Option<Selection> {
    let last = spans.len().checked_sub(1)?;
    Some(Selection {
        anchor: Caret { span: 0, byte: 0 },
        focus: Caret {
            span: last,
            byte: spans[last].text.len(),
        },
    })
}

/// What `selection` covers, as it should reach the clipboard: the text of
/// each span it touches, broken where the log breaks lines.
pub fn text(spans: &[Span], selection: Selection) -> String {
    let (start, end) = selection.range();
    let mut out = String::new();
    let mut line: Option<usize> = None;
    let mut right = 0.0_f32;
    for (index, span) in spans.iter().enumerate() {
        if index < start.span || index > end.span {
            continue;
        }
        let from = if index == start.span { start.byte } else { 0 };
        let to = if index == end.span {
            end.byte
        } else {
            span.text.len()
        };
        let (from, to) = (from.min(span.text.len()), to.min(span.text.len()));
        if to < from {
            continue;
        }
        if line.is_some_and(|line| line != span.line) {
            out.push('\n');
        } else if line.is_some() && span.rect.left - right > 2.0 {
            // A gap the layout left inside a line — after a list's bullet,
            // or an indent — is a space in the text, not nothing.
            out.push(' ');
        }
        line = Some(span.line);
        right = span.rect.right;
        out.push_str(&span.text[from..to]);
    }
    out
}

/// The rectangles to paint behind `selection`, in content coordinates.
pub fn rects(spans: &[Span], selection: Selection) -> Vec<Rect> {
    let (start, end) = selection.range();
    let mut rects = Vec::new();
    for (index, span) in spans.iter().enumerate() {
        if index < start.span || index > end.span || span.text.is_empty() {
            continue;
        }
        let left = if index == start.span {
            span.rect.left + span.advance(start.byte)
        } else {
            span.rect.left
        };
        let right = if index == end.span {
            span.rect.left + span.advance(end.byte)
        } else {
            span.rect.right
        };
        if right > left {
            rects.push(Rect::from_ltrb(
                left,
                span.rect.top,
                right,
                span.rect.bottom,
            ));
        }
    }
    rects
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_kit::typography::{get_font_with_fallback, styles};
    use skia_safe::FontStyle;

    /// Spans one line apart, each the width its text measures, so the
    /// geometry here is the geometry the log paints.
    fn spans(lines: &[&[&str]]) -> Vec<Span> {
        let font = get_font_with_fallback(styles::BODY.family, FontStyle::normal(), 14.0);
        let mut spans = Vec::new();
        for (line, runs) in lines.iter().enumerate() {
            let mut x = 20.0;
            for text in runs.iter() {
                let width = measure_runs(&font, text);
                spans.push(Span {
                    rect: Rect::from_xywh(x, line as f32 * 21.0, width, 21.0),
                    text: text.to_string(),
                    font: font.clone(),
                    line,
                });
                x += width;
            }
        }
        spans
    }

    fn caret(span: usize, byte: usize) -> Caret {
        Caret { span, byte }
    }

    #[test]
    fn a_press_on_text_lands_in_it_and_one_beside_it_lands_nowhere() {
        let spans = spans(&[&["hello"]]);
        let inside = caret_at(&spans, (spans[0].rect.left + 0.5, 10.0)).expect("on the text");
        assert_eq!(inside, caret(0, 0));
        assert_eq!(caret_at(&spans, (spans[0].rect.right + 40.0, 10.0)), None);
        assert_eq!(caret_at(&spans, (spans[0].rect.left + 1.0, 60.0)), None);
    }

    #[test]
    fn a_drag_past_the_end_of_a_line_takes_all_of_it() {
        let spans = spans(&[&["hello"], &["there"]]);
        let end = nearest_caret(&spans, (spans[0].rect.right + 200.0, 10.0)).expect("some text");
        assert_eq!(end, caret(0, "hello".len()));
        // Out in the margin below, the nearest text is the line beside it.
        let below = nearest_caret(&spans, (0.0, 30.0)).expect("some text");
        assert_eq!(below, caret(1, 0));
    }

    #[test]
    fn selected_text_breaks_where_the_log_breaks_lines() {
        let spans = spans(&[&["hello "], &["there", " you"]]);
        let all = everything(&spans).expect("something to select");
        assert_eq!(text(&spans, all), "hello \nthere you");
        // Runs sharing a line come out as one line, whichever way the
        // selection was made.
        let part = Selection {
            anchor: caret(2, 1),
            focus: caret(0, 2),
        };
        assert_eq!(text(&spans, part), "llo \nthere ");
    }

    #[test]
    fn a_selection_of_nothing_copies_nothing_and_paints_nothing() {
        let spans = spans(&[&["hello"]]);
        let empty = Selection::at(caret(0, 3));
        assert!(empty.is_empty());
        assert_eq!(text(&spans, empty), "");
        assert!(rects(&spans, empty).is_empty());
    }

    #[test]
    fn the_highlight_covers_the_selected_run_of_each_line() {
        let spans = spans(&[&["hello"], &["there"]]);
        let selection = Selection {
            anchor: caret(0, 2),
            focus: caret(1, 3),
        };
        let rects = rects(&spans, selection);
        assert_eq!(rects.len(), 2, "one band per line");
        assert!(rects[0].left > spans[0].rect.left, "starts inside the word");
        assert_eq!(rects[0].right, spans[0].rect.right, "and runs to its end");
        assert_eq!(rects[1].left, spans[1].rect.left);
        assert!(rects[1].right < spans[1].rect.right, "stops inside it");
    }

    #[test]
    fn a_double_press_takes_the_word_under_it() {
        let sentence = spans(&[&["cargo build now"]]);
        let word = word_at(&sentence, caret(0, 8));
        assert_eq!(text(&sentence, word), "build");
        // At the end of a word, that word — not the space after it.
        assert_eq!(text(&sentence, word_at(&sentence, caret(0, 5))), "cargo");
        // And a third press takes the line, however many runs it is.
        let runs = spans(&[&["cargo ", "build"]]);
        assert_eq!(text(&runs, line_at(&runs, caret(1, 2))), "cargo build");
    }

    #[test]
    fn a_selection_outliving_its_log_is_seen_to_be_stale() {
        let spans = spans(&[&["hello"]]);
        assert!(Selection::at(caret(0, 5)).fits(&spans));
        assert!(!Selection::at(caret(1, 0)).fits(&spans));
        assert!(!Selection::at(caret(0, 99)).fits(&spans));
    }
}
