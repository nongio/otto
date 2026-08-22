//! System clipboard, over `wl_data_device`.
//!
//! The Wayland selection is a *promise*, not a buffer: the client that copied
//! keeps the data and hands it over, one pipe at a time, whenever someone
//! pastes. So both halves of this module are asynchronous underneath even
//! though the API is not:
//!
//! - [`set`] registers the payload and offers its MIME types. The bytes stay
//!   here until a paste asks for them, which may be much later — or never.
//! - [`read`] asks the *other* client for bytes down a pipe. That client may be
//!   slow, or gone, so the read is bounded rather than trusted.
//!
//! Losing the selection (someone else copies) drops the payload, which is
//! correct: the compositor has already told everyone the offer is dead.
//!
//! Drag and drop is deliberately not here. It shares `wl_data_device` but needs
//! surface enter/leave/motion plumbing and an action negotiation this does not
//! attempt.

use std::io::Read;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The de-facto standard for copying files between file managers: a first line
/// of `copy` or `cut`, then one URI per line. GNOME invented it and everything
/// else followed, so it is how a *cut* survives crossing an application
/// boundary — plain `text/uri-list` cannot express one.
pub const URI_LIST_WITH_ACTION: &str = "x-special/gnome-copied-files";
/// The portable list of file URIs, CRLF separated per RFC 2483.
pub const URI_LIST: &str = "text/uri-list";
pub const TEXT_PLAIN: &str = "text/plain;charset=utf-8";

/// What this application last put on the clipboard, kept so it can be handed
/// over when someone finally pastes.
/// One MIME type and the bytes offered for it.
type Offer = (String, Vec<u8>);

static OFFERED: OnceLock<Mutex<Vec<Offer>>> = OnceLock::new();

/// The MIME types the *current* selection advertises, whoever owns it.
static AVAILABLE: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

fn offered() -> &'static Mutex<Vec<(String, Vec<u8>)>> {
    OFFERED.get_or_init(|| Mutex::new(Vec::new()))
}

fn available() -> &'static Mutex<Vec<String>> {
    AVAILABLE.get_or_init(|| Mutex::new(Vec::new()))
}

// ---------------------------------------------------------------------------
// Copying
// ---------------------------------------------------------------------------

/// Put `entries` on the clipboard as `(mime_type, bytes)`.
///
/// `serial` must come from a real input event — a click or a keystroke. The
/// compositor rejects a selection claimed with a stale or invented serial,
/// which is what stops a background client from silently owning the clipboard.
///
/// Returns whether the selection was claimed. `false` means the data device is
/// not available: no seat yet, or a compositor without `wl_data_device_manager`.
pub fn set(entries: Vec<(String, Vec<u8>)>, serial: u32) -> bool {
    let mime_types: Vec<String> = entries.iter().map(|(mime, _)| mime.clone()).collect();
    *offered().lock().unwrap() = entries;
    crate::app_runner::context::AppContext::set_selection(mime_types, serial)
}

/// The payload for `mime`, if this application is the one offering it.
///
/// Called from the data-source handler when a paste arrives.
pub(crate) fn offered_bytes(mime: &str) -> Option<Vec<u8>> {
    offered()
        .lock()
        .unwrap()
        .iter()
        .find(|(m, _)| m == mime)
        .map(|(_, bytes)| bytes.clone())
}

/// The compositor cancelled our source — someone else owns the clipboard now.
pub(crate) fn clear_offered() {
    offered().lock().unwrap().clear();
}

/// Record the MIME types of an incoming selection offer.
pub(crate) fn set_available(mime_types: Vec<String>) {
    *available().lock().unwrap() = mime_types;
}

// ---------------------------------------------------------------------------
// Pasting
// ---------------------------------------------------------------------------

/// MIME types the current selection offers.
pub fn available_mime_types() -> Vec<String> {
    available().lock().unwrap().clone()
}

/// Is any of `preferred` on the clipboard? Returns the first that matches, so
/// callers can express a preference order in one call.
pub fn first_available(preferred: &[&str]) -> Option<String> {
    let have = available_mime_types();
    preferred
        .iter()
        .find(|want| have.iter().any(|h| h == *want))
        .map(|s| (*s).to_string())
}

