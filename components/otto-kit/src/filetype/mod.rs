//! File-type detection: what kind of thing is this file?
//!
//! One implementation, shared by the file picker (portal filters), the file
//! browser (icons and the Kind column) and quick view (which renderer to
//! dispatch). See `specs/file-browser.md` under *Shared foundations*.
//!
//! It answers **two different questions**, and the distinction is load-bearing:
//!
//! - [`mime_for_name`] — the type of record, decided by the file's name. This
//!   is what drives the icon, the Kind column, portal filters and default
//!   application association. Stable, cheap, no I/O.
//! - [`sniff`] — what the content actually looks like, from its leading bytes.
//!
//! **Display follows the name; decoding follows the content.** A consumer about
//! to parse a file must dispatch its decoder on [`sniff`], never on the name, so
//! a `.png` that is not one never reaches the PNG decoder. But content must not
//! override the name for display, or an empty `.rs` file's icon flips the moment
//! a sniff comes back inconclusive — a bug the user sees. They cannot disagree,
//! because they were never asked the same thing.
//!
//! The one qualification, and it is not a loophole: a decoder should dispatch on
//! [`refine`], not on raw [`sniff`]. Container formats make the literal rule
//! wrong — an SVG is XML, so `sniff` says `application/xml` and a previewer
//! obeying the rule to the letter draws every icon as source code. [`refine`]
//! lets the name pick a *subtype of what the content already proved*, which
//! fixes that without letting a name redirect anything.
//!
//! Nothing here touches `AppContext`, `wayland-client` or a runtime, so the
//! compositor can call it from a bare draw closure.

mod db;
pub mod glob;

use std::sync::OnceLock;

pub use db::MimeDb;

/// A lossy grouping over MIME types, for the places a full type would be
/// noise: the Kind column, the icon fallback chain, whether a thumbnail is
/// worth attempting, and which previewer to reach for.
///
/// Consumers needing precision use the MIME type itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Folder,
    Image,
    Video,
    Audio,
    Text,
    Document,
    Archive,
    Application,
    Other,
}

impl Kind {
    /// A human-readable name, for the Kind column.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Folder => "Folder",
            Kind::Image => "Image",
            Kind::Video => "Movie",
            Kind::Audio => "Audio",
            Kind::Text => "Text",
            Kind::Document => "Document",
            Kind::Archive => "Archive",
            Kind::Application => "Application",
            Kind::Other => "Document",
        }
    }

    /// The generic icon-theme name to fall back on when no icon exists for the
    /// specific MIME type.
    pub fn generic_icon(self) -> &'static str {
        match self {
            Kind::Folder => "folder",
            Kind::Image => "image-x-generic",
            Kind::Video => "video-x-generic",
            Kind::Audio => "audio-x-generic",
            Kind::Text => "text-x-generic",
            Kind::Document => "x-office-document",
            Kind::Archive => "package-x-generic",
            Kind::Application => "application-x-executable",
            Kind::Other => "text-x-generic",
        }
    }

    /// Can Otto *generate* a thumbnail for this kind itself?
    ///
    /// Only images, which Skia and resvg decode in-process. This is narrower
    /// than "has a thumbnail": every kind is worth a shared-cache *lookup*,
    /// because other applications write there too — quick view contributes PDF
    /// first pages, and other file managers contribute video frames. Look up
    /// for anything; generate only for these.
    pub fn thumbnailable(self) -> bool {
        matches!(self, Kind::Image)
    }
}

// ---------------------------------------------------------------------------
// Database loading
// ---------------------------------------------------------------------------

static DB: OnceLock<MimeDb> = OnceLock::new();

/// The shared MIME database, loaded once on first use.
///
/// Reads `mime/globs2` and `mime/subclasses` from every XDG data directory,
/// lowest priority first, so a user's own database overrides the system's.
/// A missing database is not an error — every lookup simply returns `None`,
/// and callers fall back to the generic icon.
pub fn database() -> &'static MimeDb {
    DB.get_or_init(|| {
        let mut db = MimeDb::default();
        for dir in mime_dirs() {
            if let Ok(text) = std::fs::read_to_string(dir.join("globs2")) {
                db.parse_globs2(&text);
            }
            if let Ok(text) = std::fs::read_to_string(dir.join("subclasses")) {
                db.parse_subclasses(&text);
            }
        }
        db
    })
}

