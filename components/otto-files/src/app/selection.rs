//! Selecting entries: clicks, ranges, everything, and the rubber band.

use super::*;

impl Browser {
    /// Select `index` in column `depth`, replacing whatever was selected.
    ///
    /// If it is a directory, push a column for it; if not, truncate the stack
    /// so nothing stale hangs to the right.
    pub(super) fn select(&mut self, depth: usize, index: usize) {
        self.select_at(depth, index, true);
    }

    /// [`Self::select`], with the Back entry optional.
    ///
    /// A delete that moves the selection to the next row lands here without
    /// one: the user did not navigate anywhere, and a Back step that returned
    /// to a listing holding a file that no longer exists would be a step into
    /// nothing.
    pub(super) fn select_at(&mut self, depth: usize, index: usize, history: bool) {
        if depth >= self.columns.len() {
            return;
        }
        let entry = match self.visible(depth).get(index) {
            Some(e) => (*e).clone(),
            None => return,
        };

        // A directory descent is a real navigation, worth a Back entry;
        // recorded here, before the stack changes underneath it.
        let descending = entry.is_dir && self.mode == ViewMode::Columns;
        if descending && history {
            self.record_location();
        }

        let column = &mut self.columns[depth];
        column.selection.clear();
        column.selection.insert(entry.selection_key());
        column.cursor = Some(index);
        column.anchor = Some(index);

        self.active = depth;
        self.columns.truncate(depth + 1);

        // Clicking a file in a Save dialog puts its name in the field. That
        // is how a user says "overwrite this one" without retyping it, and it
        // is why the replace confirmation exists at all. A directory is a
        // place to go, not a name to save under, so it leaves the field be.
        if !entry.is_dir {
            if let Some(input) = self.save_name.as_mut() {
                input.set_value(entry.name.clone());
            }
        }

        // Only Miller view reveals a directory's contents on a plain select —
        // that eager next pane is the point of the view. List and Grid show
        // one directory at a time, so selecting there must not also swap it
        // out from under the click; opening is `open_selection`'s job there
        // (a double-click, or Return), the same as a file.
        if entry.is_dir && self.mode == ViewMode::Columns {
            self.columns.push(Column::new(entry.path.clone()));
            self.reveal_pane(depth + 1);
        }
        self.dirty = true;
    }

    /// Clear one pane's selection — what a click on nothing means.
    ///
    /// The pane still becomes the active one: the click was in it, and the
    /// keyboard should follow. In Miller view the panes to its right go too.
    /// They are there because something in this one was selected, and now
    /// nothing is; leaving them up would show a child of no parent.
    pub(super) fn clear_pane_selection(&mut self, depth: usize) {
        if depth >= self.columns.len() {
            return;
        }
        self.active = depth;
        self.clear_selection();
        if self.mode == ViewMode::Columns {
            self.columns.truncate(depth + 1);
        }
    }

    /// Start dragging a rubber band out from `(x, y)` — a press on the empty
    /// part of the icon grid.
    ///
    /// `additive` (Ctrl or Shift held) keeps what was already selected and
    /// adds to it. Without it the press has already cleared the pane, which is
    /// what makes a band that catches nothing — a plain click — mean nothing
    /// selected.
    pub(super) fn begin_marquee(&mut self, depth: usize, x: f32, y: f32, additive: bool) {
        if self.mode != ViewMode::Grid || depth >= self.columns.len() {
            return;
        }
        let scroll = self.columns[depth].scroll.offset();
        let base = if additive {
            self.columns[depth].selection.clone()
        } else {
            Default::default()
        };
        self.active = depth;
        self.marquee = Some(Marquee {
            depth,
            anchor: (x, y + scroll),
            cursor: (x, y + scroll),
            base,
        });
        self.dirty = true;
    }

