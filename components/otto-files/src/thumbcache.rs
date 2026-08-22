//! The shared thumbnail cache — freedesktop.org's Thumbnail Managing Standard.
//!
//! Every file manager on the desktop writes its thumbnails to the same place,
//! keyed the same way, so a picture Dolphin or Nautilus has already decoded is
//! one file read away rather than a decode this process has to pay for. On a
//! machine that has been used, that is most of them: the point of reading this
//! cache before scheduling any work of our own is that the first paint of a
//! photo folder costs no decoding at all.
//!
//! The standard is small enough to implement directly, which is why there is
//! no dependency here:
//!
//! * The **name** of a thumbnail is the MD5 of the file's canonical URI —
//!   `file:///home/…`, percent-encoded — in lowercase hex, plus `.png`. The
//!   hash is over the URI text, not over the file's contents, so it can be
//!   computed without opening the file at all.
//! * The **directory** is `$XDG_CACHE_HOME/thumbnails/<size>/`, one per
//!   standard size ([`Size`]).
//! * **Validity** is one comparison: the PNG carries the source's modification
//!   time in a `Thumb::MTime` text chunk, and a thumbnail whose recorded time
//!   disagrees with the file's current one is stale and must be ignored. That
//!   is the whole invalidation protocol — there is no index and nothing to
//!   keep in step.
//! * A decode that **failed** is recorded too, as a marker under
//!   `thumbnails/fail/<application>/`, so a file that cannot be thumbnailed is
//!   not retried on every visit to its folder. The application segment is what
//!   keeps our failures ours: another program's inability to read a format
//!   says nothing about ours, so [`fail_marker`] only ever looks under
//!   [`APPLICATION`].
//!
//! This module only ever *reads* the shared cache. Writing into it is a
//! promise to every other browser on the system that the bytes are correct and
//! correctly sized, and it carries obligations this half does not implement —
//! honouring the opt-outs for removable and remote media among them — so
//! producing thumbnails of our own is deliberately a separate question from
//! consuming what is already there.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use skia_safe as skia;

/// The name this application records its failures under. Only ever used for
/// the `fail/` subdirectory: a marker written by somebody else is about
/// somebody else's decoder.
const APPLICATION: &str = "otto-files";

/// The standard thumbnail sizes, largest first in the order we prefer them.
///
/// Which one to ask for is a question about the box it will be drawn in, not
/// about the file: a 128-pixel thumbnail stretched into a 256-pixel grid cell
/// looks soft, and a 512-pixel one scaled down to a list row's 16 costs
/// memory for detail nobody sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    /// 128×128.
    Normal,
    /// 256×256.
    Large,
    /// 512×512.
    XLarge,
    /// 1024×1024.
    XxLarge,
}

impl Size {
    /// The directory segment the standard gives this size.
    pub fn dir_name(self) -> &'static str {
        match self {
            Size::Normal => "normal",
            Size::Large => "large",
            Size::XLarge => "x-large",
            Size::XxLarge => "xx-large",
        }
    }

    /// The longest edge a thumbnail of this size may have.
    pub fn pixels(self) -> u32 {
        match self {
            Size::Normal => 128,
            Size::Large => 256,
            Size::XLarge => 512,
            Size::XxLarge => 1024,
        }
    }

    /// The smallest standard size that still has detail to spare for a box
    /// `edge` logical pixels across at `scale`.
    ///
    /// Rounds *up*: a thumbnail with more detail than the box needs is only
    /// scaled down, while one with less is visibly soft, so the box's own
    /// pixel size is a floor rather than a target.
    pub fn for_box(edge: f32, scale: f32) -> Size {
        let wanted = (edge * scale).max(0.0) as u32;
        for size in [Size::Normal, Size::Large, Size::XLarge] {
            if wanted <= size.pixels() {
                return size;
            }
        }
        Size::XxLarge
    }

    /// Every size, largest first — the order a lookup falls back through.
    fn descending() -> [Size; 4] {
        [Size::XxLarge, Size::XLarge, Size::Large, Size::Normal]
    }
}

/// Where the shared cache lives, honouring `XDG_CACHE_HOME`.
fn cache_root() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CACHE_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".cache"),
    };
    Some(base.join("thumbnails"))
}

