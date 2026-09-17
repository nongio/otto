//! The ask log: the conversation, wrapped to the card.
//!
//! Laid out here as lines, and painted by the view a band at a time. Requests,
//! tool calls and notes are plain text; the agent's answer is Markdown, read
//! with [`otto_md_kit`] and wrapped by the toolkit's document layout, so its
//! headings, lists and code look like a document rather than like its source.
//! The launcher lays it out again whenever the conversation changes, which is
//! at most once per pass of its loop however many pieces of text arrived in
//! between.

use otto_kit::preview::document;

/// Height of one line of plain text in the log.
pub const LINE_H: f32 = 21.0;

/// Space above and below an answer, so a heading does not sit on the request.
const ANSWER_PAD: f32 = 6.0;

/// Room between a request's words and the edge of its bubble, across.
pub const BUBBLE_PAD_X: f32 = 12.0;

/// Room between a request's words and the edge of its bubble, down.
pub const BUBBLE_PAD_Y: f32 = 6.0;

/// Space under a request's bubble, so what answers it does not touch it.
pub const BUBBLE_GAP: f32 = 6.0;

/// How a line of plain text is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// A request as it was typed, drawn in its bubble.
    Request,
    /// A question from the agent.
    Prompt,
    /// The agent's words that are not Markdown: a question's detail.
    Answer,
    /// Dimmed: a tool call, what became of a request, or what the agent is
    /// doing now.
    Note,
}

/// What a line of the log holds.
#[derive(Clone, Debug)]
pub enum Kind {
    /// One line of plain text, [`LINE_H`] tall.
    Text { text: String, style: Style },
    /// A request, wrapped into `lines` and set in a bubble `width` wide, at
    /// the right of the log like the sent side of a chat. Its height is the
    /// lines plus the bubble's padding, and [`BUBBLE_GAP`] under it.
    Bubble {
        text: String,
        lines: Vec<String>,
        width: f32,
    },
    /// An answer, wrapped as a document. Its lines' tops are relative to the
    /// answer's own top.
    Document(Vec<document::Line>),
}

/// One piece of the log, placed.
#[derive(Clone, Debug)]
pub struct Line {
    /// Top edge, down the log from its first line.
    pub top: f32,
    pub height: f32,
    pub kind: Kind,
}

impl Line {
    /// The line's words, for a screen reader.
    pub fn text(&self) -> String {
        match &self.kind {
            Kind::Text { text, .. } | Kind::Bubble { text, .. } => text.clone(),
            Kind::Document(lines) => lines
                .iter()
                .map(|line| line.runs.iter().map(|run| run.text.as_str()).collect())
                .collect::<Vec<String>>()
                .join("\n"),
        }
    }
}

/// How tall the log is, top to bottom.
pub fn length(lines: &[Line]) -> f32 {
    lines
        .last()
        .map(|line| line.top + line.height)
        .unwrap_or(0.0)
}

/// One request in the log.
pub struct Block<'a> {
    pub prompt: &'a str,
    /// The files that went with the request, said in one line.
    pub attachments: Option<&'a str>,
    pub answer: &'a str,
    /// The tool calls the agent made, already allowed or refused.
    pub steps: &'a [String],
    /// What the agent asked the person, each request as its lines: open ones
    /// with their question, settled ones with their answers.
    pub inputs: &'a [Vec<(String, Style)>],
    /// The agent's question waiting for an answer: who wants to do what, and
    /// what exactly.
    pub question: Option<(&'a str, &'a str)>,
    /// What became of the request, when there is something to say.
    pub note: Option<&'a str>,
}

/// Wraps the conversation into lines no wider than `width`: each request and
/// its attachments, then its answer, tool calls, question and note, a blank
/// line apart from the next request. The log closes with `status`, what the
/// agent is doing now.
///
/// `measure` gives the width of a piece of plain text in a line's style.
pub fn lay_out(
    blocks: &[Block],
    status: Option<&str>,
    width: f32,
    measure: impl Fn(&str, Style) -> f32,
) -> Vec<Line> {
    let mut lines = Vec::new();
    let push = |lines: &mut Vec<Line>, text: &str, style: Style| {
        for text in wrap(text, width, |piece| measure(piece, style)) {
            place(lines, LINE_H, Kind::Text { text, style });
        }
    };
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            blank(&mut lines);
        }
        bubble(&mut lines, block.prompt, width, &measure);
        if let Some(attachments) = block.attachments {
            push(&mut lines, attachments, Style::Note);
        }
        answer(&mut lines, block.answer, width);
        for step in block.steps {
            push(&mut lines, step, Style::Note);
        }
        for input in block.inputs.iter().filter(|input| !input.is_empty()) {
            blank(&mut lines);
            for (text, style) in input {
                push(&mut lines, text, *style);
            }
        }
        if let Some((title, detail)) = block.question {
            blank(&mut lines);
            push(&mut lines, title, Style::Prompt);
            push(&mut lines, detail, Style::Answer);
        }
        if let Some(note) = block.note {
            push(&mut lines, note, Style::Note);
        }
    }
    if let Some(status) = status.filter(|status| !status.is_empty()) {
        if !lines.is_empty() {
            blank(&mut lines);
        }
        push(&mut lines, status, Style::Note);
    }
    lines
}

