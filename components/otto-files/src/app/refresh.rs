//! Keeping up with the disk: loads landing, watches and renames.

use super::*;

impl Browser {
    /// Poll every column's worker. Returns whether anything landed.
    ///
    /// A freshly loaded column starts with nothing selected — no eager pick
    /// of its first entry. `move_cursor` already lands Down's first press on
    /// index 0 from an empty cursor, so there is nowhere that needs one. The
    /// exception is a pane the keyboard has already stepped into, which is
    /// what `take_entering` is for.
    pub(super) fn poll(&mut self) -> bool {
        let mut changed = false;
        let mut refreshed = false;
        let mut vanished = None;
        for depth in 0..self.columns.len() {
            if self.columns[depth].poll() {
                changed = true;
            }
            if std::mem::take(&mut self.columns[depth].refreshed) {
                refreshed = true;
            }
            // The shallowest one wins: everything below it is inside it, so
            // it is gone too.
            if self.columns[depth].gone && vanished.is_none() {
                vanished = Some(depth);
            }
        }
        if refreshed {
            self.resync_cursors();
            if self.take_pending_rename() {
                changed = true;
            }
        }
        if let Some(depth) = vanished {
            self.follow_vanished(depth);
            changed = true;
        }
        if self.take_entering() {
            changed = true;
        }
        if self.poll_job() {
            changed = true;
        }
        // What a provider's earlier runs — a script, most likely — have
        // finished with, applied the same way as a run that answered at once.
        for landed in self.commands.poll() {
            match landed {
                Ok(effect) => self.apply_effect(effect),
                Err(reason) => self.status = Some(reason),
            }
            changed = true;
        }
        changed
    }

    /// Put the cursor on the first row of a pane the keyboard stepped into
    /// while it was still being read — see [`Browser::entering`].
    ///
    /// Dropped the moment the read finishes, whatever it found, and dropped
    /// unused if the user has moved on in the meantime: the request belongs to
    /// one press, and a stale one would move a cursor nobody asked it to move.
    pub(super) fn take_entering(&mut self) -> bool {
        let Some(depth) = self.entering else {
            return false;
        };
        if depth < self.columns.len() && self.columns[depth].loading() {
            return false;
        }
        self.entering = None;
        if depth >= self.columns.len() || self.active != depth {
            return false;
        }
        if self.columns[depth].cursor.is_some() || self.visible(depth).is_empty() {
            return false;
        }
        self.select(depth, 0);
        true
    }

    /// Select the folder `new_folder` just created and open its rename field,
    /// now that the listing holding it has arrived.
    ///
    /// One shot: the request is dropped on the first refresh either way, so a
    /// folder that was renamed or removed before the read came back does not
    /// leave a rename armed for the next unrelated re-read.
    pub(super) fn take_pending_rename(&mut self) -> bool {
        let Some((depth, name)) = self.pending_rename.take() else {
            return false;
        };
        if depth >= self.columns.len() {
            return false;
        }
        let Some(index) = self.visible(depth).iter().position(|e| e.name == name) else {
            return false;
        };
        self.select(depth, index);
        // Sorting can put "untitled folder" anywhere, including past the fold
        // of a long directory. `resync_cursors` deliberately does not scroll —
        // a change somebody else made must not move the view — but this one is
        // the user's own action, and a rename field they cannot see is worse
        // than useless: the next keystroke would go into it unseen.
        //
        // The metrics have to be refreshed first. They are otherwise a frame
        // old — from before this listing landed, when the column was one row
        // shorter — and the scroll view would clamp the reveal short of a
        // folder that sorted to the very bottom.
        self.sync_scroll_metrics();
        self.reveal_cursor();
        self.start_rename();
        true
    }

    /// Put every cursor back on what is selected, after a listing was replaced
    /// underneath it.
    ///
    /// The selection is by key and survives; the cursor is an index into the
    /// visible order and does not, so a file appearing above the selection
    /// would otherwise leave the keyboard one row off from the highlight.
    /// Nothing is scrolled: a change somebody else made must not move the
    /// view out from under the person reading it.
    pub(super) fn resync_cursors(&mut self) {
        for depth in 0..self.columns.len() {
            let Some(first) = self.columns[depth].selection.iter().next().cloned() else {
                continue;
            };
            let index = self
                .visible(depth)
                .iter()
                .position(|e| e.selection_key() == first);
            self.columns[depth].cursor = index;
            self.columns[depth].anchor = index;
        }
    }

    /// A displayed directory was deleted or moved away: go to the nearest
    /// ancestor that still exists, and say why.
    pub(super) fn follow_vanished(&mut self, depth: usize) {
        let path = self.columns[depth].path.clone();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());

        if depth > 0 {
            // Its parent is on screen already — drop it and everything under
            // it, and leave the parent showing.
            self.columns.truncate(depth);
            self.active = self.columns.len() - 1;
            self.reveal_pane(self.active);
        } else {
            let home = model::home_dir().unwrap_or_else(|| PathBuf::from("/"));
            let surviving = path
                .ancestors()
                .skip(1)
                .find(|p| p.is_dir())
                .map(|p| p.to_path_buf())
                .unwrap_or(home);
            self.columns = vec![Column::new(surviving)];
            self.active = 0;
            self.pan.scroll_to(0.0);
        }
        self.status = Some(otto_kit::t_owned!("files-gone", name = name.as_str()));
        self.dirty = true;
    }

    pub(super) fn loading(&self) -> bool {
        self.columns.iter().any(|c| c.loading())
    }
}