/// A file's canonical URI, as the standard hashes it.
///
/// Percent-encoding follows RFC 3986's unreserved set, with `/` left alone so
/// the path stays a path. This must agree byte for byte with what every other
/// implementation produces — a URI that differs by one escape hashes to a
/// different name and silently misses a cache entry that is right there — so
/// the escaping is spelled out rather than delegated.
pub fn uri_for(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;

    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        match byte {
            // Unreserved, per RFC 3986 §2.3, plus the separators that make a
            // path a path. GLib's `g_filename_to_uri` leaves exactly these.
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                uri.push(byte as char)
            }
            // Sub-delims GLib also passes through unescaped. Kept because a
            // file named `a&b.png` must hash the way the rest of the desktop
            // hashes it, not the way a stricter reading would.
            b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'=' | b':'
            | b'@' => uri.push(byte as char),
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

/// The file name a thumbnail of `path` has, in any size directory.
pub fn thumbnail_name(path: &Path) -> String {
    format!("{}.png", hex(&md5(uri_for(path).as_bytes())))
}

/// Where a thumbnail of `path` would live at `size`. Says nothing about
/// whether it exists.
pub fn thumbnail_path(path: &Path, size: Size) -> Option<PathBuf> {
    Some(
        cache_root()?
            .join(size.dir_name())
            .join(thumbnail_name(path)),
    )
}

/// Whether *this application* has already failed to thumbnail `path`, and the
/// file has not changed since it did.
///
/// A stale marker — one recorded against an older version of the file — is not
/// a refusal: the file has been rewritten, and the new bytes deserve their own
/// attempt.
pub fn is_known_failure(path: &Path, modified: Option<SystemTime>) -> bool {
    let Some(marker) = fail_marker(path) else {
        return false;
    };
    let Ok(bytes) = std::fs::read(&marker) else {
        return false;
    };
    match (png_text(&bytes, "Thumb::MTime"), mtime_secs(modified)) {
        (Some(recorded), Some(actual)) => same_mtime(&recorded, actual),
        // A marker with no recorded time cannot be shown to be stale. Treat it
        // as current: the alternative is re-decoding a known-bad file forever.
        (None, _) => true,
        (_, None) => false,
    }
}

/// Where this application's failure marker for `path` would live.
fn fail_marker(path: &Path) -> Option<PathBuf> {
    Some(
        cache_root()?
            .join("fail")
            .join(APPLICATION)
            .join(thumbnail_name(path)),
    )
}

/// A cached thumbnail for `path`, if the shared cache has a valid one.
///
/// `modified` is the source's modification time, which the caller already has
/// from the directory read — passing it in keeps this off the filesystem for
/// the file itself, so a lookup costs one `read` of a small PNG and nothing
/// more. `None` means "no usable thumbnail", whether because none was ever
/// made, because the one on disk is stale, or because it will not decode.
///
/// Falls back through the sizes at or above the one asked for before settling
/// for a smaller one: a 512 scaled down is better than a 128 scaled up, and
/// either beats decoding the file ourselves.
pub fn lookup(path: &Path, modified: Option<SystemTime>, size: Size) -> Option<skia::Image> {
    let mut smaller: Option<skia::Image> = None;
    for candidate in Size::descending() {
        let Some(image) = read_valid(path, modified, candidate) else {
            continue;
        };
        if candidate.pixels() >= size.pixels() {
            // Enough detail: take the smallest such, which is the last one
            // this loop will see at or above the wanted size.
            smaller = Some(image);
            continue;
        }
        // Below the wanted size — only useful if nothing larger was found.
        return smaller.or(Some(image));
    }
    smaller
}

/// Read one size's thumbnail and check it against the source's mtime.
fn read_valid(path: &Path, modified: Option<SystemTime>, size: Size) -> Option<skia::Image> {
    let file = thumbnail_path(path, size)?;
    let bytes = std::fs::read(&file).ok()?;
    let recorded = png_text(&bytes, "Thumb::MTime")?;
    let actual = mtime_secs(modified)?;
    if !same_mtime(&recorded, actual) {
        return None;
    }
    skia::Image::from_encoded(skia::Data::new_copy(&bytes))
}

/// Seconds since the epoch, which is the unit `Thumb::MTime` is written in.
fn mtime_secs(modified: Option<SystemTime>) -> Option<u64> {
    modified?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// Compare a recorded `Thumb::MTime` against a file's actual one.
///
/// Producers disagree about the format: GNOME writes fractional seconds
/// (`1728803100.344810`), KDE writes whole ones (`1770152422`). Both mean the
/// same instant, so the fraction is dropped before comparing rather than
/// treated as a mismatch — reading the strings as equal-or-not would throw
/// away every GNOME-written thumbnail on the system.
fn same_mtime(recorded: &str, actual: u64) -> bool {
    let whole = recorded.split('.').next().unwrap_or(recorded);
    whole
        .trim()
        .parse::<u64>()
        .map(|t| t == actual)
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// PNG text chunks
// ---------------------------------------------------------------------------

/// The value of a `tEXt`/`iTXt` chunk, by keyword.
///
/// Walks the chunk structure rather than decoding the image: the metadata sits
/// before the pixel data, so a lookup that fails the mtime check never pays to
/// decompress anything. Only the two uncompressed text chunk types are read —
/// `zTXt` would need inflate, and no thumbnailer writes these keys compressed.
fn png_text(bytes: &[u8], keyword: &str) -> Option<String> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < SIGNATURE.len() || bytes[..8] != SIGNATURE {
        return None;
    }

    let mut offset = SIGNATURE.len();
    while offset + 8 <= bytes.len() {
        let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().ok()?) as usize;
        let kind = &bytes[offset + 4..offset + 8];
        let data_start = offset + 8;
        let data_end = data_start.checked_add(length)?;
        if data_end > bytes.len() {
            return None;
        }

        if kind == b"tEXt" || kind == b"iTXt" {
            let data = &bytes[data_start..data_end];
            if let Some(split) = data.iter().position(|&b| b == 0) {
                if data[..split] == *keyword.as_bytes() {
                    let value = &data[split + 1..];
                    // An `iTXt` value carries a compression flag, a
                    // compression method and two more NUL-terminated strings
                    // before the text itself; a `tEXt` value starts straight
                    // away. Skipping to the last NUL-separated field handles
                    // both without branching on the chunk type.
                    let text = if kind == b"iTXt" {
                        value.split(|&b| b == 0).next_back().unwrap_or(value)
                    } else {
                        value
                    };
                    return Some(String::from_utf8_lossy(text).into_owned());
                }
            }
        }

        // Pixel data begins here; every text chunk a thumbnailer writes comes
        // before it, so there is nothing left to find.
        if kind == b"IDAT" {
            return None;
        }

        // Length, type, data, CRC.
        offset = data_end + 4;
    }
    None
}

// ---------------------------------------------------------------------------
// MD5 (RFC 1321)
// ---------------------------------------------------------------------------
//
// Written out rather than pulled in. The cache's naming scheme is fixed for
// all time and this is the only place the workspace needs a digest at all, so
// a dependency would be carried for sixty lines that can never need to change.
// It is a naming scheme, not a security boundary — nothing here is trusting
// MD5 to be hard to collide.

/// Per-round left-rotation amounts.
#[rustfmt::skip]
const SHIFTS: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22,
    5,  9, 14, 20, 5,  9, 14, 20, 5,  9, 14, 20, 5,  9, 14, 20,
    4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23,
    6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// Round constants: `floor(abs(sin(i + 1)) * 2^32)`.
const SINES: [u32; 64] = {
    // Spelled out because `sin` is not available in a const context. These are
    // the values RFC 1321 tabulates.
    [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ]
};

/// The MD5 digest of `input`.
fn md5(input: &[u8]) -> [u8; 16] {
    let mut state: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];

    // Pad to a multiple of 64 bytes: a 1 bit, then zeros, then the original
    // length in bits as a little-endian u64.
    let mut message = input.to_vec();
    let bit_len = (input.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_le_bytes());

    for chunk in message.chunks_exact(64) {
        let mut words = [0u32; 16];
        for (i, word) in words.iter_mut().enumerate() {
            *word = u32::from_le_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }

        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (mixed, index) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let rotated = a
                .wrapping_add(mixed)
                .wrapping_add(SINES[i])
                .wrapping_add(words[index])
                .rotate_left(SHIFTS[i]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(rotated);
        }

        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
    }

    let mut digest = [0u8; 16];
    for (i, word) in state.iter().enumerate() {
        digest[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    digest
}

/// Lowercase hex, which is what the cache's file names are in.
fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 1321's own test suite. If these pass, every name this module
    /// computes agrees with every other implementation's.
    #[test]
    fn md5_matches_the_rfc_vectors() {
        let cases = [
            ("", "d41d8cd98f00b204e9800998ecf8427e"),
            ("a", "0cc175b9c0f1b6a831c399e269772661"),
            ("abc", "900150983cd24fb0d6963f7d28e17f72"),
            ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
            (
                "abcdefghijklmnopqrstuvwxyz",
                "c3fcd3d76192e4007dfb496cca67e13b",
            ),
            (
                "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
                "d174ab98d277d9f5a5611c2c9f419d9f",
            ),
            (
                "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "57edf4a22be3c955ac49da2e2107b67a",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(hex(&md5(input.as_bytes())), expected, "md5({input:?})");
        }
    }

    /// A digest that straddles the padding boundary: 56 bytes is the length at
    /// which the length field no longer fits in the final block and a second
    /// one is needed. Off-by-one padding bugs show up here and nowhere else.
    #[test]
    fn md5_pads_across_a_block_boundary() {
        for len in 54..=66 {
            let input = vec![b'x'; len];
            // Not a known vector — the check is that every length produces a
            // full digest without panicking on the chunking.
            assert_eq!(md5(&input).len(), 16);
        }
        assert_eq!(
            hex(&md5(&vec![b'x'; 56])),
            "668a72d5ba17f08e62dabcafad6db14b"
        );
    }

    #[test]
    fn uri_leaves_a_plain_path_alone() {
        assert_eq!(
            uri_for(Path::new("/home/user/photo.png")),
            "file:///home/user/photo.png"
        );
    }

    #[test]
    fn uri_escapes_spaces_and_non_ascii() {
        assert_eq!(
            uri_for(Path::new("/home/user/My Photos/café.jpg")),
            "file:///home/user/My%20Photos/caf%C3%A9.jpg"
        );
    }

    /// The characters GLib passes through. A file named `a&b(1).png` must
    /// hash the way the rest of the desktop hashes it.
    #[test]
    fn uri_keeps_the_sub_delimiters_unescaped() {
        assert_eq!(
            uri_for(Path::new("/tmp/a&b(1),v=2!.png")),
            "file:///tmp/a&b(1),v=2!.png"
        );
    }

    /// The name is the hash of the URI, so a known URI has a known name.
    /// This is the one value that must never drift: it is the contract with
    /// every other file manager on the system.
    #[test]
    fn thumbnail_name_is_the_md5_of_the_uri() {
        assert_eq!(
            thumbnail_name(Path::new("/home/user/photo.png")),
            format!("{}.png", hex(&md5(b"file:///home/user/photo.png")))
        );
    }

    #[test]
    fn size_rounds_up_to_the_next_standard_size() {
        assert_eq!(Size::for_box(64.0, 1.0), Size::Normal);
        assert_eq!(Size::for_box(128.0, 1.0), Size::Normal);
        // A 128-point cell on a 2x output wants real pixels, not points.
        assert_eq!(Size::for_box(128.0, 2.0), Size::Large);
        assert_eq!(Size::for_box(300.0, 1.0), Size::XLarge);
        assert_eq!(Size::for_box(2000.0, 1.0), Size::XxLarge);
    }

    /// Both spellings producers use for the same instant.
    #[test]
    fn mtime_compares_across_producers() {
        assert!(same_mtime("1728803100", 1728803100));
        assert!(same_mtime("1728803100.344810", 1728803100));
        assert!(!same_mtime("1728803101", 1728803100));
        assert!(!same_mtime("", 1728803100));
        assert!(!same_mtime("not a time", 1728803100));
    }

    /// A minimal PNG carrying one `tEXt` chunk, built by hand so the parser is
    /// tested against bytes rather than against whatever happens to be in the
    /// user's cache.
    fn png_with_text(pairs: &[(&str, &str)]) -> Vec<u8> {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        for (keyword, value) in pairs {
            let mut data = keyword.as_bytes().to_vec();
            data.push(0);
            data.extend_from_slice(value.as_bytes());
            png.extend_from_slice(&(data.len() as u32).to_be_bytes());
            png.extend_from_slice(b"tEXt");
            png.extend_from_slice(&data);
            png.extend_from_slice(&[0, 0, 0, 0]); // CRC, unchecked.
        }
        png.extend_from_slice(&0u32.to_be_bytes());
        png.extend_from_slice(b"IDAT");
        png.extend_from_slice(&[0, 0, 0, 0]);
        png
    }

    #[test]
    fn reads_a_text_chunk_by_keyword() {
        let png = png_with_text(&[
            ("Thumb::URI", "file:///home/user/photo.png"),
            ("Thumb::MTime", "1728803100"),
        ]);
        assert_eq!(
            png_text(&png, "Thumb::MTime").as_deref(),
            Some("1728803100")
        );
        assert_eq!(
            png_text(&png, "Thumb::URI").as_deref(),
            Some("file:///home/user/photo.png")
        );
        assert_eq!(png_text(&png, "Thumb::Size"), None);
    }

    #[test]
    fn rejects_bytes_that_are_not_a_png() {
        assert_eq!(png_text(b"", "Thumb::MTime"), None);
        assert_eq!(png_text(b"not a png at all", "Thumb::MTime"), None);
    }

    /// A truncated chunk length must not read past the buffer.
    #[test]
    fn survives_a_truncated_chunk() {
        let mut png = png_with_text(&[("Thumb::MTime", "1728803100")]);
        png.truncate(20);
        assert_eq!(png_text(&png, "Thumb::MTime"), None);
    }
}

#[cfg(test)]
mod real_cache {
    use super::*;

    /// Check this module's naming against the cache the rest of the desktop
    /// has already written.
    ///
    /// Every thumbnail records the URI it was made from. Feeding that URI's
    /// path back through [`thumbnail_name`] must reproduce the file's own
    /// name — if it does not, our hash or our escaping disagrees with the
    /// producer's, and every lookup silently misses.
    ///
    /// Ignored by default: it reads the invoking user's cache, so it proves
    /// nothing on a machine that has none and belongs to no CI run. Run it
    /// with `cargo test -p otto-files --lib -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn matches_the_real_shared_cache() {
        let Some(root) = cache_root() else {
            eprintln!("no cache root; skipping");
            return;
        };

        let (mut checked, mut matched) = (0usize, 0usize);
        let mut mismatches: Vec<String> = Vec::new();

        for size in Size::descending() {
            let dir = root.join(size.dir_name());
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let file = entry.path();
                if file.extension().and_then(|e| e.to_str()) != Some("png") {
                    continue;
                }
                let Ok(bytes) = std::fs::read(&file) else {
                    continue;
                };
                // Only entries that say what they were made from can be
                // checked; KDE omits the URI on some of its output.
                let Some(uri) = png_text(&bytes, "Thumb::URI") else {
                    continue;
                };
                let Some(path) = path_from_uri(&uri) else {
                    continue;
                };

                checked += 1;
                let ours = thumbnail_name(&path);
                let theirs = file.file_name().unwrap().to_string_lossy().into_owned();
                if ours == theirs {
                    matched += 1;
                } else if mismatches.len() < 10 {
                    mismatches.push(format!("  {uri}\n    ours:   {ours}\n    theirs: {theirs}"));
                }
            }
        }

        eprintln!("checked {checked} real thumbnails, {matched} names agree");
        for line in &mismatches {
            eprintln!("{line}");
        }
        assert!(checked > 0, "no usable entries in {}", root.display());
        assert_eq!(matched, checked, "names disagree with the shared cache");
    }

    /// The inverse of [`uri_for`], for the test's own use: percent-decode a
    /// `file://` URI back to a path.
    fn path_from_uri(uri: &str) -> Option<PathBuf> {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let rest = uri.strip_prefix("file://")?;
        let bytes = rest.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        Some(PathBuf::from(OsString::from_vec(out)))
    }
}
