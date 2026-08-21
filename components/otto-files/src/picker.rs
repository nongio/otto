//! The file picker's own model: what a portal request asks for, and what
//! answers it. See `specs/file-picker.md`.
//!
//! Everything here is pure — no Wayland, no D-Bus, no filesystem beyond
//! probing candidate directories — so the parts most likely to harbour a
//! silent data bug (glob matching, URI encoding) are unit-testable alone.

use std::path::{Path, PathBuf};

use otto_kit::filetype;

/// What the request wants done. `Save` and `SaveFiles` are carried from the
/// first version so the wire contract never has to change to gain them; the
/// picker refuses them for now rather than pretending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Open,
    Save,
    SaveFiles,
}

impl Mode {
    pub fn from_wire(mode: u32) -> Option<Self> {
        match mode {
            0 => Some(Self::Open),
            1 => Some(Self::Save),
            2 => Some(Self::SaveFiles),
            _ => None,
        }
    }

    /// The window title to use when the request supplies none.
    pub fn default_title(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Save => "Save As",
            Self::SaveFiles => "Save Files",
        }
    }

    /// The confirm button's text when the request supplies none.
    pub fn default_accept_label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Save | Self::SaveFiles => "Save",
        }
    }
}

/// One rule of a filter. MIME rules are expanded into globs when the request
/// is parsed, so only one matcher exists at match time — see the spec's
/// *MIME filters collapse into glob filters*.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub label: String,
    /// Name patterns, already expanded. Empty means the filter matches
    /// nothing, which is reported rather than silently matching everything.
    pub globs: Vec<String>,
    /// The request named a MIME type with no registered glob. Kept so the
    /// empty state can say *why* nothing matches.
    pub unresolved: bool,
    /// `inode/directory` was named: the filter is about directories.
    pub matches_directories: bool,
}

/// A filter as it arrives over the wire: `(label, [(kind, pattern)])` with
/// `kind` `0` glob and `1` MIME.
pub type WireFilter = (String, Vec<(u32, String)>);

impl Filter {
    /// Expand a wire filter, resolving every MIME rule to the globs the
    /// shared MIME database registers for it and for its descendants.
    pub fn from_wire((label, rules): WireFilter) -> Self {
        let mut globs = Vec::new();
        let mut unresolved = false;
        let mut matches_directories = false;
        for (kind, pattern) in rules {
            match kind {
                0 => globs.push(pattern),
                1 => {
                    if pattern == "inode/directory" {
                        matches_directories = true;
                        continue;
                    }
                    let expanded = filetype::globs_for(&pattern);
                    if expanded.is_empty() {
                        unresolved = true;
                    }
                    globs.extend(expanded);
                }
                // An unknown rule kind is not a reason to match everything.
                _ => unresolved = true,
            }
        }
        globs.sort();
        globs.dedup();
        Self {
            label,
            globs,
            unresolved,
            matches_directories,
        }
    }

    /// A filter with no rules at all — "All Files", offered when the request
    /// supplies none.
    pub fn all_files() -> Self {
        Self {
            label: "All Files".to_string(),
            globs: vec!["*".to_string()],
            unresolved: false,
            matches_directories: true,
        }
    }

    /// Whether `name` passes. An entry passes if it matches any rule.
    ///
    /// Matching is case-sensitive except that an all-lowercase pattern also
    /// matches an uppercase extension: `*.png` matches `PHOTO.PNG`, which is
    /// what applications mean by it.
    pub fn matches(&self, name: &str) -> bool {
        self.globs.iter().any(|pattern| {
            if pattern.chars().any(|c| c.is_ascii_uppercase()) {
                filetype::glob::matches(pattern, name)
            } else {
                filetype::glob::matches_ignore_case(pattern, name)
            }
        })
    }
}

/// A choice group the application wants answered alongside the file:
/// `(id, label, [(option_id, option_label)], default_option_id)`.
pub type WireChoice = (String, String, Vec<(String, String)>, String);

