//! Block structure: what a line, or a run of them, turns into.
//!
//! Line-based and single-pass, with one line of lookahead for setext
//! headings. Anything unrecognised ends up as a paragraph, which is the whole
//! design: Markdown's fallback is its own source text, and that reads.

use crate::inline::inline;
use crate::{Block, MAX_INDENT, MAX_LINE_BYTES};

/// Parse a document into blocks.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Span;

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