/// A request in its bubble, wrapped inside the bubble's padding and no wider
/// than its longest line needs.
fn bubble(lines: &mut Vec<Line>, text: &str, width: f32, measure: &impl Fn(&str, Style) -> f32) {
    let wrapped = wrap(text, width - BUBBLE_PAD_X * 2.0, |piece| {
        measure(piece, Style::Request)
    });
    if wrapped.is_empty() {
        return;
    }
    let widest = wrapped
        .iter()
        .map(|line| measure(line, Style::Request))
        .fold(0.0, f32::max);
    let height = wrapped.len() as f32 * LINE_H + BUBBLE_PAD_Y * 2.0 + BUBBLE_GAP;
    place(
        lines,
        height,
        Kind::Bubble {
            text: text.to_string(),
            lines: wrapped,
            width: (widest + BUBBLE_PAD_X * 2.0).min(width),
        },
    );
}

/// The answer as a document, under what came before it. An answer still
/// arriving is parsed as far as it goes: an unclosed fence reads as code
/// until the rest of it lands.
fn answer(lines: &mut Vec<Line>, markdown: &str, width: f32) {
    if markdown.trim().is_empty() {
        return;
    }
    let (blocks, _) = otto_md_kit::parse_capped(markdown, otto_md_kit::MAX_BLOCKS);
    let mut wrapped = document::wrap(&blocks, width);
    let Some(height) = wrapped.last().map(|line| line.top + line.height) else {
        return;
    };
    for line in &mut wrapped {
        line.top += ANSWER_PAD;
    }
    place(lines, height + ANSWER_PAD * 2.0, Kind::Document(wrapped));
}

fn place(lines: &mut Vec<Line>, height: f32, kind: Kind) {
    let top = length(lines);
    lines.push(Line { top, height, kind });
}

fn blank(lines: &mut Vec<Line>) {
    place(
        lines,
        LINE_H,
        Kind::Text {
            text: String::new(),
            style: Style::Answer,
        },
    );
}