/// XDG data directories holding a `mime` subdirectory, lowest priority first.
fn mime_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();

    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    // Reversed: XDG_DATA_DIRS is highest-priority-first, and later parses win.
    for dir in data_dirs.split(':').rev().filter(|d| !d.is_empty()) {
        dirs.push(std::path::Path::new(dir).join("mime"));
    }

    if let Some(home) = data_home() {
        dirs.push(home.join("mime"));
    }
    dirs
}

fn data_home() -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("XDG_DATA_HOME") {
        if !dir.is_empty() {
            return Some(dir.into());
        }
    }
    std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(|h| std::path::Path::new(&h).join(".local/share"))
}

// ---------------------------------------------------------------------------
// The two questions
// ---------------------------------------------------------------------------

/// The MIME type of a file *name*, from the shared database.
///
/// Pass the last path component, not a path. Returns `None` when nothing in the
/// database claims the name — callers show the generic icon rather than
/// guessing.
pub fn mime_for_name(name: &str) -> Option<&'static str> {
    database().mime_for_name(name)
}

/// The MIME type suggested by a file's leading bytes.
///
/// Performs no I/O: the caller passes bytes it has already read. At most the
/// first 4 KB is examined, and only signatures that are unambiguous — this is
/// for choosing a decoder safely, not for identifying everything.
///
/// Returns `None` when nothing matches, which a consumer must treat as
/// "unidentified", never as "fall back to the extension".
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    let b = &bytes[..bytes.len().min(4096)];

    // Fixed signatures at offset zero.
    const MAGIC: &[(&[u8], &str)] = &[
        (b"\x89PNG\r\n\x1a\n", "image/png"),
        (b"\xff\xd8\xff", "image/jpeg"),
        (b"GIF87a", "image/gif"),
        (b"GIF89a", "image/gif"),
        (b"BM", "image/bmp"),
        (b"%PDF-", "application/pdf"),
        (b"\x1f\x8b", "application/gzip"),
        (b"BZh", "application/x-bzip"),
        (b"\xfd7zXZ\x00", "application/x-xz"),
        (b"\x04\x22\x4d\x18", "application/x-lz4"),
        (b"\x28\xb5\x2f\xfd", "application/zstd"),
        (b"7z\xbc\xaf\x27\x1c", "application/x-7z-compressed"),
        (b"Rar!\x1a\x07", "application/vnd.rar"),
        (b"\x7fELF", "application/x-executable"),
        (b"\xca\xfe\xba\xbe", "application/x-java-applet"),
        (b"OggS", "application/ogg"),
        (b"fLaC", "audio/flac"),
        (b"ID3", "audio/mpeg"),
        (b"\x1a\x45\xdf\xa3", "video/x-matroska"),
        (b"MZ", "application/x-ms-dos-executable"),
        (b"<?xml", "application/xml"),
        (b"#!", "application/x-shellscript"),
    ];
    for (sig, mime) in MAGIC {
        if b.starts_with(sig) {
            return Some(mime);
        }
    }

    // Signatures with a fixed prefix at a non-zero offset.
    if b.len() >= 12 {
        if &b[..4] == b"RIFF" {
            match &b[8..12] {
                b"WEBP" => return Some("image/webp"),
                b"WAVE" => return Some("audio/x-wav"),
                b"AVI " => return Some("video/x-msvideo"),
                _ => {}
            }
        }
        if &b[4..8] == b"ftyp" {
            return Some(match &b[8..12] {
                b"heic" | b"heix" | b"mif1" => "image/heif",
                b"avif" => "image/avif",
                _ => "video/mp4",
            });
        }
    }

    // Zip is the container for a great many things; without reading the central
    // directory we can only honestly say "a zip".
    if b.starts_with(b"PK\x03\x04") || b.starts_with(b"PK\x05\x06") {
        return Some("application/zip");
    }

    // A tar header carries its magic 257 bytes in.
    if b.len() >= 262 && (&b[257..262] == b"ustar") {
        return Some("application/x-tar");
    }

    // SVG is XML, but is usually served without a declaration, so look for the
    // root element in the leading bytes.
    if let Ok(head) = std::str::from_utf8(&b[..b.len().min(1024)]) {
        if head.contains("<svg") {
            return Some("image/svg+xml");
        }
    }

    // Last resort: if it is valid UTF-8 with no NUL bytes, it is text. This is
    // deliberately last — it must never outrank a real signature.
    if !b.contains(&0) && std::str::from_utf8(b).is_ok() && !b.is_empty() {
        return Some("text/plain");
    }

    None
}

