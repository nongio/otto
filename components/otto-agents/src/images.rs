//! Pictures agents send, kept as files so they travel as URIs.
//!
//! ACP hands an image over as base64 in the message itself. The Agent Host
//! Protocol carries state, not payloads, so the bytes are written to the cache
//! and the chat gets a `file://` URI instead: the socket stays small, the
//! picture is a file any client can open, and the same picture sent twice is
//! stored once.
//!
//! The cache is disposable by design. It lives under `$XDG_CACHE_HOME`, it is
//! trimmed to [`BUDGET`] with the least recently written files going first, and
//! nothing outside this module is allowed to assume a file is still there — a
//! client that cannot read one says so in the picture's place.

use std::path::{Path, PathBuf};

use base64::Engine as _;

use crate::xdg;

/// How much of the cache is kept, in bytes. Pictures in a conversation are
/// screenshots and diagrams, not a photo library, so this is generous.
const BUDGET: u64 = 128 * 1024 * 1024;

/// The largest picture that is stored, in bytes. Past this the agent is sending
/// something that is not going to be read in a chat card, and decoding it in a
/// client would cost more than it is worth.
const MAX_BYTES: usize = 16 * 1024 * 1024;

/// How much of a picture's label is kept in its file name.
const SLUG_MAX: usize = 40;

/// A picture an agent sent, as a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedImage {
    /// Where the bytes are.
    pub path: PathBuf,
    /// The media type the agent gave, such as `image/png`.
    pub media_type: String,
    /// How large the file is.
    pub bytes: u64,
}

/// The folder pictures are kept in, and the rules for putting one there.
#[derive(Debug, Clone)]
pub struct ImageCache {
    dir: PathBuf,
}

impl ImageCache {
    /// The cache under `$XDG_CACHE_HOME/otto/agents/images`.
    ///
    /// `None` when there is no cache folder to be had, or it cannot be made;
    /// pictures are then dropped rather than kept somewhere unexpected.
    pub fn open() -> Option<Self> {
        let dir = xdg::cache_home()?.join("otto/agents/images");
        Self::at(dir)
    }

    /// The cache in `dir`, made if it is not there yet.
    pub fn at(dir: PathBuf) -> Option<Self> {
        if let Err(err) = std::fs::create_dir_all(&dir) {
            tracing::warn!(dir = %dir.display(), %err, "no folder to keep pictures in");
            return None;
        }
        Some(Self { dir })
    }

    /// Stores the base64 `data` an agent sent as `media_type`, under `label`
    /// when the agent said what the picture is.
    ///
    /// The file is named after its contents, so the same picture sent again in
    /// the same conversation — or replayed from the agent's own history — is
    /// the same file rather than another copy. `label` goes in the name too:
    /// what carries a picture through the Agent Host Protocol is a
    /// [`ContentRef`], which has a URI and a media type and nowhere to put a
    /// caption, so the file name is the only label a client gets.
    ///
    /// [`ContentRef`]: https://github.com/nongio/agent-host-protocol
    pub fn store(&self, data: &str, media_type: &str, label: Option<&str>) -> Option<SharedImage> {
        let extension = extension(media_type)?;
        let bytes = decode(data)?;
        if bytes.len() > MAX_BYTES {
            tracing::warn!(
                media_type,
                bytes = bytes.len(),
                "a picture too large to keep"
            );
            return None;
        }
        let name = match label.map(slug).filter(|slug| !slug.is_empty()) {
            Some(slug) => format!("{slug}-{:016x}.{extension}", hash(&bytes)),
            None => format!("image-{:016x}.{extension}", hash(&bytes)),
        };
        let path = self.dir.join(name);
        if !path.exists() {
            if let Err(err) = write_atomically(&path, &bytes) {
                tracing::warn!(path = %path.display(), %err, "could not keep a picture");
                return None;
            }
            self.trim();
        }
        Some(SharedImage {
            path,
            media_type: media_type.to_owned(),
            bytes: bytes.len() as u64,
        })
    }

    /// Takes a picture that is already a file — a resource link to one — as it
    /// is, without copying it into the cache.
    ///
    /// The agent named a file on this machine, so the chat can point at it
    /// directly. Only files that exist are taken: a link to something that was
    /// never written would show as a picture that never loads.
    pub fn adopt(path: PathBuf, media_type: Option<&str>) -> Option<SharedImage> {
        let media_type = match media_type {
            Some(media_type) if extension(media_type).is_some() => media_type.to_owned(),
            Some(_) => return None,
            None => media_type_of(&path)?.to_owned(),
        };
        let bytes = std::fs::metadata(&path)
            .ok()
            .filter(|meta| meta.is_file())?
            .len();
        Some(SharedImage {
            path,
            media_type,
            bytes,
        })
    }

    /// Brings the folder back under [`BUDGET`], dropping the files written
    /// longest ago first. A file that cannot be read or removed is left alone.
    fn trim(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
            .flatten()
            .filter_map(|entry| {
                let meta = entry.metadata().ok()?;
                if !meta.is_file() {
                    return None;
                }
                let written = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                Some((written, meta.len(), entry.path()))
            })
            .collect();
        let mut total: u64 = files.iter().map(|(_, bytes, _)| bytes).sum();
        if total <= BUDGET {
            return;
        }
        // Oldest first, so what is dropped is what the conversation has
        // scrolled past.
        files.sort_by_key(|(written, _, _)| *written);
        for (_, bytes, path) in files {
            if total <= BUDGET {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(bytes);
            }
        }
        tracing::debug!(dir = %self.dir.display(), kept = total, "trimmed the picture cache");
    }
}

