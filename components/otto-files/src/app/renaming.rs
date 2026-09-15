//! Renaming an entry in place.

use super::*;

impl Browser {
    /// Start editing the cursor entry's name in place — Return's job, the way
    /// it is Finder's, in every view mode.
    pub(super) fn start_rename(&mut self) {
        if self.rename.is_some() {
            return;
        }
        if self.is_synthetic() {
            self.refuse_synthetic();
            return;
        }
        // Renaming a trashed file would rewrite the name its sidecar is
        // keyed on, and Put Back would then have nothing to read.
        if self.trash {
            self.refuse(otto_kit::t_owned!("files-trash-cant-rename"));
            return;
        }
        let depth = self.active;
        let Some(index) = self.columns[depth].cursor else {
            return;
        };
        let Some(entry) = self.visible(depth).get(index).map(|e| (*e).clone()) else {
            return;
        };
        let theme = AppContext::current_theme();
        let selection = rename_selection(&entry.name, entry.is_dir);
        let mut input = TextInput::editing(entry.name, view::rename_field_style(theme));
        input.state.select_range(selection);
        self.rename = Some(RenameSession {
            depth,
            index,
            original: entry.path,
            input,
        });
        self.dirty = true;
    }

    /// Apply the field's text as the new name, if it actually changed.
    pub(super) fn commit_rename(&mut self) {
        let Some(session) = self.rename.take() else {
            return;
        };
        let new_name = session.input.value().trim().to_string();
        let old_name = session
            .original
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if new_name.is_empty() || new_name == old_name {
            self.dirty = true;
            return;
        }
        // The failure is reported in the status line rather than raised: the
        // field is already gone, so there is nothing left to fix in place.
        if let Err(error) = self.rename_path(session.depth, &session.original, &new_name) {
            self.status = Some(error);
            self.dirty = true;
        }
    }

    /// Rename `original` to `new_name`, and leave the selection on it.
    ///
    /// The one place a rename actually happens — the in-place field and the
    /// command palette both come through here, so they cannot drift apart over
    /// what a rename records on the undo stack.
    pub(super) fn rename_path(
        &mut self,
        depth: usize,
        original: &Path,
        new_name: &str,
    ) -> Result<(), String> {
        let new_name = new_name.trim();
        let old_name = original
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if new_name.is_empty() || new_name == old_name {
            self.dirty = true;
            return Ok(());
        }
        if new_name.contains('/') {
            return Err(otto_kit::t_owned!("files-name-invalid"));
        }
        let target = original.with_file_name(new_name);
        std::fs::rename(original, &target)
            .map_err(|err| otto_kit::t_owned!("files-rename-failed", error = err.to_string()))?;
        if let Some(column) = self.columns.get_mut(depth) {
            column.selection.clear();
            // The renamed file, by its new path: the key moves with the file,
            // so remembering the old one would leave the selection pointing at
            // something that is not there.
            column
                .selection
                .insert(target.to_string_lossy().into_owned());
        }
        self.status = Some(otto_kit::t_owned!("files-renamed-to", name = new_name));
        self.record_undo(
            otto_kit::t!("files-undo-rename"),
            vec![model::Change::Moved {
                from: original.to_path_buf(),
                to: target,
            }],
        );
        self.reload_all();
        self.dirty = true;
        Ok(())
    }

    /// Discard the field's text and leave the file as it was.
    pub(super) fn cancel_rename(&mut self) {
        if self.rename.take().is_some() {
            self.dirty = true;
        }
    }
}
