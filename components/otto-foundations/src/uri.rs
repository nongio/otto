//! `file://` URIs and percent-encoding, the one way.
//!
//! Paths are bytes on Linux, so everything here encodes and decodes the
//! path's bytes rather than a lossy UTF-8 rendering of them: a file whose name
//! is not valid Unicode survives the round trip.
//!
//! Two escape sets, because two things are asked of a URI:
//!
//! * [`path_to_uri`] escapes every byte outside RFC 3986's unreserved set,
//!   `/` excepted. What Otto hands to other programs — the clipboard, drag and
//!   drop, a `.trashinfo` — uses it.
//! * [`path_to_glib_uri`] escapes exactly what GLib's `g_filename_to_uri`
//!   does, which also leaves most sub-delimiters alone. The freedesktop
//!   thumbnail standard names a thumbnail by the MD5 of this text, so it has
//!   to agree byte for byte with every other file manager on the system.
//!
//! Decoding comes in two strengths. [`decode_path`] is lenient: a `%` not
//! followed by two hex digits is a literal `%`, because file names really do
//! contain them and a URI from another program may not have escaped one.
//! [`try_decode_path`] is strict, and refuses such a `%` outright — for a wire
//! where both ends are ours, a malformed escape is a bug to surface, not a
//! name to guess at. Both take only hex digits after a `%`: `%+f` is
//! not an escape, though `u8::from_str_radix("+f", 16)` would say it is.

use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use percent_encoding::{percent_decode, percent_encode, AsciiSet, NON_ALPHANUMERIC};

/// Everything but RFC 3986's unreserved characters and the `/` separator.
const PATH: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'/');

/// What `g_filename_to_uri` escapes: [`PATH`] less the sub-delimiters GLib
/// passes through. Not `;`, which GLib escapes.
const GLIB_PATH: &AsciiSet = &PATH
    .remove(b'!')
    .remove(b'$')
    .remove(b'&')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')')
    .remove(b'*')
    .remove(b'+')
    .remove(b',')
    .remove(b'=')
    .remove(b':')
    .remove(b'@');

/// Percent-encode a path, without a scheme — the form a `.trashinfo`'s
/// `Path=` key takes.
pub fn encode_path(path: &Path) -> String {
    percent_encode(path.as_os_str().as_bytes(), PATH).to_string()
}

/// A path as a percent-encoded `file://` URI.
pub fn path_to_uri(path: &Path) -> String {
    format!("file://{}", encode_path(path))
}

/// A path as GLib writes it as a `file://` URI — the canonical URI of the
/// freedesktop thumbnail standard. See the module docs.
pub fn path_to_glib_uri(path: &Path) -> String {
    format!(
        "file://{}",
        percent_encode(path.as_os_str().as_bytes(), GLIB_PATH)
    )
}

/// Undo percent-encoding, into a path. The inverse of [`encode_path`].
pub fn decode_path(text: &str) -> PathBuf {
    PathBuf::from(OsString::from_vec(
        percent_decode(text.as_bytes()).collect(),
    ))
}

/// Undo percent-encoding strictly, into a path: `None` when a `%` is not
/// followed by two hex digits. See the module docs.
pub fn try_decode_path(text: &str) -> Option<PathBuf> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let high = hex_digit(*bytes.get(i + 1)?)?;
            let low = hex_digit(*bytes.get(i + 2)?)?;
            decoded.push(high << 4 | low);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    Some(PathBuf::from(OsString::from_vec(decoded)))
}

fn hex_digit(byte: u8) -> Option<u8> {
    char::from(byte).to_digit(16).map(|digit| digit as u8)
}

