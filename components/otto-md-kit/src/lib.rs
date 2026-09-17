//! Markdown, read as a document.
//!
//! Parses Markdown into the block vocabulary [`otto_kit::preview::document`]
//! draws — never into HTML, and never with a browser engine anywhere near it.
//! Markdown is the one document format the desktop is full of that can be
//! rendered honestly without one: its block structure maps onto shapes the
//! toolkit already draws, so a `.md` file can look like a document rather than
//! like its own source.
//!
//! It is a *reading* parser, not a conforming one. CommonMark has corners
//! (nested blocks inside list items, reference links, HTML blocks, tight vs.
//! loose lists) whose full treatment costs far more than the difference it
//! makes at reading size, and a wrong answer in one of them degrades to a
//! paragraph rather than to nonsense. Two deliberate flattenings:
//!
//! - **A table is drawn as a code block.** Aligning columns means measuring
//!   text, and a parser has no fonts; the pipe-separated source at least lines
//!   up in a monospaced face.
//! - **An image is drawn as its alt text**, marked as a link. The parser is
//!   given text and nothing else — it cannot open the file an image refers to,
//!   and inventing a placeholder box for something that will never load would
//!   be a lie.
//!
//! The crate is deliberately dependency-free beyond the toolkit, and touches
//! no files, no network and no runtime: it takes a `&str` and returns blocks.
//! That is what lets it run inside a sandboxed decode worker with no
//! descriptors, and inside an application's UI thread, unchanged.
//!
//! ```
//! use otto_md_kit::{parse, Block};
//!
//! let blocks = parse("# Title\n\nSome *prose*.\n");
//! assert!(matches!(blocks[0], Block::Heading { level: 1, .. }));
//! ```

mod block;
mod inline;

pub use block::parse;
pub use inline::inline;

/// The toolkit's vocabulary, re-exported so a host that only parses does not
/// also have to name the toolkit.
pub use otto_kit::preview::document::{Block, Span, SpanStyle};

/// A document is a read. Past this many blocks nobody is reading, and the host
/// still has to lay every one of them out.
pub const MAX_BLOCKS: usize = 4_000;
/// How long a line inside a code block or a table may be before it is cut, so
/// one pathological line cannot become one pathological layout.
pub const MAX_LINE_BYTES: usize = 2_000;
/// How deep a nested list is allowed to indent before it stops indenting.
pub const MAX_INDENT: u8 = 6;

/// Parse, and say whether the document was cut short.
///
/// The ceiling belongs here rather than at each call site because truncation
/// is something the reader has to be *told* about: a host that silently drops
/// the tail of a document is lying about what the file contains.
pub fn parse_capped(text: &str, max_blocks: usize) -> (Vec<Block>, bool) {
    let mut blocks = parse(text);
    let truncated = blocks.len() > max_blocks;
    if truncated {
        blocks.truncate(max_blocks);
    }
    (blocks, truncated)
}

/// The text of a Markdown file, or `None` if those bytes are not text at all.
///
/// Three things every host reading a `.md` off disk has to do, in one place:
/// refuse binary, decode leniently, and drop a byte-order mark.
pub fn source_text(bytes: Vec<u8>) -> Option<String> {
    // A NUL says binary whatever the name and the sniffer agreed on.
    if bytes.iter().take(4096).any(|byte| *byte == 0) {
        return None;
    }
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        // Latin-1 maps every byte, so this cannot fail. Markdown that is not
        // UTF-8 is old, not broken.
        Err(err) => err.into_bytes().iter().map(|byte| *byte as char).collect(),
    };
    Some(match text.strip_prefix('\u{feff}') {
        Some(rest) => rest.to_string(),
        None => text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_truncated_document_says_so() {
        let long = "para\n\n".repeat(10);
        let (blocks, truncated) = parse_capped(&long, 3);
        assert_eq!(blocks.len(), 3);
        assert!(truncated);

        let (blocks, truncated) = parse_capped("one\n", MAX_BLOCKS);
        assert_eq!(blocks.len(), 1);
        assert!(!truncated);
    }

    #[test]
    fn binary_is_not_markdown() {
        assert!(source_text(vec![b'#', 0, b'a']).is_none());
        assert_eq!(
            source_text(b"\xef\xbb\xbf# Hi".to_vec()).as_deref(),
            Some("# Hi")
        );
        // Latin-1: not UTF-8, still readable.
        assert_eq!(source_text(vec![0xe9]).as_deref(), Some("é"));
    }
}
