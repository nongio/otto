//! Recent and search: the listings that are not a directory.

use super::*;

impl Browser {
    /// Show the Recent listing: what was written most recently across the
    /// user's folders, newest first, under a heading per day.
    ///
    /// Not a navigation. The column stack is replaced by a single pane holding
    /// a listing with no directory behind it, and the location — the path bar,
    /// the location field, Back and Forward — is left out of it entirely. The
    /// way out is clicking another place.
    ///
    /// The scan starts with the pane: [`Column::recent`] hands the request to
    /// [`crate::search`], and the tiles arrive in batches while the window is
    /// already up.
    pub(super) fn show_recent(&mut self) {
        if self.recent {
            return;
        }
        self.recent = true;
        self.path_entry = None;
        self.rename = None;
        self.marquee = None;
        // The grid is the whole point — a wall of thumbnails scanned by eye —
        // and Miller columns mean nothing without a hierarchy. The sort is the
        // view's premise rather than a preference, so it is set here and the
        // control that would change it is disabled.
        self.mode = ViewMode::Grid;
        self.sort = SortKey::Modified;
        self.ascending = false;
        self.sort_pinned = false;
        self.columns = vec![Column::recent()];
        self.active = 0;
        self.rebuild_recent_sections();
        self.dirty = true;
    }

    /// Go to Recent from wherever the window is, as a navigation: the place
    /// being left is recorded behind Back, and a search that was up comes
    /// down — results are not somewhere Recent can sit on top of. The one
    /// way in for the sidebar and the palette alike.
    pub(super) fn enter_recent(&mut self) {
        if self.recent {
            return;
        }
        self.record_location();
        self.close_search();
        self.show_recent();
    }

    /// Leave the Recent listing, on the way to somewhere real.
    ///
    /// Clears the flag without navigating: the caller is about to, and doing
    /// both here would read the directory twice.
    pub(super) fn leave_recent(&mut self) {
        if !self.recent {
            return;
        }
        self.recent = false;
        self.recent_sections = view::GridSections::default();
        let (sort, ascending) = Self::default_sort(self.mode);
        self.sort = sort;
        self.ascending = ascending;
    }

    /// Rebuild the day sections from the listing as it is now ordered.
    ///
    /// Driven off the *visible* order rather than the snapshot, so the
    /// headings describe the tiles actually under them however the listing has
    /// been filtered. Cheap enough to redo per frame at the sizes this view
    /// deals in; the real version will key it off the column's epoch.
    pub(super) fn rebuild_recent_sections(&mut self) {
        if !self.recent {
            if !self.recent_sections.is_flat() {
                self.recent_sections = view::GridSections::default();
            }
            return;
        }
        let now = std::time::SystemTime::now();
        let buckets: Vec<crate::recent::Bucket> = self
            .visible(0)
            .iter()
            .map(|e| crate::recent::bucket_of(e.modified, now))
            .collect();

        let mut sections: Vec<view::GridSection> = Vec::new();
        for (index, bucket) in buckets.iter().enumerate() {
            match sections.last_mut() {
                // A run continues only while the bucket is the same one, so a
                // bucket that somehow reappeared later would open a second
                // heading rather than be folded into the first. That cannot
                // happen under a date sort, and drawing it wrong if it ever
                // did would be worse than drawing the heading twice.
                Some(last) if last.header.as_deref() == Some(bucket.label()) => {
                    last.count += 1;
                }
                _ => sections.push(view::GridSection {
                    header: Some(bucket.label().to_string()),
                    first: index,
                    count: 1,
                }),
            }
        }
        let sections = view::GridSections(sections);
        if self.recent_sections != sections {
            self.recent_sections = sections;
            self.dirty = true;
        }
    }

    // -- Search --------------------------------------------------------------

    /// Whether this window is showing a listing with no directory behind it —
    /// Recent, or search results.
    ///
    /// Every command that acts on a *place* rather than on a file asks this:
    /// there is nowhere to paste into, nothing for Ctrl+L to resolve against,
    /// and no hierarchy for Miller columns to show.
    pub(super) fn is_synthetic(&self) -> bool {
        self.recent || self.searching
    }

    /// Open the search field and put the caret in it. A second Ctrl+F closes
    /// it, the way Ctrl+L toggles the path entry.
    ///
    /// Except when the strip is open but has given the keyboard back to the
    /// listing — clicked away from, to go and look at what was found. Ctrl+F
    /// then means what it meant the first time: put the caret back in the
    /// query. Closing a strip whose results are on screen, when the ring is
    /// not even lit, would be answering a question nobody asked.
    pub(super) fn toggle_search(&mut self) {
        if let Some(input) = self.search.as_mut() {
            if input.state.focused() {
                self.clear_search();
            } else {
                input.state.set_focused(true);
                input.state.select_all();
                self.dirty = true;
            }
            return;
        }
        self.path_entry = None;
        self.rename = None;
        self.search_origin = Some(self.current_path());
        self.search_where = self.title();
        self.search = Some(TextInput::editing(
            String::new(),
            view::search_field_style(AppContext::current_theme()),
        ));
        // Opening the strip moves everything below it down, so the geometry
        // has to know before anything is measured or hit-tested.
        view::set_search_band(true);
        self.dirty = true;
    }

    /// Switch which haystack the query runs against, keeping the query.
    pub(super) fn set_search_scope(&mut self, scope: model::SearchScope) {
        if self.search_scope == scope {
            return;
        }
        self.search_scope = scope;
        self.run_search();
        self.dirty = true;
    }

