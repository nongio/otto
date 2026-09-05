//! Markdown.
//!
//! Parsed here, in the worker, into the block vocabulary
//! [`otto_kit::preview::document`] draws — never into HTML, and never with a
//! browser engine anywhere near it. Markdown is the one document format the
//! desktop is full of that a previewer can render honestly without one: its
//! block structure maps onto shapes the toolkit already draws, so a `.md` file
//! can look like a document rather than like its own source.
//!
//! It is a *reading* parser, not a conforming one. CommonMark has corners
//! (nested blocks inside list items, reference links, HTML blocks, tight vs.
//! loose lists) whose full treatment costs far more than the difference it
//! makes at preview size, and a wrong answer in one of them degrades to a
//! paragraph rather than to nonsense. Two deliberate flattenings:
//!
//! - **A table is drawn as a code block.** Aligning columns means measuring
//!   text, and the worker has no fonts; the pipe-separated source at least
//!   lines up in a monospaced face.
//! - **An image is drawn as its alt text**, marked as a link. The worker holds
//!   one descriptor and cannot open the file an image refers to, and inventing
//!   a placeholder box for something it will never load would be a lie.

use std::fs::File;

use crate::payload::{self, Block, PreviewPayload, Span, SpanStyle};

use super::{read_capped, Request};

/// Beyond this a document is truncated before it is parsed. The same ceiling
/// as plain text, for the same reason: generated Markdown gets very large.
const MAX_BYTES: u64 = 8 * 1024 * 1024;
/// A preview is a look. Past this many blocks nobody is reading, and the host
/// still has to lay every one of them out.
const MAX_BLOCKS: usize = 4_000;
/// Cut on the same rule the text previewer uses, so one pathological line
/// cannot become one pathological layout.
const MAX_LINE_BYTES: usize = 2_000;
/// How deep a nested list is allowed to indent before it stops indenting.
const MAX_INDENT: u8 = 6;

pub fn read(file: &mut File, _request: &Request) -> PreviewPayload {
    let bytes = match read_capped(file, MAX_BYTES) {
        Ok(bytes) => bytes,
        Err(err) => {
            return payload::unavailable(otto_kit::t_owned!(
                "quickview-error-read-file",
                error = err.to_string()
            ))
        }
    };
    let mut truncated = bytes.len() as u64 >= MAX_BYTES;

    // A NUL says binary whatever the name and the sniffer agreed on.
    if bytes.iter().take(4096).any(|byte| *byte == 0) {
        return payload::unavailable(otto_kit::t_owned!("quickview-error-not-text"));
    }
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        // Latin-1 maps every byte, so this cannot fail. Markdown that is not
        // UTF-8 is old, not broken.
        Err(err) => err.into_bytes().iter().map(|byte| *byte as char).collect(),
    };

    let mut blocks = parse(text.strip_prefix('\u{feff}').unwrap_or(&text));
    if blocks.len() > MAX_BLOCKS {
        blocks.truncate(MAX_BLOCKS);
        truncated = true;
    }
    PreviewPayload::Document { blocks, truncated }
}

