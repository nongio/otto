//! Conversion between `file://` URIs and local paths.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

/// `path` as a `file://` URI, percent-encoding everything outside the
/// unreserved set.
pub fn from_path(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_encoded_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(byte as char)
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

/// The local path of an absolute `file://` URI.
///
/// A relative URI is not a path, and a truncated escape is not a literal `%`:
/// both give `None`, so a folder means one thing on both sides of the socket.
pub fn to_path(uri: &str) -> Option<PathBuf> {
    let encoded = uri.strip_prefix("file://")?.as_bytes();
    if encoded.first() != Some(&b'/') {
        return None;
    }
    let mut decoded = Vec::with_capacity(encoded.len());
    let mut i = 0;
    while i < encoded.len() {
        if encoded[i] == b'%' {
            let hex = encoded.get(i + 1..i + 3)?;
            decoded.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
            i += 3;
        } else {
            decoded.push(encoded[i]);
            i += 1;
        }
    }
    Some(PathBuf::from(OsString::from_vec(decoded)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_round_trips_through_a_uri() {
        for path in ["/home/me/My Projects", "/tmp/a+b", "/srv/ünicode"] {
            assert_eq!(to_path(&from_path(Path::new(path))), Some(path.into()));
        }
    }

    #[test]
    fn what_is_not_an_absolute_file_uri_is_not_a_path() {
        assert_eq!(to_path("file://relative/dir"), None);
        assert_eq!(to_path("/home/me"), None);
        assert_eq!(to_path("file:///truncated%"), None);
        assert_eq!(to_path("file:///bad%zz"), None);
    }
}
