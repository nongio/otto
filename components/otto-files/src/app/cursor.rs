//! The keyboard cursor and type-ahead.

use super::*;

impl Browser {
    /// How far one Up/Down press moves. In the grid that is a whole row of
    /// cells — the arrows walk the grid in two dimensions, so vertical motion
    /// crosses a row and Left/Right steps one cell — and one entry everywhere
    /// else, where the listing is a single column.
    pub(super) fn row_step(&self) -> i32 {
        if self.mode != ViewMode::Grid {
            return 1;
        }
        if let Some(step) = self.overflow_panel_row_step() {
            return step;
        }
        let area = view::content_viewport(self.size.0, self.content_h(), ViewMode::Grid);
        view::grid_columns_in(area, &self.recent_sections) as i32
    }

    /// Move the cursor within the active column. With `extend`, the selection
    /// grows from the anchor instead of being replaced.
    pub(super) fn move_cursor(&mut self, delta: i32, extend: bool) {
        let count = self.visible(self.active).len();
        if count == 0 {
            return;
        }
        // With nothing selected, the first press should land the cursor on an
        // end, whatever the step: Down's obvious first stop is index 0, not
        // one grid row in.
        // On a desk with an overflow tile the keyboard stops at the tile, or
        // stays in the overflow panel while that is open.
        let (low, high) = self
            .desk_cursor_range()
            .map_or((0, count - 1), |range| (*range.start(), *range.end()));
        let next = match self.columns[self.active].cursor {
            Some(cursor) => (cursor as i32 + delta).clamp(low as i32, high as i32) as usize,
            None if delta >= 0 => low,
            None => high,
        };
        self.move_cursor_to(next, extend);
    }

    /// Put the cursor on `next`, selecting it, or extending the selection to
    /// it with `extend`, and scroll it into view.
    pub(super) fn move_cursor_to(&mut self, next: usize, extend: bool) {
        if extend {
            self.extend_select(self.active, next);
        } else {
            self.select(self.active, next);
        }
        self.reveal_cursor();
    }

    /// Move the cursor to the first entry whose name starts with the
    /// type-ahead buffer, case-insensitively.
    ///
    /// Nothing is filtered and nothing is drawn: the only sign it happened is
    /// the selection moving, which is the whole point of the gesture —
    /// reaching a file in a long directory without leaving the keyboard.
    /// The buffer expires after a second of silence, so the next burst of
    /// typing starts a fresh name rather than extending a stale one.
    ///
    /// Repeating one character with nothing in between cycles through the
    /// entries beginning with it, rather than looking for a doubled letter
    /// that almost no name has.
    pub(super) fn typeahead(&mut self, ch: char) {
        const EXPIRY: std::time::Duration = std::time::Duration::from_secs(1);

        let names: Vec<String> = self
            .visible(self.active)
            .iter()
            .map(|entry| entry.name.to_lowercase())
            .collect();
        if names.is_empty() {
            return;
        }

        let now = std::time::Instant::now();
        let live = self
            .typeahead
            .take()
            .filter(|(_, last)| now.duration_since(*last) < EXPIRY)
            .map(|(buffer, _)| buffer);
        let typed = ch.to_lowercase().to_string();
        let cycling = live.as_deref() == Some(typed.as_str());
        let buffer = match live {
            Some(buffer) if cycling => buffer,
            Some(mut buffer) => {
                buffer.push_str(&typed);
                buffer
            }
            None => typed,
        };

        // Cycling resumes just past the cursor and wraps; a buffer that grew
        // answers from the top, so the same keys always land on the same file.
        let from = match (cycling, self.columns[self.active].cursor) {
            (true, Some(cursor)) => cursor + 1,
            _ => 0,
        };
        let hit = (0..names.len())
            .map(|step| (from + step) % names.len())
            .find(|&index| names[index].starts_with(&buffer));

        // Kept even when nothing matched: the miss is part of the word being
        // typed, and dropping it would make the next character search for a
        // prefix the user never asked for.
        self.typeahead = Some((buffer, now));
        if let Some(index) = hit {
            self.select(self.active, index);
            self.reveal_cursor();
        }
    }

