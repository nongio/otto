//! Markdown.
//!
//! Parsing lives in [`otto_md_kit`], which is shared: the previewer is not the
//! only thing on the desktop that wants to read a `.md` as a document. What
//! stays here is what belongs to the worker — the read budget, the refusal to
//! parse something that is not text, and the shape of the answer on the wire.

use std::fs::File;

use crate::payload::{self, PreviewPayload};

use super::{read_capped, Request};

/// Beyond this a document is truncated before it is parsed. The same ceiling
/// as plain text, for the same reason: generated Markdown gets very large.
const MAX_BYTES: u64 = 8 * 1024 * 1024;

pub fn read(file: &mut File, _request: &Request) -> PreviewPayload {
    let bytes = match read_capped(file, MAX_BYTES) {
        Ok(bytes) => bytes,
        Err(err) => {
            return payload::unavailable(otto_kit::t_owned!(
                "peek-error-read-file",
                error = err.to_string()
            ))
        }
    };
    let capped = bytes.len() as u64 >= MAX_BYTES;

    let Some(text) = otto_md_kit::source_text(bytes) else {
        return payload::unavailable(otto_kit::t_owned!("peek-error-not-text"));
    };

    let (blocks, too_many) = otto_md_kit::parse_capped(&text, otto_md_kit::MAX_BLOCKS);
    PreviewPayload::Document {
        blocks,
        truncated: capped || too_many,
    }
}
