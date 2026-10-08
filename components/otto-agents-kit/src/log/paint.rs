//! The log, painted: its fonts, the pictures in it and its attachments,
//! painted a band at a time at whatever width the host lays it out at.
//!
//! The log is laid out by [`crate::log`] as lines; [`LogPainter`] measures
//! their text for that layout, paints them into a scroll pane's band, and says
//! what is under the pointer. It knows nothing of the card around it: the
//! host gives the width, and the log's content is that wide with [`INSET`]
//! either side.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use otto_kit::components::attachments::{AttachmentList, Options as AttachmentOptions, HOVER_PAD};
use otto_kit::preview::document;
use otto_kit::theme::Theme;
use otto_kit::typography::{draw_runs, get_font_with_fallback, measure_runs, styles};
use skia_safe::font_style::{Slant, Weight, Width};
use skia_safe::{Canvas, Color, Color4f, Font, FontStyle, Image, Paint, Rect};

use crate::chat::transcript::Attachment;
use crate::log::selection::Span;
use crate::log::{
    Kind, Line, Style, BUBBLE_GAP, BUBBLE_PAD_X, BUBBLE_PAD_Y, FOOTER_H, IMAGE_PAD, LINE_H, TEXT,
};
use crate::rows::{row_subtitle_color, row_title_color};

/// Space either side of the log's text, inside the pane it is painted in.
pub const INSET: f32 = 20.0;

/// Size of a note in the log — a tool call, a status line. Smaller than the
/// conversation, so what the agent said outranks what it is doing.
const NOTE_TEXT: f32 = 11.5;

/// Corner radius of a request's bubble; a one-line request is a pill.
const BUBBLE_RADIUS: f32 = 16.0;

/// Room either side of the mode's name inside its pill.
const PILL_PAD_X: f32 = 7.0;
/// How tall the mode's pill is.
const PILL_H: f32 = 16.0;
/// Space between the footer's pieces: the agent, its mode, the hint.
const FOOTER_GAP: f32 = 7.0;
/// Corner radius of the fill under a group of tool calls the pointer is on.
const STEPS_RADIUS: f32 = 6.0;
/// Room around that fill, so the words are not against its edge.
const STEPS_PAD: f32 = 4.0;

/// How rounded a picture in the log is: the corner every other surface
/// around it wears.
const IMAGE_RADIUS: f32 = 8.0;

/// A pending attachment under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttachmentHit {
    /// The log line the attachments are on.
    pub line: usize,
    /// Which attachment, by its index among them.
    pub item: usize,
    /// Whether the pointer is on its remove button.
    pub remove: bool,
    /// Whether it goes with the next request, rather than went with one.
    pub pending: bool,
}

/// Measures, paints and hit-tests the log.
pub struct LogPainter {
    /// Pictures the agent sent, decoded once. The log is laid out again on
    /// every chunk of an answer, and each pass asks every picture how large it
    /// is. Behind a cell because painting a band only borrows the painter.
    pictures: RefCell<HashMap<PathBuf, Option<Image>>>,
    /// Attachments, as otto-stash's card shows them, with what was read
    /// about each file kept for the next layout.
    attachments: RefCell<AttachmentList>,
    dark: bool,
}

impl LogPainter {
    pub fn new(dark: bool) -> Self {
        Self {
            pictures: RefCell::new(HashMap::new()),
            attachments: RefCell::new(AttachmentList::default()),
            dark,
        }
    }

    /// Switch the colour scheme. The host repaints the log after.
    pub fn set_dark(&mut self, dark: bool) {
        self.dark = dark;
    }

    /// How large the picture at `path` is, in its own pixels, for the log to
    /// scale into its width. `None` when there is no reading it — the file is
    /// gone, or it is not a picture — and the log says its name instead.
    pub fn picture_size(&self, path: &Path) -> Option<(f32, f32)> {
        let image = self.picture(path)?;
        Some((image.width() as f32, image.height() as f32))
    }

    /// The picture at `path`, decoded once and kept.
    ///
    /// Decoded eagerly into raster pixels: `Image::from_encoded` is lazy, and a
    /// picture left lazy is decoded again on every band the log paints.
    fn picture(&self, path: &Path) -> Option<Image> {
        if let Some(cached) = self.pictures.borrow().get(path) {
            return cached.clone();
        }
        let decoded = std::fs::read(path)
            .ok()
            .and_then(|bytes| Image::from_encoded(skia_safe::Data::new_copy(&bytes)))
            .and_then(|image| image.make_raster_image(None, None));
        self.pictures
            .borrow_mut()
            .insert(path.to_path_buf(), decoded.clone());
        decoded
    }

