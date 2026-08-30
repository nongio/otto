//! Audio, video, and everything else — described rather than rendered.
//!
//! v1 deliberately gets most of the value here without a decoder: tags and
//! embedded cover art need no PCM decode, and container headers give dimensions
//! and duration without reading a 2 GB file. Playback and poster frames are
//! later stages; see `specs/quickview.md`.
//!
//! The hard rule in this file is that **nothing reads the whole file**. A
//! metadata previewer that streams two gigabytes to find a duration has missed
//! the point of being a previewer.

use std::fs::{File, Metadata};
use std::io::{Read, Seek, SeekFrom};

use otto_kit::filetype;
use skia_safe::{Codec, Data};

use crate::payload::{Fact, Pixels, PreviewPayload};

use super::{human_size, Request};

/// How much of a media file the metadata readers may look at. ID3 and MP4
/// headers live at one end or the other; nothing here needs the middle.
const HEADER_BYTES: u64 = 1024 * 1024;

/// The last resort: a file we can name and size but not interpret.
pub fn generic(metadata: &Metadata, request: &Request, mime: &str) -> PreviewPayload {
    PreviewPayload::Card {
        title: request.name.clone(),
        subtitle: filetype::kind_of(mime).label().to_string(),
        facts: vec![
            Fact {
                key: otto_kit::t_owned!("quickview-fact-kind"),
                value: describe(mime),
            },
            Fact {
                key: otto_kit::t_owned!("quickview-fact-size"),
                value: human_size(metadata.len()),
            },
        ],
        hero: None,
        // Stamped by `decode`, which is where the sniffed type is known.
        icon: Vec::new(),
    }
}

pub fn audio(
    file: &mut File,
    metadata: &Metadata,
    request: &Request,
    mime: &str,
) -> PreviewPayload {
    let head = read_head(file);
    let tags = id3(&head);

    let mut facts = Vec::new();
    for (key, value) in [
        ("quickview-fact-artist", tags.artist.as_deref()),
        ("quickview-fact-album", tags.album.as_deref()),
        ("quickview-fact-year", tags.year.as_deref()),
    ] {
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            facts.push(Fact {
                key: otto_kit::t_owned!(key),
                value: value.to_string(),
            });
        }
    }
    facts.push(Fact {
        key: otto_kit::t_owned!("quickview-fact-kind"),
        value: describe(mime),
    });
    facts.push(Fact {
        key: otto_kit::t_owned!("quickview-fact-size"),
        value: human_size(metadata.len()),
    });

    PreviewPayload::Card {
        title: tags.title.unwrap_or_else(|| request.name.clone()),
        subtitle: tags
            .artist
            .clone()
            .unwrap_or_else(|| filetype::kind_of(mime).label().to_string()),
        facts,
        // Cover art is an embedded JPEG or PNG, which Skia decodes like any
        // other image — no audio decoder involved.
        hero: tags.cover.as_deref().and_then(decode_cover),
        // Stamped by `decode`, which is where the sniffed type is known.
        icon: Vec::new(),
    }
}

pub fn video(
    file: &mut File,
    metadata: &Metadata,
    request: &Request,
    mime: &str,
) -> PreviewPayload {
    let head = read_head(file);
    let mut facts = Vec::new();

    if let Some((width, height)) = mp4_dimensions(&head) {
        facts.push(Fact {
            key: otto_kit::t_owned!("quickview-fact-dimensions"),
            value: format!("{width} × {height}"),
        });
    }
    if let Some(seconds) = mp4_duration(&head) {
        facts.push(Fact {
            key: otto_kit::t_owned!("quickview-fact-duration"),
            value: clock(seconds),
        });
    }
    facts.push(Fact {
        key: otto_kit::t_owned!("quickview-fact-kind"),
        value: describe(mime),
    });
    facts.push(Fact {
        key: otto_kit::t_owned!("quickview-fact-size"),
        value: human_size(metadata.len()),
    });

    PreviewPayload::Card {
        title: request.name.clone(),
        subtitle: filetype::kind_of(mime).label().to_string(),
        facts,
        hero: None,
        // Stamped by `decode`, which is where the sniffed type is known.
        icon: Vec::new(),
    }
}

/// Read the front of the file, and rewind so nothing downstream is surprised.
fn read_head(file: &mut File) -> Vec<u8> {
    let mut head = Vec::new();
    let _ = file.seek(SeekFrom::Start(0));
    let _ = file.take(HEADER_BYTES).read_to_end(&mut head);
    let _ = file.seek(SeekFrom::Start(0));
    head
}

