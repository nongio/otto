//! The file dialogs, through the desktop portal's `FileChooser`: Otto's own
//! picker, the same door any application uses. One asks which file to open
//! when Studio starts with none named; the other where to save a copy.
//!
//! This module only says what each dialog offers; otto-kit's
//! `portal_file_chooser` puts it up. The caller spawns the request, so the
//! dialog blocks a thread of its own and the run loop is woken whatever the
//! answer.

// Rust guideline compliant 2026-02-21

use std::path::{Path, PathBuf};

use otto_kit::portal_file_chooser::{Request, Rule};

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

/// The name the dialog suggests for a copy of `file`: its own.
fn suggested_name(file: &Path) -> String {
    file.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document.pdf".into())
}

/// Rules matching every MIME type in `kinds`.
fn mime_rules(kinds: &[&str]) -> Vec<Rule> {
    kinds
        .iter()
        .map(|kind| Rule::Mime((*kind).to_owned()))
        .collect()
}

/// The rules for a PDF: its extension or its MIME type.
fn pdf_rules() -> [Rule; 2] {
    [
        Rule::Glob("*.pdf".into()),
        Rule::Mime("application/pdf".into()),
    ]
}

/// Every type Studio is installed for: pictures, then documents.
fn every_type() -> Vec<&'static str> {
    PICTURE_TYPES
        .iter()
        .chain(DOCUMENT_TYPES)
        .copied()
        .collect()
}

/// The dialog asking which file to open, titled `title`, as a dialog of the
/// window `parent` names (`wayland:<handle>`, or empty for none). It offers
/// everything Studio shows first, then pictures, PDFs and Markdown on their
/// own.
pub fn open(title: &str, parent: &str) -> Request {
    Request::open_file(title)
        .parent(parent)
        .filter(
            otto_kit::t_owned!("studio-open-filter-all"),
            &mime_rules(&every_type()),
        )
        .filter(
            otto_kit::t_owned!("studio-open-filter-pictures"),
            &mime_rules(PICTURE_TYPES),
        )
        .filter("PDF", &pdf_rules())
        .filter(
            "Markdown",
            &[
                Rule::Glob("*.md".into()),
                Rule::Mime("text/markdown".into()),
            ],
        )
}

/// The dialog asking where to save a copy of `file`, titled `title`. It
/// starts on the file's own name and folder. Pass what it chooses through
/// [`saved_path`].
pub fn save_as(file: &Path, title: &str) -> Request {
    let mut request = Request::save_file(title).current_name(suggested_name(file));
    if let Some(folder) = file.parent() {
        request = request.current_folder(folder);
    }
    request.filter("PDF", &pdf_rules())
}

/// Where a copy goes when the save dialog chose `path`: there, with `.pdf`
/// added when the name has no extension.
pub fn saved_path(path: PathBuf) -> PathBuf {
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
        assert_eq!(
            saved_path(PathBuf::from("/home/me/filled")),
            PathBuf::from("/home/me/filled.pdf")
        );
        assert_eq!(
            saved_path(PathBuf::from("/home/me/filled.PDF")),
            PathBuf::from("/home/me/filled.PDF")
        );
    }

    #[test]
    fn the_open_dialog_offers_every_type_studio_is_installed_for() {
        let rules = mime_rules(&every_type());
        for kind in PICTURE_TYPES.iter().chain(DOCUMENT_TYPES) {
            assert!(
                rules.contains(&Rule::Mime((*kind).to_owned())),
                "{kind} missing from the first filter"
            );
        }
    }
}
