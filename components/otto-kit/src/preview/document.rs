//! A formatted document: the shape a Markdown file arrives in, and how it is
//! drawn.
//!
//! The vocabulary is deliberately small — headings, paragraphs, list items,
//! quotes, code and rules, with four inline attributes. It is not an HTML
//! subset and must not grow into one: everything here is a shape the toolkit
//! already knows how to draw with its own typography, which is what keeps a
//! previewed document looking like the rest of Otto rather than like a web
//! page. Anything a faithful renderer would need beyond this (tables that
//! align, images, arbitrary HTML) is either flattened by the decoder or left
//! out; see `specs/quickview.md`.
//!
//! The split matches the rest of [`crate::preview`]: the decoder produces
//! [`Block`]s in a sandboxed worker and never measures anything, and this
//! module — which has the fonts — wraps them into [`Line`]s and paints. Line
//! wrapping is *layout*, so it happens here, once, and drawing and
//! hit-testing read the same answer.

use std::sync::Arc;

use crate::typography::{draw_runs, measure_runs};
use skia_safe::{Canvas, Color, Font, Paint, Rect};

use crate::theme::Theme;
use crate::typography::{styles, TextStyle};

/// Inline attributes on a run of text. A closed set: these are the four a
/// heading, a paragraph or a list item can carry, and they compose.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpanStyle {
    pub bold: bool,
    pub italic: bool,
    /// Inline `code`, drawn monospaced on a tint.
    pub code: bool,
    /// Link text. Where the link *goes* is on the span rather than in here,
    /// because this is the part that survives being packed into a byte — see
    /// [`Span::href`].
    pub link: bool,
}

impl SpanStyle {
    /// The wire form: four flags in one byte.
    pub fn to_bits(self) -> u8 {
        (self.bold as u8)
            | (self.italic as u8) << 1
            | (self.code as u8) << 2
            | (self.link as u8) << 3
    }

    pub fn from_bits(bits: u8) -> Self {
        Self {
            bold: bits & 1 != 0,
            italic: bits & 2 != 0,
            code: bits & 4 != 0,
            link: bits & 8 != 0,
        }
    }
}

/// A run of text sharing one set of inline attributes.
#[derive(Debug, Clone, Default)]
pub struct Span {
    pub text: String,
    pub style: SpanStyle,
    /// Where a link goes, for the hosts that can act on it. `None` on
    /// everything that is not a link, and on a link whose destination the
    /// decoder could not resolve — a reference-style `[label][ref]`, say.
    ///
    /// It is a destination as written, not a promise: nothing here opens it,
    /// checks it, or fetches it, and a host that acts on one is responsible
    /// for deciding which schemes it is willing to hand on. The text came out
    /// of a file the user has not read yet.
    ///
    /// Shared rather than owned per run: wrapping splits a link across as many
    /// runs as it has words, and every one of them points at the same URL.
    pub href: Option<Arc<str>>,
}

impl Span {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: SpanStyle::default(),
            href: None,
        }
    }

    /// Link text with somewhere to go.
    pub fn link(text: impl Into<String>, href: impl Into<Arc<str>>) -> Self {
        Self {
            text: text.into(),
            style: SpanStyle {
                link: true,
                ..SpanStyle::default()
            },
            href: Some(href.into()),
        }
    }
}

