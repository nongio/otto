//! Conversion between `file://` URIs and local paths.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

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

/// Returns the local path of an absolute `file://` URI.
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
    fn round_trips_paths_with_reserved_characters() {
        let path = Path::new("/home/me/My Projects/a#b%c");
        let uri = from_path(path);
        assert_eq!(uri, "file:///home/me/My%20Projects/a%23b%25c");
        assert_eq!(to_path(&uri).as_deref(), Some(path));
    }

    #[test]
    fn rejects_relative_and_malformed_uris() {
        assert_eq!(to_path("file://relative/dir"), None);
        assert_eq!(to_path("https://example.com/"), None);
        assert_eq!(to_path("file:///bad%2"), None);
    }
}