    fn theme(&self) -> Theme {
        if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        }
    }

    /// Lay out attachments as the log shows them: pending ones as a pile to
    /// strike out or take off, the newest in full; sent ones as a record.
    fn lay_out_attachments(
        &self,
        items: &[(Attachment, bool)],
        pending: bool,
        width: f32,
    ) -> otto_kit::components::attachments::Layout {
        let listed: Vec<_> = items.iter().map(|(item, struck)| (item, *struck)).collect();
        let options = AttachmentOptions {
            width,
            newest_first: pending,
            removable: pending,
        };
        self.attachments
            .borrow_mut()
            .layout(&listed, options, &self.theme())
    }

    /// The files in the attachments last laid out whose thumbnails are still
    /// to be made; see [`AttachmentList::thumbnails_wanted`].
    pub fn thumbnails_wanted(&self) -> Vec<std::path::PathBuf> {
        self.attachments.borrow_mut().thumbnails_wanted()
    }

    /// Hand in the thumbnail of `path`, or that it has none.
    pub fn set_thumbnail(&self, path: &std::path::Path, image: Option<skia_safe::Image>) {
        self.attachments.borrow_mut().set_thumbnail(path, image);
    }

    /// How tall a log line of attachments is, laid out `width` wide,
    /// highlight room included.
    pub fn attachments_height(
        &self,
        items: &[(Attachment, bool)],
        pending: bool,
        width: f32,
    ) -> f32 {
        self.lay_out_attachments(items, pending, width).height + 2.0 * HOVER_PAD
    }

    /// The pending attachment at `point`, in the log's content coordinates,
    /// with the log laid out `width` wide.
    pub fn attachment_at(
        &self,
        lines: &[Line],
        point: (f32, f32),
        width: f32,
    ) -> Option<AttachmentHit> {
        let (x, y) = point;
        let (index, line) = lines
            .iter()
            .enumerate()
            .find(|(_, line)| (line.top..line.top + line.height).contains(&y))?;
        let Kind::Attachments { items, pending } = &line.kind else {
            return None;
        };
        let layout = self.lay_out_attachments(items, *pending, width);
        let (x, y) = (x - INSET, y - line.top - HOVER_PAD);
        let item = layout.item_at(x, y)?;
        Some(AttachmentHit {
            line: index,
            item,
            remove: layout.remove_at(x, y) == Some(item),
            pending: *pending,
        })
    }

    /// How wide `text` is in the log, drawn in `style`.
    pub fn measure(&self, text: &str, style: Style) -> f32 {
        measure_runs(&self.font(style), text)
    }

    fn font(&self, style: Style) -> Font {
        let weight = match style {
            Style::Prompt => Weight::SEMI_BOLD,
            Style::Request | Style::Answer | Style::Note => Weight::NORMAL,
        };
        let size = match style {
            Style::Note => NOTE_TEXT,
            Style::Request | Style::Prompt | Style::Answer => TEXT,
        };
        get_font_with_fallback(
            styles::BODY.family,
            FontStyle::new(weight, Width::NORMAL, Slant::Upright),
            size,
        )
    }

    /// Every piece of text in the log, laid out `width` wide, in reading
    /// order, with the box it is painted in — what [`crate::log::selection`]
    /// selects over.
    ///
    /// This walks the log exactly as [`LogPainter::paint`] does, because a
    /// highlight that does not sit on the words is worse than no highlight at
    /// all: the two have to agree about where each line was put.
    pub fn spans(&self, lines: &[Line], width: f32) -> Vec<Span> {
        let plain = self.font(Style::Answer);
        let mut spans = Vec::new();
        let mut row = 0usize;
        for line in lines {
            match &line.kind {
                Kind::Text { text, style } => {
                    let font = self.font(*style);
                    let width = measure_runs(&font, text);
                    spans.push(Span {
                        rect: Rect::from_xywh(INSET, line.top, width, line.height),
                        text: text.clone(),
                        font,
                        line: row,
                    });
                    row += 1;
                }
                Kind::Bubble {
                    lines: words,
                    width: bubble,
                    ..
                } => {
                    let left = INSET + width - bubble + BUBBLE_PAD_X;
                    for (index, words) in words.iter().enumerate() {
                        let top = line.top + BUBBLE_PAD_Y + index as f32 * LINE_H;
                        spans.push(Span {
                            rect: Rect::from_xywh(left, top, measure_runs(&plain, words), LINE_H),
                            text: words.clone(),
                            font: plain.clone(),
                            line: row,
                        });
                        row += 1;
                    }
                }
                Kind::Document(doc) => {
                    for text_line in doc {
                        for run in &text_line.runs {
                            spans.push(Span {
                                rect: Rect::from_xywh(
                                    INSET + run.x,
                                    line.top + text_line.top,
                                    run.width,
                                    text_line.height,
                                ),
                                text: run.text.clone(),
                                font: run.font(),
                                line: row,
                            });
                        }
                        row += 1;
                    }
                }
                Kind::Steps { lines: words, .. } => {
                    let font = self.font(Style::Note);
                    let height = Style::Note.line_h();
                    for (index, words) in words.iter().enumerate() {
                        spans.push(Span {
                            rect: Rect::from_xywh(
                                INSET,
                                line.top + index as f32 * height,
                                measure_runs(&font, words),
                                height,
                            ),
                            text: words.clone(),
                            font: font.clone(),
                            line: row,
                        });
                        row += 1;
                    }
                }
                // Neither a picture nor the footer has words to select over —
                // the footer is the card talking about itself, not the
                // conversation — but each is a line of the log all the same,
                // so the numbering carries on past it.
                // Attachments are clicked, not selected.
                Kind::Image { .. } | Kind::Footer { .. } | Kind::Attachments { .. } => row += 1,
            }
        }
        spans
    }

    /// The group of tool calls under `point` in the log, as the log line
    /// holding it and the request it belongs to. `point` is in the log's
    /// content coordinates, and the log is laid out `width` wide.
    pub fn steps_at(
        &self,
        lines: &[Line],
        point: (f32, f32),
        width: f32,
    ) -> Option<(usize, usize)> {
        lines.iter().enumerate().find_map(|(index, line)| {
            let Kind::Steps { block, .. } = &line.kind else {
                return None;
            };
            let rect = steps_rect(line, width);
            let (x, y) = point;
            (x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom)
                .then_some((index, *block))
        })
    }

    /// The code block under `point` in the log, as the log line holding the
    /// answer and the block within it, and whether the point is on the
    /// block's copy button. `point` is in the log's content coordinates, and
    /// the log is laid out `width` wide.
    pub fn code_at(
        &self,
        lines: &[Line],
        point: (f32, f32),
        width: f32,
    ) -> Option<(usize, document::CodeHit)> {
        lines.iter().enumerate().find_map(|(index, line)| {
            let Kind::Document(doc) = &line.kind else {
                return None;
            };
            let content = Rect::from_xywh(INSET, line.top, width, line.height);
            document::code_at(content, doc, 0.0, point).map(|hit| (index, hit))
        })
    }

    /// The link under `point` in the log, as its destination. `point` and
    /// `width` are as for [`LogPainter::code_at`].
    pub fn link_at<'a>(&self, lines: &'a [Line], point: (f32, f32), width: f32) -> Option<&'a str> {
        lines.iter().find_map(|line| {
            let Kind::Document(doc) = &line.kind else {
                return None;
            };
            let content = Rect::from_xywh(INSET, line.top, width, line.height);
            document::link_at_scrolled(content, doc, 0.0, point)
        })
    }

    /// The text of the `block`th code block of the answer on log line `line`.
    pub fn code_text(lines: &[Line], line: usize, block: usize) -> Option<String> {
        let Kind::Document(doc) = &lines.get(line)?.kind else {
            return None;
        };
        document::code_blocks(doc)
            .into_iter()
            .nth(block)
            .map(|block| block.text)
    }

    /// Paint the lines of the log, laid out `width` wide, that fall inside
    /// `band`, in the log's content coordinates. `copy` is the copy button to show on a code
    /// block, as the log line holding its answer and the button. `steps` is
    /// the log line of the group of tool calls the pointer is on, which is
    /// drawn as something to click.
    #[expect(
        clippy::too_many_arguments,
        reason = "the log, and what the pointer is doing to it"
    )]
    pub fn paint(
        &self,
        canvas: &Canvas,
        band: Rect,
        lines: &[Line],
        width: f32,
        selection: &[Rect],
        copy: Option<(usize, document::CopyButton)>,
        steps: Option<usize>,
        attachment: Option<AttachmentHit>,
    ) {
        let prompt_font = self.font(Style::Prompt);
        let note_font = self.font(Style::Note);
        let font = self.font(Style::Answer);
        let mut text = Paint::new(Color4f::from(row_title_color(self.dark)), None);
        text.set_anti_alias(true);
        let mut dim = Paint::new(Color4f::from(row_subtitle_color(self.dark)), None);
        dim.set_anti_alias(true);

        let theme = self.theme();
        let mut request = Paint::new(Color4f::from(theme.text_primary), None);
        request.set_anti_alias(true);

        // The highlight goes down first, so the words sit on top of it.
        let mut highlight = Paint::new(Color4f::from(selection_color(&theme)), None);
        highlight.set_anti_alias(true);
        for rect in selection {
            if rect.bottom < band.top || rect.top > band.bottom {
                continue;
            }
            canvas.draw_round_rect(*rect, 2.0, 2.0, &highlight);
        }
        let mut bubble_fill = Paint::new(Color4f::from(theme.fill_secondary), None);
        bubble_fill.set_anti_alias(true);

        for (index, line) in lines.iter().enumerate() {
            if line.top + line.height < band.top || line.top > band.bottom {
                continue;
            }
            match &line.kind {
                Kind::Text { text: words, .. } if words.is_empty() => {}
                Kind::Text { text: words, style } => {
                    let baseline = line.top + line.height * 0.72;
                    let (font, paint) = match style {
                        Style::Prompt => (&prompt_font, &text),
                        Style::Request | Style::Answer => (&font, &text),
                        Style::Note => (&note_font, &dim),
                    };
                    draw_runs(canvas, words, (INSET, baseline), font, paint);
                }
                Kind::Bubble {
                    lines: words,
                    width: bubble_w,
                    ..
                } => {
                    let bubble = Rect::from_xywh(
                        INSET + width - bubble_w,
                        line.top,
                        *bubble_w,
                        line.height - BUBBLE_GAP,
                    );
                    let radius = BUBBLE_RADIUS.min(bubble.height() / 2.0);
                    canvas.draw_round_rect(bubble, radius, radius, &bubble_fill);
                    for (index, words) in words.iter().enumerate() {
                        let baseline =
                            line.top + BUBBLE_PAD_Y + index as f32 * LINE_H + LINE_H * 0.72;
                        draw_runs(
                            canvas,
                            words,
                            (bubble.left + BUBBLE_PAD_X, baseline),
                            &font,
                            &request,
                        );
                    }
                }
                Kind::Steps { lines: words, .. } => {
                    // Under the pointer the group takes a fill: painted text
                    // says nothing about being clickable on its own, and the
                    // cursor alone is only found by the person already there.
                    if steps == Some(index) {
                        let mut fill = Paint::new(Color4f::from(theme.fill_secondary), None);
                        fill.set_anti_alias(true);
                        canvas.draw_round_rect(
                            steps_rect(line, width),
                            STEPS_RADIUS,
                            STEPS_RADIUS,
                            &fill,
                        );
                    }
                    let height = Style::Note.line_h();
                    for (row, words) in words.iter().enumerate() {
                        let baseline = line.top + row as f32 * height + height * 0.72;
                        draw_runs(canvas, words, (INSET, baseline), &note_font, &dim);
                    }
                }
                Kind::Footer { agent, mode, hint } => {
                    let baseline = line.top + FOOTER_H * 0.68;
                    let mut x = INSET;
                    draw_runs(canvas, agent, (x, baseline), &note_font, &dim);
                    x += measure_runs(&note_font, agent) + FOOTER_GAP;

                    // The mode is the one thing here that can be changed, so
                    // it is the one thing wearing a control's shape.
                    let pill_w = measure_runs(&note_font, mode) + PILL_PAD_X * 2.0;
                    let pill =
                        Rect::from_xywh(x, line.top + (FOOTER_H - PILL_H) / 2.0, pill_w, PILL_H);
                    let mut fill = Paint::new(Color4f::from(theme.fill_secondary), None);
                    fill.set_anti_alias(true);
                    canvas.draw_round_rect(pill, PILL_H / 2.0, PILL_H / 2.0, &fill);
                    draw_runs(canvas, mode, (x + PILL_PAD_X, baseline), &note_font, &text);
                    x += pill_w + FOOTER_GAP;

                    if let Some(hint) = hint {
                        draw_runs(canvas, hint, (x, baseline), &note_font, &dim);
                    }
                }
                Kind::Document(doc) => {
                    let content = Rect::from_xywh(INSET, line.top, width, line.height);
                    let copy = copy
                        .filter(|(on_line, _)| *on_line == index)
                        .map(|(_, button)| button);
                    document::draw_scrolled(canvas, content, doc, 0.0, &theme, copy);
                }
                Kind::Attachments { items, pending } => {
                    let layout = self.lay_out_attachments(items, *pending, width);
                    let hovered = attachment
                        .filter(|hit| hit.line == index)
                        .map(|hit| hit.item);
                    canvas.save();
                    canvas.translate((INSET, line.top + HOVER_PAD));
                    self.attachments
                        .borrow()
                        .paint(canvas, &layout, &theme, hovered);
                    canvas.restore();
                }
                Kind::Image {
                    path,
                    label,
                    width: image_w,
                    height: image_h,
                } => {
                    let Some(image) = self.picture(path) else {
                        // The layout only makes a picture line for a file it
                        // could read; if it has gone since, its name goes in
                        // its place rather than a hole in the log.
                        let baseline = line.top + LINE_H * 0.72;
                        let words = format!("picture: {label}");
                        draw_runs(canvas, &words, (INSET, baseline), &note_font, &dim);
                        continue;
                    };
                    let box_ = Rect::from_xywh(INSET, line.top + IMAGE_PAD, *image_w, *image_h);
                    let radius = IMAGE_RADIUS.min(box_.height() / 2.0);
                    canvas.save();
                    // Rounded like every other surface in the card, and clipped
                    // rather than drawn rounded: the picture's own edge pixels
                    // must not bleed past the corner.
                    canvas.clip_rrect(
                        skia_safe::RRect::new_rect_xy(box_, radius, radius),
                        None,
                        true,
                    );
                    let source = (image.width(), image.height());
                    canvas.draw_image_rect_with_sampling_options(
                        &image,
                        None,
                        box_,
                        otto_kit::utils::icon_sampling(source, (*image_w, *image_h)),
                        &Paint::default(),
                    );
                    canvas.restore();
                    // A hairline border, so a picture that is nearly the colour
                    // of the card still reads as a picture.
                    let mut edge = Paint::new(Color4f::from(theme.hairline), None);
                    edge.set_anti_alias(true);
                    edge.set_style(skia_safe::paint::Style::Stroke);
                    edge.set_stroke_width(1.0);
                    canvas.draw_round_rect(box_, radius, radius, &edge);
                }
            }
        }
    }
}