/// One block of a document.
#[derive(Debug, Clone)]
pub enum Block {
    /// `level` is 1–6, clamped by the decoder.
    Heading {
        level: u8,
        spans: Vec<Span>,
    },
    Paragraph {
        spans: Vec<Span>,
    },
    /// A list item. `marker` is already resolved by the decoder — a bullet or
    /// a number — because which one it is depends on the source's own
    /// numbering, not on where the item lands on screen.
    Item {
        indent: u8,
        marker: String,
        spans: Vec<Span>,
    },
    Quote {
        spans: Vec<Span>,
    },
    /// A code block, and anything else that has to keep its own columns —
    /// a table gets here too, since drawing one properly means measuring
    /// columns the decoder cannot see.
    Code {
        lines: Vec<String>,
    },
    Rule,
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

/// Leading, as a multiple of the point size. Generous, because a preview is
/// read at a glance and tight lines are what makes a wall of text unreadable.
const LEADING: f32 = 1.5;
/// Space above a block, per block kind. Below is always zero: one rule for
/// the gap between two blocks is easier to keep consistent than two.
const BLOCK_GAP: f32 = 10.0;
const HEADING_GAP: f32 = 18.0;
/// One level of list indent.
const INDENT: f32 = 18.0;
/// The gutter a list marker is drawn in, to the left of its text.
const MARKER: f32 = 18.0;
/// The bar down the side of a quote, and the gap between it and the text.
const QUOTE_BAR: f32 = 3.0;
const QUOTE_INSET: f32 = 14.0;
/// Padding inside a code block's tinted panel.
const CODE_PAD: f32 = 8.0;
const CODE_RADIUS: f32 = 6.0;

/// A run of text, measured and placed on a line.
#[derive(Debug, Clone)]
pub struct Run {
    pub text: String,
    /// Left edge, relative to the content box.
    pub x: f32,
    pub width: f32,
    pub style: SpanStyle,
    /// The destination of the link this run is part of — see [`Span::href`].
    pub href: Option<Arc<str>>,
    base: TextStyle,
    /// A run drawn dimmer than the body: quote text, a list marker.
    muted: bool,
}

/// What is painted behind or around a line, beyond its runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decoration {
    None,
    Rule,
    Quote,
    /// A code line, and whether it opens or closes the panel — the tint is
    /// one rounded rect per line so a long block can be scrolled into
    /// without the panel's corners appearing in the middle of it.
    Code {
        first: bool,
        last: bool,
    },
}

/// One visual line: what wrapping produced, and the unit the host scrolls by.
#[derive(Debug, Clone)]
pub struct Line {
    /// Top edge, relative to the content box, with every preceding line and
    /// block gap accounted for.
    pub top: f32,
    pub height: f32,
    pub runs: Vec<Run>,
    decoration: Decoration,
}

/// Wrap `blocks` to `width`, in the order they will be drawn.
///
/// Every line is laid out, not just the visible ones: the count is what the
/// host scrolls against, and a document is bounded by the decoder long before
/// measuring all of it costs anything.
pub fn wrap(blocks: &[Block], width: f32) -> Vec<Line> {
    let width = width.max(1.0);
    let mut lines: Vec<Line> = Vec::new();
    let mut y = 0.0f32;

    for (index, block) in blocks.iter().enumerate() {
        // No gap above the first block: the padding is already there.
        if index > 0 {
            y += match block {
                Block::Heading { .. } => HEADING_GAP,
                _ => BLOCK_GAP,
            };
        }

        match block {
            Block::Rule => {
                lines.push(Line {
                    top: y,
                    height: BLOCK_GAP,
                    runs: Vec::new(),
                    decoration: Decoration::Rule,
                });
                y += BLOCK_GAP;
            }
            Block::Code { lines: source } => {
                let base = mono();
                let height = base.size * LEADING;
                for (row, text) in source.iter().enumerate() {
                    let font = font_for(base, SpanStyle::default());
                    let width_of = measure_runs(&font, text);
                    lines.push(Line {
                        top: y,
                        height: height
                            + if row == 0 || row + 1 == source.len() {
                                CODE_PAD
                            } else {
                                0.0
                            },
                        runs: vec![Run {
                            text: text.clone(),
                            // Code is not wrapped: a wrapped line of code is
                            // a different line of code. It is clipped, and
                            // the panel is what says there is more.
                            x: CODE_PAD,
                            width: width_of,
                            style: SpanStyle::default(),
                            // A link inside a code block is text about a
                            // link, not a link.
                            href: None,
                            base,
                            muted: false,
                        }],
                        decoration: Decoration::Code {
                            first: row == 0,
                            last: row + 1 == source.len(),
                        },
                    });
                    y += height
                        + if row == 0 || row + 1 == source.len() {
                            CODE_PAD
                        } else {
                            0.0
                        };
                }
            }
            Block::Heading { level, spans } => {
                let base = heading_style(*level);
                wrap_spans(
                    &mut lines,
                    &mut y,
                    spans,
                    base,
                    0.0,
                    0.0,
                    width,
                    Decoration::None,
                    false,
                    None,
                );
            }
            Block::Paragraph { spans } => {
                wrap_spans(
                    &mut lines,
                    &mut y,
                    spans,
                    styles::BODY,
                    0.0,
                    0.0,
                    width,
                    Decoration::None,
                    false,
                    None,
                );
            }
            Block::Quote { spans } => {
                wrap_spans(
                    &mut lines,
                    &mut y,
                    spans,
                    styles::BODY,
                    QUOTE_INSET,
                    0.0,
                    width,
                    Decoration::Quote,
                    true,
                    None,
                );
            }
            Block::Item {
                indent,
                marker,
                spans,
            } => {
                let left = *indent as f32 * INDENT;
                wrap_spans(
                    &mut lines,
                    &mut y,
                    spans,
                    styles::BODY,
                    left + MARKER,
                    left,
                    width,
                    Decoration::None,
                    false,
                    Some(marker.as_str()),
                );
            }
        }
    }
    lines
}

