//! The ask log: the conversation, wrapped to the card.
//!
//! Laid out here as lines of text, and painted by the view a band at a time.
//! The launcher lays it out again whenever the conversation changes, which is
//! at most once per pass of its loop however many pieces of text arrived in
//! between.

/// How a line is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// A request as it was typed, or a question from the agent.
    Prompt,
    /// The agent's answer.
    Answer,
    /// Dimmed: a tool call, what became of a request, or what the agent is
    /// doing now.
    Note,
}

/// One line of the log, as it is drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    pub style: Style,
}

/// One request in the log.
pub struct Block<'a> {
    pub prompt: &'a str,
    /// The files that went with the request, said in one line.
    pub attachments: Option<&'a str>,
    pub answer: &'a str,
    /// The tool calls the agent made, already allowed or refused.
    pub steps: &'a [String],
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
/// `measure` gives the width of a piece of text in a line's style.
pub fn lay_out(
    blocks: &[Block],
    status: Option<&str>,
    width: f32,
    measure: impl Fn(&str, Style) -> f32,
) -> Vec<Line> {
    let mut lines = Vec::new();
    let push = |lines: &mut Vec<Line>, text: &str, style: Style| {
        lines.extend(
            wrap(text, width, |piece| measure(piece, style))
                .into_iter()
                .map(|text| Line { text, style }),
        );
    };
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            lines.push(blank());
        }
        push(&mut lines, block.prompt, Style::Prompt);
        if let Some(attachments) = block.attachments {
            push(&mut lines, attachments, Style::Note);
        }
        push(&mut lines, block.answer, Style::Answer);
        for step in block.steps {
            push(&mut lines, step, Style::Note);
        }
        if let Some((title, detail)) = block.question {
            lines.push(blank());
            push(&mut lines, title, Style::Prompt);
            push(&mut lines, detail, Style::Answer);
        }
        if let Some(note) = block.note {
            push(&mut lines, note, Style::Note);
        }
    }
    if let Some(status) = status.filter(|status| !status.is_empty()) {
        if !lines.is_empty() {
            lines.push(blank());
        }
        push(&mut lines, status, Style::Note);
    }
    lines
}

fn blank() -> Line {
    Line {
        text: String::new(),
        style: Style::Answer,
    }
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

    fn styled(lines: &[Line]) -> Vec<(&str, Style)> {
        lines.iter().map(|l| (l.text.as_str(), l.style)).collect()
    }

    fn block<'a>(prompt: &'a str, answer: &'a str, note: Option<&'a str>) -> Block<'a> {
        Block {
            prompt,
            attachments: None,
            answer,
            steps: &[],
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
                ("hi", Style::Prompt),
                ("Hello", Style::Answer),
                ("", Style::Answer),
                ("more", Style::Prompt),
                ("Queued", Style::Note),
                ("", Style::Answer),
                ("Working…", Style::Note),
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
            question: Some(("Claude wants to run a command", "rm -rf build")),
            note: None,
        }];
        let lines = lay_out(&blocks, None, 40.0, |text, _| chars(text));
        assert_eq!(
            styled(&lines),
            [
                ("tidy up", Style::Prompt),
                ("Sure", Style::Answer),
                ("✓ ls", Style::Note),
                ("", Style::Answer),
                ("Claude wants to run a command", Style::Prompt),
                ("rm -rf build", Style::Answer),
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
                ("summarise", Style::Prompt),
                ("Attached: notes.md", Style::Note),
                ("Done", Style::Answer),
            ]
        );
    }

    #[test]
    fn each_style_is_measured_as_it_is_drawn() {
        // Prompts are drawn wider here, so the same words wrap sooner.
        let blocks = [block("aa bb", "aa bb", None)];
        let lines = lay_out(&blocks, None, 5.0, |text, style| match style {
            Style::Prompt => chars(text) * 2.0,
            _ => chars(text),
        });
        assert_eq!(
            styled(&lines),
            [
                ("aa", Style::Prompt),
                ("bb", Style::Prompt),
                ("aa bb", Style::Answer),
            ]
        );
    }
}
