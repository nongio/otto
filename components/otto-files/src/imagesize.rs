//! An image's pixel size from its header, without decoding it.
//!
//! Plain byte arithmetic over the few formats a photo folder is made of —
//! JPEG, PNG, GIF, WebP and BMP — so the Photos view can lay a folder out
//! without handing every file's bytes to a codec in this process. Untrusted
//! bytes are only ever indexed with bounds checks here: a malformed header is
//! an unknown size, never a panic. Anything else is [`Header::Unknown`], and
//! the caller lays it out square.

/// What a prefix of a file says about its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Header {
    /// Width and height as the picture is shown: a JPEG's EXIF orientation is
    /// already applied, so a portrait taken on a phone held upright is tall.
    Size(u32, u32),
    /// A format this reads, whose size is further into the file than the
    /// bytes given. Worth asking again with more.
    NeedMore,
    /// Not a format this reads, or a header that makes no sense. More bytes
    /// would not help.
    Unknown,
}

/// Read the size from the start of an image file.
pub fn parse(bytes: &[u8]) -> Header {
    if bytes.starts_with(&[0xFF, 0xD8]) {
        return jpeg(bytes);
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return png(bytes);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return sized(le16(bytes, 6), le16(bytes, 8));
    }
    if bytes.starts_with(b"RIFF") {
        return webp(bytes);
    }
    if bytes.starts_with(b"BM") {
        return bmp(bytes);
    }
    // Too short to tell what it is yet.
    if bytes.len() < 12 {
        return Header::NeedMore;
    }
    Header::Unknown
}

fn be16(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([b[0], b[1]]) as u32)
}

fn le16(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([b[0], b[1]]) as u32)
}

fn le24(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at.checked_add(3)?)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], 0]))
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn le32(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// A size from two fields that may not have been in the bytes. Missing is
/// [`Header::NeedMore`]; a zero side is [`Header::Unknown`].
fn sized(w: Option<u32>, h: Option<u32>) -> Header {
    match (w, h) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Header::Size(w, h),
        (Some(_), Some(_)) => Header::Unknown,
        _ => Header::NeedMore,
    }
}

/// PNG: the first chunk is IHDR, width and height big-endian after its type.
fn png(bytes: &[u8]) -> Header {
    match bytes.get(12..16) {
        None => Header::NeedMore,
        Some(kind) if kind != b"IHDR" => Header::Unknown,
        Some(_) => sized(be32(bytes, 16), be32(bytes, 20)),
    }
}

/// BMP: the size is in the DIB header, whose own length says which layout
/// it has. A negative height is a top-down bitmap, the same size.
fn bmp(bytes: &[u8]) -> Header {
    match le32(bytes, 14) {
        None => Header::NeedMore,
        Some(12) => sized(le16(bytes, 18), le16(bytes, 20)),
        Some(n) if n >= 40 => sized(
            le32(bytes, 18).map(|w| (w as i32).unsigned_abs()),
            le32(bytes, 22).map(|h| (h as i32).unsigned_abs()),
        ),
        Some(_) => Header::Unknown,
    }
}

/// WebP: a RIFF container whose first chunk is lossy, lossless or extended.
fn webp(bytes: &[u8]) -> Header {
    match bytes.get(8..16) {
        None => Header::NeedMore,
        Some(b) if &b[..4] != b"WEBP" => Header::Unknown,
        Some(b) => match &b[4..] {
            // Lossy: after the three-byte frame tag and the start code, two
            // little-endian 14-bit sides.
            b"VP8 " => match bytes.get(23..26) {
                None => Header::NeedMore,
                Some(code) if code != [0x9D, 0x01, 0x2A] => Header::Unknown,
                Some(_) => sized(
                    le16(bytes, 26).map(|w| w & 0x3FFF),
                    le16(bytes, 28).map(|h| h & 0x3FFF),
                ),
            },
            // Lossless: a signature byte, then both sides less one packed
            // into fourteen bits each.
            b"VP8L" => match (bytes.get(20), le32(bytes, 21)) {
                (Some(0x2F), Some(bits)) => {
                    sized(Some((bits & 0x3FFF) + 1), Some(((bits >> 14) & 0x3FFF) + 1))
                }
                (Some(0x2F), None) | (None, _) => Header::NeedMore,
                _ => Header::Unknown,
            },
            // Extended: the canvas, each side less one in 24 bits.
            b"VP8X" => sized(
                le24(bytes, 24).map(|w| w + 1),
                le24(bytes, 27).map(|h| h + 1),
            ),
            _ => Header::Unknown,
        },
    }
}

