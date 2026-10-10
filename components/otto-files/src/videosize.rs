//! A video's size and the day it was recorded, read from an MP4 or QuickTime
//! header: what the Photos view lays a video out by, next to the pictures.
//!
//! Header arithmetic only, like [`crate::imagesize`] and [`crate::camera`]:
//! these are a file's bytes, read in the file manager's own process, so every
//! read is bounds-checked and anything malformed is simply nothing known.
//!
//! The index — the `moov` box — is at the front of a file written for
//! streaming and at the *end* of one a camera or phone wrote as it recorded,
//! so both ends are looked at. Nothing in between is read.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// How much of each end of the file is read looking for the index. A `moov`
/// grows with the length of the recording, a few hundred kilobytes for an
/// hour of phone video; this leaves room for much longer.
const READ_LIMIT: u64 = 4 << 20;

/// Seconds from the QuickTime epoch, 1904-01-01, to the Unix one.
const QUICKTIME_EPOCH: i64 = 2_082_844_800;

/// The widest or tallest picture believed, twice 16K video. A larger side is
/// a damaged header, and laying it out would make a tile nobody can see.
const MAX_SIDE: u32 = 32_768;

/// What the header says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Video {
    /// The picture's size as it is shown: a portrait phone video, stored on
    /// its side with a rotation, comes back tall.
    pub size: Option<(u32, u32)>,
    /// When it was recorded, as Unix seconds. UTC by the format's rules.
    pub recorded: Option<i64>,
}

/// Read the header of the video at `path`. `None` when it is not an MP4 or
/// QuickTime file, or no index could be found.
///
/// **Blocks** on the file. Belongs on a background thread.
pub fn read(path: &Path) -> Option<Video> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut head = Vec::new();
    (&mut file).take(READ_LIMIT).read_to_end(&mut head).ok()?;
    if head.get(4..8) != Some(b"ftyp") {
        return None;
    }
    if let Some(video) = parse(&head) {
        return Some(video);
    }
    if len <= READ_LIMIT {
        return None;
    }
    file.seek(SeekFrom::Start(len - READ_LIMIT)).ok()?;
    let mut tail = Vec::new();
    file.take(READ_LIMIT).read_to_end(&mut tail).ok()?;
    parse(&tail)
}

/// [`read`] over bytes from either end of a file: the first `moov` box found
/// whole in them.
pub fn parse(bytes: &[u8]) -> Option<Video> {
    let moov = find_moov(bytes)?;
    let mut video = Video::default();
    for (kind, body) in boxes(moov) {
        match kind {
            b"mvhd" => video.recorded = recorded(body),
            // The first track with a picture: an audio track's size is zero.
            b"trak" if video.size.is_none() => {
                video.size = boxes(body)
                    .find(|(kind, _)| *kind == b"tkhd")
                    .and_then(|(_, tkhd)| track_size(tkhd));
            }
            _ => {}
        }
    }
    (video.size.is_some() || video.recorded.is_some()).then_some(video)
}

/// The body of a whole `moov` box somewhere in `bytes`.
///
/// Found by its type rather than by walking the boxes before it, because the
/// tail of a file starts in the middle of one. A match is taken only when the
/// size in front of it is a box's and the whole box is in hand.
fn find_moov(bytes: &[u8]) -> Option<&[u8]> {
    let mut from = 4;
    while let Some(found) = bytes.get(from..)?.windows(4).position(|w| w == b"moov") {
        let at = from + found;
        let start = at - 4;
        let size = u32::from_be_bytes(bytes[start..at].try_into().ok()?) as usize;
        if size >= 8 {
            if let Some(body) = bytes.get(at + 4..start + size) {
                return Some(body);
            }
        }
        from = at + 1;
    }
    None
}

/// The boxes laid end to end in `bytes`, as type and body. Stops at the
/// first that does not fit.
fn boxes(bytes: &[u8]) -> impl Iterator<Item = (&[u8; 4], &[u8])> {
    let mut at = 0usize;
    std::iter::from_fn(move || {
        let size = u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize;
        let kind: &[u8; 4] = bytes.get(at + 4..at + 8)?.try_into().ok()?;
        let (header, size) = match size {
            // A 64-bit size follows the type.
            1 => (
                16,
                u64::from_be_bytes(bytes.get(at + 8..at + 16)?.try_into().ok()?) as usize,
            ),
            // To the end of the enclosing box.
            0 => (8, bytes.len() - at),
            size => (8, size),
        };
        if size < header {
            return None;
        }
        let body = bytes.get(at + header..at.checked_add(size)?)?;
        at += size;
        Some((kind, body))
    })
}