/// Parse a document into blocks.
///
/// Line-based, single pass, with one line of lookahead for setext headings.
/// Anything it does not recognise ends up as a paragraph, which is the whole
/// design: Markdown's fallback is its own source text, and that reads.
pub fn parse(text: &str) -> Vec<Block> {
    let lines: Vec<&str> = text
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .collect();
    let mut blocks: Vec<Block> = Vec::new();
    let mut paragraph: Vec<String> = Vec::new();
    let mut index = 0usize;

    // YAML front matter: metadata about the file, not content of it. Dropped
    // rather than shown, because every `.md` in a docs tree would otherwise
    // open on a block of keys instead of on its title.
    if lines.first().is_some_and(|line| line.trim_end() == "---") {
        if let Some(end) = lines
            .iter()
            .skip(1)
            .position(|line| matches!(line.trim_end(), "---" | "..."))
        {
            index = end + 2;
        }
    }

    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start();
        let indent_spaces = line.len() - trimmed.len();

        // A fence closes the paragraph above it and swallows everything until
        // its match — including lines that would otherwise be headings, which
        // is the point of a fence.
        if let Some(fence) = fence_of(trimmed) {
            flush(&mut blocks, &mut paragraph);
            let mut body: Vec<String> = Vec::new();
            index += 1;
            while index < lines.len() {
                let candidate = lines[index].trim_start();
                if candidate.starts_with(fence)
                    && candidate
                        .trim_end()
                        .chars()
                        .all(|c| c == fence.chars().next().unwrap())
                {
                    index += 1;
                    break;
                }
                body.push(clamp(lines[index]));
                index += 1;
            }
            blocks.push(Block::Code { lines: body });
            continue;
        }

        if trimmed.is_empty() {
            flush(&mut blocks, &mut paragraph);
            index += 1;
            continue;
        }

        // Setext: the underline belongs to the paragraph above it, and only
        // to a paragraph — an underline under nothing is a rule or a
        // paragraph of its own, which is what the rule below makes it.
        if !paragraph.is_empty() {
            if let Some(level) = setext_level(trimmed) {
                let text = paragraph.join(" ");
                paragraph.clear();
                blocks.push(Block::Heading {
                    level,
                    spans: inline(&text),
                });
                index += 1;
                continue;
            }
        }

        if is_rule(trimmed) {
            flush(&mut blocks, &mut paragraph);
            blocks.push(Block::Rule);
            index += 1;
            continue;
        }

        if let Some((level, rest)) = atx_heading(trimmed) {
            flush(&mut blocks, &mut paragraph);
            blocks.push(Block::Heading {
                level,
                spans: inline(rest),
            });
            index += 1;
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix('>') {
            flush(&mut blocks, &mut paragraph);
            // Consecutive quoted lines are one quote, and a nested `>` only
            // deepens a quote this renderer draws one way anyway.
            let mut body = vec![rest.trim_start_matches('>').trim().to_string()];
            index += 1;
            while index < lines.len() {
                let next = lines[index].trim_start();
                match next.strip_prefix('>') {
                    Some(rest) => body.push(rest.trim_start_matches('>').trim().to_string()),
                    // Lazy continuation: a plain line under a quote is still
                    // in it, but a blank one ends it.
                    None if !next.is_empty() && !is_rule(next) && atx_heading(next).is_none() => {
                        body.push(next.to_string())
                    }
                    _ => break,
                }
                index += 1;
            }
            blocks.push(Block::Quote {
                spans: inline(&body.join(" ")),
            });
            continue;
        }

        if let Some((marker, rest)) = list_marker(trimmed) {
            flush(&mut blocks, &mut paragraph);
            // Two spaces to a level, which is what everyone writes, and four
            // works out to the same answer at depth.
            let indent = ((indent_spaces / 2).min(MAX_INDENT as usize)) as u8;
            let mut body = vec![rest.to_string()];
            index += 1;
            // Continuation lines: indented, not themselves an item, not blank.
            while index < lines.len() {
                let next = lines[index];
                let next_trimmed = next.trim_start();
                let deeper = next.len() - next_trimmed.len() > indent_spaces;
                if next_trimmed.is_empty()
                    || !deeper
                    || list_marker(next_trimmed).is_some()
                    || fence_of(next_trimmed).is_some()
                {
                    break;
                }
                body.push(next_trimmed.to_string());
                index += 1;
            }
            blocks.push(Block::Item {
                indent,
                marker,
                spans: inline(&body.join(" ")),
            });
            continue;
        }

        // A table: pipes, with a delimiter row under the header. Kept as its
        // own source in a monospaced block — see the module note.
        if is_table_row(trimmed)
            && lines
                .get(index + 1)
                .is_some_and(|next| is_table_delimiter(next.trim_start()))
        {
            flush(&mut blocks, &mut paragraph);
            let mut body: Vec<String> = Vec::new();
            while index < lines.len() && is_table_row(lines[index].trim_start()) {
                body.push(clamp(lines[index].trim()));
                index += 1;
            }
            blocks.push(Block::Code { lines: body });
            continue;
        }

        // An indented code block, but only where a paragraph is not already
        // running: four spaces under a paragraph is a continuation, not code.
        if indent_spaces >= 4 && paragraph.is_empty() {
            let mut body: Vec<String> = Vec::new();
            while index < lines.len() {
                let next = lines[index];
                if next.trim().is_empty() {
                    // A blank line inside indented code only ends it if what
                    // follows is not indented too.
                    let continues = lines
                        .get(index + 1)
                        .is_some_and(|after| after.starts_with("    "));
                    if !continues {
                        break;
                    }
                    body.push(String::new());
                    index += 1;
                    continue;
                }
                if !next.starts_with("    ") && !next.starts_with('\t') {
                    break;
                }
                body.push(clamp(next.get(4..).unwrap_or("")));
                index += 1;
            }
            blocks.push(Block::Code { lines: body });
            continue;
        }

        paragraph.push(trimmed.to_string());
        index += 1;
    }

    flush(&mut blocks, &mut paragraph);
    blocks
}

