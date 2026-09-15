//! What a pane lists: its entries sorted, filtered and counted.

use super::*;

impl Browser {
    /// The entries of column `depth`, filtered and sorted for display.
    ///
    /// Rebuilt per frame rather than cached: at the sizes a window shows this
    /// is cheap, and a cache is one more thing to invalidate when the directory
    /// changes underneath. The moment it stops being cheap it moves to the
    /// model thread, which is where the spec puts it.
    pub(super) fn visible(&self, depth: usize) -> Vec<&Entry> {
        let _t = perf::now();
        let entries = self.visible_uncounted(depth);
        perf::mark(perf::Stage::Visible, _t);
        entries
    }

    /// How many entries the column shows, without materialising the list.
    ///
    /// `counts()` and the subtitle only ever wanted the length, and building
    /// a `Vec` of twenty-five thousand references to throw away is the kind
    /// of per-frame cost that is invisible until the directory is big.
    pub(super) fn visible_len(&self, depth: usize) -> usize {
        self.ensure_sorted(depth);
        self.columns[depth].sorted.borrow().order.len()
    }

    /// Bring the column's cached order in line with the listing and the
    /// current sort settings, recomputing only if one of them moved.
    pub(super) fn ensure_sorted(&self, depth: usize) {
        let column = &self.columns[depth];
        // The picker's filter is part of the key: picking a different one
        // re-filters in place, with no filesystem access, and picking the
        // same one re-uses the order already computed.
        let filter = self.picker.as_ref().map(|p| p.current_filter).unwrap_or(0);
        let key = (
            column.epoch,
            self.sort,
            self.ascending,
            self.show_hidden,
            filter,
        );
        if column.sorted.borrow().key == Some(key) {
            return;
        }
        let entries = &column.snapshot.entries;
        let mut order: Vec<usize> = (0..entries.len())
            .filter(|&i| self.show_hidden || !entries[i].hidden)
            .filter(|&i| match &self.picker {
                Some(session) => session.shows(&entries[i].name, entries[i].is_dir),
                None => true,
            })
            .collect();
        order.sort_by(|&a, &b| self.compare(&entries[a], &entries[b]));
        *column.sorted.borrow_mut() = model::SortCache {
            key: Some(key),
            order,
        };
    }

    pub(super) fn visible_uncounted(&self, depth: usize) -> Vec<&Entry> {
        self.ensure_sorted(depth);
        let column = &self.columns[depth];
        let entries = &column.snapshot.entries;
        column
            .sorted
            .borrow()
            .order
            .iter()
            .map(|&i| &entries[i])
            .collect()
    }

    /// The order two entries sort in, under the current settings.
    pub(super) fn compare(&self, a: &Entry, b: &Entry) -> std::cmp::Ordering {
        let dirs_first = b.is_dir.cmp(&a.is_dir);
        if dirs_first != std::cmp::Ordering::Equal {
            return dirs_first;
        }
        let ord = match self.sort {
            SortKey::Name => model::natural_cmp(&a.name, &b.name),
            SortKey::Size => a.size.unwrap_or(0).cmp(&b.size.unwrap_or(0)),
            SortKey::Kind => a
                .kind_label()
                .cmp(b.kind_label())
                .then_with(|| model::natural_cmp(&a.name, &b.name)),
            SortKey::Modified => a.modified.cmp(&b.modified),
        };
        if self.ascending {
            ord
        } else {
            ord.reverse()
        }
    }

    pub(super) fn counts(&self) -> Vec<usize> {
        (0..self.columns.len())
            .map(|d| self.visible_len(d))
            .collect()
    }
}