    /// Follow the pointer, and reselect everything the band now covers.
    ///
    /// Recomputed rather than accumulated: an entry the band has moved off
    /// leaves the selection again, unless it was in `base`.
    pub(super) fn update_marquee(&mut self, x: f32, y: f32) -> bool {
        let Some(depth) = self.marquee.as_ref().map(|m| m.depth) else {
            return false;
        };
        let Some(scroll) = self.columns.get(depth).map(|c| c.scroll.offset()) else {
            return false;
        };

        let (band, mut selection) = {
            let marquee = self.marquee.as_mut().unwrap();
            marquee.cursor = (x, y + scroll);
            (marquee.rect(), marquee.base.clone())
        };
        // The band is drawn from its cursor, so it repaints as the pointer
        // moves whether or not what it covers changes.
        self.dirty = true;

        // The band is in content coordinates, so the hit test is asked about
        // the unscrolled grid: `grid_cell_rect(area, i, 0.0)` is where cell `i`
        // sits in that same space.
        let area = view::content_viewport(self.size.0, self.size.1, ViewMode::Grid);
        let keys: Vec<String> = self
            .visible(depth)
            .iter()
            .map(|e| e.selection_key())
            .collect();
        let caught =
            view::grid_cells_in_rect_in(area, &self.recent_sections, keys.len(), 0.0, band);

        let last = caught.last().copied();
        for index in caught {
            selection.insert(keys[index].clone());
        }

        let column = &mut self.columns[depth];
        if column.selection == selection && column.cursor == last {
            return false;
        }
        column.selection = selection;
        column.cursor = last;
        column.anchor = last;
        self.dirty = true;
        true
    }

    /// The band as it should be drawn: window coordinates, or `None` when no
    /// band is out.
    pub(super) fn marquee_band(&self) -> Option<skia_safe::Rect> {
        let marquee = self.marquee.as_ref()?;
        let scroll = self.columns.get(marquee.depth)?.scroll.offset();
        let mut band = marquee.rect();
        band.offset((0.0, -scroll));
        Some(band)
    }

    /// Add or remove one entry, leaving the rest of the selection alone —
    /// Ctrl+click. The cursor and the anchor both move to it, so a following
    /// Shift+click ranges from here.
    pub(super) fn toggle_select(&mut self, depth: usize, index: usize) {
        if depth >= self.columns.len() {
            return;
        }
        let Some(key) = self.visible(depth).get(index).map(|e| e.selection_key()) else {
            return;
        };
        let column = &mut self.columns[depth];
        if !column.selection.remove(&key) {
            column.selection.insert(key);
        }
        column.cursor = Some(index);
        column.anchor = Some(index);
        self.active = depth;
        // A multi-selection has no single child, so nothing hangs to the right.
        self.columns.truncate(depth + 1);
        self.dirty = true;
    }

    /// Select the contiguous run from the anchor to `index` — Shift+click and
    /// Shift+Arrow. Replaces the previous range rather than accumulating, so
    /// dragging the far end back shrinks it.
    pub(super) fn extend_select(&mut self, depth: usize, index: usize) {
        if depth >= self.columns.len() {
            return;
        }
        let keys: Vec<String> = self
            .visible(depth)
            .iter()
            .map(|e| e.selection_key())
            .collect();
        if index >= keys.len() {
            return;
        }
        let anchor = self.columns[depth]
            .anchor
            .unwrap_or(index)
            .min(keys.len() - 1);
        let (lo, hi) = if anchor <= index {
            (anchor, index)
        } else {
            (index, anchor)
        };

        let column = &mut self.columns[depth];
        column.selection.clear();
        for key in &keys[lo..=hi] {
            column.selection.insert(key.clone());
        }
        column.cursor = Some(index);
        self.active = depth;
        self.columns.truncate(depth + 1);
        self.dirty = true;
    }

    pub(super) fn select_all(&mut self) {
        let depth = self.active;
        let keys: Vec<String> = self
            .visible(depth)
            .iter()
            .map(|e| e.selection_key())
            .collect();
        let column = &mut self.columns[depth];
        column.selection = keys.into_iter().collect();
        self.columns.truncate(depth + 1);
        self.dirty = true;
    }

    pub(super) fn clear_selection(&mut self) {
        let column = &mut self.columns[self.active];
        column.selection.clear();
        column.cursor = None;
        column.anchor = None;
        self.dirty = true;
    }

    /// What `org.otto.Files1.FocusedSelection` answers: the selected
    /// entries' paths while the window holds the keyboard, and `None` while
    /// it does not. A folder being viewed with nothing selected in it is not
    /// a selection.
    pub(super) fn focused_selection(&self, focused: bool) -> Option<Vec<String>> {
        focused.then(|| {
            crate::files_service::wire_paths(
                true,
                self.selected_entries().into_iter().map(|entry| entry.path),
            )
        })
    }

    /// Every selected entry in the active column, in view order.
    pub(super) fn selected_entries(&self) -> Vec<Entry> {
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let selection = &self.columns[depth].selection;
        self.visible(depth)
            .into_iter()
            .filter(|e| selection.contains(&e.selection_key()))
            .cloned()
            .collect()
    }
}