/// The local path a `file://` URI names, or `None` for anything that is not
/// one: another scheme (a browser puts `https://` on the clipboard), or a
/// `file://host/…` naming another machine. An empty authority and
/// `localhost` are both this machine.
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.trim().strip_prefix("file://")?;
    let path = match rest.find('/') {
        Some(0) => rest,
        Some(slash) if &rest[..slash] == "localhost" => &rest[slash..],
        _ => return None,
    };
    Some(decode_path(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uri_keeps_the_path_separators_unescaped() {
        assert_eq!(path_to_uri(Path::new("/a/b/c.txt")), "file:///a/b/c.txt");
    }

    #[test]
    fn everything_outside_the_unreserved_set_is_escaped() {
        assert_eq!(
            path_to_uri(Path::new("/home/me/My Photos/café&1;.jpg")),
            "file:///home/me/My%20Photos/caf%C3%A9%261%3B.jpg"
        );
    }

    #[test]
    fn a_name_full_of_reserved_characters_round_trips() {
        let path = PathBuf::from("/home/u/a b&c#d?e+f%g.txt");
        let uri = path_to_uri(&path);
        assert!(!uri.contains(' '));
        assert_eq!(uri_to_path(&uri), Some(path));
    }

    #[test]
    fn a_non_utf8_name_round_trips() {
        let path = PathBuf::from(OsString::from_vec(b"/tmp/bad\xffname".to_vec()));
        let uri = path_to_uri(&path);
        assert!(uri.contains("%FF"));
        assert_eq!(uri_to_path(&uri), Some(path.clone()));
        assert_eq!(decode_path(&encode_path(&path)), path);
    }

    #[test]
    fn decodes_ordinary_escaped_and_localhost_uris() {
        assert_eq!(
            uri_to_path("file:///home/me/holiday%20photo.jpg"),
            Some("/home/me/holiday photo.jpg".into())
        );
        assert_eq!(
            uri_to_path("file://localhost/etc/hosts"),
            Some("/etc/hosts".into())
        );
        assert_eq!(
            uri_to_path(" file:///a\r\n"),
            Some("/a".into()),
            "a line of a uri-list"
        );
    }

    #[test]
    fn refuses_anything_that_is_not_a_local_file_uri() {
        assert!(uri_to_path("http://example.com/a").is_none());
        assert!(uri_to_path("file://remote-host/share/a").is_none());
        assert!(uri_to_path("/not/a/uri").is_none());
    }

    #[test]
    fn a_stray_percent_is_a_literal_percent() {
        assert_eq!(
            uri_to_path("file:///tmp/100%.txt"),
            Some("/tmp/100%.txt".into())
        );
        assert_eq!(decode_path("/tmp/50%zz"), PathBuf::from("/tmp/50%zz"));
    }

    /// `from_str_radix` takes a leading sign, so a hand-written decoder built
    /// on it read `%+f` as byte 0x0f. Neither decoder here does.
    #[test]
    fn a_sign_is_not_a_hex_digit() {
        assert_eq!(decode_path("/a%+fb"), PathBuf::from("/a%+fb"));
        assert_eq!(decode_path("/a%-1b"), PathBuf::from("/a%-1b"));
        assert_eq!(try_decode_path("/a%+fb"), None);
        assert_eq!(try_decode_path("/a%-1b"), None);
    }

    #[test]
    fn the_strict_decoder_refuses_what_the_lenient_one_keeps() {
        assert_eq!(
            try_decode_path("/My%20Projects/caf%C3%a9"),
            Some(PathBuf::from("/My Projects/café"))
        );
        assert_eq!(try_decode_path("/plain"), Some(PathBuf::from("/plain")));
        for malformed in ["/truncated%", "/truncated%4", "/bad%zz", "/100%.txt"] {
            assert_eq!(try_decode_path(malformed), None, "{malformed}");
            assert_eq!(decode_path(malformed), PathBuf::from(malformed));
        }
        let path = PathBuf::from(OsString::from_vec(b"/tmp/bad\xffname".to_vec()));
        assert_eq!(try_decode_path(&encode_path(&path)), Some(path));
    }

    /// Checked against `GLib.filename_to_uri` itself: the sub-delimiters
    /// pass through, `;` does not.
    #[test]
    fn the_glib_form_matches_glib() {
        assert_eq!(
            path_to_glib_uri(Path::new("/tmp/a b&c;d(1)é.png")),
            "file:///tmp/a%20b&c%3Bd(1)%C3%A9.png"
        );
        let every_printable: String = (0x21u8..0x7f)
            .map(char::from)
            .filter(|c| *c != '/')
            .collect();
        assert_eq!(
            path_to_glib_uri(&Path::new("/").join(every_printable)),
            "file:///!%22%23$%25&'()*+,-.0123456789:%3B%3C=%3E%3F@ABCDEFGHIJKLMNOPQRSTUVWXYZ\
             %5B%5C%5D%5E_%60abcdefghijklmnopqrstuvwxyz%7B%7C%7D~"
        );
    }
}
