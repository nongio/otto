//! Opening a file through the XDG desktop portal.
//!
//! This goes through the **frontend** (`org.freedesktop.portal.Desktop`), the
//! same door any other application uses — not straight to Otto's backend. That
//! is the point: it exercises the whole chain, including the `portals.conf`
//! routing that decides Otto's picker serves `FileChooser` at all. A misrouted
//! frontend is the most likely thing to be wrong, and calling the backend
//! directly would hide exactly that.
//!
//! The call blocks for as long as the dialog is up, so it runs on a thread of
//! its own — the same shape, and for the same reason, as
//! [`crate::settings_client::spawn_change_listener`]: the client only has
//! `zbus::blocking`, and blocking the main thread would freeze drawing and
//! input for as long as the user is choosing a file.

use std::collections::HashMap;

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value as ZValue};

const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const FILE_CHOOSER: &str = "org.freedesktop.portal.FileChooser";
const REQUEST: &str = "org.freedesktop.portal.Request";

/// What a finished request came back with.
pub enum Outcome {
    /// The user chose something. Absolute local paths, in view order.
    Chosen(Vec<std::path::PathBuf>),
    /// The user cancelled, or the request ended without an answer.
    Dismissed,
    /// The request never got as far as a dialog.
    Failed(String),
}

/// Open a file chooser and block until it is answered.
///
/// `filters` are `(label, [glob])` pairs, translated into the portal's
/// `(sa(us))` filter shape with rule kind `0` (glob).
pub fn open_file(title: &str, filters: &[(&str, &[&str])]) -> Outcome {
    let connection = match Connection::session() {
        Ok(connection) => connection,
        Err(err) => return Outcome::Failed(format!("no session bus: {err}")),
    };

    // The frontend derives the request's object path from our unique name and
    // the token we hand it, so we can subscribe *before* calling and never
    // race the reply. Deriving it is part of the portal contract precisely so
    // this race has an answer.
    let token = format!("otto_settings_{}", std::process::id());
    let Some(unique) = connection.unique_name().map(|n| n.to_string()) else {
        return Outcome::Failed("no unique bus name".into());
    };
    let sender = unique.trim_start_matches(':').replace('.', "_");
    let request_path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");

    let request = match zbus::blocking::Proxy::new(
        &connection,
        PORTAL_NAME,
        request_path.as_str(),
        REQUEST,
    ) {
        Ok(proxy) => proxy,
        Err(err) => return Outcome::Failed(format!("cannot watch the request: {err}")),
    };
    let mut responses = match request.receive_signal("Response") {
        Ok(signals) => signals,
        Err(err) => return Outcome::Failed(format!("cannot subscribe to Response: {err}")),
    };

    let portal =
        match zbus::blocking::Proxy::new(&connection, PORTAL_NAME, PORTAL_PATH, FILE_CHOOSER) {
            Ok(proxy) => proxy,
            Err(err) => return Outcome::Failed(format!("no portal: {err}")),
        };

    // `(label, [(kind, pattern)])`, kind 0 = glob.
    let filters: Vec<(String, Vec<(u32, String)>)> = filters
        .iter()
        .map(|(label, globs)| {
            (
                (*label).to_string(),
                globs.iter().map(|g| (0u32, (*g).to_string())).collect(),
            )
        })
        .collect();

    let mut options: HashMap<&str, ZValue> = HashMap::new();
    options.insert("handle_token", ZValue::from(token.as_str()));
    options.insert("modal", ZValue::from(true));
    if !filters.is_empty() {
        options.insert("filters", ZValue::from(filters));
    }

    // The returned handle is discarded: we derived the same path above and
    // are already listening on it. Owned, because a borrowed `ObjectPath`
    // cannot outlive the reply it was deserialised from.
    let handle: zbus::Result<OwnedObjectPath> = portal.call("OpenFile", &("", title, options));
    if let Err(err) = handle {
        return Outcome::Failed(format!("OpenFile failed: {err}"));
    }

    // Blocks until the dialog is answered. There is no timeout on purpose:
    // the user may reasonably take minutes, and a timeout would leave the
    // picker on screen answering to nobody.
    let Some(message) = responses.next() else {
        return Outcome::Failed("the portal closed without answering".into());
    };

    let (response, results): (u32, HashMap<String, OwnedValue>) = match message.body().deserialize()
    {
        Ok(body) => body,
        Err(err) => return Outcome::Failed(format!("malformed Response: {err}")),
    };

    if response != 0 {
        return Outcome::Dismissed;
    }

    let uris = results
        .get("uris")
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| Vec::<String>::try_from(v).ok())
        .unwrap_or_default();

    let paths: Vec<std::path::PathBuf> = uris.iter().filter_map(|u| uri_to_path(u)).collect();
    if paths.is_empty() {
        // Accepted with nothing usable in it. Saying "dismissed" would be a
        // lie, and returning an empty selection would look like success.
        return Outcome::Failed(format!("no local file in the reply ({uris:?})"));
    }
    Outcome::Chosen(paths)
}

/// Percent-decode a `file://` URI into a path.
///
/// Deliberately a local copy rather than a dependency on otto-quickview: this
/// app links otto-kit and nothing else, and pulling in an image-decoding crate
/// for twenty lines of URI handling would be a poor trade. If a third consumer
/// appears, the pair belongs in otto-kit.
fn uri_to_path(uri: &str) -> Option<std::path::PathBuf> {
    use std::os::unix::ffi::OsStringExt;

    let rest = uri.strip_prefix("file://")?;
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

    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[at + 1..at + 3]).ok()?, 16)
            {
                out.push(byte);
                at += 3;
                continue;
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    Some(std::path::PathBuf::from(std::ffi::OsString::from_vec(out)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_escaped_uri_decodes_to_its_path() {
        assert_eq!(
            uri_to_path("file:///home/me/holiday%20photo.jpg"),
            Some("/home/me/holiday photo.jpg".into())
        );
    }

    #[test]
    fn a_remote_uri_is_refused() {
        assert!(uri_to_path("file://elsewhere/share/a").is_none());
        assert!(uri_to_path("http://example.com/a").is_none());
    }
}
