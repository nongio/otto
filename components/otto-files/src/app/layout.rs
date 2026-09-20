//! Chrome geometry and hit tests: the footer, the path bar, panes and entries.

use super::*;

impl Browser {
    /// Switch view, taking the new view's default sort unless the user has
    /// pinned one of their own by clicking a column header.
    pub(super) fn set_mode(&mut self, mode: ViewMode) {
        // Recent is a grid or it is nothing — see `Frame::mode_locked`.
        if self.recent && mode != ViewMode::Grid {
            self.refuse(otto_kit::t_owned!("files-recent-grid-only"));
            return;
        }
        // Results have no hierarchy for Miller columns to show, but they read
        // perfectly well as either a list or a grid.
        if self.searching && mode == ViewMode::Columns {
            self.refuse(otto_kit::t_owned!("files-search-no-columns"));
            return;
        }
        self.mode = mode;
        if !self.sort_pinned {
            let (sort, ascending) = Self::default_sort(mode);
            self.sort = sort;
            self.ascending = ascending;
        }
        self.dirty = true;
    }

    /// The sort a view opens with. List shows the modified column, and the
    /// answer it is usually asked for is "what changed most recently", so it
    /// leads with newest first; the other views sort by name.
    pub(super) fn default_sort(mode: ViewMode) -> (SortKey, bool) {
        match mode {
            ViewMode::List => (SortKey::Modified, false),
            ViewMode::Grid | ViewMode::Columns => (SortKey::Name, true),
        }
    }

    /// How much of the window height the action row takes — zero in the
    /// browser, which has none.
    pub(super) fn footer_h(&self) -> f32 {
        match &self.picker {
            // Save mode stacks a name row on top of the buttons; the buttons
            // themselves stay anchored to the window bottom, so every rect
            // below the name row is unchanged by this.
            Some(session) if session.request.mode.names_a_file() => {
                view::FOOTER_H + view::FOOTER_NAME_H
            }
            Some(_) => view::FOOTER_H,
            None => 0.0,
        }
    }

    /// How much of the window height the path bar takes.
    ///
    /// Zero in the picker, whose bottom edge belongs to the action row, and
    /// zero in the Trash, where every path would spell out the same stretch
    /// of `.local/share/Trash` — the Original-location column already says
    /// the thing a user of that window wants to know.
    pub(super) fn path_bar_h(&self) -> f32 {
        if self.picker.is_some() || self.trash {
            0.0
        } else {
            view::PATH_BAR_H
        }
    }

    /// The bottom of the *file area*, which is the window bottom less the
    /// chrome under it — the path bar, and the picker's action row. Every
    /// piece of geometry in [`view`] takes this as its `height`, so none of
    /// it has to know either one exists.
    pub(super) fn content_h(&self) -> f32 {
        self.size.1 - self.footer_h() - self.path_bar_h()
    }

    /// What a scroll repaints in the window, when that is less than all of
    /// it: the file area of a list or a grid, with nothing laid over it. The
    /// column stack scrolls and pans on surfaces of its own and has nothing
    /// here to repaint, and an overlay scrolls — or covers — on its own terms,
    /// so neither has an area to name.
    pub(super) fn scroll_damage(&self) -> Option<Rect> {
        let plain = self.mode != ViewMode::Columns
            && self.peek.is_none()
            && self.palette.is_none()
            && self.rename.is_none();
        plain.then(|| view::content_viewport(self.size.0, self.content_h(), self.mode))
    }

    /// Whether a scroll is presented by the columns' own surfaces, leaving the
    /// window nothing to repaint for it. The scroll still has to be stepped —
    /// that is the update loop's job — but not by repainting the window.
    pub(super) fn scroll_on_surfaces(&self) -> bool {
        self.mode == ViewMode::Columns
            && self.peek.is_none()
            && self.palette.is_none()
            && self.rename.is_none()
    }

    /// What the path bar spells out: the one thing selected in the active
    /// column, or the column's own directory when nothing — or a crowd — is
    /// selected. Carries the leaf's icon chain and whether it is a directory,
    /// both of which the bar needs and neither of which the path itself says.
    pub(super) fn path_bar_target(&self) -> Option<(PathBuf, Vec<String>, bool)> {
        let depth = self.active.min(self.columns.len() - 1);
        let column = &self.columns[depth];
        if column.selection.len() == 1 {
            if let Some(key) = column.selection.iter().next() {
                if let Some(entry) = column
                    .snapshot
                    .entries
                    .iter()
                    .find(|e| &e.selection_key() == key)
                {
                    return Some((entry.path.clone(), entry.icon_chain(), entry.is_dir));
                }
            }
        }
        // Recent and search results have no directory of their own: the
        // sentinel standing in for one is not a path, and spelling it out
        // would be worse than saying nothing. Their bar fills in as soon as a
        // row is selected, which is the case it is there for.
        (!self.is_synthetic()).then(|| (column.path.clone(), vec!["folder".to_string()], true))
    }

