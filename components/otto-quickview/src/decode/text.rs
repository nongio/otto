//! Text and source code.
//!
//! No syntax highlighting in v1 — see `specs/quickview.md` for why, and for
//! what it will be when it arrives (a hand-written token scanner, not
//! `syntect`). The `language` field is carried on the wire already so that
//! adding it later is not a wire change.
//!
//! This decoder does the encoding work too. The parent must never see bytes it
//! has to interpret: it receives validated UTF-8 lines, bounded in both
//! directions, or nothing.

use std::fs::File;

use crate::payload;
use crate::payload::PreviewPayload;

use super::{read_capped, Request};

/// How much of a file is worth showing. A preview is a look, not a reader —
/// and this bounds what the parent has to lay out.
const MAX_LINES: usize = 4_000;
const MAX_LINE_BYTES: usize = 2_000;
/// A text file larger than this is truncated before it is even decoded. Minified
/// bundles and log files run to hundreds of megabytes.
const MAX_TEXT_BYTES: u64 = 8 * 1024 * 1024;

pub fn read(file: &mut File, request: &Request, mime: &str) -> PreviewPayload {
    let bytes = match read_capped(file, MAX_TEXT_BYTES) {
        Ok(bytes) => bytes,
        Err(err) => {
            return payload::unavailable(otto_kit::t_owned!(
                "quickview-error-read-file",
                error = err.to_string()
            ))
        }
    };
    let read_everything = (bytes.len() as u64) < MAX_TEXT_BYTES;

    let Some(text) = decode_text(&bytes) else {
        return payload::unavailable(otto_kit::t_owned!("quickview-error-not-text"));
    };

    let mut lines: Vec<String> = Vec::new();
    let mut truncated = !read_everything;
    for line in text.lines() {
        if lines.len() >= MAX_LINES {
            truncated = true;
            break;
        }
        lines.push(clamp_line(line));
    }

    PreviewPayload::Text {
        lines,
        truncated,
        language: language_for(mime, &request.name),
    }
}

/// UTF-8, then Latin-1 as the fallback that cannot fail.
///
/// A file that is neither is reported as not-text rather than shown as mojibake:
/// the sniffer already said it was text, so disagreeing loudly is more useful
/// than rendering nonsense.
fn decode_text(bytes: &[u8]) -> Option<String> {
    // A NUL in the first few kilobytes means binary, whatever the type said.
    if bytes.iter().take(4096).any(|byte| *byte == 0) {
        return None;
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => Some(strip_bom(text).to_string()),
        Err(_) => {
            // Latin-1 maps every byte to a codepoint, so this always succeeds.
            // It is the right guess for the old files that are not UTF-8.
            Some(bytes.iter().map(|byte| *byte as char).collect())
        }
    }
}

fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// Keep a pathological line from becoming a pathological layout. Cut on a char
/// boundary — a minified file is one line and slicing it by bytes would panic.
fn clamp_line(line: &str) -> String {
    let line = line.trim_end_matches('\r');
    if line.len() <= MAX_LINE_BYTES {
        return line.to_string();
    }
    let mut end = MAX_LINE_BYTES;
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &line[..end])
}

/// A hint for the highlighter that does not exist yet. Derived from the MIME
/// type where it says something (`text/x-rust`), from the extension otherwise.
fn language_for(mime: &str, name: &str) -> String {
    if let Some(rest) = mime.strip_prefix("text/x-") {
        return rest.to_string();
    }
    if let Some(rest) = mime.strip_prefix("application/x-") {
        return rest.to_string();
    }
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_is_refused_rather_than_shown() {
        assert!(decode_text(b"abc\0def").is_none());
    }

    #[test]
    fn latin1_falls_back_instead_of_failing() {
        // 0xE9 is not valid UTF-8 but is 'é' in Latin-1.
        let decoded = decode_text(&[b'c', b'a', b'f', 0xE9]).expect("latin-1 fallback");
        assert_eq!(decoded, "café");
    }

    #[test]
    fn a_bom_does_not_become_a_visible_character() {
        let decoded = decode_text("\u{feff}hello".as_bytes()).unwrap();
        assert_eq!(decoded, "hello");
    }

    #[test]
    fn a_very_long_line_is_cut_on_a_char_boundary() {
        // Multi-byte characters straddling the cut must not panic.
        let line = "é".repeat(MAX_LINE_BYTES);
        let clamped = clamp_line(&line);
        assert!(clamped.len() <= MAX_LINE_BYTES + 4);
        assert!(clamped.ends_with('…'));
    }

    #[test]
    fn language_comes_from_the_type_then_the_name() {
        assert_eq!(language_for("text/x-rust", "a.rs"), "rust");
        assert_eq!(language_for("text/plain", "notes.MD"), "md");
        assert_eq!(language_for("text/plain", "LICENSE"), "");
    }
}
