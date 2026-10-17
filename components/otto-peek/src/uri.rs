//! `file://` URIs — how every host names the file to preview, and how the
//! file picker names the file it returns. The encoding lives in
//! [`otto_kit::uri`]; re-exported so the hosts keep one import path.

pub use otto_kit::uri::{path_to_uri, uri_to_path};