/// What a group of tool calls answers the pointer over, in a log `width` wide:
/// its words and the padding the fill is drawn in.
fn steps_rect(line: &Line, width: f32) -> Rect {
    Rect::from_xywh(
        INSET - STEPS_PAD,
        line.top - STEPS_PAD / 2.0,
        width + STEPS_PAD * 2.0,
        line.height + STEPS_PAD,
    )
}

/// What selected text in the log sits on: the accent, faint enough to read
/// through.
fn selection_color(theme: &Theme) -> Color {
    let accent = theme.accent;
    Color::from_argb(90, accent.r(), accent.g(), accent.b())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::transcript::{Picture, Said};
    use crate::log::Block;

    /// The width the launcher's card lays the log out at.
    const WIDTH: f32 = 580.0;

    /// An agent that shares a link writes it bare, so the log has to find one
    /// under the pointer — a link that cannot be clicked is not a link.
    #[test]
    fn a_bare_link_in_the_log_is_under_the_pointer() {
        let painter = LogPainter::new(true);
        let (blocks, _) = otto_md_kit::parse_capped(
            "the notes are at https://example.com/a now",
            otto_md_kit::MAX_BLOCKS,
        );
        let doc = otto_kit::preview::document::wrap_at(&blocks, WIDTH, crate::log::body());
        let run = doc
            .iter()
            .flat_map(|line| line.runs.iter().map(move |run| (line, run)))
            .find(|(_, run)| run.style.link)
            .expect("the bare link is a link");
        let (text_line, run) = run;
        let height = doc
            .last()
            .map(|line| line.top + line.height)
            .expect("a laid-out answer");
        let lines = vec![Line {
            top: 0.0,
            height,
            kind: Kind::Document(doc.clone()),
        }];

        let point = (
            INSET + run.x + run.width / 2.0,
            text_line.top + text_line.height / 2.0,
        );
        assert_eq!(
            painter.link_at(&lines, point, WIDTH),
            Some("https://example.com/a")
        );
        assert_eq!(
            painter.link_at(&lines, (INSET + 1.0, point.1), WIDTH),
            None,
            "the words before the link go nowhere"
        );
    }

    /// Selecting text in the log is hit-testing against the boxes this file
    /// says each piece of text was painted in, so those boxes have to hold
    /// every kind of line the log draws — a request in its bubble, an answer
    /// laid out as a document, a tool call, the status — and be where the
    /// words are.
    #[test]
    fn every_kind_of_line_in_the_log_can_be_pointed_at() {
        let painter = LogPainter::new(true);
        let steps = ["✓ ls".to_string()];
        let answer = [Said::Text(
            "Run `cargo build` first\n\n- then the tests".to_owned(),
        )];
        let blocks = [Block {
            prompt: "how do I build it",
            attachments: &[],
            answer: &answer,
            steps: &steps,
            steps_expanded: false,

            inputs: &[],
            question: None,
            action: &[],
            note: None,
        }];
        let lines = crate::log::lay_out(
            &blocks,
            Some("Working…"),
            &[],
            None,
            WIDTH,
            |text, style| painter.measure(text, style),
            |path| painter.picture_size(path),
            |_, _| 0.0,
        );
        let spans = painter.spans(&lines, WIDTH);
        // All of it, copied, reads as the conversation does on screen: the
        // request, the answer with its code and its list, the tool call and
        // the status, each on its own line.
        let all = crate::log::selection::everything(&spans).expect("something to select");
        assert_eq!(
            crate::log::selection::text(&spans, all),
            "how do I build it\nRun cargo build first\n• then the tests\n✓ ls\n\nWorking…"
        );

        let length = crate::log::length(&lines);
        for span in &spans {
            assert!(
                span.rect.left >= 0.0 && span.rect.right <= WIDTH + INSET * 2.0 + 0.5,
                "{:?} is drawn off the card",
                span.text
            );
            assert!(
                span.rect.top >= 0.0 && span.rect.bottom <= length + 0.5,
                "{:?} is drawn outside the log",
                span.text
            );
        }
        // Reading order: a span never starts above the one before it.
        for pair in spans.windows(2) {
            assert!(pair[1].rect.top >= pair[0].rect.top - 0.5);
        }
        // And a press in the middle of a span lands in that span.
        let request = spans
            .iter()
            .position(|span| span.text.contains("how do I build it"))
            .expect("the request is there");
        let rect = spans[request].rect;
        let caret = crate::log::selection::caret_at(
            &spans,
            (rect.left + rect.width() / 2.0, rect.center_y()),
        )
        .expect("a press on the words selects them");
        assert_eq!(caret.span, request);
        assert!(caret.byte > 0 && caret.byte < spans[request].text.len());
    }

    /// A picture has no words in it, but it is still a line of the log. The
    /// spans on either side of it have to keep the line numbers they would have
    /// had, or copying an answer with a picture in it puts the words in the
    /// wrong order.
    #[test]
    fn a_picture_keeps_its_line_without_adding_a_span() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let path = dir.path().join("shot.png");
        std::fs::write(&path, PIXEL_PNG).expect("written");
        let painter = LogPainter::new(true);
        let answer = [
            Said::Text("here:".to_owned()),
            Said::Image(Picture {
                path: path.clone(),
                label: "shot".to_owned(),
            }),
            Said::Text("that is all".to_owned()),
        ];
        let blocks = [Block {
            prompt: "draw",
            attachments: &[],
            answer: &answer,
            steps: &[],
            steps_expanded: false,

            inputs: &[],
            question: None,
            action: &[],
            note: None,
        }];
        let lines = crate::log::lay_out(
            &blocks,
            None,
            &[],
            None,
            WIDTH,
            |text, style| painter.measure(text, style),
            |path| painter.picture_size(path),
            |_, _| 0.0,
        );
        assert!(
            matches!(lines[2].kind, Kind::Image { .. }),
            "the picture is laid out: {:?}",
            lines[2].kind
        );

        let spans = painter.spans(&lines, WIDTH);
        let all = crate::log::selection::everything(&spans).expect("something to select");
        // The picture has nothing to copy, so it contributes no line of its
        // own — but it takes a line number, which is what keeps the words after
        // it from being run onto the words before it.
        assert_eq!(
            crate::log::selection::text(&spans, all),
            "draw\nhere:\nthat is all"
        );
        let mut rows: Vec<usize> = spans.iter().map(|span| span.line).collect();
        rows.dedup();
        assert_eq!(rows, [0, 1, 3], "the picture is line 2");
    }

    /// A one-pixel PNG, so the decoder has something real to read.
    const PIXEL_PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xfc,
        0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0xab, 0xce, 0x36, 0x89, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    /// A conversation with one of each kind of line that has words in it.
    fn conversation(painter: &LogPainter, width: f32) -> Vec<Line> {
        let steps = ["✓ ls".to_string(), "✓ cargo build --workspace".to_string()];
        let answer = [Said::Text(
            "Run `cargo build` first, and when it has finished run the tests \
             with `cargo test`\n\n- then the tests"
                .to_owned(),
        )];
        let blocks = [Block {
            prompt: "how do I build it, and how do I know it worked once it has",
            attachments: &[],
            answer: &answer,
            steps: &steps,
            steps_expanded: true,
            inputs: &[],
            question: None,
            action: &[],
            note: None,
        }];
        crate::log::lay_out(
            &blocks,
            Some("Working…"),
            &[],
            None,
            width,
            |text, style| painter.measure(text, style),
            |path| painter.picture_size(path),
            |items, pending| painter.attachments_height(items, pending, width),
        )
    }

    /// The log is laid out at the host's width, not the card's. At a narrow
    /// one the same conversation wraps onto more lines rather than running
    /// past the edge, and at either every piece of text is still where a
    /// press finds it.
    #[test]
    fn the_log_wraps_to_any_width_and_stays_pointable() {
        let painter = LogPainter::new(true);
        let narrow = 220.0;
        let wide_lines = conversation(&painter, WIDTH);
        let narrow_lines = conversation(&painter, narrow);
        let wide_spans = painter.spans(&wide_lines, WIDTH);
        let narrow_spans = painter.spans(&narrow_lines, narrow);
        assert!(
            narrow_spans.len() > wide_spans.len(),
            "a narrow log wraps onto more lines"
        );
        assert!(crate::log::length(&narrow_lines) > crate::log::length(&wide_lines));

        for (lines, spans, width) in [
            (&wide_lines, &wide_spans, WIDTH),
            (&narrow_lines, &narrow_spans, narrow),
        ] {
            for span in spans {
                assert!(
                    span.rect.left >= INSET - 0.5 && span.rect.right <= INSET + width + 0.5,
                    "{:?} runs past a log {width} wide",
                    span.text
                );
                // A blank line is a span with nothing in it to press on.
                if span.rect.width() <= 0.0 {
                    continue;
                }
                let centre = (span.rect.center_x(), span.rect.center_y());
                let caret = crate::log::selection::caret_at(spans, centre).expect("a press lands");
                assert_eq!(&spans[caret.span].text, &span.text, "at width {width}");
            }
            // And it paints at that width, every band of it.
            let mut surface = skia_safe::surfaces::raster_n32_premul((
                (width + INSET * 2.0) as i32,
                crate::log::length(lines).ceil() as i32 + 1,
            ))
            .expect("a raster surface");
            let band = Rect::from_xywh(0.0, 0.0, width + INSET * 2.0, crate::log::length(lines));
            painter.paint(surface.canvas(), band, lines, width, &[], None, None, None);
        }
    }

    /// The group of tool calls answers the pointer across the whole width it
    /// is laid out at, and no further.
    #[test]
    fn tool_calls_answer_the_pointer_across_the_log_width() {
        let painter = LogPainter::new(false);
        let narrow = 220.0;
        let lines = conversation(&painter, narrow);
        let (index, line) = lines
            .iter()
            .enumerate()
            .find(|(_, line)| matches!(line.kind, Kind::Steps { .. }))
            .expect("the tool calls are a group");
        let y = line.top + line.height / 2.0;
        assert_eq!(
            painter.steps_at(&lines, (INSET + narrow - 1.0, y), narrow),
            Some((index, 0))
        );
        assert_eq!(
            painter.steps_at(&lines, (INSET + narrow + 20.0, y), narrow),
            None
        );
    }
}