    /// Scroll the active pane the shortest distance that brings the cursor
    /// fully into view, so walking a long directory with the arrow keys does
    /// not leave the selection behind the edge of the viewport.
    ///
    /// Metrics come from the last [`Self::sync_scroll_metrics`], which runs at
    /// the top of every frame — a key press always follows one.
    pub(super) fn reveal_cursor(&mut self) {
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let Some(index) = self.columns[depth].cursor else {
            return;
        };
        // A desk with an overflow tile never scrolls its grid; the overflow
        // panel scrolls instead.
        if self.reveal_in_desk_overflow(index) {
            return;
        }
        let (width, height) = (self.size.0, self.content_h());
        let viewport = view::pane_viewport(
            width,
            height,
            self.mode,
            depth,
            self.pan.offset(),
            &self.miller_widths(),
        );
        if viewport.is_empty() {
            return;
        }
        let (top, item_h) = view::item_span_in(
            width,
            height,
            self.mode,
            &self.recent_sections,
            &self.photos,
            index,
        );

        let scroll = &mut self.columns[depth].scroll;
        let offset = scroll.offset();
        let target = if top < offset {
            top
        } else if top + item_h > offset + viewport.height() {
            top + item_h - viewport.height()
        } else {
            return;
        };
        // A fling still in the air would undo this on the next tick.
        scroll.stop();
        if scroll.state.set_offset(target) {
            self.dirty = true;
        }
    }

    /// Left/right in Miller view: out of a column, or into its child.
    pub(super) fn move_lateral(&mut self, delta: i32) {
        if self.mode != ViewMode::Columns {
            if delta < 0 {
                self.go_up();
            } else {
                self.descend_selection();
            }
            return;
        }
        if delta < 0 {
            if self.active > 0 {
                self.active -= 1;
                self.reveal_pane(self.active);
                self.dirty = true;
            }
        } else {
            self.descend_selection();
        }
    }

    /// Ctrl+O: open the entry at the cursor.
    ///
    /// A folder opens here, the way a double-click opens it: Files is the
    /// file manager, and handing a folder to the desktop's would at best open
    /// another window of this one. Anything else goes to its type's default
    /// application (see `open_file`), and pulses: Ctrl+O is a deliberate ask
    /// with no click to acknowledge it, and whatever answers can take a moment
    /// to appear. Same in every view.
    pub(super) fn open_cursor_entry(&mut self) {
        // The picker answers one request in one window: a second window would
        // have nothing to do with the request, and no way to answer it.
        if self.picker.is_some() {
            self.open_selection();
            return;
        }

        let depth = self.active;
        let Some(index) = self.columns[depth].cursor else {
            return;
        };
        let Some(entry) = self.visible(depth).get(index).map(|e| (*e).clone()) else {
            return;
        };
        if entry.is_dir {
            self.open_selection();
            return;
        }

        self.opening = Some((depth, std::time::Instant::now()));
        self.opening_stage = None;
        self.dirty = true;
        self.open_file(&entry.path);
    }

    /// Go *into* the selection, and only that: a directory is descended, and
    /// anything else is left alone.
    ///
    /// An arrow key is navigation. Opening a file hands it to another
    /// application, which is a thing to ask for deliberately — with a
    /// double-click or Ctrl+O — not something a cursor key should do on its way
    /// across a listing.
    pub(super) fn descend_selection(&mut self) {
        let depth = self.active;
        let is_dir = self.columns[depth]
            .cursor
            .and_then(|index| self.visible(depth).get(index).map(|entry| entry.is_dir))
            .unwrap_or(false);
        if is_dir {
            self.open_selection();
        }
    }

    /// The entry the cursor is on, if any.
    pub(super) fn selected_entry(&self) -> Option<Entry> {
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let index = self.columns[depth].cursor?;
        self.visible(depth).get(index).map(|e| (*e).clone())
    }
}
