//! Getting a [`PreviewPayload`] across the process boundary.
//!
//! The payload *types* live in `otto_kit::preview`, with the code that draws
//! them — the drawing side owns the vocabulary, and this module is only the
//! wire. That is why encoding is free functions rather than methods: the types
//! are not ours to hang an impl on, which is the orphan rule usefully pointing
//! at where the boundary is.
//!
//! The format is hand-rolled rather than serde: both ends are this binary, the
//! shapes are fixed and closed, and the project prefers writing a hundred lines
//! to taking a dependency. Every read is bounds-checked, because the parent is
//! parsing bytes produced by a process it expects to sometimes die badly.

use std::io::{self, Write};

pub use otto_kit::preview::{Fact, Pixels, Preview as PreviewPayload, Row};

/// Wire magic. Bumped if the encoding below ever changes shape.
const MAGIC: &[u8; 4] = b"OQV2";

/// Ceiling on any single length field. A corrupt worker must not be able to
/// make the parent allocate a gigabyte because a length byte flipped.
const MAX_LEN: u32 = 512 * 1024 * 1024;

/// Nothing could be shown, and why.
pub fn unavailable(reason: impl Into<String>) -> PreviewPayload {
    PreviewPayload::Unavailable {
        reason: reason.into(),
        // Filled in by [`with_icon`] on the way out, so the twenty-odd places
        // that give up on a file do not each have to know what it looked like.
        icon: Vec::new(),
    }
}

/// Give a payload the file's icon, unless it already brought one.
///
/// Stamped in one place rather than by each decoder because it is the same
/// answer for all of them, and because the interesting case is the payload no
/// decoder produced: a worker that died or overran still comes back as a card
/// with the file's icon on it rather than as a bare line of text.
/// Whether a payload describes a video — one a host with a player can play.
///
/// Read off the icon chain the worker stamped from the type it *sniffed*: a
/// card whose first icon is `video-…` came from the video previewer, and the
/// bytes said so. The name is not consulted, for the same reason dispatch
/// never consults it. A host that can play video checks this rather than the
/// file's extension, so a `.mp4` full of something else is never handed to a
/// demuxer by the previewer.
pub fn is_video(payload: &PreviewPayload) -> bool {
    match payload {
        PreviewPayload::Card { icon, .. } => icon
            .first()
            .is_some_and(|name| name.starts_with("video-") && name != "video-x-generic"),
        _ => false,
    }
}

pub fn with_icon(payload: PreviewPayload, chain: Vec<String>) -> PreviewPayload {
    match payload {
        PreviewPayload::Card {
            title,
            subtitle,
            facts,
            hero,
            icon,
        } => PreviewPayload::Card {
            title,
            subtitle,
            facts,
            hero,
            icon: if icon.is_empty() { chain } else { icon },
        },
        PreviewPayload::Unavailable { reason, icon } => PreviewPayload::Unavailable {
            reason,
            icon: if icon.is_empty() { chain } else { icon },
        },
        // Everything else drew the file itself, and has no room for an icon.
        other => other,
    }
}