/// A track header's picture size, turned the way the track's matrix turns
/// it. `None` for a track with no picture.
fn track_size(tkhd: &[u8]) -> Option<(u32, u32)> {
    // Version and flags, then times and ids whose width depends on the
    // version, then reserved words, layer, group, volume and the matrix.
    let (matrix, size) = match *tkhd.first()? {
        1 => (52, 88),
        _ => (40, 76),
    };
    let word = |at: usize| -> Option<i32> {
        Some(i32::from_be_bytes(tkhd.get(at..at + 4)?.try_into().ok()?))
    };
    // Width and height are unsigned 16.16 fixed point. Read signed, a
    // damaged or hostile header's top bit made them four billion wide.
    let side = |at: usize| -> Option<u32> {
        let fixed = u32::from_be_bytes(tkhd.get(at..at + 4)?.try_into().ok()?);
        Some(fixed >> 16).filter(|side| (1..=MAX_SIDE).contains(side))
    };
    let width = side(size)?;
    let height = side(size + 4)?;
    // A quarter turn either way puts zeros on the matrix's diagonal.
    let turned = word(matrix)? == 0 && word(matrix + 16)? == 0;
    Some(if turned {
        (height, width)
    } else {
        (width, height)
    })
}

/// When the movie header says it was made. A recorder that does not know
/// writes zero.
fn recorded(mvhd: &[u8]) -> Option<i64> {
    let since_1904 = match *mvhd.first()? {
        1 => u64::from_be_bytes(mvhd.get(4..12)?.try_into().ok()?),
        _ => u64::from(u32::from_be_bytes(mvhd.get(4..8)?.try_into().ok()?)),
    };
    let unix = i64::try_from(since_1904).ok()? - QUICKTIME_EPOCH;
    // Before 1990 is a recorder that left the field blank or counted from
    // the wrong epoch, not a recording.
    (unix > 631_152_000).then_some(unix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    fn tkhd(width: u32, height: u32, turned: bool) -> Vec<u8> {
        let mut body = vec![0u8; 84];
        let fixed = |v: i32| v.to_be_bytes();
        let (a, d, b, c) = if turned {
            (0, 0, 1 << 16, -(1 << 16))
        } else {
            (1 << 16, 1 << 16, 0, 0)
        };
        body[40..44].copy_from_slice(&fixed(a));
        body[44..48].copy_from_slice(&fixed(b));
        body[52..56].copy_from_slice(&fixed(c));
        body[56..60].copy_from_slice(&fixed(d));
        body[76..80].copy_from_slice(&(width << 16).to_be_bytes());
        body[80..84].copy_from_slice(&(height << 16).to_be_bytes());
        boxed(b"tkhd", &body)
    }

    fn mvhd(unix: i64) -> Vec<u8> {
        let mut body = vec![0u8; 100];
        body[4..8].copy_from_slice(&((unix + QUICKTIME_EPOCH) as u32).to_be_bytes());
        boxed(b"mvhd", &body)
    }

    fn movie(tracks: &[Vec<u8>], made: i64) -> Vec<u8> {
        let mut moov = mvhd(made);
        for track in tracks {
            moov.extend(boxed(b"trak", track));
        }
        boxed(b"moov", &moov)
    }

    #[test]
    fn a_phone_video_is_as_tall_as_it_was_held() {
        // Audio first, as some recorders write it, then the picture on its
        // side with the quarter turn that stands it up.
        let file = movie(&[tkhd(0, 0, false), tkhd(1920, 1080, true)], 1_732_886_759);
        let video = parse(&file).expect("a header");
        assert_eq!(video.size, Some((1080, 1920)));
        assert_eq!(video.recorded, Some(1_732_886_759));
    }

    /// The tail of a file starts mid-box; the index is found whole after it.
    #[test]
    fn the_index_is_found_at_the_end_of_a_recording() {
        let mut tail = b"...the end of the picture data, moov in passing...".to_vec();
        tail.extend(movie(&[tkhd(1280, 720, false)], 0));
        let video = parse(&tail).expect("a header");
        assert_eq!(video.size, Some((1280, 720)));
        // A recorder that left the date blank.
        assert_eq!(video.recorded, None);
    }

    /// Real files, when there are some: `cargo test -- --ignored`, with
    /// `OTTO_VIDEOS` naming a folder of videos.
    #[test]
    #[ignore]
    fn reads_real_videos() {
        let Some(dir) = std::env::var_os("OTTO_VIDEOS") else {
            return;
        };
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            println!("{}: {:?}", path.display(), read(&path));
        }
    }

    /// A size with its top bit set, which read as a signed number came out
    /// as a negative one cast to four billion, or one past any real video,
    /// is no size at all.
    #[test]
    fn a_negative_or_impossible_size_is_nothing_known() {
        let mut negative = tkhd(1280, 720, false);
        // Width -1.0 in 16.16, the header's offset past the box's own eight.
        negative[8 + 76..8 + 80].copy_from_slice(&(-(1i32 << 16)).to_be_bytes());
        let size = |file: Vec<u8>| parse(&file).and_then(|video| video.size);
        assert_eq!(size(movie(&[negative], 0)), None);
        assert_eq!(size(movie(&[tkhd(40_000, 720, false)], 0)), None);
        // The real track after a bad one is still found.
        assert_eq!(
            size(movie(&[tkhd(0, 720, false), tkhd(1280, 720, false)], 0)),
            Some((1280, 720))
        );
    }

    #[test]
    fn a_cut_off_index_is_nothing_known() {
        let file = movie(&[tkhd(1280, 720, false)], 1_732_886_759);
        assert_eq!(parse(&file[..file.len() - 10]), None);
        assert_eq!(parse(b"not a video"), None);
    }
}