/// JPEG: walk the marker segments to the frame header, noting the EXIF
/// orientation on the way — it is in APP1, which comes first.
fn jpeg(bytes: &[u8]) -> Header {
    let mut at = 2;
    let mut quarter_turn = false;
    loop {
        // Markers are 0xFF then a code; any number of 0xFF fill bytes may
        // come first.
        match bytes.get(at) {
            None => return Header::NeedMore,
            Some(0xFF) => {}
            Some(_) => return Header::Unknown,
        }
        while bytes.get(at) == Some(&0xFF) {
            at += 1;
        }
        let Some(&marker) = bytes.get(at) else {
            return Header::NeedMore;
        };
        at += 1;
        match marker {
            // Standalone markers carry no length.
            0x01 | 0xD0..=0xD7 => continue,
            // End of image, or the start of scan data, before any frame
            // header: nothing to find.
            0xD9 | 0xDA => return Header::Unknown,
            _ => {}
        }
        let Some(len) = be16(bytes, at) else {
            return Header::NeedMore;
        };
        let len = len as usize;
        if len < 2 {
            return Header::Unknown;
        }
        let body = at + 2;
        match marker {
            // Every SOFn but DHT (C4), JPG (C8) and DAC (CC).
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                let (h, w) = (be16(bytes, body + 1), be16(bytes, body + 3));
                return match sized(w, h) {
                    Header::Size(w, h) if quarter_turn => Header::Size(h, w),
                    other => other,
                };
            }
            0xE1 => {
                if let Some(segment) = bytes.get(body..at + len) {
                    quarter_turn |= matches!(exif_orientation(segment), Some(5..=8));
                }
            }
            _ => {}
        }
        at += len;
    }
}