/// How long to wait for the owning client to write its data.
///
/// The source is another application, and it may be busy, hung, or malicious.
/// Since this runs on the UI thread the wait is bounded: a clipboard that does
/// not answer promptly is treated as empty rather than freezing the window.
const READ_TIMEOUT: Duration = Duration::from_millis(500);
/// A clipboard payload larger than this is refused. Selections are names and
/// small text; anything at this scale is a mistake or an attack.
const READ_LIMIT: usize = 8 * 1024 * 1024;

/// Read the current selection as `mime`.
///
/// Returns `None` when nothing is offered, the type is not available, or the
/// owning client did not answer within [`READ_TIMEOUT`].
pub fn read(mime: &str) -> Option<Vec<u8>> {
    let pipe = crate::app_runner::context::AppContext::receive_selection(mime)?;
    read_bounded(pipe)
}

/// Read to EOF, but never past the limit and never past the deadline.
pub(crate) fn read_bounded(mut pipe: std::fs::File) -> Option<Vec<u8>> {
    use std::os::fd::AsRawFd;

    // Non-blocking, so a source that opens the pipe and then stalls cannot pin
    // the UI thread until it feels like writing.
    // SAFETY: `pipe` owns the descriptor for the whole call.
    unsafe {
        let fd = pipe.as_raw_fd();
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }

    let deadline = Instant::now() + READ_TIMEOUT;
    let mut out = Vec::new();
    let mut chunk = [0u8; 8192];

    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => return Some(out),
            Ok(n) => {
                out.extend_from_slice(&chunk[..n]);
                if out.len() > READ_LIMIT {
                    tracing::warn!("clipboard payload exceeded {READ_LIMIT} bytes; discarding");
                    return None;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    tracing::warn!("clipboard read timed out; treating as empty");
                    return None;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => {
                tracing::warn!(%err, "clipboard read failed");
                return None;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// URI list encoding
// ---------------------------------------------------------------------------

/// Percent-encode a path into a `file://` URI.
///
/// Everything outside the unreserved set is escaped, `/` excepted — it is the
/// path separator, not data. Paths are bytes on Linux, so this encodes bytes
/// rather than characters and a non-UTF-8 name survives the round trip.
pub fn path_to_uri(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;

    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(byte as char)
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

/// The inverse of [`path_to_uri`]. Returns `None` for anything that is not a
/// `file://` URI — a browser may put `https://` on the clipboard, and that is
/// not a path.
pub fn uri_to_path(uri: &str) -> Option<std::path::PathBuf> {
    use std::os::unix::ffi::OsStringExt;

    let rest = uri.trim().strip_prefix("file://")?;
    // An authority component (`file://host/path`) is not a local path unless
    // the host is empty or `localhost`.
    let path = match rest.find('/') {
        Some(0) => rest,
        Some(slash) if &rest[..slash] == "localhost" => &rest[slash..],
        _ => return None,
    };

    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    Some(std::path::PathBuf::from(std::ffi::OsString::from_vec(out)))
}

/// Build every payload a file selection should offer, so the copy is legible
/// to other file managers, to text editors, and to us.
pub fn file_payloads(paths: &[std::path::PathBuf], cut: bool) -> Vec<(String, Vec<u8>)> {
    let uris: Vec<String> = paths.iter().map(|p| path_to_uri(p)).collect();

    // RFC 2483 says CRLF.
    let uri_list = uris.join("\r\n");
    let gnome = format!("{}\n{}", if cut { "cut" } else { "copy" }, uris.join("\n"));
    let text = paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("\n");

    vec![
        (URI_LIST_WITH_ACTION.to_string(), gnome.into_bytes()),
        (URI_LIST.to_string(), uri_list.into_bytes()),
        (TEXT_PLAIN.to_string(), text.into_bytes()),
    ]
}

/// Parse a clipboard payload into paths, and whether it was a cut.
///
/// Understands both the GNOME form (with its leading `copy`/`cut` line) and a
/// plain `text/uri-list`, which cannot express an action and so is a copy.
pub fn parse_file_payload(mime: &str, bytes: &[u8]) -> (Vec<std::path::PathBuf>, bool) {
    let text = String::from_utf8_lossy(bytes);
    let mut cut = false;
    let mut lines = text.lines().peekable();

    if mime == URI_LIST_WITH_ACTION {
        match lines.peek().map(|l| l.trim()) {
            Some("cut") => {
                cut = true;
                lines.next();
            }
            Some("copy") => {
                lines.next();
            }
            _ => {}
        }
    }

    let paths = lines
        .map(str::trim)
        // `text/uri-list` allows `#` comment lines.
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(uri_to_path)
        .collect();

    (paths, cut)
}

/// The mime types a file paste understands, best first.
pub fn file_mime_preference() -> &'static [&'static str] {
    &[URI_LIST_WITH_ACTION, URI_LIST]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn uris_round_trip_including_awkward_names() {
        for name in [
            "/tmp/plain.txt",
            "/tmp/with space.txt",
            "/tmp/a b&c#d.txt",
            "/tmp/percent%20literal.txt",
            "/tmp/héllo.txt",
            "/tmp/quote'and\"quote.txt",
        ] {
            let path = PathBuf::from(name);
            let uri = path_to_uri(&path);
            assert!(!uri.contains(' '), "space must be encoded: {uri}");
            assert_eq!(uri_to_path(&uri).as_deref(), Some(path.as_path()), "{uri}");
        }
    }

    #[test]
    fn separators_are_not_escaped() {
        assert_eq!(path_to_uri(&PathBuf::from("/a/b/c")), "file:///a/b/c");
    }

    #[test]
    fn a_hash_in_a_name_survives() {
        // The one most likely to break a naive implementation: `#` starts a
        // comment in `text/uri-list`, so it must never appear raw.
        let uri = path_to_uri(&PathBuf::from("/tmp/a#b.txt"));
        assert!(!uri.contains('#'), "{uri}");
        let (paths, _) = parse_file_payload(URI_LIST, uri.as_bytes());
        assert_eq!(paths, vec![PathBuf::from("/tmp/a#b.txt")]);
    }

    #[test]
    fn non_utf8_names_survive_the_round_trip() {
        use std::os::unix::ffi::OsStringExt;
        let raw = std::ffi::OsString::from_vec(vec![b'/', b't', b'm', b'p', b'/', 0xFF, 0xFE]);
        let path = PathBuf::from(raw);
        let uri = path_to_uri(&path);
        assert_eq!(uri_to_path(&uri), Some(path));
    }

    #[test]
    fn non_file_uris_are_rejected() {
        assert_eq!(uri_to_path("https://example.com/x"), None);
        assert_eq!(uri_to_path("file://otherhost/tmp/x"), None);
        // An empty authority and `localhost` both mean this machine.
        assert_eq!(
            uri_to_path("file://localhost/tmp/x"),
            Some(PathBuf::from("/tmp/x"))
        );
    }

    #[test]
    fn the_gnome_payload_carries_the_cut_action() {
        let paths = vec![PathBuf::from("/tmp/a.txt"), PathBuf::from("/tmp/b.txt")];

        let payloads = file_payloads(&paths, true);
        let (mime, bytes) = &payloads[0];
        assert_eq!(mime, URI_LIST_WITH_ACTION);
        let (parsed, cut) = parse_file_payload(mime, bytes);
        assert!(cut, "a cut must survive the round trip");
        assert_eq!(parsed, paths);

        let payloads = file_payloads(&paths, false);
        let (mime, bytes) = &payloads[0];
        let (parsed, cut) = parse_file_payload(mime, bytes);
        assert!(!cut);
        assert_eq!(parsed, paths);
    }

    #[test]
    fn a_plain_uri_list_is_always_a_copy() {
        // It has no way to say otherwise, and guessing "cut" would delete
        // someone's files.
        let paths = vec![PathBuf::from("/tmp/a.txt")];
        let payloads = file_payloads(&paths, true);
        let (_, bytes) = payloads.iter().find(|(m, _)| m == URI_LIST).unwrap();
        let (parsed, cut) = parse_file_payload(URI_LIST, bytes);
        assert!(!cut, "text/uri-list cannot express a cut");
        assert_eq!(parsed, paths);
    }

    #[test]
    fn uri_lists_use_crlf() {
        let paths = vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")];
        let payloads = file_payloads(&paths, false);
        let (_, bytes) = payloads.iter().find(|(m, _)| m == URI_LIST).unwrap();
        assert_eq!(
            String::from_utf8_lossy(bytes),
            "file:///tmp/a\r\nfile:///tmp/b"
        );
    }

    #[test]
    fn comment_and_blank_lines_are_ignored() {
        let payload = "# a comment\n\nfile:///tmp/a\r\n";
        let (paths, _) = parse_file_payload(URI_LIST, payload.as_bytes());
        assert_eq!(paths, vec![PathBuf::from("/tmp/a")]);
    }
}
