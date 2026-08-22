//! The file picker's own model: what a portal request asks for, and what
//! answers it. See `specs/file-picker.md`.
//!
//! Everything here is pure — no Wayland, no D-Bus, no filesystem beyond
//! probing candidate directories — so the parts most likely to harbour a
//! silent data bug (glob matching, URI encoding) are unit-testable alone.

use std::path::{Path, PathBuf};

use otto_kit::filetype;

/// What the request wants done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Open,
    Save,
    SaveFiles,
}

impl Mode {
    /// Whether the mode names the file being written, and so wants a name
    /// field. `SaveFiles` does not: the application already named every file
    /// and the user only chooses the directory they land in.
    pub fn names_a_file(self) -> bool {
        matches!(self, Self::Save)
    }

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

    /// What the name field starts with in save mode: the name the
    /// application proposed, or failing that the base name of the file being
    /// re-saved. Empty when it proposed neither — the field is then the one
    /// thing standing between the user and a nameless file, so it starts
    /// blank rather than inventing something.
    pub fn initial_name(&self) -> String {
        if !self.current_name.is_empty() {
            return self.current_name.clone();
        }
        self.current_file
            .as_deref()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
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

/// The byte range of a save name that should start selected: the stem, so
/// typing replaces the base name and leaves the extension alone. The same
/// rule an in-place rename uses, and for the same reason — the application
/// chose that extension and the user almost never means to change it.
pub fn name_stem_range(name: &str) -> std::ops::Range<usize> {
    match name.rfind('.') {
        Some(0) | None => 0..name.len(),
        Some(dot) => 0..dot,
    }
}

/// What accepting the save field should do.
///
/// Pure on purpose: the caller does the one `stat` and passes the answer in,
/// so every branch of the rule below is testable without a filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveAction {
    /// The name cannot be used. Accept stays disabled and this says why.
    Blocked(&'static str),
    /// The name is a directory that already exists: navigate into it and
    /// clear the field, rather than refusing a name the user may still want.
    Descend,
    /// A file is already there. Confirm before answering.
    Replace,
    /// Nothing is in the way: answer with the path.
    Write,
}

/// Decide what a typed save name means.
///
/// `existing` is what is at the target path today: `None` for nothing,
/// `Some(true)` for a directory, `Some(false)` for anything else. "Anything
/// else" deliberately includes a symlink, a socket and a device node: the
/// contract is that the picker returns a path, and every one of those is a
/// path the application is about to overwrite, so every one is worth a
/// confirmation.
pub fn save_action(name: &str, existing: Option<bool>) -> SaveAction {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return SaveAction::Blocked("Enter a name");
    }
    // A name is one path component. `/` would silently write somewhere the
    // user is not looking, which is worse than refusing it.
    if trimmed.contains('/') {
        return SaveAction::Blocked("A name cannot contain \u{201c}/\u{201d}");
    }
    if trimmed == "." || trimmed == ".." {
        return SaveAction::Blocked("That name is reserved");
    }
    match existing {
        Some(true) => SaveAction::Descend,
        Some(false) => SaveAction::Replace,
        None => SaveAction::Write,
    }
}

/// Whether a directory can be written into.
///
/// `access(2)` semantics: ask the kernel about the effective user rather than
/// reading the mode bits, which get the answer wrong for every group and ACL
/// case. A read-only mount still reports writable bits and fails the write
/// anyway, which is why accept has to survive the application's own `open`
/// failing too — this catches the common case (someone else's home, `/proc`,
/// a mounted disc) early enough to say so before the user types a name.
pub fn is_writable_dir(dir: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(dir) else {
        return false;
    };
    metadata.is_dir() && probe_writable(dir)
}

#[cfg(unix)]
fn probe_writable(dir: &Path) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `path` is a NUL-terminated C string that outlives the call, and
    // `access` only reads it.
    unsafe { libc::access(path.as_ptr(), libc::W_OK | libc::X_OK) == 0 }
}