/// Wrap one run of spans into lines starting at `y`.
///
/// `text_left` is where the text starts and where a wrapped continuation
/// lines up — which is to the right of `marker_left`, so the second line of a
/// bullet sits under the first word rather than under the bullet.
#[allow(clippy::too_many_arguments)]
fn wrap_spans(
    lines: &mut Vec<Line>,
    y: &mut f32,
    spans: &[Span],
    base: TextStyle,
    text_left: f32,
    marker_left: f32,
    width: f32,
    decoration: Decoration,
    muted: bool,
    marker: Option<&str>,
) {
    let height = base.size * LEADING;
    let mut current: Vec<Run> = Vec::new();
    let mut x = text_left;
    let mut first_line = true;

    let mut flush = |current: &mut Vec<Run>, y: &mut f32, first_line: &mut bool| {
        if current.is_empty() && !*first_line {
            return;
        }
        let mut runs = std::mem::take(current);
        // The marker belongs to the first line only, and is placed in its own
        // gutter rather than prepended to the text — otherwise a wrapped
        // second line would indent by the bullet's width.
        if *first_line {
            if let Some(marker) = marker {
                let style = base;
                let font = font_for(style, SpanStyle::default());
                runs.insert(
                    0,
                    Run {
                        text: marker.to_string(),
                        x: marker_left,
                        width: measure_runs(&font, marker),
                        style: SpanStyle::default(),
                        href: None,
                        base: style,
                        muted: true,
                    },
                );
            }
        }
        lines.push(Line {
            top: *y,
            height,
            runs,
            decoration,
        });
        *y += height;
        *first_line = false;
    };

    for span in spans {
        let font = font_for(base, span.style);
        for word in words(&span.text) {
            let measured = measure_runs(&font, &word);
            if x > text_left && x + measured > width {
                flush(&mut current, y, &mut first_line);
                x = text_left;
                // A wrapped line does not open with the space that broke it.
                let word = word.trim_start().to_string();
                if word.is_empty() {
                    continue;
                }
                let measured = measure_runs(&font, &word);
                current.push(Run {
                    text: word,
                    x,
                    width: measured,
                    style: span.style,
                    href: span.href.clone(),
                    base,
                    muted,
                });
                x += measured;
                continue;
            }
            current.push(Run {
                text: word,
                x,
                width: measured,
                style: span.style,
                href: span.href.clone(),
                base,
                muted,
            });
            x += measured;
        }
    }
    flush(&mut current, y, &mut first_line);
}