/// The icon-theme chain for a file, most specific first.
///
/// The same rule the browser's listing uses, so the picture in the preview is
/// the picture in the row it was opened from.
pub fn icon_names_for(name: &str, is_dir: bool) -> Vec<String> {
    if is_dir {
        return vec!["folder".to_string(), "inode-directory".to_string()];
    }
    match otto_kit::filetype::mime_for_name(name) {
        Some(mime) => otto_kit::filetype::icon_names(mime),
        None => vec![otto_kit::filetype::kind_for_name(name)
            .generic_icon()
            .to_string()],
    }
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_str(out: &mut Vec<u8>, value: &str) {
    put_u32(out, value.len() as u32);
    out.extend_from_slice(value.as_bytes());
}

fn put_strs(out: &mut Vec<u8>, values: &[String]) {
    put_u32(out, values.len() as u32);
    for value in values {
        put_str(out, value);
    }
}

fn put_pixels(out: &mut Vec<u8>, pixels: &Pixels) {
    put_u32(out, pixels.width);
    put_u32(out, pixels.height);
    put_u32(out, pixels.intrinsic_width);
    put_u32(out, pixels.intrinsic_height);
    put_u32(out, pixels.data.len() as u32);
    out.extend_from_slice(&pixels.data);
}

pub fn encode(payload: &PreviewPayload) -> Vec<u8> {
    let mut out = Vec::with_capacity(4096);
    out.extend_from_slice(MAGIC);
    match payload {
        PreviewPayload::Pixels {
            pixels,
            pages,
            page,
        } => {
            out.push(1);
            put_pixels(&mut out, pixels);
            put_u32(&mut out, *pages);
            put_u32(&mut out, *page);
        }
        PreviewPayload::Text {
            lines,
            truncated,
            language,
        } => {
            out.push(2);
            put_u32(&mut out, lines.len() as u32);
            for line in lines {
                put_str(&mut out, line);
            }
            out.push(*truncated as u8);
            put_str(&mut out, language);
        }
        PreviewPayload::Rows {
            rows,
            truncated,
            summary,
        } => {
            out.push(3);
            put_u32(&mut out, rows.len() as u32);
            for row in rows {
                put_str(&mut out, &row.name);
                put_u64(&mut out, row.size);
                put_u64(&mut out, row.mtime as u64);
                put_str(&mut out, &row.icon);
                out.push(row.is_dir as u8);
            }
            out.push(*truncated as u8);
            put_str(&mut out, summary);
        }
        PreviewPayload::Card {
            title,
            subtitle,
            facts,
            hero,
            icon,
        } => {
            out.push(4);
            put_str(&mut out, title);
            put_str(&mut out, subtitle);
            put_u32(&mut out, facts.len() as u32);
            for fact in facts {
                put_str(&mut out, &fact.key);
                put_str(&mut out, &fact.value);
            }
            match hero {
                Some(pixels) => {
                    out.push(1);
                    put_pixels(&mut out, pixels);
                }
                None => out.push(0),
            }
            put_strs(&mut out, icon);
        }
        PreviewPayload::Unavailable { reason, icon } => {
            out.push(5);
            put_str(&mut out, reason);
            put_strs(&mut out, icon);
        }
    }
    out
}

pub fn write_to(payload: &PreviewPayload, sink: &mut impl Write) -> io::Result<()> {
    sink.write_all(&encode(payload))
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// A cursor that refuses to read past the end rather than panicking. The input
/// is the output of a process that may have been killed mid-write.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(count)?;
        let slice = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(slice)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    /// A length field, rejected if it is implausible before it is used to
    /// allocate anything.
    fn len(&mut self) -> Option<usize> {
        let value = self.u32()?;
        (value <= MAX_LEN).then_some(value as usize)
    }

    fn string(&mut self) -> Option<String> {
        let count = self.len()?;
        String::from_utf8(self.take(count)?.to_vec()).ok()
    }

    /// A list of strings. Capped like every other count, so a flipped length
    /// byte cannot make the parent reserve for four billion names.
    fn strings(&mut self) -> Option<Vec<String>> {
        let count = self.len()?;
        let mut values = Vec::with_capacity(count.min(16));
        for _ in 0..count {
            values.push(self.string()?);
        }
        Some(values)
    }

    fn pixels(&mut self) -> Option<Pixels> {
        let width = self.u32()?;
        let height = self.u32()?;
        let intrinsic_width = self.u32()?;
        let intrinsic_height = self.u32()?;
        let count = self.len()?;
        // The buffer must be exactly the size the dimensions imply, or the
        // drawing side would read past the end of it.
        let expected = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        if count != expected {
            return None;
        }
        Some(Pixels {
            width,
            height,
            intrinsic_width,
            intrinsic_height,
            data: self.take(count)?.to_vec(),
        })
    }
}

/// Parse a payload. `None` means the worker produced something malformed,
/// which the caller reports as an unavailable preview — never a panic.
pub fn decode(bytes: &[u8]) -> Option<PreviewPayload> {
    let mut cursor = Cursor { bytes, at: 0 };
    if cursor.take(4)? != MAGIC {
        return None;
    }
    match cursor.u8()? {
        1 => {
            let pixels = cursor.pixels()?;
            Some(PreviewPayload::Pixels {
                pixels,
                pages: cursor.u32()?,
                page: cursor.u32()?,
            })
        }
        2 => {
            let count = cursor.len()?;
            let mut lines = Vec::with_capacity(count.min(4096));
            for _ in 0..count {
                lines.push(cursor.string()?);
            }
            Some(PreviewPayload::Text {
                lines,
                truncated: cursor.u8()? != 0,
                language: cursor.string()?,
            })
        }
        3 => {
            let count = cursor.len()?;
            let mut rows = Vec::with_capacity(count.min(4096));
            for _ in 0..count {
                rows.push(Row {
                    name: cursor.string()?,
                    size: cursor.u64()?,
                    mtime: cursor.u64()? as i64,
                    icon: cursor.string()?,
                    is_dir: cursor.u8()? != 0,
                });
            }
            Some(PreviewPayload::Rows {
                rows,
                truncated: cursor.u8()? != 0,
                summary: cursor.string()?,
            })
        }
        4 => {
            let title = cursor.string()?;
            let subtitle = cursor.string()?;
            let count = cursor.len()?;
            let mut facts = Vec::with_capacity(count.min(256));
            for _ in 0..count {
                facts.push(Fact {
                    key: cursor.string()?,
                    value: cursor.string()?,
                });
            }
            let hero = match cursor.u8()? {
                0 => None,
                _ => Some(cursor.pixels()?),
            };
            Some(PreviewPayload::Card {
                title,
                subtitle,
                facts,
                hero,
                icon: cursor.strings()?,
            })
        }
        5 => {
            let reason = cursor.string()?;
            Some(PreviewPayload::Unavailable {
                reason,
                icon: cursor.strings()?,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_variant() {
        let pixels = Pixels {
            width: 2,
            height: 2,
            intrinsic_width: 8,
            intrinsic_height: 8,
            data: vec![0xAB; 16],
        };
        let cases = vec![
            PreviewPayload::Pixels {
                pixels: pixels.clone(),
                pages: 3,
                page: 2,
            },
            PreviewPayload::Text {
                lines: vec!["fn main() {".into(), "}".into()],
                truncated: true,
                language: "rust".into(),
            },
            PreviewPayload::Rows {
                rows: vec![Row {
                    name: "a.txt".into(),
                    size: 12,
                    mtime: 99,
                    icon: "text-x-generic".into(),
                    is_dir: false,
                }],
                truncated: false,
                summary: "1 item".into(),
            },
            PreviewPayload::Card {
                title: "Song".into(),
                subtitle: "Artist".into(),
                facts: vec![Fact {
                    key: "Duration".into(),
                    value: "3:21".into(),
                }],
                hero: Some(pixels),
                icon: vec!["audio-mpeg".into(), "audio-x-generic".into()],
            },
            unavailable("no decoder"),
            with_icon(unavailable("no decoder"), vec!["video-x-generic".into()]),
        ];
        for case in cases {
            let decoded = decode(&encode(&case)).expect("round trip");
            assert_eq!(format!("{case:?}"), format!("{decoded:?}"));
        }
    }

    #[test]
    fn an_icon_is_stamped_only_when_the_decoder_had_none() {
        let stamped = with_icon(unavailable("no decoder"), vec!["text-x-generic".into()]);
        let PreviewPayload::Unavailable { icon, .. } = &stamped else {
            panic!("still unavailable");
        };
        assert_eq!(icon, &["text-x-generic".to_string()]);

        // A decoder that knew better keeps its answer: it sniffed the bytes,
        // and the stamp is only ever a guess from the name.
        let kept = with_icon(stamped, vec!["image-png".into()]);
        let PreviewPayload::Unavailable { icon, .. } = &kept else {
            panic!("still unavailable");
        };
        assert_eq!(icon, &["text-x-generic".to_string()]);
    }

    #[test]
    fn a_folder_gets_the_folder_icon_whatever_it_is_called() {
        assert_eq!(icon_names_for("Pictures.png", true)[0], "folder");
        // A name with no known type still resolves to something generic
        // rather than to nothing at all.
        assert!(!icon_names_for("mystery", false).is_empty());
    }

    #[test]
    fn rejects_truncated_and_corrupt_input() {
        let good = encode(&unavailable("x"));
        // Every prefix of a valid payload must be rejected, not panic.
        for cut in 0..good.len() {
            assert!(decode(&good[..cut]).is_none());
        }
        assert!(decode(b"XXXX\x01").is_none());
    }

    #[test]
    fn rejects_pixel_buffer_that_contradicts_its_dimensions() {
        // A length that does not match width*height*4 must be refused: the
        // drawing side trusts these dimensions when it wraps the buffer.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.push(1);
        for value in [4u32, 4, 4, 4, 8] {
            put_u32(&mut bytes, value);
        }
        bytes.extend_from_slice(&[0; 8]);
        put_u32(&mut bytes, 1);
        put_u32(&mut bytes, 1);
        assert!(decode(&bytes).is_none());
    }
}
