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
    Some(otto_kit::xdg::cache_home()?.join("thumbnails"))
}

/// A file's canonical URI, as the standard hashes it: GLib's
/// `g_filename_to_uri`, byte for byte. A URI that differs by one escape hashes
/// to a different name and silently misses a cache entry that is right
/// there. See [`otto_kit::uri::path_to_glib_uri`].
pub fn uri_for(path: &Path) -> String {
    otto_kit::uri::path_to_glib_uri(path)
}

/// The file name a thumbnail of `path` has, in any size directory.
pub fn thumbnail_name(path: &Path) -> String {
    format!("{}.png", key_for(path))
}

/// The cache key for `path`: the lowercase hex MD5 of its URI. Shared with
/// every other per-file cache Otto keeps, so one keying rule serves them all.
pub fn key_for(path: &Path) -> String {
    hex(&md5(uri_for(path).as_bytes()))
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

/// The value of a text chunk (`tEXt`, `zTXt` or `iTXt`), by keyword.
///
/// Reads the header chunks only, not the image: the metadata sits before the
/// pixel data, so a lookup that fails the mtime check never pays to
/// decompress anything.
fn png_text(bytes: &[u8], keyword: &str) -> Option<String> {
    let reader = png::Decoder::new(bytes).read_info().ok()?;
    let info = reader.info();
    if let Some(chunk) = info
        .uncompressed_latin1_text
        .iter()
        .find(|chunk| chunk.keyword == keyword)
    {
        return Some(chunk.text.clone());
    }
    if let Some(chunk) = info
        .compressed_latin1_text
        .iter()
        .find(|chunk| chunk.keyword == keyword)
    {
        return chunk.get_text().ok();
    }
    info.utf8_text
        .iter()
        .find(|chunk| chunk.keyword == keyword)
        .and_then(|chunk| chunk.get_text().ok())
}

// ---------------------------------------------------------------------------
// Naming
// ---------------------------------------------------------------------------

/// The MD5 digest of `input`. The cache names files by it; nothing here
/// trusts MD5 to be hard to collide.
fn md5(input: &[u8]) -> [u8; 16] {
    use md5::{Digest, Md5};
    Md5::digest(input).into()
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

    /// Hashes checked against GLib (`GLib.filename_to_uri` then MD5), the
    /// way Nautilus names the same thumbnails. `;` is the one sub-delimiter
    /// GLib escapes.
    #[test]
    fn thumbnail_names_match_glib() {
        assert_eq!(
            key_for(Path::new("/home/user/photo.png")),
            "6a24f7556d0ea4de5b81d0349cef0444"
        );
        assert_eq!(
            key_for(Path::new("/tmp/a b&c;d(1)é.png")),
            "43a0f01a7f0f7ea77e0b145804dbda3a"
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

    /// A one-pixel PNG carrying `pairs` as `tEXt` chunks, so the reader is
    /// tested against bytes rather than against whatever happens to be in the
    /// user's cache.
    fn png_with_text(pairs: &[(&str, &str)]) -> Vec<u8> {
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, 1, 1);
        encoder.set_color(png::ColorType::Grayscale);
        for (keyword, value) in pairs {
            encoder
                .add_text_chunk((*keyword).to_owned(), (*value).to_owned())
                .unwrap();
        }
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[0]).unwrap();
        writer.finish().unwrap();
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
                let Some(path) = otto_kit::uri::uri_to_path(&uri) else {
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

    /// End to end against a real file: take a thumbnail the desktop has
    /// already made, find the file it was made from, and ask [`lookup`] for it
    /// exactly as the browser would — mtime included.
    ///
    /// This is the test that would have caught a validity check comparing the
    /// wrong two things, which unit tests over synthetic PNGs cannot: it uses
    /// the real file's real mtime.
    #[test]
    #[ignore]
    fn serves_a_real_file_from_the_real_cache() {
        let Some(root) = cache_root() else {
            eprintln!("no cache root; skipping");
            return;
        };

        let mut served = 0usize;
        let mut looked_at = 0usize;
        for size in [Size::Large, Size::Normal] {
            let Ok(entries) = std::fs::read_dir(root.join(size.dir_name())) else {
                continue;
            };
            for entry in entries.flatten().take(400) {
                let Ok(bytes) = std::fs::read(entry.path()) else {
                    continue;
                };
                let Some(uri) = png_text(&bytes, "Thumb::URI") else {
                    continue;
                };
                let Some(source) = otto_kit::uri::uri_to_path(&uri) else {
                    continue;
                };
                // Only files still on disk and still unmodified can be
                // expected to resolve.
                let Ok(meta) = std::fs::metadata(&source) else {
                    continue;
                };
                let modified = meta.modified().ok();
                let Some(recorded) = png_text(&bytes, "Thumb::MTime") else {
                    continue;
                };
                let Some(actual) = mtime_secs(modified) else {
                    continue;
                };
                if !same_mtime(&recorded, actual) {
                    continue; // The file moved on; the cache is stale for it.
                }

                looked_at += 1;
                let found = lookup(&source, modified, size);
                assert!(
                    found.is_some(),
                    "cache has a current thumbnail for {} but lookup missed it",
                    source.display()
                );
                let image = found.unwrap();
                assert!(image.width() > 0 && image.height() > 0);
                served += 1;
            }
        }

        eprintln!("served {served} of {looked_at} live files from the shared cache");
        assert!(looked_at > 0, "no live source files to check against");
    }
}
