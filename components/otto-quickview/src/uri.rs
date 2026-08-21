//! `file://` URIs — how every host names the file to preview, and how the
//! file picker names the file it returns.
//!
//! Hand-rolled because it is twenty lines and the alternative is a URL crate
//! for one scheme. The encoder and the decoder live together so the pair
//! stays honest: whatever one escapes, the other must give back.

/// `file://` URI to a path, with percent-decoding.
///
/// Hand-rolled because it is twenty lines and the alternative is a URL crate
/// for one scheme. Anything that is not a plain local `file://` URI is refused
/// rather than guessed at — the contract says `file://` only.
pub fn uri_to_path(uri: &str) -> Option<std::path::PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // An authority component is either empty or "localhost"; anything else is a
    // remote location, which is out of scope by design.
    let path = match rest.find('/') {
        Some(0) => rest,
        Some(slash) => {
            let authority = &rest[..slash];
            if !authority.is_empty() && authority != "localhost" {
                return None;
            }
            &rest[slash..]
        }
        None => return None,
    };

    let mut out = Vec::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[at + 1..at + 3]).ok()?;
            match u8::from_str_radix(hex, 16) {
                Ok(byte) => {
                    out.push(byte);
                    at += 3;
                    continue;
                }
                // A stray '%' is a literal '%', not a failure: filenames
                // contain them.
                Err(_) => {}
            }
        }
        out.push(bytes[at]);
        at += 1;
    }

    use std::os::unix::ffi::OsStringExt;
    Some(std::path::PathBuf::from(std::ffi::OsString::from_vec(out)))
}

/// A path as a percent-encoded `file://` URI — the inverse of
/// [`uri_to_path`].
///
/// Encoding works on the path's **bytes**, not on a lossy UTF-8 rendering of
/// them: Linux file names are bytes, and a file must survive the round trip
/// whether or not it happens to be nameable in Unicode. Every byte outside
/// RFC 3986's unreserved set is escaped, except `/`, which is the URI's own
/// separator and here means what it means in the path.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uri_keeps_the_path_separators_unescaped() {
        assert_eq!(
            path_to_uri(std::path::Path::new("/a/b/c.txt")),
            "file:///a/b/c.txt"
        );
    }

    #[test]
    fn a_name_full_of_reserved_characters_round_trips() {
        // The single most likely place for a silent data bug in the picker.
        let path = std::path::PathBuf::from("/home/u/a b&c#d?e+f%g.txt");
        let uri = path_to_uri(&path);
        assert!(!uri.contains(' '));
        assert_eq!(uri_to_path(&uri), Some(path));
    }

    #[test]
    fn a_non_utf8_name_round_trips() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let path = std::path::PathBuf::from(OsString::from_vec(b"/tmp/bad\xffname".to_vec()));
        let uri = path_to_uri(&path);
        assert!(uri.contains("%FF"));
        assert_eq!(uri_to_path(&uri), Some(path));
    }

    #[test]
    fn decodes_ordinary_and_escaped_paths() {
        assert_eq!(
            uri_to_path("file:///home/me/a.txt"),
            Some("/home/me/a.txt".into())
        );
        assert_eq!(
            uri_to_path("file:///home/me/holiday%20photo.jpg"),
            Some("/home/me/holiday photo.jpg".into())
        );
        assert_eq!(
            uri_to_path("file://localhost/etc/hosts"),
            Some("/etc/hosts".into())
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
        // Filenames really do contain these, and refusing would be worse than
        // taking it literally.
        assert_eq!(
            uri_to_path("file:///tmp/100%.txt"),
            Some("/tmp/100%.txt".into())
        );
    }

    #[test]
    fn a_percent_encoded_byte_that_is_not_utf8_still_round_trips() {
        // Paths are bytes, not strings. A Latin-1 filename must survive.
        let path = uri_to_path("file:///tmp/caf%E9").expect("decodes");
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(path.file_name().unwrap().as_bytes(), b"caf\xe9");
    }
}