/// Greedy word wrap. Line breaks in `text` are kept; a word wider than the
/// line on its own is broken between characters, because it has nowhere else
/// to go.
pub fn wrap(text: &str, width: f32, measure: impl Fn(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    if text.is_empty() {
        return lines;
    }
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let candidate = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if measure(&candidate) <= width {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            // The word starts a line of its own, broken if it has to be.
            for c in word.chars() {
                let mut next = line.clone();
                next.push(c);
                if !line.is_empty() && measure(&next) > width {
                    lines.push(std::mem::take(&mut line));
                    line.push(c);
                } else {
                    line = next;
                }
            }
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One point per character, so widths read as character counts.
    fn chars(text: &str) -> f32 {
        text.chars().count() as f32
    }

    #[test]
    fn words_move_to_the_next_line_rather_than_overflow() {
        assert_eq!(
            wrap("the quick brown fox", 10.0, chars),
            ["the quick", "brown fox"]
        );
    }

    #[test]
    fn line_breaks_and_blank_lines_are_kept() {
        assert_eq!(wrap("one\n\ntwo", 10.0, chars), ["one", "", "two"]);
    }

    #[test]
    fn a_word_too_long_for_a_line_is_broken() {
        assert_eq!(
            wrap("see abcdefghijkl", 5.0, chars),
            ["see", "abcde", "fghij", "kl"]
        );
    }

    #[test]
    fn nothing_to_wrap_is_no_lines() {
        assert!(wrap("", 10.0, chars).is_empty());
    }

    /// Each line as its text and how it is drawn, `None` for an answer laid
    /// out as a document.
    fn styled(lines: &[Line]) -> Vec<(String, Option<Style>)> {
        lines
            .iter()
            .map(|line| match &line.kind {
                Kind::Text { text, style } => (text.clone(), Some(*style)),
                Kind::Bubble { text, .. } => (text.clone(), Some(Style::Request)),
                Kind::Document(_) => (line.text(), None),
            })
            .collect()
    }

    fn text(text: &str, style: Style) -> (String, Option<Style>) {
        (text.to_string(), Some(style))
    }

    fn doc(text: &str) -> (String, Option<Style>) {
        (text.to_string(), None)
    }

    fn block<'a>(prompt: &'a str, answer: &'a str, note: Option<&'a str>) -> Block<'a> {
        Block {
            prompt,
            attachments: None,
            answer,
            steps: &[],
            inputs: &[],
            question: None,
            note,
        }
    }

    #[test]
    fn requests_answers_and_notes_read_in_order_with_the_status_last() {
        let blocks = [
            block("hi", "Hello", None),
            block("more", "", Some("Queued")),
        ];
        let lines = lay_out(&blocks, Some("Working…"), 20.0, |text, _| chars(text));
        assert_eq!(
            styled(&lines),
            [
                text("hi", Style::Request),
                doc("Hello"),
                text("", Style::Answer),
                text("more", Style::Request),
                text("Queued", Style::Note),
                text("", Style::Answer),
                text("Working…", Style::Note),
            ]
        );
    }

    #[test]
    fn tool_calls_and_a_question_follow_the_answer() {
        let steps = ["✓ ls".to_string()];
        let blocks = [Block {
            prompt: "tidy up",
            attachments: None,
            answer: "Sure",
            steps: &steps,
            inputs: &[],
            question: Some(("Claude wants to run a command", "rm -rf build")),
            note: None,
        }];
        let lines = lay_out(&blocks, None, 40.0, |text, _| chars(text));
        assert_eq!(
            styled(&lines),
            [
                text("tidy up", Style::Request),
                doc("Sure"),
                text("✓ ls", Style::Note),
                text("", Style::Answer),
                text("Claude wants to run a command", Style::Prompt),
                text("rm -rf build", Style::Answer),
            ]
        );
    }

    #[test]
    fn what_the_agent_asked_follows_its_tool_calls() {
        let steps = ["✓ ls".to_string()];
        let inputs = [
            vec![("Which database?: Postgres".to_string(), Style::Note)],
            vec![
                ("Question 2 of 2".to_string(), Style::Note),
                ("Port?".to_string(), Style::Prompt),
            ],
        ];
        let blocks = [Block {
            steps: &steps,
            inputs: &inputs,
            ..block("set it up", "", None)
        }];
        let lines = lay_out(&blocks, None, 40.0, |text, _| chars(text));
        assert_eq!(
            styled(&lines),
            [
                text("set it up", Style::Request),
                text("✓ ls", Style::Note),
                text("", Style::Answer),
                text("Which database?: Postgres", Style::Note),
                text("", Style::Answer),
                text("Question 2 of 2", Style::Note),
                text("Port?", Style::Prompt),
            ]
        );
    }

    #[test]
    fn attachments_follow_their_request() {
        let blocks = [Block {
            attachments: Some("Attached: notes.md"),
            ..block("summarise", "Done", None)
        }];
        let lines = lay_out(&blocks, None, 40.0, |text, _| chars(text));
        assert_eq!(
            styled(&lines),
            [
                text("summarise", Style::Request),
                text("Attached: notes.md", Style::Note),
                doc("Done"),
            ]
        );
    }

    #[test]
    fn each_style_is_measured_as_it_is_drawn() {
        // Questions are drawn wider here, so the same words wrap sooner.
        let blocks = [Block {
            question: Some(("aa bb", "aa bb")),
            ..block("", "", None)
        }];
        let lines = lay_out(&blocks, None, 5.0, |text, style| match style {
            Style::Prompt => chars(text) * 2.0,
            _ => chars(text),
        });
        assert_eq!(
            styled(&lines),
            [
                text("", Style::Answer),
                text("aa", Style::Prompt),
                text("bb", Style::Prompt),
                text("aa bb", Style::Answer),
            ]
        );
    }

    #[test]
    fn an_answer_is_read_as_markdown_and_lines_stack() {
        let answer = "# Plan\n\n- one\n- two\n\n```\ncargo build\n```\n";
        let blocks = [block("go", answer, Some("Done"))];
        let lines = lay_out(&blocks, None, 400.0, |text, _| chars(text));
        let Kind::Document(doc) = &lines[1].kind else {
            panic!("the answer is a document");
        };
        assert!(doc.len() >= 4, "heading, two items and code");
        assert!(!lines[1].text().contains('#'), "the heading marker is gone");
        // Each line starts where the one above it ends.
        for pair in lines.windows(2) {
            assert_eq!(pair[1].top, pair[0].top + pair[0].height);
        }
        assert_eq!(length(&lines), lines[2].top + LINE_H);
    }

    #[test]
    fn a_request_wraps_inside_its_bubble_and_fits_its_words() {
        let blocks = [block("aaaa bb", "", None)];
        let width = 4.0 + BUBBLE_PAD_X * 2.0;
        let lines = lay_out(&blocks, None, width, |text, _| chars(text));
        let Kind::Bubble {
            lines: wrapped,
            width: bubble,
            ..
        } = &lines[0].kind
        else {
            panic!("the request is a bubble");
        };
        assert_eq!(wrapped, &["aaaa", "bb"]);
        assert_eq!(*bubble, width);
        assert_eq!(
            lines[0].height,
            LINE_H * 2.0 + BUBBLE_PAD_Y * 2.0 + BUBBLE_GAP
        );

        let lines = lay_out(&[block("bb", "", None)], None, 400.0, |text, _| chars(text));
        let Kind::Bubble { width: bubble, .. } = &lines[0].kind else {
            panic!("the request is a bubble");
        };
        assert_eq!(*bubble, 2.0 + BUBBLE_PAD_X * 2.0);
    }

    #[test]
    fn an_answer_not_started_takes_no_room() {
        let blocks = [block("go", "", None)];
        let lines = lay_out(&blocks, None, 400.0, |text, _| chars(text));
        assert_eq!(styled(&lines), [text("go", Style::Request)]);
    }
}