/// A parsed `Present` request. Field for field the wire tuple, with the
/// paths validated and the filters expanded.
#[derive(Debug, Clone)]
pub struct Request {
    pub mode: Mode,
    pub handle: String,
    pub app_id: String,
    pub parent_window: String,
    pub title: String,
    pub accept_label: String,
    pub multiple: bool,
    pub directory: bool,
    pub modal: bool,
    pub current_name: String,
    pub current_folder: Option<PathBuf>,
    pub current_file: Option<PathBuf>,
    pub files: Vec<String>,
    pub filters: Vec<Filter>,
    /// Index into `filters` of the one to preselect.
    pub current_filter: usize,
    pub choices: Vec<WireChoice>,
}

impl Request {
    /// The window title to show.
    pub fn window_title(&self) -> String {
        if self.title.is_empty() {
            self.mode.default_title().to_string()
        } else {
            self.title.clone()
        }
    }

    /// The confirm button's text.
    pub fn accept_text(&self) -> String {
        if self.accept_label.is_empty() {
            self.mode.default_accept_label().to_string()
        } else {
            self.accept_label.clone()
        }
    }

    /// Where to open, in the spec's order of preference: the directory of
    /// `current_file`, then `current_folder`, then the directory this
    /// `app_id` last accepted from, then home. A candidate that is not a
    /// readable directory falls through to the next.
    pub fn starting_directory(&self, remembered: Option<PathBuf>) -> PathBuf {
        let candidates = [
            self.current_file.as_deref().and_then(Path::parent),
            self.current_folder.as_deref(),
            remembered.as_deref(),
        ];
        for candidate in candidates.into_iter().flatten() {
            if std::fs::read_dir(candidate).is_ok() {
                return candidate.to_path_buf();
            }
        }
        crate::model::home_dir().unwrap_or_else(|| PathBuf::from("/"))
    }
}

/// How a request ended.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    /// `0` accepted, `1` cancelled by the user, `2` ended for another reason.
    pub response: u32,
    /// Percent-encoded absolute `file://` URIs. Empty unless `response` is 0.
    pub uris: Vec<String>,
    /// The label of the filter in effect at acceptance.
    pub current_filter: String,
    pub choices: Vec<(String, String)>,
}

impl Outcome {
    /// The user cancelled.
    pub fn cancelled() -> Self {
        Self {
            response: 1,
            ..Default::default()
        }
    }

