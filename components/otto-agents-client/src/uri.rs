//! Conversion between `file://` URIs and local paths.
//!
//! The escaping is `otto_foundations::uri`'s, so a URI made here is the one
//! the toolkit makes; the decoding is its strict form, because both ends of
//! the socket are ours.

use std::path::{Path, PathBuf};

use otto_foundations::uri;

/// `path` as a `file://` URI, percent-encoding everything outside the
/// unreserved set.
pub fn from_path(path: &Path) -> String {
    uri::path_to_uri(path)
}

/// The local path of an absolute `file://` URI.
///
/// A relative URI is not a path, and a truncated escape is not a literal `%`:
/// both give `None`, so a folder means one thing on both sides of the socket.
pub fn to_path(uri: &str) -> Option<PathBuf> {
    let path = uri.strip_prefix("file://")?;
    if !path.starts_with('/') {
        return None;
    }
    uri::try_decode_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_bytes_are_escaped_and_separators_are_not() {
        assert_eq!(
            from_path(Path::new("/home/me/My Projects")),
            "file:///home/me/My%20Projects"
        );
    }

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
        assert_eq!(to_path("file:///signed%+f"), None);
    }
}
