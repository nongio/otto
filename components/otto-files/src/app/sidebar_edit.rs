//! Editing the sidebar from the sidebar: folders dropped between its rows
//! join it, and a right-click on a row takes it out.
//!
//! Nothing here keeps a list of its own. Every edit is written to
//! `files.toml` through [`places_config`] and the rows are read back from
//! it, so the file stays the one answer to what the sidebar holds — the one a
//! person editing it by hand, Settings and every other open window all see.

use super::*;

use crate::places_config::{self, Builtin, SidebarConfig};

/// The prefix of the menu ids that show a hidden built-in row again; the
/// row's config id follows it.
pub(super) const SHOW_PREFIX: &str = "sidebar_show_";

impl Browser {
    /// The gap a drag at `(x, y)` would add folders at, if it would add any.
    ///
    /// The user's own folders follow the built-in rows, so a gap among the
    /// built-ins means the first place theirs can go — and the line is drawn
    /// there, where the row will actually appear, rather than where the
    /// pointer is.
    pub(super) fn sidebar_gap_at(&self, x: f32, y: f32) -> Option<usize> {
        if self.trash || self.desk {
            return None;
        }
        let slot = view::place_gap_at(x, y, self.places.len())?;
        // Our own drag says what it carries. Files alone, or folders already
        // built in, would add nothing, and the row's edge is still the row.
        // Someone else's drag only says at the drop, so it is taken on trust.
        if let Some(paths) = otto_kit::dnd::own_files() {
            if !paths.iter().any(|path| self.would_add(path)) {
                return None;
            }
        }
        Some(slot.max(self.first_own_place()))
    }

    /// Where the user's own folders start in the sidebar.
    fn first_own_place(&self) -> usize {
        self.places
            .iter()
            .position(|place| place.builtin.is_none())
            .unwrap_or(self.places.len())
    }

    /// Whether adding `path` would change the sidebar: a folder, and not one
    /// of the built-in rows under another name. One of the user's own rows
    /// does change it — dropping it elsewhere is how they are reordered.
    fn would_add(&self, path: &Path) -> bool {
        path.is_dir()
            && !self
                .places
                .iter()
                .any(|place| place.builtin.is_some() && place.path == path)
    }

    /// Add the folders among `paths` to the sidebar, as the row at `slot`.
    pub(super) fn add_to_sidebar(&mut self, paths: &[PathBuf], slot: usize) {
        let paths: Vec<PathBuf> = paths
            .iter()
            .filter(|path| self.would_add(path))
            .cloned()
            .collect();
        if paths.is_empty() {
            return;
        }
        let before = self
            .places
            .get(slot)
            .filter(|place| place.builtin.is_none())
            .map(|place| place.path.clone());
        let home = places_config::home_dir();
        self.edit_sidebar(|text| {
            places_config::with_places_added(text, &paths, before.as_deref(), home.as_deref())
        });
    }

    /// The selected folders, if adding them to the sidebar would add
    /// anything.
    pub(super) fn selection_for_sidebar(&self) -> Option<Vec<PathBuf>> {
        if self.trash || self.desk {
            return None;
        }
        let entries = self.selected_entries();
        if entries.is_empty() || entries.iter().any(|entry| !entry.is_dir) {
            return None;
        }
        let paths: Vec<PathBuf> = entries.into_iter().map(|entry| entry.path).collect();
        paths
            .iter()
            .any(|path| self.would_add(path))
            .then_some(paths)
    }

    /// Add the selected folders after the last row.
    pub(super) fn add_selection_to_sidebar(&mut self) {
        if let Some(paths) = self.selection_for_sidebar() {
            self.add_to_sidebar(&paths, self.places.len());
        }
    }

    /// Take row `index` out of the sidebar: one of the user's own folders
    /// leaves the config, and a built-in row is hidden.
    pub(super) fn remove_place(&mut self, index: usize) {
        let Some(place) = self.places.get(index).cloned() else {
            return;
        };
        match place.builtin {
            Some(builtin) => {
                self.edit_sidebar(|text| places_config::with_builtin_hidden(text, builtin, true));
            }
            None => {
                let home = places_config::home_dir();
                self.edit_sidebar(|text| {
                    places_config::with_place_removed(text, &place.path, home.as_deref())
                });
            }
        }
    }

    /// The row the last right-click on the sidebar landed on, removed.
    pub(super) fn remove_menu_place(&mut self) {
        if let Some(index) = self.menu_place.take() {
            self.remove_place(index);
        }
    }

    /// Bring back the built-in row the config calls `id`.
    pub(super) fn show_builtin(&mut self, id: &str) {
        if let Some(builtin) = Builtin::from_id(id) {
            self.edit_sidebar(|text| places_config::with_builtin_hidden(text, builtin, false));
        }
    }

    /// The built-in rows hidden now that would be listed if shown — a hidden
    /// Music with no Music folder behind it is nothing to offer back.
    fn hidden_places() -> Vec<(Builtin, Place)> {
        let config = SidebarConfig::load();
        model::places_with(&config.hiding_nothing())
            .into_iter()
            .filter_map(|place| {
                let builtin = place.builtin.filter(|builtin| config.hides(*builtin))?;
                Some((builtin, place))
            })
            .collect()
    }

    /// The menu for a right-click on the sidebar: removing the row under the
    /// pointer, and bringing back whatever built-in rows were hidden — the
    /// one edit with no row left to click.
    pub(super) fn sidebar_menu_items(&mut self, x: f32, y: f32) -> Vec<MenuItem> {
        self.menu_place = view::place_at(x, y, self.places.len());
        let mut items = Vec::new();
        if self.menu_place.is_some() {
            items.push(
                MenuItem::action(otto_kit::t!("files-remove-from-sidebar"))
                    .with_action_id("sidebar_remove"),
            );
        }
        let hidden = Self::hidden_places();
        if !hidden.is_empty() && !items.is_empty() {
            items.push(MenuItem::separator());
        }
        for (builtin, place) in hidden {
            items.push(
                MenuItem::action(otto_kit::t_owned!(
                    "files-sidebar-show",
                    place = place.label
                ))
                .with_action_id(format!("{SHOW_PREFIX}{}", builtin.id())),
            );
        }
        items
    }

    /// Write one edit to the sidebar config, then show what it says now.
    fn edit_sidebar(&mut self, edit: impl FnOnce(&str) -> Result<String, toml_edit::TomlError>) {
        if let Err(err) = places_config::edit_file(edit) {
            tracing::warn!(%err, "cannot update the sidebar config");
            self.status = Some(otto_kit::t_owned!(
                "files-sidebar-save-failed",
                error = err.to_string()
            ));
        }
        self.reload_places();
        self.dirty = true;
    }

    /// Read the sidebar again, keeping the lit row on the folder it led to.
    ///
    /// Called after this window's own edits and whenever `files.toml`
    /// changes, which is how an edit in one window, in Settings or in an
    /// editor reaches every window already open.
    pub(super) fn reload_places(&mut self) {
        if self.trash || self.desk {
            return;
        }
        self.set_places(model::places());
    }

    /// Show `next` as the sidebar.
    pub(super) fn set_places(&mut self, next: Vec<Place>) {
        if next == self.places {
            return;
        }
        let lit = self
            .active_place
            .and_then(|index| self.places.get(index))
            .map(|place| place.path.clone());
        self.active_place = lit.and_then(|path| next.iter().position(|place| place.path == path));
        self.menu_place = None;
        self.places = next;
        // A drag still hovering points at a gap or a row that has moved.
        self.drop_target = None;
        self.dirty = true;
    }
}
