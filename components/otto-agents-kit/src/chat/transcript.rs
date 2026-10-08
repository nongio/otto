//! What a conversation is made of, as the log draws it: what the agent said,
//! the pictures it sent and what went with a request.
//!
//! Kept apart from [`crate::chat`], which talks to the service, so the log can
//! be laid out from a conversation without the connection that carries it.

use std::path::PathBuf;

use otto_agents_client::uri::to_path as path_from_uri;

pub use otto_kit::components::attachments::Attachment;

/// A piece of what the agent answered.
///
/// An answer is mostly Markdown, and the pieces of it that arrive one after
/// another read as one document. A picture breaks it in two: what was said
/// before it, the picture, then the rest — so a diagram sits where the agent put
/// it rather than at the end.
#[derive(Clone, Debug, PartialEq)]
pub enum Said {
    /// Markdown, as [`otto_md_kit`] reads it.
    Text(String),
    /// A picture the agent sent, as the file the service keeps it in.
    Image(Picture),
}

/// A picture in an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub path: PathBuf,
    /// What it is called, for a screen reader and for when it cannot be drawn:
    /// the file's name without the digest the service appends to it.
    pub label: String,
}

impl Picture {
    /// The picture at `uri`, when it is a local file the launcher can read.
    ///
    /// The service writes pictures under its own cache and names them
    /// `<label>-<digest>.<extension>`; the digest is how the same picture stays
    /// one file, and is not something to show anyone.
    pub(crate) fn at(uri: &str, content_type: Option<&str>) -> Option<Self> {
        let path = path_from_uri(uri)?;
        if !content_type.is_none_or(|kind| kind.starts_with("image/")) {
            return None;
        }
        let stem = path.file_stem()?.to_string_lossy();
        let label = match stem.rsplit_once('-') {
            Some((label, digest)) if is_digest(digest) && !label.is_empty() => {
                label.replace('-', " ")
            }
            _ => stem.into_owned(),
        };
        Some(Self { path, label })
    }
}

/// Whether `text` is the hexadecimal digest the service appends to a picture's
/// name, rather than part of what the picture is called.
fn is_digest(text: &str) -> bool {
    text.len() == 16 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}
