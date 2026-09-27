//! The view each folder was last shown in, by choice.
//!
//! A folder of photographs is best in Photos and a source tree in Columns, and
//! the user says so once, by picking the view while in it. That choice is kept
//! per folder and put back when the window next goes to that folder — and
//! only then: nothing the window does on its own (Recent's grid, a search) is
//! a choice, and moving around inside the column view is not going anywhere.
//!
//! What "the folder" is depends on the view. List, Icons and Photos show one
//! folder, the deepest in the stack. The column view shows a hierarchy from
//! its root, and the root is what only a real navigation changes — so the root
//! is its location: clicking and arrowing through columns moves below it and
//! never changes the view, even into a folder remembered as Photos.

use super::*;

/// How many folders' views are kept. Past this the least recently chosen
/// are forgotten.
pub(super) const FOLDER_VIEWS_KEPT: usize = 500;

impl Browser {
    /// Whether this window keeps views per folder at all: the browser does;
    /// the Trash, the picker, the desk and a listing with no folder behind it
    /// do not.
    fn keeps_folder_views(&self) -> bool {
        !self.trash && !self.desk && self.picker.is_none() && !self.is_synthetic()
    }

    /// The folder the window is at, as `mode` sees it: the column view's
    /// root, or the one folder the other views show.
    fn view_location(&self, mode: ViewMode) -> Option<PathBuf> {
        match mode {
            ViewMode::Columns => self.columns.first(),
            ViewMode::List | ViewMode::Grid | ViewMode::Photos => self.columns.last(),
        }
        .map(|column| column.path.clone())
    }

    /// Switch view because the user asked to — the switcher, Ctrl+1 to 4, the
    /// palette — and remember it for the folder the window is at.
    pub(super) fn choose_mode(&mut self, mode: ViewMode) {
        self.set_mode(mode);
        // Refused (Recent is a grid or nothing): nothing was chosen.
        if self.mode != mode || !self.keeps_folder_views() {
            return;
        }
        let Some(path) = self.view_location(mode) else {
            return;
        };
        self.folder_views.retain(|(folder, _)| folder != &path);
        self.folder_views.push((path, mode));
        let excess = self.folder_views.len().saturating_sub(FOLDER_VIEWS_KEPT);
        self.folder_views.drain(..excess);
        self.save_folder_views();
    }

    /// The view chosen for `folder`, if one was.
    pub(super) fn folder_view(&self, folder: &Path) -> Option<ViewMode> {
        self.folder_views
            .iter()
            .rev()
            .find(|(path, _)| path == folder)
            .map(|(_, mode)| *mode)
    }

    /// The window has gone to `folder` as its location: put its chosen view
    /// back. A folder with none keeps the view the window is in.
    pub(super) fn apply_folder_view(&mut self, folder: &Path) {
        if !self.keeps_folder_views() {
            return;
        }
        if let Some(mode) = self.folder_view(folder).filter(|mode| *mode != self.mode) {
            self.set_mode(mode);
        }
    }

    /// After the stack was replaced by `restore_location`: put back the
    /// chosen view of where it now is — unless the column view is only
    /// stepping back or forward below the same root, which is moving within
    /// the columns rather than going anywhere.
    pub(super) fn apply_folder_view_after_restore(&mut self, old_root: Option<PathBuf>) {
        let Some(location) = self.view_location(self.mode) else {
            return;
        };
        if self.mode == ViewMode::Columns && old_root.as_ref() == Some(&location) {
            return;
        }
        self.apply_folder_view(&location);
    }

    /// Take on the views remembered from the last run.
    pub(super) fn remember_folder_views_from(&mut self, remembered: &remembered::Remembered) {
        self.folder_views = remembered
            .views
            .iter()
            .filter_map(|entry| Some((PathBuf::from(&entry.path), view_from_id(&entry.view)?)))
            .collect();
        let excess = self.folder_views.len().saturating_sub(FOLDER_VIEWS_KEPT);
        self.folder_views.drain(..excess);
    }

    /// Write the views to the state file — only once the window remembers on
    /// disk at all, which a test's never does.
    fn save_folder_views(&self) {
        if !self.palette_memory_on_disk {
            return;
        }
        let mut remembered = remembered::Remembered::load();
        remembered.views = self
            .folder_views
            .iter()
            .map(|(path, mode)| remembered::FolderView {
                path: path.to_string_lossy().into_owned(),
                view: view_id(*mode).to_string(),
            })
            .collect();
        remembered.save();
    }
}
