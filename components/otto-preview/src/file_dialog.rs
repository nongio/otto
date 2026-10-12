//! The file dialogs, through the desktop portal's `FileChooser`: Otto's own
//! picker, the same door any application uses. One asks which file to open
//! when Studio starts with none named; the other where to save a copy.
//!
//! A call blocks for as long as the dialog is up. Opening happens before
//! there is a window, so it may hold the main thread; saving runs on a
//! thread of its own, as Settings' file picker does.

// Rust guideline compliant 2026-02-21

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const FILE_CHOOSER: &str = "org.freedesktop.portal.FileChooser";
const REQUEST: &str = "org.freedesktop.portal.Request";

/// The pictures Studio is installed for, as the desktop entry lists them.
const PICTURE_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/bmp",
    "image/svg+xml",
    "image/avif",
    "image/heif",
];
/// The documents Studio is installed for.
const DOCUMENT_TYPES: &[&str] = &["application/pdf", "text/markdown"];

/// A filter as the portal takes it: `(label, [(kind, pattern)])`, `kind`
/// `0` a glob and `1` a MIME type.
type Filter = (String, Vec<(u32, String)>);

/// What the dialog came back with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The file chosen, an absolute local path.
    Chosen(PathBuf),
    /// The person cancelled.
    Dismissed,
    /// No dialog could be shown.
    Failed(String),
}

/// The name the dialog suggests for a copy of `file`: its own.
fn suggested_name(file: &Path) -> String {
    file.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document.pdf".into())
}

/// A folder as the portal takes it: its bytes, ending in a NUL.
fn folder_bytes(folder: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    let mut bytes = folder.as_os_str().as_bytes().to_vec();
    bytes.push(0);
    bytes
}

/// A filter matching every MIME type in `kinds`.
fn mime_filter(label: String, kinds: &[&str]) -> Filter {
    (
        label,
        kinds.iter().map(|kind| (1, (*kind).to_owned())).collect(),
    )
}

/// What the open dialog offers: everything Studio shows first, then
/// pictures, PDFs and Markdown on their own.
fn open_filters() -> Vec<Filter> {
    let all: Vec<&str> = PICTURE_TYPES
        .iter()
        .chain(DOCUMENT_TYPES)
        .copied()
        .collect();
    vec![
        mime_filter(otto_kit::t_owned!("studio-open-filter-all"), &all),
        mime_filter(
            otto_kit::t_owned!("studio-open-filter-pictures"),
            PICTURE_TYPES,
        ),
        (
            "PDF".into(),
            vec![(0, "*.pdf".into()), (1, "application/pdf".into())],
        ),
        (
            "Markdown".into(),
            vec![(0, "*.md".into()), (1, "text/markdown".into())],
        ),
    ]
}

/// Ask which file to open, titled `title`, and block until the dialog is
/// answered.
pub fn open(title: &str) -> Outcome {
    let mut options: HashMap<&str, Value<'static>> = HashMap::new();
    options.insert("filters", Value::from(open_filters()));
    ask("OpenFile", title, options)
}

/// Ask where to save a copy of `file`, titled `title`, and block until the
/// dialog is answered.
pub fn save_as(file: &Path, title: &str) -> Outcome {
    let name = suggested_name(file);
    let filters: Vec<Filter> = vec![(
        "PDF".into(),
        vec![(0, "*.pdf".into()), (1, "application/pdf".into())],
    )];
    let mut options: HashMap<&str, Value<'static>> = HashMap::new();
    options.insert("current_name", Value::from(name));
    if let Some(folder) = file.parent() {
        options.insert("current_folder", Value::from(folder_bytes(folder)));
    }
    options.insert("filters", Value::from(filters));
    match ask("SaveFile", title, options) {
        Outcome::Chosen(path) => Outcome::Chosen(with_pdf_extension(path)),
        other => other,
    }
}

/// Call the portal's `method` (`OpenFile` or `SaveFile`) titled `title` with
/// `options`, and block for the first local file in its answer.
fn ask(method: &str, title: &str, mut options: HashMap<&str, Value<'static>>) -> Outcome {
    let connection = match Connection::session() {
        Ok(connection) => connection,
        Err(err) => return Outcome::Failed(format!("no session bus: {err}")),
    };
    // The request's path is derived from our unique name and the token, so
    // the answer is listened for before the call and can't be missed.
    let token = format!(
        "otto_preview_{}_{}",
        method.to_ascii_lowercase(),
        std::process::id()
    );
    let Some(unique) = connection.unique_name().map(|name| name.to_string()) else {
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
        Err(err) => return Outcome::Failed(format!("can't watch the request: {err}")),
    };
    let mut responses = match request.receive_signal("Response") {
        Ok(signals) => signals,
        Err(err) => return Outcome::Failed(format!("can't hear the answer: {err}")),
    };
    let portal =
        match zbus::blocking::Proxy::new(&connection, PORTAL_NAME, PORTAL_PATH, FILE_CHOOSER) {
            Ok(proxy) => proxy,
            Err(err) => return Outcome::Failed(format!("no portal: {err}")),
        };

    options.insert("handle_token", Value::from(token.clone()));
    options.insert("modal", Value::from(true));

    let handle: zbus::Result<OwnedObjectPath> = portal.call(method, &("", title, options));
    if let Err(err) = handle {
        return Outcome::Failed(format!("{method} failed: {err}"));
    }
    let Some(message) = responses.next() else {
        return Outcome::Failed("the portal closed without answering".into());
    };
    let (response, results): (u32, HashMap<String, OwnedValue>) = match message.body().deserialize()
    {
        Ok(body) => body,
        Err(err) => return Outcome::Failed(format!("a malformed answer: {err}")),
    };
    if response != 0 {
        return Outcome::Dismissed;
    }
    let uri = results
        .get("uris")
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| Vec::<String>::try_from(value).ok())
        .and_then(|uris| uris.into_iter().next());
    match uri.as_deref().and_then(otto_kit::uri::uri_to_path) {
        Some(path) => Outcome::Chosen(path),
        None => Outcome::Failed(format!("no local file in the answer ({uri:?})")),
    }
}

/// `path`, with `.pdf` added when the name has no extension.
fn with_pdf_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_some() {
        path
    } else {
        path.with_extension("pdf")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dialog_starts_on_the_file_and_a_bare_name_becomes_a_pdf() {
        assert_eq!(suggested_name(Path::new("/home/me/form.pdf")), "form.pdf");
        assert_eq!(folder_bytes(Path::new("/home/me")), b"/home/me\0".to_vec());
        assert_eq!(
            with_pdf_extension(PathBuf::from("/home/me/filled")),
            PathBuf::from("/home/me/filled.pdf")
        );
        assert_eq!(
            with_pdf_extension(PathBuf::from("/home/me/filled.PDF")),
            PathBuf::from("/home/me/filled.PDF")
        );
    }

    #[test]
    fn the_open_dialog_offers_every_type_studio_is_installed_for() {
        let filters = open_filters();
        let all = &filters[0].1;
        for kind in PICTURE_TYPES.iter().chain(DOCUMENT_TYPES) {
            assert!(
                all.iter()
                    .any(|(mime, pattern)| *mime == 1 && pattern == kind),
                "{kind} missing from the first filter"
            );
        }
        assert_eq!(filters.len(), 4);
    }
}