/// Turn the pending paragraph lines, if any, into a block.
fn flush(blocks: &mut Vec<Block>, paragraph: &mut Vec<String>) {
    if paragraph.is_empty() {
        return;
    }
    // Joined with spaces: a paragraph's own line breaks are not breaks, and
    // re-wrapping is the host's job anyway.
    let text = std::mem::take(paragraph).join(" ");
    blocks.push(Block::Paragraph {
        spans: inline(&text),
    });
}

/// The fence a line opens, if it opens one.
fn fence_of(line: &str) -> Option<&'static str> {
    if line.starts_with("```") {
        Some("```")
    } else if line.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

/// `---`, `***`, `___`: three or more of one character, and nothing else.
fn is_rule(line: &str) -> bool {
    let stripped: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    stripped.len() >= 3
        && matches!(stripped.chars().next(), Some('-') | Some('*') | Some('_'))
        && stripped
            .chars()
            .all(|c| c == stripped.chars().next().unwrap())
}

/// `## Heading`, with the level and the text.
fn atx_heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &line[hashes..];
    // A `#` with no space after it is not a heading — `#hashtag` is prose.
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some((hashes as u8, rest.trim().trim_end_matches('#').trim_end()))
}

/// An underline under a paragraph: `===` is a level 1, `---` a level 2.
fn setext_level(line: &str) -> Option<u8> {
    let trimmed = line.trim_end();
    if trimmed.len() >= 2 && trimmed.chars().all(|c| c == '=') {
        return Some(1);
    }
    if trimmed.len() >= 2 && trimmed.chars().all(|c| c == '-') {
        return Some(2);
    }
    None
}

/// A list item's marker and its text. Bullets become one bullet character
/// whatever they were written as; numbers keep their own number, because a
/// list starting at 4 means it.
fn list_marker(line: &str) -> Option<(String, &str)> {
    if let Some(rest) = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "))
    {
        // A task list keeps its box, as a character rather than a control:
        // nothing here is clickable.
        let rest = rest.trim_start();
        if let Some(text) = rest.strip_prefix("[ ] ") {
            return Some(("☐".to_string(), text));
        }
        if let Some(text) = rest
            .strip_prefix("[x] ")
            .or_else(|| rest.strip_prefix("[X] "))
        {
            return Some(("☑".to_string(), text));
        }
        return Some(("•".to_string(), rest));
    }
    let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && digits <= 9 {
        let rest = &line[digits..];
        if let Some(text) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return Some((format!("{}.", &line[..digits]), text.trim_start()));
        }
    }
    None
}

fn is_table_row(line: &str) -> bool {
    line.starts_with('|') && line.matches('|').count() >= 2
}

/// The `|---|:--:|` row that makes the line above it a header.
fn is_table_delimiter(line: &str) -> bool {
    is_table_row(line)
        && line
            .chars()
            .all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

fn clamp(line: &str) -> String {
    if line.len() <= MAX_LINE_BYTES {
        return line.to_string();
    }
    let mut end = MAX_LINE_BYTES;
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &line[..end])
}

// ---------------------------------------------------------------------------
// Inline
// ---------------------------------------------------------------------------