    /// That path, one crumb per component, root first.
    ///
    /// The home directory is named the way the sidebar names it rather than by
    /// the login it is called on disk — a trail reading `/ › home › riccardo`
    /// is the truth, but "Home" is the answer to the question the bar is
    /// asking.
    pub(super) fn path_crumbs(&self) -> Vec<view::PathCrumb> {
        if self.path_bar_h() == 0.0 {
            return Vec::new();
        }
        let Some((target, leaf_icon, leaf_is_dir)) = self.path_bar_target() else {
            return Vec::new();
        };
        crumbs_for(
            &target,
            leaf_icon,
            leaf_is_dir,
            model::home_dir().as_deref(),
        )
    }

    /// The crumb under `(x, y)` and where it leads, if it leads anywhere.
    pub(super) fn path_crumb_target(&self, x: f32, y: f32) -> Option<PathBuf> {
        let crumbs = self.path_crumbs();
        let index = view::path_crumb_at(x, y, &crumbs, self.size.0, self.size.1, self.footer_h())?;
        Some(crumbs[index].path.clone())
    }

    /// Which crumb the pointer is over, for the hover lighting.
    pub(super) fn path_crumb_hovered(&self, x: f32, y: f32) -> Option<usize> {
        let crumbs = self.path_crumbs();
        view::path_crumb_at(x, y, &crumbs, self.size.0, self.size.1, self.footer_h())
    }

    /// Which sidebar row is lit.
    ///
    /// The row that was clicked, while the window is still showing what it
    /// led to — that is the only way to tell two rows naming the same folder
    /// apart. Arriving anywhere else, the first row whose path matches, which
    /// is what lights Documents when you walk into it from Home.
    pub(super) fn selected_place(&self) -> Option<usize> {
        // A search is nowhere: results and their sentinel path are not a
        // place, and matching on it lit Recent as soon as anyone typed.
        if self.searching {
            return None;
        }
        let here = &self.columns[0].path;
        if let Some(index) = self.active_place {
            if self.places.get(index).is_some_and(|p| &p.path == here) {
                return Some(index);
            }
        }
        self.places.iter().position(|p| &p.path == here)
    }

    /// The entry under a point, if any — the same hit test the left-click
    /// handler uses for the current view mode. `None` means empty space: the
    /// background, a gap between rows, or (in List view) the header.
    pub(super) fn entry_at(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        let (width, height) = (self.size.0, self.content_h());
        match self.mode {
            ViewMode::Grid => {
                let depth = self.columns.len() - 1;
                let count = self.visible_len(depth);
                let scroll = self.columns[depth].scroll.offset();
                let area = view::content_viewport(width, height, ViewMode::Grid);
                view::grid_cell_at_in(area, &self.recent_sections, x, y, count, scroll)
                    .map(|i| (depth, i))
            }
            ViewMode::List => {
                let depth = self.columns.len() - 1;
                let count = self.visible_len(depth);
                let scroll = self.columns[depth].scroll.offset();
                view::row_at(x, y, width, height, count, scroll).map(|i| (depth, i))
            }
            ViewMode::Columns => {
                let counts = self.counts();
                match view::miller_at(
                    x,
                    y,
                    width,
                    height,
                    &self.columns,
                    &counts,
                    self.pan.offset(),
                    self.miller_w,
                ) {
                    Some((depth, Some(index))) => Some((depth, index)),
                    _ => None,
                }
            }
        }
    }

    /// Where entry `index` of pane `depth` sits in the window right now.
    pub(super) fn entry_rect(&self, depth: usize, index: usize) -> Rect {
        let (width, height) = (self.size.0, self.content_h());
        let count = self.visible_len(depth);
        let scroll = self.columns[depth].scroll.offset();

        match self.mode {
            ViewMode::Grid => view::grid_cell_rect_in(
                view::content_viewport(width, height, ViewMode::Grid),
                &self.recent_sections,
                index,
                scroll,
            ),
            ViewMode::List => view::list_row_rect(width, count, index, scroll),
            ViewMode::Columns => view::miller_row_rect(
                depth,
                height,
                self.pan.offset(),
                self.miller_w,
                count,
                index,
                scroll,
            ),
        }
    }
}