    /// Hand the keyboard from the query back to the listing, leaving the
    /// strip, the query and the results exactly where they are.
    ///
    /// The two ways out of the field — clicking a result, or arrowing down
    /// into one — mean the same thing and have to do the same thing, which is
    /// why they share this rather than each blurring the field their own way.
    /// Nothing here closes the search: what you found is still on screen, and
    /// Ctrl+F or a click in the field comes back to it.
    ///
    /// Answers whether the keyboard actually moved, so a caller that was
    /// already in the listing does not ask for a repaint that draws the same
    /// frame.
    pub(super) fn blur_search(&mut self) -> bool {
        let moved = self
            .search
            .as_ref()
            .is_some_and(|input| input.state.focused());
        if moved {
            if let Some(input) = self.search.as_mut() {
                input.state.set_focused(false);
            }
            self.dirty = true;
        }
        moved
    }

    /// Put the field away and the listing back.
    ///
    /// Restoring rather than staying put is the point of `search_origin`: an
    /// abandoned search is not a navigation, and leaving the window on a
    /// results set with no field to explain it would be worse than either.
    pub(super) fn clear_search(&mut self) {
        let origin = self
            .search_origin
            .clone()
            .or_else(model::home_dir)
            .unwrap_or_else(|| PathBuf::from("/"));
        if self.searching {
            self.leave_synthetic_to(&origin);
        } else {
            self.close_search();
        }
    }

    /// Take the search down without going anywhere, and say whether results
    /// were on screen.
    ///
    /// The half of [`Self::clear_search`] that every *other* way out of a
    /// search wants: clicking a place in the sidebar, or a folder in the path
    /// bar, is a navigation to somewhere chosen, and restoring the folder the
    /// search began in first would read a directory only to throw it away —
    /// and would leave that detour in the history behind the Back button.
    ///
    /// The same shape as [`Self::leave_recent`], and for the same reason: the
    /// caller is about to navigate, and doing it here as well would do it
    /// twice.
    pub(super) fn close_search(&mut self) -> bool {
        self.search = None;
        self.search_where = String::new();
        self.search_scope = model::SearchScope::default();
        self.search_origin = None;
        view::set_search_band(false);
        self.dirty = true;
        std::mem::take(&mut self.searching)
    }

    /// Put a remembered search back on screen and ask it again.
    ///
    /// The results are re-run rather than restored: an index answers about the
    /// disk as it is, and stepping back into a search to be shown files that
    /// have since been renamed away would be a worse answer than the one it
    /// gave the first time.
    pub(super) fn enter_search(
        &mut self,
        query: String,
        scope: model::SearchScope,
        origin: Option<PathBuf>,
        label: String,
    ) {
        self.path_entry = None;
        self.rename = None;
        self.search_origin = origin;
        self.search_where = label;
        self.search_scope = scope;
        self.search = Some(TextInput::editing(
            query,
            view::search_field_style(AppContext::current_theme()),
        ));
        view::set_search_band(true);
        self.run_search();
    }

    /// The query as it stands, trimmed. `None` when the field is closed or has
    /// nothing worth searching for.
    pub(super) fn query(&self) -> Option<String> {
        let text = self.search.as_ref()?.value().trim().to_string();
        (!text.is_empty()).then_some(text)
    }

    /// Rebuild the results from the query.
    ///
    /// Both scopes are the same question put to the desktop's index, differing
    /// only in how much of the disk they let it answer from:
    /// [`SearchScope::Folder`] narrows it to the folder the search was opened
    /// in and everything below, [`SearchScope::Everywhere`] to the whole of
    /// home. Neither looks at what happens to be on screen — a filter over the
    /// visible rows would find nothing in the subfolders, which is most of
    /// what "this folder" means to the person asking.
    ///
    /// An empty query puts the listing the search started from back.
    pub(super) fn run_search(&mut self) {
        let Some(query) = self.query() else {
            // The field was emptied, which is itself something to show.
            self.dirty = true;
            if self.searching {
                let origin = self
                    .search_origin
                    .clone()
                    .or_else(model::home_dir)
                    .unwrap_or_else(|| PathBuf::from("/"));
                self.searching = false;
                self.navigate_to(&origin);
            }
            return;
        };

        // Each run starts from scratch and abandons whatever the last one
        // started, so a query refined while the first is still out costs the
        // answer to the second alone.
        let request = match self.search_scope {
            model::SearchScope::Folder => {
                let dir = self
                    .search_origin
                    .clone()
                    .unwrap_or_else(|| self.current_path());
                crate::search::Request::folder(query, dir)
            }
            model::SearchScope::Everywhere => crate::search::Request::everywhere(query),
        };
        let column = Column::searching(request);

        if !self.searching {
            // Only the way in. A second query is the same page asking a
            // different question, and stacking one history entry per Return
            // would make Back a way to walk your own typing backwards.
            self.record_location();
        }
        self.searching = true;
        self.recent = false;
        self.recent_sections = view::GridSections::default();
        // Grid or List, but never Miller: results have no hierarchy. Whichever
        // of the two the user was in is kept.
        if self.mode == ViewMode::Columns {
            self.mode = ViewMode::Grid;
        }
        self.columns = vec![column];
        self.active = 0;
        self.dirty = true;
    }

    /// Refuse something a listing with no directory behind it cannot do.
    ///
    /// The files are real, but the pane is not a folder: there is nothing to
    /// paste beside, no location for Ctrl+L to resolve against, and no
    /// directory for a new folder to appear in. Every command that acts on the
    /// *place* lands here.
    ///
    /// Commands that act on a file do not: previewing it, and opening it, are
    /// about the file alone and work wherever it was found. Opening a *folder*
    /// found this way leaves the listing rather than descending inside it —
    /// see [`Self::leave_synthetic_to`].
    pub(super) fn refuse_synthetic(&mut self) {
        self.refuse(otto_kit::t_owned!("files-synthetic-no-action"));
    }
}