/// Split a line into styled runs.
///
/// A single left-to-right pass with no backtracking: an unclosed `**` stays
/// literal text rather than swallowing the rest of the document, which is the
/// behaviour that matters when the input is half-written notes.
pub fn inline(text: &str) -> Vec<Span> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans: Vec<Span> = Vec::new();
    let mut current = String::new();
    let mut style = SpanStyle::default();
    let mut at = 0usize;

    let push = |current: &mut String, spans: &mut Vec<Span>, style: SpanStyle| {
        if !current.is_empty() {
            spans.push(Span {
                text: std::mem::take(current),
                style,
            });
        }
    };

    while at < chars.len() {
        let ch = chars[at];

        // An escape is one character taken literally, and is why `\*` can be
        // written at all.
        if ch == '\\' && at + 1 < chars.len() {
            current.push(chars[at + 1]);
            at += 2;
            continue;
        }

        // Code spans win over everything: nothing inside them is markup.
        if ch == '`' {
            let ticks = chars[at..].iter().take_while(|c| **c == '`').count();
            if let Some(end) = find_run(&chars, at + ticks, '`', ticks) {
                push(&mut current, &mut spans, style);
                let body: String = chars[at + ticks..end].iter().collect();
                spans.push(Span {
                    text: body.trim().to_string(),
                    style: SpanStyle {
                        code: true,
                        ..style
                    },
                });
                at = end + ticks;
                continue;
            }
        }

        // `![alt](src)` and `[text](href)`. The destination is dropped: see
        // the module note.
        if ch == '[' || (ch == '!' && chars.get(at + 1) == Some(&'[')) {
            let open = if ch == '!' { at + 1 } else { at };
            if let Some((label, next)) = link_at(&chars, open) {
                push(&mut current, &mut spans, style);
                spans.extend(inline(&label).into_iter().map(|mut span| {
                    span.style.link = true;
                    span.style.bold |= style.bold;
                    span.style.italic |= style.italic;
                    span
                }));
                at = next;
                continue;
            }
        }

        // `<https://example.com>` — an autolink, shown as itself.
        if ch == '<' {
            if let Some(end) = chars[at..].iter().position(|c| *c == '>') {
                let body: String = chars[at + 1..at + end].iter().collect();
                if body.starts_with("http://") || body.starts_with("https://") {
                    push(&mut current, &mut spans, style);
                    spans.push(Span {
                        text: body,
                        style: SpanStyle {
                            link: true,
                            ..style
                        },
                    });
                    at += end + 1;
                    continue;
                }
            }
        }

        if let Some(marker) = emphasis_at(&chars, at) {
            let width = marker.len();
            let bold = width >= 2;
            // Only toggle off what is on, and only toggle on what has a
            // partner: an unmatched marker is a character.
            let matched = if (bold && style.bold) || (!bold && style.italic) {
                true
            } else {
                find_run(&chars, at + width, chars[at], width).is_some()
            };
            if matched {
                push(&mut current, &mut spans, style);
                if bold {
                    style.bold = !style.bold;
                } else {
                    style.italic = !style.italic;
                }
                at += width;
                continue;
            }
        }

        // `~~struck~~` reads as ordinary text here: there is no strike
        // attribute, and dropping the tildes says less than keeping them.
        current.push(ch);
        at += 1;
    }

    push(&mut current, &mut spans, style);
    if spans.is_empty() {
        spans.push(Span::plain(""));
    }
    spans
}

/// Where the emphasis marker at `at` starts, if one does.
///
/// `_` is only a marker at a word boundary, so `snake_case_names` stays one
/// word rather than becoming half-italic — the single most visible difference
/// between a naive parser and a usable one on a page of technical prose.
fn emphasis_at(chars: &[char], at: usize) -> Option<&'static str> {
    let ch = chars[at];
    if ch != '*' && ch != '_' {
        return None;
    }
    if ch == '_' {
        let before = at.checked_sub(1).map(|i| chars[i]);
        if before.is_some_and(|c| c.is_alphanumeric()) {
            return None;
        }
    }
    let run = chars[at..].iter().take_while(|c| **c == ch).count();
    // Three is bold *and* italic; taking two here and one on the next pass
    // gets both without a separate case.
    Some(if run >= 2 { "**" } else { "*" })
}

/// The index of the next run of `count` `needle`s at or after `from`.
fn find_run(chars: &[char], from: usize, needle: char, count: usize) -> Option<usize> {
    let mut at = from;
    while at + count <= chars.len() {
        if chars[at] == '\\' {
            at += 2;
            continue;
        }
        if chars[at..at + count].iter().all(|c| *c == needle) {
            return Some(at);
        }
        at += 1;
    }
    None
}