/// The Orientation tag from an APP1 segment's body, if it is EXIF and has
/// one. Values 5 to 8 are the ones that turn the picture a quarter.
fn exif_orientation(segment: &[u8]) -> Option<u32> {
    let tiff = segment.strip_prefix(b"Exif\0\0")?;
    let big = match tiff.get(..4)? {
        b"MM\0\x2A" => true,
        b"II\x2A\0" => false,
        _ => return None,
    };
    let u16_at = |at: usize| if big { be16(tiff, at) } else { le16(tiff, at) };
    let u32_at = |at: usize| if big { be32(tiff, at) } else { le32(tiff, at) };
    let ifd = u32_at(4)? as usize;
    let count = u16_at(ifd)? as usize;
    // An IFD claiming more entries than the segment could hold is rubbish;
    // the bound keeps a hostile count from costing anything.
    for entry in 0..count.min(tiff.len() / 12) {
        let at = ifd.checked_add(2 + entry * 12)?;
        if u16_at(at)? == 0x0112 {
            return u16_at(at + 8);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        out.extend_from_slice(&w.to_be_bytes());
        out.extend_from_slice(&h.to_be_bytes());
        out.extend_from_slice(&[8, 6, 0, 0, 0]);
        out
    }

    fn jpeg(w: u16, h: u16, orientation: Option<u16>) -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8];
        // An APP0 to walk past first.
        out.extend_from_slice(&[0xFF, 0xE0, 0, 16]);
        out.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        if let Some(o) = orientation {
            let mut exif = b"Exif\0\0II\x2A\0\x08\0\0\0\x01\0".to_vec();
            exif.extend_from_slice(&[0x12, 0x01, 3, 0, 1, 0, 0, 0]);
            exif.extend_from_slice(&o.to_le_bytes());
            exif.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
            out.extend_from_slice(&[0xFF, 0xE1]);
            out.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
            out.extend_from_slice(&exif);
        }
        out.extend_from_slice(&[0xFF, 0xC2, 0, 11, 8]);
        out.extend_from_slice(&h.to_be_bytes());
        out.extend_from_slice(&w.to_be_bytes());
        out.extend_from_slice(&[1, 1, 0x11, 0]);
        out.extend_from_slice(&[0xFF, 0xDA]);
        out
    }

    fn webp(chunk: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = b"RIFF\0\0\0\0WEBP".to_vec();
        out.extend_from_slice(chunk);
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn every_format_gives_its_size() {
        assert_eq!(parse(&png(640, 480)), Header::Size(640, 480));
        assert_eq!(parse(&jpeg(4032, 3024, None)), Header::Size(4032, 3024));
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&[0x40, 0x01, 0xF0, 0x00]);
        assert_eq!(parse(&gif), Header::Size(320, 240));

        let lossy = webp(
            b"VP8 ",
            &[0, 0, 0, 0x9D, 0x01, 0x2A, 0x80, 0x02, 0xE0, 0x01],
        );
        assert_eq!(parse(&lossy), Header::Size(640, 480));
        let bits: u32 = (640 - 1) | ((480 - 1) << 14);
        let mut body = vec![0x2F];
        body.extend_from_slice(&bits.to_le_bytes());
        assert_eq!(parse(&webp(b"VP8L", &body)), Header::Size(640, 480));
        let mut body = vec![0, 0, 0, 0];
        body.extend_from_slice(&(639u32.to_le_bytes()[..3]));
        body.extend_from_slice(&(479u32.to_le_bytes()[..3]));
        assert_eq!(parse(&webp(b"VP8X", &body)), Header::Size(640, 480));

        let mut bmp = b"BM".to_vec();
        bmp.extend_from_slice(&[0; 12]);
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&64i32.to_le_bytes());
        bmp.extend_from_slice(&(-32i32).to_le_bytes());
        assert_eq!(parse(&bmp), Header::Size(64, 32));
    }

    #[test]
    fn a_jpeg_turned_a_quarter_is_reported_upright() {
        assert_eq!(parse(&jpeg(40, 10, Some(6))), Header::Size(10, 40));
        assert_eq!(parse(&jpeg(40, 10, Some(8))), Header::Size(10, 40));
        assert_eq!(parse(&jpeg(40, 10, Some(3))), Header::Size(40, 10));
        assert_eq!(parse(&jpeg(40, 10, Some(1))), Header::Size(40, 10));
    }

    #[test]
    fn a_truncated_header_asks_for_more() {
        for sample in [png(64, 64), jpeg(64, 64, Some(6))] {
            let full = parse(&sample);
            assert!(matches!(full, Header::Size(..)));
            for cut in 0..sample.len() - 1 {
                let got = parse(&sample[..cut]);
                assert!(
                    matches!(got, Header::NeedMore | Header::Size(..)),
                    "cut at {cut}: {got:?}"
                );
            }
        }
    }

    #[test]
    fn other_formats_and_nonsense_are_unknown() {
        assert_eq!(
            parse(b"<svg xmlns='http://www.w3.org/2000/svg'/>"),
            Header::Unknown
        );
        assert_eq!(parse(b"\0\0\0\x18ftypheic\0\0\0\0"), Header::Unknown);
        assert_eq!(parse(&png(0, 10)), Header::Unknown);
        let mut bad = png(10, 10);
        bad[12..16].copy_from_slice(b"IDAT");
        assert_eq!(parse(&bad), Header::Unknown);
    }

    /// Garbage after every magic number, at every length, must come back as
    /// an answer rather than a panic — and must not loop.
    #[test]
    fn garbage_never_panics() {
        let mut seed: u32 = 0x1234_5678;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as u8
        };
        let magics: [&[u8]; 7] = [
            &[0xFF, 0xD8],
            b"\x89PNG\r\n\x1a\n",
            b"GIF89a",
            b"RIFF\0\0\0\0WEBPVP8 ",
            b"RIFF\0\0\0\0WEBPVP8L",
            b"BM",
            b"",
        ];
        for magic in magics {
            for len in 0..300 {
                let mut bytes = magic.to_vec();
                bytes.extend((0..len).map(|_| next()));
                let _ = parse(&bytes);
                // A JPEG whose segments are all 0xFF fill, or lengths that
                // point past the end.
                let mut fill = magic.to_vec();
                fill.extend(std::iter::repeat_n(0xFF, len));
                let _ = parse(&fill);
            }
        }
        // An EXIF block claiming a huge IFD far past its end.
        let mut hostile = vec![0xFF, 0xD8, 0xFF, 0xE1, 0, 18];
        hostile.extend_from_slice(b"Exif\0\0MM\0\x2A\xFF\xFF\xFF\xF0\xFF\xFF");
        hostile.extend_from_slice(&[0xFF, 0xC0, 0, 11, 8, 0, 5, 0, 7, 1, 1, 0x11, 0]);
        assert_eq!(parse(&hostile), Header::Size(7, 5));
    }
}