/// Combine a sniffed type with the file's name, letting the name make the
/// answer **more specific** but never different.
///
/// This exists because "dispatch on `sniff`, never on the name" is right about
/// security and wrong about container formats. A typical SVG is an XML document
/// whose `<svg>` element sits behind a declaration and a licence comment, so
/// [`sniff`] honestly reports `application/xml` — and a previewer obeying the
/// rule literally draws every icon as source code. (Observed for real: Adwaita's
/// SVGs sniff as `application/xml`, WhiteSur's as `image/svg+xml`, so the naive
/// rule is not even consistent between icon themes.)
///
/// The rule here keeps the security property intact. The name is consulted only
/// to *refine within the hierarchy the content already established*:
///
/// - `.svg` + sniffed `application/xml` → `image/svg+xml`, because SVG is a
///   subclass of XML. The content already said "this is XML"; the name only
///   says which XML.
/// - `liar.png` containing XML → stays `application/xml`, because `image/png`
///   is **not** a subclass of XML. An attacker-chosen extension still cannot
///   reach the PNG decoder.
///
/// Returns the sniffed type unchanged when the name claims nothing, claims the
/// same thing, or claims something outside the sniffed type's subtree.
pub fn refine(sniffed: &'static str, name: &str) -> &'static str {
    match mime_for_name(name) {
        Some(named) if named != sniffed && is_subclass_of(named, sniffed) => named,
        _ => sniffed,
    }
}

/// [`refine`] for a file that may not have sniffed at all.
///
/// `None` in means the content was unidentifiable, and that answer is kept:
/// an unreadable file does not become readable because of what it is called.
pub fn refine_opt(sniffed: Option<&'static str>, name: &str) -> Option<&'static str> {
    sniffed.map(|s| refine(s, name))
}

/// Is `mime` the same as, or a descendant of, `parent`?
pub fn is_subclass_of(mime: &str, parent: &str) -> bool {
    database().is_subclass_of(mime, parent)
}

/// Expand a MIME type into the set of name globs that match it, including
/// those of its descendants. This is how a portal MIME filter becomes the
/// name-based filter the picker actually applies.
pub fn globs_for(mime: &str) -> Vec<String> {
    database().globs_for(mime)
}

/// The [`Kind`] of a MIME type.
pub fn kind_of(mime: &str) -> Kind {
    let (top, sub) = mime.split_once('/').unwrap_or((mime, ""));
    match top {
        "inode" if sub == "directory" => Kind::Folder,
        "image" => Kind::Image,
        "video" => Kind::Video,
        "audio" => Kind::Audio,
        "text" => Kind::Text,
        "font" => Kind::Other,
        _ => document_or_archive(mime, sub),
    }
}

fn document_or_archive(mime: &str, sub: &str) -> Kind {
    const ARCHIVES: &[&str] = &[
        "zip",
        "gzip",
        "x-tar",
        "x-7z-compressed",
        "vnd.rar",
        "x-bzip",
        "x-bzip2",
        "x-xz",
        "zstd",
        "x-lz4",
        "x-compressed-tar",
        "x-xz-compressed-tar",
        "x-bzip-compressed-tar",
        "vnd.debian.binary-package",
        "x-rpm",
    ];
    const APPS: &[&str] = &[
        "x-executable",
        "x-sharedlib",
        "x-shellscript",
        "x-ms-dos-executable",
        "x-desktop",
        "vnd.appimage",
    ];
    const DOCS: &[&str] = &["pdf", "rtf", "postscript", "epub+zip", "x-abiword"];

    if ARCHIVES.contains(&sub) {
        Kind::Archive
    } else if APPS.contains(&sub) {
        Kind::Application
    } else if DOCS.contains(&sub)
        || sub.starts_with("vnd.oasis.opendocument")
        || sub.starts_with("vnd.openxmlformats")
        || sub.starts_with("vnd.ms-")
        || sub.starts_with("msword")
    {
        Kind::Document
    } else if is_subclass_of(mime, "text/plain") {
        Kind::Text
    } else {
        Kind::Other
    }
}

/// The kind of a file name, in one call: the common case for a directory
/// listing, where the name is all that has been read.
pub fn kind_for_name(name: &str) -> Kind {
    mime_for_name(name).map_or(Kind::Other, kind_of)
}

/// Icon-theme names to try for a MIME type, most specific first.
///
/// `image/png` yields `image-png`, then the type's generic icon, then the
/// kind's fallback — the standard chain, so a theme that only ships generic
/// icons still resolves.
pub fn icon_names(mime: &str) -> Vec<String> {
    let mut names = Vec::new();
    let dashed = mime.replace('/', "-");
    names.push(dashed.clone());
    if let Some((top, _)) = mime.split_once('/') {
        names.push(format!("{top}-x-generic"));
    }
    let generic = kind_of(mime).generic_icon().to_string();
    if !names.contains(&generic) {
        names.push(generic);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refine_lets_the_name_specialise_but_never_redirect() {
        if database().mime_for_name("a.svg").is_none() {
            return; // no shared-mime-info installed
        }
        // The case that motivated it: an SVG whose <svg> is past the window.
        assert_eq!(refine("application/xml", "logo.svg"), "image/svg+xml");
        // The attack it must not open: content says XML, name says PNG.
        assert_eq!(refine("application/xml", "liar.png"), "application/xml");
        // A name claiming something unrelated to the sniffed type is ignored.
        assert_eq!(refine("image/png", "photo.jpg"), "image/png");
        // Agreement is a no-op.
        assert_eq!(refine("image/png", "photo.png"), "image/png");
        // A name the database does not know changes nothing.
        assert_eq!(refine("application/xml", "no-extension"), "application/xml");
        // Unidentified content stays unidentified.
        assert_eq!(refine_opt(None, "logo.svg"), None);
    }

    #[test]
    fn kinds_from_mime_types() {
        assert_eq!(kind_of("inode/directory"), Kind::Folder);
        assert_eq!(kind_of("image/png"), Kind::Image);
        assert_eq!(kind_of("video/mp4"), Kind::Video);
        assert_eq!(kind_of("audio/flac"), Kind::Audio);
        assert_eq!(kind_of("text/x-rust"), Kind::Text);
        assert_eq!(kind_of("application/zip"), Kind::Archive);
        assert_eq!(kind_of("application/pdf"), Kind::Document);
        assert_eq!(
            kind_of("application/vnd.oasis.opendocument.text"),
            Kind::Document
        );
        assert_eq!(kind_of("application/x-executable"), Kind::Application);
    }

    #[test]
    fn sniff_recognises_real_signatures() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n\x00\x00"), Some("image/png"));
        assert_eq!(sniff(b"\xff\xd8\xff\xe0junk"), Some("image/jpeg"));
        assert_eq!(sniff(b"%PDF-1.7\n"), Some("application/pdf"));
        assert_eq!(sniff(b"RIFF\x00\x00\x00\x00WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff(b"\x00\x00\x00\x20ftypavif"), Some("image/avif"));
    }

    #[test]
    fn sniff_falls_back_to_text_only_as_a_last_resort() {
        assert_eq!(sniff(b"hello, world"), Some("text/plain"));
        // A real signature must outrank the text fallback even though the
        // bytes after it happen to be printable.
        assert_eq!(sniff(b"%PDF-and then ascii"), Some("application/pdf"));
        // Binary with no known signature is honestly unidentified.
        assert_eq!(sniff(&[0x00, 0x01, 0x02, 0xff]), None);
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn sniff_reads_at_most_four_kilobytes() {
        // A signature past the window must not be found — the bound is a
        // promise to the caller about how much it needs to read.
        let mut bytes = vec![b'\0'; 5000];
        bytes.extend_from_slice(b"%PDF-");
        assert_eq!(sniff(&bytes), None);
    }

    #[test]
    fn icon_chain_is_most_specific_first() {
        assert_eq!(
            icon_names("image/png"),
            vec!["image-png", "image-x-generic"]
        );
        let pdf = icon_names("application/pdf");
        assert_eq!(pdf[0], "application-pdf");
        assert!(pdf.contains(&"x-office-document".to_string()));
    }

    #[test]
    fn name_lookup_uses_the_system_database_when_there_is_one() {
        // Skipped rather than failed where no shared-mime-info is installed:
        // this asserts integration, and its absence is an environment fact.
        if database().mime_for_name("a.png").is_none() {
            return;
        }
        assert_eq!(mime_for_name("photo.png"), Some("image/png"));
        assert_eq!(kind_for_name("photo.png"), Kind::Image);
        assert_eq!(kind_for_name("notes.txt"), Kind::Text);
    }
}