/// A `[label](destination)` starting at `at`, and where it ends.
fn link_at(chars: &[char], at: usize) -> Option<(String, usize)> {
    if chars.get(at) != Some(&'[') {
        return None;
    }
    let mut depth = 0usize;
    let mut close = None;
    for (offset, ch) in chars[at..].iter().enumerate() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(at + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    let label: String = chars[at + 1..close].iter().collect();

    match chars.get(close + 1) {
        // An inline destination, skipped over.
        Some('(') => {
            let end = chars[close..].iter().position(|c| *c == ')')? + close;
            Some((label, end + 1))
        }
        // `[label][ref]` and `[label]` — the reference definition is not
        // resolved, and the label is what the reader wanted anyway.
        Some('[') => {
            let end = chars[close + 1..].iter().position(|c| *c == ']')? + close + 1;
            Some((label, end + 1))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(spans: &[Span]) -> String {
        spans.iter().map(|span| span.text.as_str()).collect()
    }

    #[test]
    fn headings_are_headings_and_hashtags_are_not() {
        let blocks = parse("# Title\n\n#nothashtag\n\n### Third\n");
        assert!(matches!(blocks[0], Block::Heading { level: 1, .. }));
        assert!(matches!(blocks[1], Block::Paragraph { .. }));
        assert!(matches!(blocks[2], Block::Heading { level: 3, .. }));
    }

    #[test]
    fn a_paragraph_is_one_block_however_it_was_typed() {
        let blocks = parse("one\ntwo\n\nthree\n");
        assert_eq!(blocks.len(), 2);
        let Block::Paragraph { spans } = &blocks[0] else {
            panic!("paragraph");
        };
        assert_eq!(text_of(spans), "one two");
    }

    #[test]
    fn a_fence_swallows_what_would_otherwise_be_markup() {
        let blocks = parse("```rust\n# not a heading\n**not bold**\n```\n");
        let Block::Code { lines } = &blocks[0] else {
            panic!("code, got {:?}", blocks[0]);
        };
        assert_eq!(lines, &["# not a heading", "**not bold**"]);
    }

    #[test]
    fn an_unclosed_fence_still_ends_the_document() {
        let blocks = parse("```\nnever closed\n");
        assert!(matches!(blocks[0], Block::Code { .. }));
    }

    #[test]
    fn lists_carry_their_own_markers_and_depth() {
        let blocks = parse("- one\n  - nested\n3. third\n- [x] done\n");
        let markers: Vec<(&str, u8)> = blocks
            .iter()
            .filter_map(|block| match block {
                Block::Item { marker, indent, .. } => Some((marker.as_str(), *indent)),
                _ => None,
            })
            .collect();
        assert_eq!(markers, [("•", 0), ("•", 1), ("3.", 0), ("☑", 0)]);
    }

    #[test]
    fn a_rule_is_not_a_setext_underline_without_a_paragraph() {
        let blocks = parse("---\n");
        assert!(matches!(blocks[0], Block::Rule));

        let blocks = parse("Title\n---\n");
        assert!(matches!(blocks[0], Block::Heading { level: 2, .. }));
    }

    #[test]
    fn front_matter_is_not_shown() {
        let blocks = parse("---\ntitle: Notes\n---\n\n# Notes\n");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(blocks[0], Block::Heading { level: 1, .. }));
    }

    #[test]
    fn emphasis_composes_and_leaves_snake_case_alone() {
        let spans = inline("**bold** and *italic* and snake_case_name");
        let bold = spans.iter().find(|span| span.style.bold).expect("bold run");
        assert_eq!(bold.text, "bold");
        let italic = spans
            .iter()
            .find(|span| span.style.italic)
            .expect("italic run");
        assert_eq!(italic.text, "italic");
        assert!(text_of(&spans).contains("snake_case_name"));
    }

    #[test]
    fn an_unclosed_marker_stays_a_character() {
        let spans = inline("2 * 3 is not emphasis");
        assert!(spans.iter().all(|span| !span.style.italic));
        assert_eq!(text_of(&spans), "2 * 3 is not emphasis");
    }

    #[test]
    fn code_spans_are_literal_inside() {
        let spans = inline("call `a * b` twice");
        let code = spans.iter().find(|span| span.style.code).expect("code run");
        assert_eq!(code.text, "a * b");
        assert!(spans.iter().all(|span| !span.style.italic));
    }

    #[test]
    fn a_link_keeps_its_label_and_drops_its_destination() {
        let spans = inline("see [the spec](https://example.com/a) now");
        assert_eq!(text_of(&spans), "see the spec now");
        assert!(spans.iter().any(|span| span.style.link));
        // An image is its alt text.
        let spans = inline("![a diagram](d.png)");
        assert_eq!(text_of(&spans), "a diagram");
    }

    #[test]
    fn a_table_keeps_its_columns_as_code() {
        let blocks = parse("| a | b |\n|---|---|\n| 1 | 2 |\n");
        let Block::Code { lines } = &blocks[0] else {
            panic!("code, got {:?}", blocks[0]);
        };
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn a_quote_absorbs_its_continuation() {
        let blocks = parse("> quoted\nstill quoted\n\nafter\n");
        let Block::Quote { spans } = &blocks[0] else {
            panic!("quote, got {:?}", blocks[0]);
        };
        assert_eq!(text_of(spans), "quoted still quoted");
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn nothing_pathological_panics() {
        // Every prefix of a nasty document must parse rather than panic: the
        // input is a file, and files are truncated, half-written and hostile.
        let nasty = "# *a\n> `b\n- [c](\n***\n| x |\n\\\n~~~\n_d_ **e\n![f](g)h[i][j]";
        for cut in 0..nasty.len() {
            if nasty.is_char_boundary(cut) {
                let _ = parse(&nasty[..cut]);
            }
        }
    }
}