fn describe(mime: &str) -> String {
    // `image/jpeg` reads better as "JPEG" than as its full type.
    mime.rsplit('/')
        .next()
        .map(|subtype| subtype.trim_start_matches("x-").to_ascii_uppercase())
        .unwrap_or_else(|| mime.to_string())
}

fn clock(seconds: u64) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

// ---------------------------------------------------------------------------
// ID3v2
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Tags {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    year: Option<String>,
    cover: Option<Vec<u8>>,
}

/// A deliberately small ID3v2.3/2.4 reader: the four text frames worth showing
/// and the attached picture. Unsynchronisation, compression and encryption are
/// not handled — a frame we do not understand is skipped, not guessed at.
fn id3(bytes: &[u8]) -> Tags {
    let mut tags = Tags::default();
    if bytes.len() < 10 || &bytes[0..3] != b"ID3" {
        return tags;
    }
    // A syncsafe integer: seven bits per byte, so the size can never contain a
    // byte that looks like a frame sync.
    let size = syncsafe(&bytes[6..10]) as usize;
    let end = (10 + size).min(bytes.len());
    let mut at = 10usize;

    while at + 10 <= end {
        let id = &bytes[at..at + 4];
        if id == b"\0\0\0\0" {
            break;
        }
        // 2.4 uses syncsafe frame sizes, 2.3 plain ones. Reading it as plain
        // and sanity-checking is enough for the frames we want.
        let frame_size =
            u32::from_be_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]])
                as usize;
        let body_at = at + 10;
        if frame_size == 0 || body_at + frame_size > end {
            break;
        }
        let body = &bytes[body_at..body_at + frame_size];

        match id {
            b"TIT2" => tags.title = text_frame(body),
            b"TPE1" => tags.artist = text_frame(body),
            b"TALB" => tags.album = text_frame(body),
            b"TYER" | b"TDRC" => tags.year = text_frame(body),
            b"APIC" => tags.cover = picture_frame(body),
            _ => {}
        }
        at = body_at + frame_size;
    }
    tags
}

fn syncsafe(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .take(4)
        .fold(0u32, |total, byte| (total << 7) | (*byte as u32 & 0x7F))
}

/// A text frame: one encoding byte, then the string.
fn text_frame(body: &[u8]) -> Option<String> {
    let (encoding, rest) = body.split_first()?;
    let text = match encoding {
        // ISO-8859-1 and UTF-8 both decode bytewise for our purposes; the
        // lossy path is correct for the former and exact for the latter.
        0 => rest.iter().map(|byte| *byte as char).collect(),
        3 => String::from_utf8_lossy(rest).into_owned(),
        // UTF-16, with or without a BOM.
        1 | 2 => utf16(rest)?,
        _ => return None,
    };
    let text = text.trim_end_matches('\0').trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn utf16(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 2 {
        return None;
    }
    let (little_endian, body) = match (bytes[0], bytes[1]) {
        (0xFF, 0xFE) => (true, &bytes[2..]),
        (0xFE, 0xFF) => (false, &bytes[2..]),
        _ => (true, bytes),
    };
    let units: Vec<u16> = body
        .chunks_exact(2)
        .map(|pair| {
            if little_endian {
                u16::from_le_bytes([pair[0], pair[1]])
            } else {
                u16::from_be_bytes([pair[0], pair[1]])
            }
        })
        .collect();
    String::from_utf16(&units).ok()
}

/// An attached picture: encoding byte, MIME string, picture type, description,
/// then the image itself.
fn picture_frame(body: &[u8]) -> Option<Vec<u8>> {
    let (encoding, rest) = body.split_first()?;
    let mime_end = rest.iter().position(|byte| *byte == 0)?;
    let after_mime = rest.get(mime_end + 1..)?;
    let after_type = after_mime.get(1..)?;
    // The description's terminator is one NUL for the byte encodings and two
    // for the UTF-16 ones.
    let description_end = if matches!(encoding, 1 | 2) {
        after_type
            .chunks_exact(2)
            .position(|pair| pair == [0, 0])
            .map(|index| index * 2 + 2)?
    } else {
        after_type.iter().position(|byte| *byte == 0)? + 1
    };
    let image = after_type.get(description_end..)?;
    (!image.is_empty()).then(|| image.to_vec())
}

/// Cover art, decoded small: it is shown at card size and nothing is gained by
/// holding a full-resolution copy.
fn decode_cover(bytes: &[u8]) -> Option<Pixels> {
    let mut codec = Codec::from_data(Data::new_copy(bytes))?;
    let intrinsic = codec.dimensions();
    if intrinsic.width <= 0 || intrinsic.height <= 0 {
        return None;
    }
    let scale = (512.0 / intrinsic.width as f32).min(1.0);
    let scaled = if scale >= 1.0 {
        intrinsic
    } else {
        codec.get_scaled_dimensions(scale)
    };
    let info = codec
        .info()
        .with_dimensions(scaled)
        .with_color_type(skia_safe::ColorType::RGBA8888)
        .with_alpha_type(skia_safe::AlphaType::Premul);
    let image = codec.get_image(info, None).ok()?;
    super::image::to_pixels(&image, intrinsic)
}

// ---------------------------------------------------------------------------
// MP4 / QuickTime
// ---------------------------------------------------------------------------

/// Walk the top-level atoms for `moov`, then find the movie header inside it.
///
/// Atom walking is a length and a four-character code; it needs no library, and
/// it never reads past the header region we already have.
fn find_atom(bytes: &[u8], want: &[u8; 4]) -> Option<(usize, usize)> {
    let mut at = 0usize;
    while at + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize;
        let kind = bytes.get(at + 4..at + 8)?;
        let body = at + 8;
        if kind == want {
            let end = if size == 0 { bytes.len() } else { at + size };
            return Some((body, end.min(bytes.len())));
        }
        // A size of 0 means "to end of file"; 1 means a 64-bit size follows.
        // Neither is worth chasing in a header scan.
        if size < 8 {
            return None;
        }
        at += size;
    }
    None
}

