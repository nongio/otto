//! File operations: copy, paste, trash, undo, new folders and sorting.

use super::*;

impl Browser {
    /// Select every entry in the active pane whose name matches `pattern`.
    ///
    /// Matched against the *visible* listing rather than the directory, so a
    /// pattern selects what is on screen: hidden files stay out of it unless
    /// they are being shown, and a filtered listing narrows what can be
    /// picked. That is what someone typing at a listing means by it.
    pub(super) fn select_matching(&mut self, pattern: &str) -> Result<(), String> {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return Err(otto_kit::t_owned!("files-no-pattern"));
        }
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let matched: Vec<(usize, String)> = self
            .visible(depth)
            .into_iter()
            .enumerate()
            .filter(|(_, entry)| {
                // Case-sensitive when the pattern carries case, the way the
                // picker's filters read: `*.png` finds `PHOTO.PNG`, and
                // `*.PNG` means the shouty one.
                if pattern.chars().any(|c| c.is_ascii_uppercase()) {
                    otto_kit::filetype::glob::matches(pattern, &entry.name)
                } else {
                    otto_kit::filetype::glob::matches_ignore_case(pattern, &entry.name)
                }
            })
            .map(|(index, entry)| (index, entry.selection_key()))
            .collect();

        if matched.is_empty() {
            return Err(otto_kit::t_owned!(
                "files-nothing-matches",
                pattern = pattern
            ));
        }
        let count = matched.len();
        let first = matched[0].0;
        let column = &mut self.columns[depth];
        column.selection.clear();
        for (_, key) in matched {
            column.selection.insert(key);
        }
        // The cursor goes to the first match so the selection can be walked
        // from somewhere, and the anchor with it so a following Shift+Arrow
        // extends from there rather than from wherever the cursor last was.
        column.cursor = Some(first);
        column.anchor = Some(first);
        self.reveal_cursor();
        self.status = Some(otto_kit::t_owned!(
            "files-selected-count",
            count = count as f64
        ));
        self.dirty = true;
        Ok(())
    }

    /// Put the selection into a folder of its own, made for it.
    ///
    /// One undo entry covers both halves, and the folder is recorded *before*
    /// the moves so that taking it back walks them in the right order: the
    /// files come out first, and the folder — empty again — goes last.
    ///
    /// Given no name the folder takes the default one and lands in rename, the
    /// way New Folder does; given one it is created with it.
    pub(super) fn new_folder_with_selection(&mut self, name: &str) -> Result<(), String> {
        if self.trash {
            return Err(otto_kit::t_owned!("files-trash-cant-rename"));
        }
        if self.is_synthetic() {
            return Err(otto_kit::t_owned!("files-recent-not-a-folder"));
        }
        let paths: Vec<PathBuf> = self
            .selected_entries()
            .into_iter()
            .map(|entry| entry.path)
            .collect();
        if paths.is_empty() {
            return Err(otto_kit::t_owned!("files-nothing-selected"));
        }

        let name = name.trim();
        let dest = self.current_directory();
        let folder = if name.is_empty() {
            model::create_folder(&dest)?
        } else {
            model::create_folder_named(&dest, name)?
        };

        let clip = model::Clipboard { paths, cut: true };
        let result = model::paste(&clip, &folder, model::OnConflict::KeepBoth);

        // Nothing made it in: take the folder away again rather than leaving
        // an empty one behind as the only trace of a command that failed.
        if result.changes.is_empty() {
            let _ = std::fs::remove_dir(&folder);
            return Err(result
                .errors
                .first()
                .cloned()
                .unwrap_or_else(|| otto_kit::t_owned!("files-nothing-selected")));
        }

        let mut changes = vec![model::Change::Created {
            path: folder.clone(),
        }];
        changes.extend(result.changes.iter().cloned());
        self.report(&result);
        self.record_undo(
            otto_kit::t!("files-undo-new-folder-with-selection"),
            changes,
        );

        let folder_name = folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        // The re-read is off-thread, so the folder is not in the listing yet.
        // Named without a name of its own goes into rename, the way the
        // toolbar's New Folder does; named outright is already what was asked
        // for, and is only selected.
        if name.is_empty() {
            self.pending_rename = Some((depth, folder_name));
        } else {
            self.pending_pick = Some((depth, Some(folder_name)));
        }
        self.reload_all();
        self.dirty = true;
        Ok(())
    }

    /// Move the selection into `path`.
    ///
    /// The move a cut and paste would make, said in one go: the same
    /// conflict rule, the same sound, the same undo entry.
    pub(super) fn move_selection_to(&mut self, path: &str) -> Result<(), String> {
        let paths: Vec<PathBuf> = self
            .selected_entries()
            .into_iter()
            .map(|entry| entry.path)
            .collect();
        if paths.is_empty() {
            return Err(otto_kit::t_owned!("files-nothing-selected"));
        }
        let dest = self
            .resolve_typed_path(path)
            .filter(|dest| dest.is_dir())
            .ok_or_else(|| otto_kit::t_owned!("files-no-such-folder", path = path.trim()))?;
        // Moving a folder into itself, or into its own child, would take the
        // whole subtree somewhere it cannot be reached from.
        if paths.iter().any(|from| dest.starts_with(from)) {
            return Err(otto_kit::t_owned!("files-cant-move-into-itself"));
        }

        let clip = model::Clipboard { paths, cut: true };
        // Keep Both, like a paste with no sheet to ask with: the only default
        // that cannot destroy anything.
        let result = model::paste(&clip, &dest, model::OnConflict::KeepBoth);
        self.report(&result);
        self.record_undo(otto_kit::t!("files-undo-move"), result.changes);
        self.reload_all();
        Ok(())
    }

    /// Sort by `key`, in the direction that key reads best in — and remember
    /// that the choice was the user's, so a view change does not overwrite it.
    pub(super) fn set_sort(&mut self, key: SortKey) {
        if self.sort == key {
            self.ascending = !self.ascending;
        } else {
            self.sort = key;
            self.ascending = key != SortKey::Modified;
        }
        self.sort_pinned = true;
        self.dirty = true;
    }

    /// Rename the cursor entry outright, without the in-place field. Takes the
    /// same undo entry an in-place rename does.
    pub(super) fn rename_cursor_to(&mut self, name: &str) -> Result<(), String> {
        if self.trash {
            return Err(otto_kit::t_owned!("files-trash-cant-rename"));
        }
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let index = self.columns[depth]
            .cursor
            .ok_or_else(|| otto_kit::t_owned!("files-palette-no-matches"))?;
        let original = self
            .visible(depth)
            .get(index)
            .map(|entry| entry.path.clone())
            .ok_or_else(|| otto_kit::t_owned!("files-palette-no-matches"))?;
        self.rename_path(depth, &original, name)
    }

    /// Create a folder with the name the user gave, or — given nothing — the
    /// default name the toolbar's New Folder uses, which lands in rename.
    pub(super) fn new_folder_named(&mut self, name: &str) -> Result<(), String> {
        if name.is_empty() {
            self.new_folder();
            return Ok(());
        }
        let dest = self.current_directory();
        let created = model::create_folder_named(&dest, name)?;
        let name = created
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.status = Some(otto_kit::t_owned!(
            "files-new-folder-created",
            name = name.as_str()
        ));
        self.record_undo(
            otto_kit::t!("files-new-folder"),
            vec![model::Change::Created { path: created }],
        );
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        self.pending_pick = Some((depth, Some(name)));
        self.reload_all();
        self.dirty = true;
        Ok(())
    }

    /// Put the selection on the system clipboard.
    ///
    /// A cut only *marks*: nothing moves until the paste, so an abandoned cut
    /// costs nothing and cannot lose a file.
    ///
    /// `serial` must be from a real input event — the compositor refuses a
    /// selection claimed without one.
    /// Turn down a command the Trash does not have, saying why in the status
    /// line. Refusing silently would read as the window being broken.
    pub(super) fn refuse(&mut self, why: String) {
        self.status = Some(why);
        self.dirty = true;
    }

    pub(super) fn copy_selection(&mut self, cut: bool, serial: u32) {
        // A cut in the Trash would offer a paste that moves a file out of it
        // without dropping its sidecar, leaving a phantom row behind. Put
        // Back is the way out of the Trash.
        if self.trash {
            return;
        }
        let paths: Vec<PathBuf> = self
            .selected_entries()
            .into_iter()
            .map(|e| e.path)
            .collect();
        if paths.is_empty() {
            return;
        }
        let count = paths.len();

        // Kept locally as well as offered: the local copy is what draws the
        // cut entries dimmed, and it means a paste back into this window does
        // not have to round-trip through the compositor.
        self.clipboard = model::Clipboard {
            paths: paths.clone(),
            cut,
        };

        let claimed = clipboard::set(clipboard::file_payloads(&paths, cut), serial);
        self.status = Some(if claimed {
            format!(
                "{} {count} item{} to paste",
                if cut { "Cut" } else { "Copied" },
                if count == 1 { "" } else { "s" }
            )
        } else {
            // Say so rather than pretending: the files are still pasteable
            // here, just not anywhere else.
            format!(
                "{} {count} item{} (this window only)",
                if cut { "Cut" } else { "Copied" },
                if count == 1 { "" } else { "s" }
            )
        });
        self.dirty = true;
    }

    /// Paste into the directory being viewed.
    ///
    /// Runs on the calling thread today, which is the UI thread — acceptable
    /// only because it is a deliberate keystroke on a known selection, not
    /// something that happens while scrolling. The spec puts this on the worker
    /// pool with progress and cancellation, and that is the next change; the
    /// `OpResult` it returns is already the shape that path reports.
    pub(super) fn paste(&mut self) {
        // A synthetic listing is not a folder: there is nowhere in it to put
        // anything.
        if self.is_synthetic() {
            self.refuse(otto_kit::t_owned!("files-recent-not-a-folder"));
            return;
        }
        // Pasting into the Trash would put files there with no sidecar
        // saying where they came from — items that can never be put back.
        // Trashing is how a file gets in; that path writes the sidecar.
        if self.trash {
            return;
        }
        // The system clipboard wins over our own copy: if another application
        // has copied since, that is what the user means by "paste", and our
        // local clipboard is stale.
        let from_system = clipboard::first_available(clipboard::file_mime_preference())
            .and_then(|mime| clipboard::read(&mime).map(|bytes| (mime, bytes)))
            .map(|(mime, bytes)| clipboard::parse_file_payload(&mime, &bytes))
            .filter(|(paths, _)| !paths.is_empty());

        let clip = match from_system {
            Some((paths, cut)) => model::Clipboard { paths, cut },
            None => self.clipboard.clone(),
        };
        if clip.is_empty() {
            return;
        }
        let dest = self.columns[self.active].path.clone();

        // Keep Both rather than Replace: without a conflict sheet to ask with,
        // the only safe default is the one that cannot destroy anything.
        let result = model::paste(&clip, &dest, model::OnConflict::KeepBoth);

        // A cut is consumed by its paste; a copy stays available to paste again.
        if clip.cut && result.errors.is_empty() {
            self.clipboard = model::Clipboard::default();
        }

        let summary = result.summary();
        self.status = (!summary.is_empty()).then_some(summary);
        Self::play_op_sound(&result);
        self.record_undo(
            if clip.cut {
                otto_kit::t!("files-undo-move")
            } else {
                otto_kit::t!("files-undo-copy")
            },
            result.changes,
        );
        self.reload_all();
        self.dirty = true;
    }

    /// Say out loud what an operation did.
    ///
    /// Chosen from the outcome rather than from the command, which is what
    /// makes undo sound right without a special case: undoing a delete is a
    /// restore, and a restore moves files back into place, so it gets the
    /// arriving sound. Undoing a copy takes files away, and gets the other
    /// one. An operation that did nothing stays quiet.
    ///
    /// Destroying is heard apart from deleting, and putting back apart from
    /// pasting, because those are the pairs a user tells apart by ear: the
    /// delete that can be taken back sounds different from the one that
    /// cannot, and the sound that answers a delete is the put-back.
    ///
    /// Ordered most specific first. An operation counts under one heading in
    /// practice, but a mixed outcome should be named by the part that cannot
    /// be undone.
    pub(super) fn play_op_sound(result: &model::OpResult) {
        if result.deleted > 0 {
            otto_kit::sound::play_first(&SOUND_DESTROYED);
        } else if result.trashed > 0 {
            otto_kit::sound::play_first(&SOUND_REMOVED);
        } else if result.restored > 0 {
            otto_kit::sound::play_first(&SOUND_RESTORED);
        } else if result.moved + result.copied > 0 {
            otto_kit::sound::play_first(&SOUND_ARRIVED);
        }
    }

    /// Put `result`'s changes on the undo stack, if it changed anything.
    ///
    /// Called with every operation's outcome rather than only the clean ones:
    /// a paste that half worked still moved real files, and those are exactly
    /// the ones a user reaches for Ctrl+Z about.
    pub(super) fn record_undo(&mut self, label: &'static str, changes: Vec<model::Change>) {
        if changes.is_empty() {
            return;
        }
        self.undo.push(UndoStep { label, changes });
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
    }

    /// Ctrl+Z — take back the last operation that changed files.
    pub(super) fn undo_last(&mut self) {
        let Some(step) = self.undo.pop() else {
            self.status = Some(otto_kit::t_owned!("files-nothing-to-undo"));
            self.dirty = true;
            return;
        };

        let result = model::undo(&step.changes);
        Self::play_op_sound(&result);
        self.status = Some(if result.errors.is_empty() {
            otto_kit::t_owned!("files-undid", label = step.label)
        } else {
            result.errors[0].clone()
        });
        // Deliberately not pushed back on as a redo: an undo of a delete is a
        // restore, and re-deleting it would be a second trip to the Trash
        // rather than the inverse of anything. Redo is its own feature.
        self.reload_all();
        self.dirty = true;
    }

    /// Move the current selection to Trash.
    ///
    /// The selection does not go with it: it moves to the row that takes the
    /// deleted one's place, so a run of deletes is one key held down rather
    /// than a delete-then-reach-for-the-mouse each time. Which row that is has
    /// to be decided *here*, against the listing still on screen — once the
    /// re-read lands there is nothing left to measure the gap from.
    pub(super) fn move_selected_to_trash(&mut self) {
        if self.is_synthetic() {
            self.refuse_synthetic();
            return;
        }
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let paths: Vec<PathBuf> = self
            .selected_entries()
            .into_iter()
            .map(|e| e.path)
            .collect();
        if paths.is_empty() {
            return;
        }
        let successor = self.successor_after_delete(depth);
        let result = model::move_to_trash(&paths);
        self.report(&result);
        self.record_undo(otto_kit::t!("files-undo-delete"), result.changes);
        // By key, before the re-read: the selection is held by key, so this
        // is already the right answer for `resync_cursors` when the listing
        // lands. `settle_pick` then does the rest — the child column a newly
        // selected directory wants, or the walk out of a folder left empty.
        self.hand_selection_to(successor);
    }

    /// Say what an operation did and play its sound. The one place a result
    /// turns into something the user can see, so no operation can quietly
    /// half-fail.
    pub(super) fn report(&mut self, result: &model::OpResult) {
        let summary = result.summary();
        self.status = (!summary.is_empty()).then_some(summary);
        Self::play_op_sound(result);
        self.dirty = true;
    }

    /// What Delete means here.
    ///
    /// In the browser it is a trip to the Trash, which is undoable and so
    /// needs no question. In the Trash there is nowhere further to send a
    /// file, so the same key destroys it — and that one is asked about first,
    /// every time, because nothing can put it back.
    pub(super) fn delete_key(&mut self) {
        if self.trash {
            self.ask_delete_forever();
        } else {
            self.move_selected_to_trash();
        }
    }

    /// Put the Trash selection back where each item came from.
    ///
    /// The selection moves on exactly the way a delete's does: restoring is a
    /// row leaving the listing, and holding the button down through a run of
    /// them should not need the mouse between each.
    pub(super) fn put_back_selection(&mut self) {
        if !self.trash {
            return;
        }
        let paths: Vec<PathBuf> = self
            .selected_entries()
            .into_iter()
            .map(|e| e.path)
            .collect();
        if paths.is_empty() {
            return;
        }
        let successor = self.successor_after_delete(0);
        let result = model::restore_from_trash(&paths);
        self.report(&result);
        self.record_undo(otto_kit::t!("files-put-back"), result.changes);
        self.hand_selection_to(successor);
    }

    /// Ask before destroying the Trash selection. Nothing happens here — the
    /// sheet's affirmative button is what deletes.
    pub(super) fn ask_delete_forever(&mut self) {
        if !self.trash {
            return;
        }
        let entries = self.selected_entries();
        if entries.is_empty() {
            return;
        }
        let message = if entries.len() == 1 {
            otto_kit::t_owned!("files-delete-forever-one", name = entries[0].name.as_str())
        } else {
            otto_kit::t_owned!("files-delete-forever-many", count = entries.len() as i64)
        };
        self.confirm = Some(ConfirmSheet {
            message,
            detail: otto_kit::t_owned!("files-delete-forever-detail"),
            accept_label: otto_kit::t_owned!("common-delete"),
            action: ConfirmAction::DeleteForever(entries.into_iter().map(|e| e.path).collect()),
            pressed: None,
        });
        self.dirty = true;
    }

    /// Carry out a confirmed permanent delete.
    pub(super) fn destroy(&mut self, paths: Vec<PathBuf>) {
        let successor = self.successor_after_delete(0);
        let result = model::delete_forever(&paths);
        self.report(&result);
        // Deliberately not recorded: there is nothing to put back.
        self.hand_selection_to(successor);
    }

    /// Ask before emptying the Trash.
    pub(super) fn ask_empty_trash(&mut self) {
        let count = self.visible(0).len();
        if !self.trash || count == 0 {
            return;
        }
        self.confirm = Some(ConfirmSheet {
            message: otto_kit::t_owned!("files-empty-trash-confirm"),
            detail: otto_kit::t_owned!("files-empty-trash-detail", count = count as i64),
            accept_label: otto_kit::t_owned!("files-empty-trash"),
            action: ConfirmAction::EmptyTrash,
            pressed: None,
        });
        self.dirty = true;
    }

    /// Hand the selection to `successor` once the re-read lands, and start it.
    ///
    /// Shared by every operation that takes rows out of the listing — a
    /// trash, a Put Back, a permanent delete — so all three leave the cursor
    /// in the same place and carry Quick View along in the same way.
    pub(super) fn hand_selection_to(&mut self, successor: Option<String>) {
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let column = &mut self.columns[depth];
        column.selection.clear();
        column.cursor = None;
        column.anchor = None;
        if let Some(key) = successor.clone() {
            column.selection.insert(key);
        }
        self.pending_pick = Some((depth, successor));
        self.reload_all();
        self.dirty = true;
    }

    /// Which entry should hold the selection once everything selected in pane
    /// `depth` is gone: the first survivor below the deleted block, and
    /// failing that the nearest one above it. `None` when the pane is being
    /// emptied outright.
    pub(super) fn successor_after_delete(&self, depth: usize) -> Option<String> {
        let doomed = &self.columns[depth].selection;
        let entries = self.visible(depth);
        let last = entries
            .iter()
            .rposition(|e| doomed.contains(&e.selection_key()))?;
        entries
            .iter()
            .skip(last + 1)
            .find(|e| !doomed.contains(&e.selection_key()))
            .or_else(|| {
                entries[..last]
                    .iter()
                    .rev()
                    .find(|e| !doomed.contains(&e.selection_key()))
            })
            .map(|e| e.selection_key())
    }

    /// Land the selection a delete set aside, once the re-read has arrived.
    ///
    /// Doing it through `select_at` rather than by hand is what keeps Miller
    /// view consistent: a directory taking the selection gets its child column
    /// the same way a click on it would. A pane left with nothing in it hands
    /// the keyboard back to its parent, where the folder itself is selected —
    /// there is nowhere else in an empty directory to stand.
    pub(super) fn settle_pick(&mut self) {
        if self.pending_pick.is_none() || self.loading() {
            return;
        }
        let Some((depth, key)) = self.pending_pick.take() else {
            return;
        };
        if depth >= self.columns.len() {
            return;
        }
        // Quick View is anchored to the cursor, and the cursor is about to
        // move off a file that no longer exists. Whichever way this lands —
        // on the survivor, or on nothing at all — the panel has to be told.
        self.quickview_follow = self.quickview.is_some();

        let index = key.as_deref().and_then(|key| {
            self.visible(depth)
                .iter()
                .position(|e| e.selection_key() == key)
        });
        if let Some(index) = index {
            self.select_at(depth, index, false);
            self.reveal_cursor();
            self.dirty = true;
            return;
        }
        if !self.visible(depth).is_empty() || depth == 0 {
            return;
        }
        // Empty, and there is a parent to go back to. Miller keeps the empty
        // pane on screen — it is the folder the parent has selected, and the
        // stack shows what is selected; the other two views show one directory
        // at a time, so the emptied one is dropped instead.
        if self.mode != ViewMode::Columns {
            self.columns.truncate(depth);
        }
        self.active = depth - 1;
        self.reveal_pane(self.active);
        self.dirty = true;
    }

    /// Create "untitled folder" in the active pane and start renaming it in
    /// place, the way Finder and Explorer's New Folder both do.
    pub(super) fn new_folder(&mut self) {
        if self.trash {
            return;
        }
        if self.is_synthetic() {
            self.refuse(otto_kit::t_owned!("files-recent-not-a-folder"));
            return;
        }
        let dest = self.columns[self.active].path.clone();
        match model::create_folder(&dest) {
            Ok(path) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                self.record_undo(
                    otto_kit::t!("files-new-folder"),
                    vec![model::Change::Created { path: path.clone() }],
                );
                self.reload_all();
                // The re-read is off-thread: the folder is not in the column's
                // listing yet, so the selection and the rename field wait for
                // it in `poll`.
                self.pending_rename = Some((self.active, name.clone()));
                self.status = Some(otto_kit::t_owned!(
                    "files-new-folder-created",
                    name = name.as_str()
                ));
            }
            Err(err) => {
                self.status = Some(otto_kit::t_owned!(
                    "files-new-folder-failed",
                    error = err.to_string()
                ));
            }
        }
        self.dirty = true;
    }

    /// Re-read **every** column in the stack, keeping the user where they are.
    ///
    /// Not just the active one: a paste changes the destination *and* — for a
    /// cut — the directory the files came from, which is a different column and
    /// usually still on screen. Reloading only the destination leaves the source
    /// column listing files that are no longer there.
    ///
    /// Re-reading is in place: a column keeps its selection, cursor and scroll
    /// while only its listing is replaced. The watcher would get here on its
    /// own within a debounce, but an operation the user just performed should
    /// not visibly lag behind their own hand.
    pub(super) fn reload_all(&mut self) {
        for column in &mut self.columns {
            column.reload();
        }
    }
}