/// The media type a picture's file name implies, for a file the agent linked to
/// without saying. `None` for anything that is not a picture Otto can draw.
pub fn media_type_of(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        "heic" | "heif" => "image/heif",
        _ => return None,
    })
}

/// The file extension for `media_type`, and the test of whether Otto takes it
/// at all.
///
/// SVG is deliberately absent: what reads these files decodes bitmaps, and a
/// picture that never appears is worse than its name in its place.
fn extension(media_type: &str) -> Option<&'static str> {
    let media_type = media_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    Some(match media_type.as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" | "image/x-bmp" => "bmp",
        "image/avif" => "avif",
        "image/heif" | "image/heic" => "heif",
        _ => return None,
    })
}

/// `data` decoded, taking either base64 alphabet and tolerating missing
/// padding: agents differ, and the picture is the same either way.
fn decode(data: &str) -> Option<Vec<u8>> {
    let data = data.trim();
    let engines = [
        base64::engine::general_purpose::STANDARD,
        base64::engine::general_purpose::STANDARD_NO_PAD,
        base64::engine::general_purpose::URL_SAFE,
        base64::engine::general_purpose::URL_SAFE_NO_PAD,
    ];
    for engine in engines {
        if let Ok(bytes) = engine.decode(data) {
            return Some(bytes);
        }
    }
    tracing::warn!(len = data.len(), "a picture that is not base64");
    None
}

/// `label` reduced to something safe in a file name: lower case, words joined
/// by dashes, at most [`SLUG_MAX`] characters, and no directory separators.
fn slug(label: &str) -> String {
    let mut slug = String::new();
    for character in label.chars() {
        if slug.chars().count() >= SLUG_MAX {
            break;
        }
        match character {
            'a'..='z' | '0'..='9' => slug.push(character),
            'A'..='Z' => slug.extend(character.to_lowercase()),
            _ if slug.ends_with('-') || slug.is_empty() => {}
            _ => slug.push('-'),
        }
    }
    slug.trim_matches('-').to_owned()
}

/// FNV-1a over the bytes, to name the file after its contents.
///
/// Not a checksum anyone relies on: the file name carries the length too, so
/// two different pictures would have to collide on both to be confused.
fn hash(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Writes `bytes` to `path` through a temporary file beside it, so a reader
/// never sees half a picture.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let temporary = path.with_extension("part");
    std::fs::write(&temporary, bytes)?;
    match std::fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = std::fs::remove_file(&temporary);
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-pixel PNG, as an agent would send it.
    const PIXEL: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==";

    fn cache() -> (tempfile::TempDir, ImageCache) {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let cache = ImageCache::at(dir.path().join("images")).expect("a cache");
        (dir, cache)
    }

    #[test]
    fn a_picture_is_kept_as_a_file_named_after_its_contents() {
        let (_dir, cache) = cache();
        let first = cache.store(PIXEL, "image/png", None).expect("stored");
        let again = cache.store(PIXEL, "image/png", None).expect("stored");
        assert_eq!(first.path, again.path, "the same picture is one file");
        assert_eq!(first.path.extension().unwrap(), "png");
        assert!(first.path.is_file());
        assert_eq!(first.bytes, std::fs::metadata(&first.path).unwrap().len());
    }

    /// The file name is the only label a `contentRef` carries, so what the
    /// agent called the picture has to survive into it.
    #[test]
    fn the_label_the_agent_gave_becomes_the_file_name() {
        let (_dir, cache) = cache();
        let named = cache
            .store(PIXEL, "image/png", Some("Mock Up v2.PNG"))
            .expect("stored");
        let name = named.path.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("mock-up-v2-png-"), "{name}");
        assert!(name.ends_with(".png"), "{name}");

        let unnamed = cache.store(PIXEL, "image/png", None).expect("stored");
        let name = unnamed.path.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("image-"), "{name}");

        // A label that reduces to nothing, or that names another folder, must
        // not escape the cache.
        for label in ["///", "../../etc/passwd", "   "] {
            let escaped = cache
                .store(PIXEL, "image/png", Some(label))
                .expect("stored");
            assert_eq!(
                escaped.path.parent(),
                unnamed.path.parent(),
                "{label} left the cache",
            );
        }
    }

    #[test]
    fn a_media_type_otto_cannot_draw_is_refused() {
        let (_dir, cache) = cache();
        assert!(cache.store(PIXEL, "image/svg+xml", None).is_none());
        assert!(cache.store(PIXEL, "application/pdf", None).is_none());
        // A parameter after the type is still the type.
        assert!(
            cache
                .store(PIXEL, "image/png; charset=binary", None)
                .is_some()
        );
    }

    #[test]
    fn data_that_is_not_base64_is_dropped_rather_than_written() {
        let (_dir, cache) = cache();
        assert!(
            cache
                .store("not base64 at all !!", "image/png", None)
                .is_none()
        );
    }

    #[test]
    fn a_linked_file_is_taken_where_it_lies() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let path = dir.path().join("shot.png");
        std::fs::write(&path, b"pretend png").expect("written");
        let image = ImageCache::adopt(path.clone(), None).expect("adopted");
        assert_eq!(image.path, path, "a linked file is not copied");
        assert_eq!(image.media_type, "image/png");
        assert_eq!(image.bytes, 11);
        assert!(ImageCache::adopt(dir.path().join("gone.png"), None).is_none());
        assert!(
            ImageCache::adopt(path, Some("text/plain")).is_none(),
            "a link that says it is not a picture is not one"
        );
    }
}