/// Duration in seconds, from the movie header's timescale and duration.
fn mp4_duration(bytes: &[u8]) -> Option<u64> {
    let (moov, end) = find_atom(bytes, b"moov")?;
    let (mvhd, _) = find_atom(bytes.get(moov..end)?, b"mvhd")?;
    let header = bytes.get(moov + mvhd..)?;
    let version = *header.first()?;
    // Version 1 uses 64-bit creation/modification times, moving the fields on.
    let (timescale_at, duration_at) = if version == 1 { (20, 24) } else { (12, 16) };
    let timescale = u32::from_be_bytes(
        header
            .get(timescale_at..timescale_at + 4)?
            .try_into()
            .ok()?,
    );
    if timescale == 0 {
        return None;
    }
    let duration = if version == 1 {
        u64::from_be_bytes(header.get(duration_at..duration_at + 8)?.try_into().ok()?)
    } else {
        u32::from_be_bytes(header.get(duration_at..duration_at + 4)?.try_into().ok()?) as u64
    };
    Some(duration / timescale as u64)
}

/// Dimensions from the track header's width/height, which are 16.16 fixed
/// point at the end of the atom.
fn mp4_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let (moov, moov_end) = find_atom(bytes, b"moov")?;
    let region = bytes.get(moov..moov_end)?;
    let (trak, trak_end) = find_atom(region, b"trak")?;
    let (tkhd, _) = find_atom(region.get(trak..trak_end)?, b"tkhd")?;
    let header = region.get(trak + tkhd..)?;
    let version = *header.first()?;
    let size_at = if version == 1 { 96 } else { 84 };
    let width = u32::from_be_bytes(header.get(size_at..size_at + 4)?.try_into().ok()?) >> 16;
    let height = u32::from_be_bytes(header.get(size_at + 4..size_at + 8)?.try_into().ok()?) >> 16;
    (width > 0 && height > 0).then_some((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_reads_as_a_duration() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(61), "1:01");
        assert_eq!(clock(3_661), "1:01:01");
    }

    #[test]
    fn describe_shortens_a_mime_type() {
        assert_eq!(describe("image/jpeg"), "JPEG");
        assert_eq!(describe("application/x-tar"), "TAR");
    }

    #[test]
    fn syncsafe_ignores_the_high_bit() {
        assert_eq!(syncsafe(&[0, 0, 2, 1]), 257);
    }

    #[test]
    fn a_text_frame_decodes_in_each_encoding() {
        assert_eq!(text_frame(&[0, b'h', b'i']).as_deref(), Some("hi"));
        assert_eq!(text_frame(&[3, b'h', b'i']).as_deref(), Some("hi"));
        let utf16_le = [1u8, 0xFF, 0xFE, b'h', 0, b'i', 0];
        assert_eq!(text_frame(&utf16_le).as_deref(), Some("hi"));
    }

    #[test]
    fn tags_are_empty_rather_than_wrong_for_a_file_with_no_id3() {
        let tags = id3(b"not an mp3 at all");
        assert!(tags.title.is_none() && tags.cover.is_none());
    }

    #[test]
    fn atom_walking_stops_rather_than_looping_on_a_bad_size() {
        // A zero-length atom must not spin forever.
        let bytes = [0, 0, 0, 0, b'm', b'o', b'o', b'v'];
        assert!(find_atom(&bytes, b"trak").is_none());
    }
}
