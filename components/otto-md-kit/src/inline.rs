//! Inline structure: what a line of prose turns into inside its block.

use std::sync::Arc;

use crate::{Span, SpanStyle};

/// Longest destination kept. A URL this long is not one a person wrote, and
/// the span is carried through a wire format and into a host that may hand it
/// to another program.
const MAX_HREF: usize = 2_000;

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
                href: None,
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
                    href: None,
                });
                at = end + ticks;
                continue;
            }
        }

        // `![alt](src)` and `[text](href)`. An image is its alt text, marked
        // as a link to the file it names — see the crate note.
        if ch == '[' || (ch == '!' && chars.get(at + 1) == Some(&'[')) {
            let open = if ch == '!' { at + 1 } else { at };
            if let Some((label, href, next)) = link_at(&chars, open) {
                push(&mut current, &mut spans, style);
                // The label is itself Markdown, so every run of it becomes a
                // link to the same place — which is why the destination is
                // shared rather than cloned per word.
                let href = href.map(|href| Arc::<str>::from(href.as_str()));
                spans.extend(inline(&label).into_iter().map(|mut span| {
                    span.style.link = true;
                    span.style.bold |= style.bold;
                    span.style.italic |= style.italic;
                    // A code span inside a link keeps its own look and gains
                    // the destination; nothing else nests.
                    span.href = href.clone();
                    span
                }));
                at = next;
                continue;
            }
        }

        // A bare `https://example.com`, with no markup around it at all —
        // how most links are actually written, so it has to be a link here
        // or it is one nowhere.
        if (ch == 'h' || ch == 'w') && autolink_starts_at(&chars, at) {
            if let Some(end) = autolink_end(&chars, at) {
                let body: String = chars[at..end].iter().collect();
                // `www.example.com` names no scheme, so it gets the one a
                // host can act on; the text keeps what was written.
                let target = match body.strip_prefix("www.") {
                    Some(_) => format!("https://{body}"),
                    None => body.clone(),
                };
                if let Some(href) = destination(&target) {
                    push(&mut current, &mut spans, style);
                    spans.push(Span {
                        text: body,
                        style: SpanStyle {
                            link: true,
                            ..style
                        },
                        href: Some(Arc::from(href.as_str())),
                    });
                    at = end;
                    continue;
                }
            }
        }

        // `<https://example.com>` — an autolink, shown as itself.
        if ch == '<' {
            if let Some(end) = chars[at..].iter().position(|c| *c == '>') {
                let body: String = chars[at + 1..at + end].iter().collect();
                if body.starts_with("http://") || body.starts_with("https://") {
                    push(&mut current, &mut spans, style);
                    spans.push(Span {
                        text: body.clone(),
                        style: SpanStyle {
                            link: true,
                            ..style
                        },
                        href: destination(&body).map(Arc::from),
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
fn link_at(chars: &[char], at: usize) -> Option<(String, Option<String>, usize)> {
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
        // An inline destination.
        Some('(') => {
            let end = chars[close..].iter().position(|c| *c == ')')? + close;
            let inside: String = chars[close + 2..end].iter().collect();
            Some((label, destination(&inside), end + 1))
        }
        // `[label][ref]` and `[label]` — the reference definition is not
        // resolved, so the link goes nowhere and says so. The label is what
        // the reader wanted anyway.
        Some('[') => {
            let end = chars[close + 1..].iter().position(|c| *c == ']')? + close + 1;
            Some((label, None, end + 1))
        }
        _ => None,
    }
}

/// Whether a bare link can start at `at`, by what comes before it.
///
/// A URL has to start a word: without this, the tail of `see-https://a` or of
/// a path would each become a link of their own.
fn autolink_starts_at(chars: &[char], at: usize) -> bool {
    let scheme = chars[at..].iter().collect::<String>();
    if !(scheme.starts_with("https://")
        || scheme.starts_with("http://")
        || scheme.starts_with("www."))
    {
        return false;
    }
    match at.checked_sub(1).map(|before| chars[before]) {
        None => true,
        Some(before) => !before.is_alphanumeric() && !matches!(before, '/' | '.' | ':' | '@' | '-'),
    }
}

/// Where the bare link starting at `at` ends.
///
/// It runs to the first space, and then gives back the punctuation that ends
/// the sentence rather than the URL — the trailing full stop of `see
/// https://example.com.`, and a closing bracket that has no opener inside the
/// link, as in `(https://example.com)`.
fn autolink_end(chars: &[char], at: usize) -> Option<usize> {
    let prefix = if chars[at..].starts_with(&['w', 'w', 'w', '.']) {
        "www.".len()
    } else if chars[at..].starts_with(&['h', 't', 't', 'p', 's']) {
        "https://".len()
    } else {
        "http://".len()
    };
    let mut end = at + prefix;
    while end < chars.len() && !chars[end].is_whitespace() && chars[end] != '<' {
        end += 1;
    }
    while end > at + prefix {
        let last = chars[end - 1];
        if matches!(
            last,
            '?' | '!' | '.' | ',' | ':' | ';' | '*' | '_' | '~' | '\'' | '"'
        ) {
            end -= 1;
            continue;
        }
        if last == ')' {
            let opened = chars[at..end].iter().filter(|c| **c == '(').count();
            let closed = chars[at..end].iter().filter(|c| **c == ')').count();
            if closed > opened {
                end -= 1;
                continue;
            }
        }
        break;
    }
    // Nothing but the scheme is not a link, it is the word `https`.
    (end > at + prefix).then_some(end)
}

/// A link destination as the source wrote it, or `None` when there is nothing
/// a host could act on.
///
/// Three things happen here and nothing else: the title is dropped
/// (`[a](url "title")`), `<url>` angle brackets are unwrapped, and anything
/// with whitespace, a control character or absurd length in it is refused.
/// **The scheme is deliberately not judged.** A parser cannot know whether a
/// host is a previewer that opens nothing, a notes window that opens `https`
/// in a browser, or a reader that resolves relative paths against the file —
/// so it hands over what was written and each host decides what it will act
/// on.
fn destination(inside: &str) -> Option<String> {
    let trimmed = inside.trim();
    // A title, if there is one, starts at the first space outside the URL.
    let url = trimmed.split_whitespace().next()?;
    let url = url
        .strip_prefix('<')
        .and_then(|rest| rest.strip_suffix('>'))
        .unwrap_or(url);
    if url.is_empty() || url.len() > MAX_HREF {
        return None;
    }
    // A control character in a URL is either a mistake or an attempt to make
    // one thing look like another in whatever the host hands it to.
    if url.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(spans: &[Span]) -> String {
        spans.iter().map(|span| span.text.as_str()).collect()
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
    fn a_bare_url_is_a_link() {
        let spans = inline("the notes are at https://example.com/a now");
        let link = spans.iter().find(|span| span.style.link).expect("a link");
        assert_eq!(link.text, "https://example.com/a");
        assert_eq!(link.href.as_deref(), Some("https://example.com/a"));
        assert_eq!(
            text_of(&spans),
            "the notes are at https://example.com/a now"
        );
    }

    #[test]
    fn a_bare_url_gives_back_the_punctuation_that_ends_the_sentence() {
        for (line, expected) in [
            ("see https://example.com/a.", "https://example.com/a"),
            ("see (https://example.com/a)", "https://example.com/a"),
            ("see https://example.com/a(b)", "https://example.com/a(b)"),
            ("see https://example.com/a?", "https://example.com/a"),
        ] {
            let spans = inline(line);
            let link = spans.iter().find(|span| span.style.link).expect("a link");
            assert_eq!(link.text, expected, "in {line}");
            assert_eq!(text_of(&spans), line, "in {line}");
        }
    }

    #[test]
    fn a_bare_www_host_gets_a_scheme_to_open() {
        let spans = inline("www.example.com has it");
        let link = spans.iter().find(|span| span.style.link).expect("a link");
        assert_eq!(link.text, "www.example.com");
        assert_eq!(link.href.as_deref(), Some("https://www.example.com"));
    }

    #[test]
    fn a_url_that_is_already_marked_up_is_not_linked_twice() {
        let spans = inline("[the spec](https://example.com/a) and <https://example.com/b>");
        let links: Vec<&Span> = spans.iter().filter(|span| span.style.link).collect();
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].text, "the spec");
        assert_eq!(links[1].text, "https://example.com/b");
    }

    #[test]
    fn a_url_in_a_code_span_stays_text() {
        let spans = inline("run `curl https://example.com/a`");
        assert!(spans.iter().all(|span| !span.style.link));
    }

    #[test]
    fn a_word_that_only_looks_like_a_url_is_not_one() {
        for line in [
            "https:// on its own",
            "say https not http",
            "a-https://x.test",
        ] {
            let spans = inline(line);
            assert_eq!(text_of(&spans), line);
        }
        let spans = inline("https:// on its own");
        assert!(spans.iter().all(|span| !span.style.link));
    }

    #[test]
    fn a_link_keeps_its_label_and_its_destination() {
        let spans = inline("see [the spec](https://example.com/a) now");
        assert_eq!(text_of(&spans), "see the spec now");
        let link = spans.iter().find(|span| span.style.link).expect("a link");
        assert_eq!(link.href.as_deref(), Some("https://example.com/a"));
        // An image is its alt text, pointing at the file it names.
        let spans = inline("![a diagram](d.png)");
        assert_eq!(text_of(&spans), "a diagram");
        assert_eq!(spans[0].href.as_deref(), Some("d.png"));
    }

    #[test]
    fn a_multi_word_link_points_every_word_at_one_place() {
        let spans = inline("[two words here](/a/path)");
        // One destination, shared: wrapping will split this into runs and
        // each of them has to know where it goes.
        assert!(spans
            .iter()
            .all(|span| span.href.as_deref() == Some("/a/path")));
    }

    #[test]
    fn a_link_that_goes_nowhere_says_so() {
        // A reference definition is not resolved, so the label is a link that
        // is honest about having no destination.
        let spans = inline("[label][ref] and [bare]");
        assert!(spans.iter().all(|span| span.href.is_none()));
        let spans = inline("[label][ref]");
        assert!(spans[0].style.link);
    }

    #[test]
    fn a_destination_is_taken_apart_before_it_is_kept() {
        // A title is not part of the URL.
        assert_eq!(
            inline("[a](https://example.com \"Title\")")[0]
                .href
                .as_deref(),
            Some("https://example.com")
        );
        // Angle brackets are punctuation around it, not part of it.
        assert_eq!(
            inline("[a](<https://example.com/b>)")[0].href.as_deref(),
            Some("https://example.com/b")
        );
        // An autolink is its own destination.
        assert_eq!(
            inline("<https://example.com/c>")[0].href.as_deref(),
            Some("https://example.com/c")
        );
        // Nothing to act on: an empty destination, and one with a control
        // character hidden in it.
        assert!(inline("[a]()")[0].href.is_none());
        assert!(inline("[a](htt\u{7}p://x)")[0].href.is_none());
    }

    #[test]
    fn code_and_emphasis_inside_a_link_still_point_at_it() {
        let spans = inline("[`code` and *italics*](https://example.com)");
        assert!(spans.iter().all(|span| span.style.link));
        assert!(spans
            .iter()
            .all(|span| span.href.as_deref() == Some("https://example.com")));
        assert!(spans.iter().any(|span| span.style.code));
        assert!(spans.iter().any(|span| span.style.italic));
    }
}