#[cfg(not(unix))]
fn probe_writable(_dir: &Path) -> bool {
    true
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
    fn a_name_with_nothing_in_the_way_is_simply_written() {
        assert_eq!(save_action("notes.txt", None), SaveAction::Write);
    }

    #[test]
    fn an_existing_file_is_confirmed_before_it_is_answered_with() {
        assert_eq!(save_action("notes.txt", Some(false)), SaveAction::Replace);
    }

    #[test]
    fn an_existing_directory_is_navigated_into_rather_than_replaced() {
        assert_eq!(save_action("Documents", Some(true)), SaveAction::Descend);
    }

    #[test]
    fn an_empty_or_whitespace_name_blocks_accept() {
        assert!(matches!(save_action("", None), SaveAction::Blocked(_)));
        assert!(matches!(save_action("   ", None), SaveAction::Blocked(_)));
    }

    #[test]
    fn a_name_cannot_reach_out_of_the_directory_being_viewed() {
        assert!(matches!(
            save_action("../.bashrc", None),
            SaveAction::Blocked(_)
        ));
        assert!(matches!(
            save_action("sub/file.txt", None),
            SaveAction::Blocked(_)
        ));
        assert!(matches!(save_action("..", None), SaveAction::Blocked(_)));
    }

    #[test]
    fn a_name_is_trimmed_before_it_is_judged() {
        assert_eq!(save_action("  notes.txt  ", None), SaveAction::Write);
    }

    #[test]
    fn the_save_field_starts_on_the_proposed_name() {
        let mut r = request();
        r.current_name = "untitled.png".into();
        assert_eq!(r.initial_name(), "untitled.png");
    }

    #[test]
    fn re_saving_starts_on_the_file_being_re_saved() {
        let mut r = request();
        r.current_file = Some(PathBuf::from("/home/someone/report.pdf"));
        assert_eq!(r.initial_name(), "report.pdf");
    }

    #[test]
    fn a_proposed_name_wins_over_the_file_being_re_saved() {
        let mut r = request();
        r.current_name = "copy.pdf".into();
        r.current_file = Some(PathBuf::from("/home/someone/report.pdf"));
        assert_eq!(r.initial_name(), "copy.pdf");
    }

    #[test]
    fn a_request_proposing_nothing_starts_the_field_empty() {
        assert_eq!(request().initial_name(), "");
    }

    #[test]
    fn the_save_field_preselects_the_stem_and_spares_the_extension() {
        assert_eq!(name_stem_range("untitled.png"), 0..8);
        // A leading dot is not an extension separator.
        assert_eq!(name_stem_range(".bashrc"), 0..7);
        assert_eq!(name_stem_range("Makefile"), 0..8);
        // The *last* dot, so `archive.tar.gz` keeps `.gz` and no more.
        assert_eq!(name_stem_range("archive.tar.gz"), 0..11);
    }

    #[test]
    fn a_directory_nobody_may_write_to_is_not_writable() {
        // `/proc` exists everywhere this runs and is not writable by anyone,
        // root included, which is what makes it a stable negative.
        assert!(!is_writable_dir(Path::new("/proc")));
        assert!(!is_writable_dir(Path::new("/definitely/not/here")));
        // A file is not a directory, however writable it is.
        assert!(!is_writable_dir(Path::new("/etc/hostname")));
    }

    #[test]
    fn a_temporary_directory_is_writable() {
        assert!(is_writable_dir(&std::env::temp_dir()));
    }

    fn request() -> Request {
        Request {
            mode: Mode::Save,
            handle: "req1".into(),
            app_id: String::new(),
            parent_window: String::new(),
            title: String::new(),
            accept_label: String::new(),
            multiple: false,
            directory: false,
            modal: true,
            current_name: String::new(),
            current_folder: None,
            current_file: None,
            files: Vec::new(),
            filters: Vec::new(),
            current_filter: 0,
            choices: Vec::new(),
        }
    }

    #[test]
    fn all_files_matches_everything_including_dotfiles() {
        let f = Filter::all_files();
        assert!(f.matches("photo.png"));
        assert!(f.matches(".bashrc"));
    }
}