    /// The request ended without the user answering — withdrawn, or the
    /// picker could not serve it.
    pub fn ended() -> Self {
        Self {
            response: 2,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// A request in progress
// ---------------------------------------------------------------------------

/// A request the picker window is currently serving.
///
/// It owns the reply channel, and answers on drop if nothing else did — a
/// file dialog that never returns hangs the requesting application's UI
/// thread, so "the window went away without deciding" has to resolve to
/// *something*, and `response = 2` is what the portal contract says that is.
pub struct Session {
    pub request: Request,
    /// The request's filters, plus "All Files" when it supplied none.
    pub filters: Vec<Filter>,
    pub current_filter: usize,
    /// The filter menu is showing.
    pub filter_open: bool,
    /// The accept button's text and the filter control's labels, resolved
    /// once. The frame borrows them for the length of a paint, which owned
    /// values built per frame could not survive.
    pub accept_label: String,
    pub filter_labels: Vec<String>,
    responder: Option<tokio::sync::oneshot::Sender<Outcome>>,
}

impl Session {
    pub fn new(request: Request, responder: tokio::sync::oneshot::Sender<Outcome>) -> Self {
        let mut filters = request.filters.clone();
        let current_filter = request.current_filter.min(filters.len().saturating_sub(1));
        if filters.is_empty() {
            filters.push(Filter::all_files());
        }
        Self {
            accept_label: request.accept_text(),
            filter_labels: filters.iter().map(|f| f.label.clone()).collect(),
            request,
            filters,
            current_filter,
            filter_open: false,
            responder: Some(responder),
        }
    }

    /// The filter currently in force.
    pub fn filter(&self) -> &Filter {
        &self.filters[self.current_filter.min(self.filters.len() - 1)]
    }

    /// Whether `name` should be shown, given the filter in force.
    ///
    /// **Filters never hide directories.** A filter is the application saying
    /// what it can open, not what the user may navigate through; hiding a
    /// folder because it does not match `*.png` would make the files that do
    /// match unreachable.
    pub fn shows(&self, name: &str, is_dir: bool) -> bool {
        if is_dir || self.request.directory {
            // Directory mode still lists files — greyed, so the user can see
            // where they are — so nothing is filtered out on that path either.
            return true;
        }
        self.filter().matches(name)
    }

    /// Whether an entry may be returned to the application.
    pub fn selectable(&self, name: &str, is_dir: bool) -> bool {
        if self.request.directory {
            return is_dir;
        }
        !is_dir && self.filter().matches(name)
    }

    /// Answer the request. Only the first call is delivered, so a
    /// double-click that both accepts and closes cannot try to send two
    /// outcomes down a one-shot channel.
    pub fn resolve(&mut self, outcome: Outcome) {
        if let Some(tx) = self.responder.take() {
            let _ = tx.send(outcome);
        }
    }

    /// Accept `paths`, recording the filter that was in force.
    ///
    /// The URIs come from otto-quickview's encoder, which lives beside the
    /// decoder every consumer of them uses — the round trip is the property
    /// that matters, and it is only testable with both halves together.
    pub fn accept(&mut self, paths: &[PathBuf]) {
        let outcome = Outcome {
            response: 0,
            uris: paths
                .iter()
                .map(|p| otto_quickview::uri::path_to_uri(p))
                .collect(),
            current_filter: self.filter().label.clone(),
            choices: Vec::new(),
        };
        self.resolve(outcome);
    }

    pub fn answered(&self) -> bool {
        self.responder.is_none()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.resolve(Outcome::ended());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lowercase_glob_also_matches_an_uppercase_extension() {
        let f = Filter::from_wire(("Images".into(), vec![(0, "*.png".into())]));
        assert!(f.matches("photo.png"));
        assert!(f.matches("PHOTO.PNG"));
        assert!(!f.matches("photo.jpg"));
    }

    #[test]
    fn a_glob_carrying_case_is_matched_with_case() {
        let f = Filter::from_wire(("Makefiles".into(), vec![(0, "Makefile".into())]));
        assert!(f.matches("Makefile"));
        assert!(!f.matches("makefile"));
    }

    #[test]
    fn an_entry_passes_if_any_rule_matches() {
        let f = Filter::from_wire((
            "Images".into(),
            vec![(0, "*.png".into()), (0, "*.jpg".into())],
        ));
        assert!(f.matches("a.png"));
        assert!(f.matches("a.jpg"));
        assert!(!f.matches("a.gif"));
    }

    #[test]
    fn a_mime_rule_expands_to_the_globs_registered_for_it() {
        let f = Filter::from_wire(("PNG".into(), vec![(1, "image/png".into())]));
        assert!(!f.unresolved, "image/png should be in the shared MIME db");
        assert!(f.matches("photo.png"));
        assert!(!f.matches("photo.txt"));
    }

    #[test]
    fn a_mime_rule_with_no_registered_glob_matches_nothing_and_says_so() {
        let f = Filter::from_wire((
            "Nonsense".into(),
            vec![(1, "application/x-otto-not-a-real-type".into())],
        ));
        assert!(f.unresolved);
        assert!(!f.matches("anything.at.all"));
    }

    #[test]
    fn inode_directory_is_a_directory_rule_not_a_name_pattern() {
        let f = Filter::from_wire(("Folders".into(), vec![(1, "inode/directory".into())]));
        assert!(f.matches_directories);
        assert!(f.globs.is_empty());
    }

    #[test]
    fn all_files_matches_everything_including_dotfiles() {
        let f = Filter::all_files();
        assert!(f.matches("photo.png"));
        assert!(f.matches(".bashrc"));
    }
}