/// Split text into wrappable pieces, each keeping the space that preceded it.
///
/// Keeping the space attached is what lets a break drop it: the piece is
/// trimmed when it starts a line, and the spacing between words is otherwise
/// exactly what the source had.
fn words(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut seen_word = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if seen_word {
                out.push(std::mem::take(&mut current));
                seen_word = false;
            }
            current.push(' ');
        } else {
            current.push(ch);
            seen_word = true;
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// The type scale a heading level maps onto. Six levels collapse into the
/// four the toolkit actually has: past the fourth a heading is a label, and
/// inventing two more sizes for it would only make the scale noisier.
fn heading_style(level: u8) -> TextStyle {
    match level {
        1 => styles::TITLE_2_EMPHASIZED,
        2 => styles::TITLE_3_EMPHASIZED,
        3 => styles::HEADLINE_EMPHASIZED,
        _ => styles::BODY_EMPHASIZED,
    }
}

/// A monospaced style for code, derived from the body style so it tracks the
/// theme's sizing rather than hardcoding a second scale.
fn mono() -> TextStyle {
    let mut style = styles::FOOTNOTE;
    style.family = "monospace";
    style
}

/// The font a run is drawn in: the block's style, with the span's attributes
/// folded in.
///
/// Bold and italic are applied here rather than carried as separate styles
/// because [`TextStyle`] has no slant — the font is built directly so a real
/// italic face is used where one exists, and synthesised nowhere.
fn font_for(base: TextStyle, span: SpanStyle) -> Font {
    use skia_safe::font_style::{Slant, Weight, Width};

    let (family, size) = if span.code {
        (mono().family, base.size * 0.94)
    } else {
        (base.family, base.size)
    };
    // Bold over an already-bold heading is left alone: heavier than the
    // heading's own weight is not a distinction anyone can see.
    let weight = if span.bold {
        base.weight.max(700)
    } else {
        base.weight
    };
    let slant = if span.italic {
        Slant::Italic
    } else {
        Slant::Upright
    };
    let style = skia_safe::FontStyle::new(Weight::from(weight), Width::NORMAL, slant);
    let mut font = crate::typography::get_font_with_fallback(family, style, size);
    font.set_subpixel(true);
    font.set_edging(skia_safe::font::Edging::SubpixelAntiAlias);
    font
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

impl Line {
    /// The link under `x`, in the space [`wrap`] laid this line out in.
    ///
    /// A run without a destination answers `None` even when it is styled as a
    /// link: a reference-style link the decoder could not resolve looks like
    /// one and goes nowhere, and saying so is better than inventing a target.
    pub fn link_at(&self, x: f32) -> Option<&str> {
        self.runs
            .iter()
            .find(|run| x >= run.x && x < run.x + run.width)
            .and_then(|run| run.href.as_deref())
    }
}

/// The link under a point of the content box, for a host scrolling by the
/// line — the hit-test half of [`draw`].
pub fn link_at(content: Rect, lines: &[Line], first: usize, point: (f32, f32)) -> Option<&str> {
    link_at_scrolled(content, lines, top_of(lines, first), point)
}

/// The link under a point of the content box — the hit-test half of
/// [`draw_scrolled`], reading the same geometry it paints.
///
/// `point` is in the same space as `content`, so a host passes the pointer
/// position it was given and nothing else.
pub fn link_at_scrolled(
    content: Rect,
    lines: &[Line],
    offset: f32,
    point: (f32, f32),
) -> Option<&str> {
    let (x, y) = point;
    if x < content.left || x >= content.right || y < content.top || y >= content.bottom {
        return None;
    }
    // Into document space: the inverse of the translation `draw_scrolled`
    // applies, which is why the two cannot disagree about where a word is.
    let x = x - content.left;
    let y = y - content.top + offset;
    lines
        .iter()
        .find(|line| y >= line.top && y < line.top + line.height)
        .and_then(|line| line.link_at(x))
}

/// Paint the wrapped lines into `content`, starting at line `first`.
///
/// Scrolling is a translation of the whole document rather than a per-line
/// offset, so a line half off the top is drawn half, as a scrolled document
/// should be. A host that scrolls by the line — a previewer stepping with the
/// arrow keys — names the line; one that scrolls by the point, as a
/// [`ScrollView`](crate::components::scroll::ScrollView) does, calls
/// [`draw_scrolled`] instead. They are the same function.
pub fn draw(canvas: &Canvas, content: Rect, lines: &[Line], first: usize, theme: &Theme) {
    draw_scrolled(canvas, content, lines, top_of(lines, first), theme);
}

/// The document position drawn at the top of the content box, for a host
/// scrolling by the line.
fn top_of(lines: &[Line], first: usize) -> f32 {
    lines.get(first).map(|line| line.top).unwrap_or(0.0)
}

/// Paint the wrapped lines into `content`, with `offset` points of the
/// document above its top edge.
///
/// `offset` is measured in the space [`wrap`] laid the lines out in, where the
/// document starts at zero — which is exactly the offset a scroll view holds,
/// so a host can hand one straight over.
pub fn draw_scrolled(canvas: &Canvas, content: Rect, lines: &[Line], offset: f32, theme: &Theme) {
    let origin = offset;

    canvas.save();
    canvas.clip_rect(content, None, false);
    canvas.translate((content.left, content.top - origin));

    let mut paint = Paint::default();
    paint.set_anti_alias(true);

    let visible = Rect::from_ltrb(0.0, origin, content.width(), origin + content.height());

    for line in lines {
        if line.top + line.height < visible.top || line.top > visible.bottom {
            continue;
        }
        match line.decoration {
            Decoration::None => {}
            Decoration::Rule => {
                paint.set_color(theme.fill_tertiary);
                canvas.draw_rect(
                    Rect::from_xywh(0.0, line.top + line.height / 2.0, content.width(), 1.0),
                    &paint,
                );
            }
            Decoration::Quote => {
                paint.set_color(theme.fill_secondary);
                canvas.draw_rect(
                    Rect::from_xywh(0.0, line.top, QUOTE_BAR, line.height),
                    &paint,
                );
            }
            Decoration::Code { first, last } => {
                paint.set_color(theme.fill_tertiary);
                let panel = Rect::from_xywh(0.0, line.top, content.width(), line.height);
                if first || last {
                    // Only the ends are rounded; the middle lines are square
                    // so the panel reads as one block.
                    let radius = CODE_RADIUS;
                    let grown = Rect::from_ltrb(
                        panel.left,
                        if first { panel.top } else { panel.top - radius },
                        panel.right,
                        if last {
                            panel.bottom
                        } else {
                            panel.bottom + radius
                        },
                    );
                    canvas.save();
                    canvas.clip_rect(panel, None, false);
                    canvas.draw_round_rect(grown, radius, radius, &paint);
                    canvas.restore();
                } else {
                    canvas.draw_rect(panel, &paint);
                }
            }
        }

        for run in &line.runs {
            let font = font_for(run.base, run.style);
            let (_, metrics) = font.metrics();
            // Centred on the line box from the font's own metrics, so runs of
            // different sizes on one line share a baseline.
            let baseline = line.top + line.height / 2.0 - (metrics.ascent + metrics.descent) / 2.0;

            if run.style.code {
                paint.set_color(tint(theme));
                canvas.draw_round_rect(
                    Rect::from_ltrb(
                        run.x - 3.0,
                        line.top + 2.0,
                        run.x + run.width + 3.0,
                        line.top + line.height - 2.0,
                    ),
                    4.0,
                    4.0,
                    &paint,
                );
            }

            paint.set_color(colour(run, theme));
            draw_runs(canvas, &run.text, (run.x, baseline), &font, &paint);

            if run.style.link {
                // Underlined rather than only coloured: an accent that is
                // close to the text colour in some themes is not a
                // distinction, and this one always is.
                canvas.draw_rect(
                    Rect::from_xywh(run.x, baseline + 2.0, run.width, 1.0),
                    &paint,
                );
            }
        }
    }
    canvas.restore();
}

fn colour(run: &Run, theme: &Theme) -> Color {
    if run.style.link {
        theme.accent
    } else if run.muted {
        theme.text_secondary
    } else {
        theme.text_primary
    }
}

/// The tint behind inline code. `fill_tertiary` is the same wash the code
/// panel uses, so the two read as the same material at two sizes.
fn tint(theme: &Theme) -> Color {
    theme.fill_tertiary
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paragraph(text: &str) -> Block {
        Block::Paragraph {
            spans: vec![Span::plain(text)],
        }
    }

    #[test]
    fn a_long_paragraph_wraps_into_several_lines() {
        let blocks = vec![paragraph(&"word ".repeat(80))];
        let lines = wrap(&blocks, 200.0);
        assert!(lines.len() > 4, "wrapped into {} lines", lines.len());
        // Every line stays inside the column it was wrapped to.
        for line in &lines {
            if let Some(last) = line.runs.last() {
                assert!(last.x + last.width <= 200.0 + 1.0);
            }
        }
    }

    #[test]
    fn lines_are_stacked_in_order_and_never_overlap() {
        let blocks = vec![
            Block::Heading {
                level: 1,
                spans: vec![Span::plain("Title")],
            },
            paragraph("Some prose."),
            Block::Rule,
            Block::Code {
                lines: vec!["fn main() {".into(), "}".into()],
            },
        ];
        let lines = wrap(&blocks, 400.0);
        for pair in lines.windows(2) {
            assert!(
                pair[0].top + pair[0].height <= pair[1].top + 0.01,
                "line at {} overlaps the next at {}",
                pair[0].top,
                pair[1].top
            );
        }
    }

    #[test]
    fn a_link_survives_wrapping_and_can_be_hit() {
        let blocks = vec![Block::Paragraph {
            spans: vec![
                Span::plain("before "),
                Span::link("the whole spec is here", "https://example.com/a"),
                Span::plain(" after"),
            ],
        }];
        let lines = wrap(&blocks, 120.0);
        assert!(
            lines.len() > 1,
            "the link has to wrap for this to mean anything"
        );

        // Every piece of the link, on whichever line it landed, points at the
        // same place; nothing around it points anywhere.
        let linked: Vec<&Run> = lines
            .iter()
            .flat_map(|line| &line.runs)
            .filter(|run| run.style.link)
            .collect();
        assert!(linked.len() > 2);
        assert!(linked
            .iter()
            .all(|run| run.href.as_deref() == Some("https://example.com/a")));
        assert!(lines
            .iter()
            .flat_map(|line| &line.runs)
            .filter(|run| !run.style.link)
            .all(|run| run.href.is_none()));

        // The hit-test reads the geometry the drawing uses: a point inside a
        // linked run answers, and the same point scrolled away does not.
        let content = Rect::from_xywh(10.0, 20.0, 120.0, 400.0);
        let run = lines[0]
            .runs
            .iter()
            .find(|run| run.style.link)
            .expect("a link on the first line");
        let point = (
            content.left + run.x + run.width / 2.0,
            content.top + lines[0].top + lines[0].height / 2.0,
        );
        assert_eq!(
            link_at(content, &lines, 0, point),
            Some("https://example.com/a")
        );
        assert_eq!(
            link_at(content, &lines, 0, (content.left + 1.0, point.1)),
            None
        );
        // Scrolled past, the same point is over something else entirely.
        assert_ne!(
            link_at_scrolled(content, &lines, 10_000.0, point),
            Some("https://example.com/a")
        );
    }

    #[test]
    fn a_link_with_nowhere_to_go_is_not_a_hit() {
        // Styled as a link, no destination: what a reference-style link the
        // decoder could not resolve looks like.
        let blocks = vec![Block::Paragraph {
            spans: vec![Span {
                text: "unresolved".into(),
                style: SpanStyle {
                    link: true,
                    ..SpanStyle::default()
                },
                href: None,
            }],
        }];
        let lines = wrap(&blocks, 400.0);
        let content = Rect::from_xywh(0.0, 0.0, 400.0, 200.0);
        let run = &lines[0].runs[0];
        assert_eq!(
            link_at(content, &lines, 0, (run.x + 1.0, lines[0].top + 1.0)),
            None
        );
    }

    #[test]
    fn a_wrapped_item_lines_up_under_its_own_text() {
        let blocks = vec![Block::Item {
            indent: 0,
            marker: "•".into(),
            spans: vec![Span::plain("word ".repeat(40))],
        }];
        let lines = wrap(&blocks, 160.0);
        assert!(lines.len() > 1);
        // The marker is on the first line, in the gutter; the second line's
        // first run starts where the first line's text did.
        let first_text = lines[0].runs[1].x;
        assert!(lines[0].runs[0].x < first_text);
        assert_eq!(lines[1].runs[0].x, first_text);
    }

    #[test]
    fn code_keeps_its_own_lines() {
        let source: Vec<String> = (0..5)
            .map(|n| format!("line {n} that is quite long indeed"))
            .collect();
        let lines = wrap(&[Block::Code { lines: source }], 40.0);
        // Narrow box, but code is never wrapped: five lines in, five out.
        assert_eq!(lines.len(), 5);
    }

    #[test]
    fn span_flags_survive_the_wire() {
        for bits in 0u8..16 {
            assert_eq!(SpanStyle::from_bits(bits).to_bits(), bits);
        }
    }
}
