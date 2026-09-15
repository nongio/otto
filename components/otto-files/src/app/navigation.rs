//! Where the browser is: locations, history and the typed path.

use super::*;

impl Browser {
    /// The directory the window is "at": the deepest column, or the selected
    /// directory within it.
    pub(super) fn current_path(&self) -> PathBuf {
        self.columns
            .last()
            .map(|c| c.path.clone())
            .unwrap_or_default()
    }

    /// The column stack as a [`Location`], for the Back/Forward pair.
    pub(super) fn location(&self) -> Location {
        Location {
            columns: self
                .columns
                .iter()
                .map(|c| ColumnState {
                    path: c.path.clone(),
                    selection: c.selection.clone(),
                    cursor: c.cursor,
                    anchor: c.anchor,
                })
                .collect(),
            active: self.active,
            synthetic: if self.searching {
                Some(Synthetic::Search {
                    query: self.query().unwrap_or_default(),
                    scope: self.search_scope,
                    origin: self.search_origin.clone(),
                    label: self.search_where.clone(),
                })
            } else if self.recent {
                Some(Synthetic::Recent)
            } else {
                None
            },
        }
    }

    /// Record where the browser is now, before a navigation moves it
    /// somewhere else — Back's undo point. Any Forward history is dropped:
    /// once the user branches off by navigating anew, the old "future" no
    /// longer applies, the same rule a web browser follows.
    pub(super) fn record_location(&mut self) {
        self.back.push(self.location());
        self.forward.clear();
    }

    /// Replace the column stack with a remembered one, as Back and Forward
    /// both do.
    pub(super) fn restore_location(&mut self, location: Location) {
        // Whatever is up now is being left, whichever kind of listing the one
        // arriving turns out to be.
        self.close_search();
        self.leave_recent();
        match location.synthetic {
            Some(Synthetic::Recent) => {
                self.show_recent();
                return;
            }
            Some(Synthetic::Search {
                query,
                scope,
                origin,
                label,
            }) => {
                self.enter_search(query, scope, origin, label);
                return;
            }
            None => {}
        }
        self.columns = location
            .columns
            .into_iter()
            .map(|state| {
                let mut column = Column::new(state.path);
                column.selection = state.selection;
                column.cursor = state.cursor;
                column.anchor = state.anchor;
                column
            })
            .collect();
        if self.columns.is_empty() {
            self.columns.push(Column::new(self.current_path()));
        }
        self.active = location.active.min(self.columns.len() - 1);
        self.pan.scroll_to(0.0);
        self.reveal_pane(self.active);
        self.pending_restore = true;
        self.dirty = true;
    }

    /// Finish a Back/Forward step once its directories have been read.
    ///
    /// The selection is held by key, so it survives the reload untouched;
    /// the cursor is an index, so it is re-derived from that selection rather
    /// than trusted — a file added or removed while the user was away would
    /// otherwise leave the keyboard one row off from the highlight. Then the
    /// restored row is scrolled back into view, which needs the metrics of
    /// the listing that just landed.
    /// Ask the Empty Trash question once the listing it counts has landed.
    pub(super) fn settle_empty_ask(&mut self) {
        if !self.pending_empty_ask || self.loading() {
            return;
        }
        self.pending_empty_ask = false;
        // An already-empty can has nothing to ask about: the window is left
        // open saying so, which is the answer.
        self.ask_empty_trash();
    }

    pub(super) fn settle_restore(&mut self) {
        if !self.pending_restore || self.loading() {
            return;
        }
        self.pending_restore = false;
        for depth in 0..self.columns.len() {
            let Some(first) = self.columns[depth].selection.iter().next().cloned() else {
                continue;
            };
            let index = self
                .visible(depth)
                .iter()
                .position(|e| e.selection_key() == first);
            if let Some(index) = index {
                self.columns[depth].cursor = Some(index);
                self.columns[depth].anchor = Some(index);
            }
        }
        self.reveal_cursor();
        self.dirty = true;
    }

    /// The nav arrow under `(x, y)`, and only when it has somewhere to go: a
    /// half with an empty history is drawn dimmed, and a dimmed control must
    /// not light up under a press either.
    /// Which Trash header action is under `(x, y)`, if this is the Trash
    /// window and that button has anything to act on.
    pub(super) fn trash_action_at(&self, x: f32, y: f32) -> Option<view::TrashAction> {
        let chrome = view::TrashChrome {
            can_put_back: !self.columns[0].selection.is_empty(),
            can_empty: !self.visible(0).is_empty(),
            pressed: None,
        };
        self.trash
            .then(|| view::trash_action_at(x, y, self.size.0, &chrome))
            .flatten()
    }

    pub(super) fn nav_button_at(&self, x: f32, y: f32) -> Option<view::NavButton> {
        let button = view::nav_button_at(x, y)?;
        let live = match button {
            view::NavButton::Back => !self.back.is_empty(),
            view::NavButton::Forward => !self.forward.is_empty(),
        };
        live.then_some(button)
    }

    /// Step to the previous location, if there is one.
    pub(super) fn go_back(&mut self) {
        let Some(location) = self.back.pop() else {
            return;
        };
        self.forward.push(self.location());
        self.restore_location(location);
    }

    /// Step to the location Back left, if there is one.
    pub(super) fn go_forward(&mut self) {
        let Some(location) = self.forward.pop() else {
            return;
        };
        self.back.push(self.location());
        self.restore_location(location);
    }

    /// Go to the parent directory.
    pub(super) fn go_up(&mut self) {
        // A synthetic listing has no parent: the sentinel's is a path nobody
        // asked for, and going there would carry Recent's flag along with it.
        if self.is_synthetic() {
            self.refuse(otto_kit::t_owned!("files-recent-no-location"));
            return;
        }
        if self.columns.len() > 1 {
            self.record_location();
            self.columns.truncate(self.columns.len() - 1);
            self.active = self.columns.len() - 1;
            self.reveal_pane(self.active);
            self.dirty = true;
            return;
        }
        // At the root of the stack: re-root one level up.
        let path = self.columns[0].path.clone();
        if let Some(parent) = path.parent() {
            self.record_location();
            self.columns = vec![Column::new(parent.to_path_buf())];
            self.active = 0;
            self.pan.scroll_to(0.0);
            self.dirty = true;
        }
    }

    /// Replace the whole stack, as clicking a place does.
    ///
    /// Whatever synthetic listing is up — Recent, or search results — comes
    /// down with the pane it was in. The two are one state and have to leave
    /// together: the pane's search dies with it (see [`crate::search::Search`]'s
    /// `Drop`), so no stale batch can land, but a `recent` or `searching`
    /// flag left standing would dress the folder up as the listing it
    /// replaced the moment its read arrives — Recent's title, day headings
    /// and locked grid over a home directory, and every place-bound command
    /// still refusing for want of a location. Every route into a folder goes
    /// through here, so every one of them gets that right.
    pub(super) fn navigate_to(&mut self, path: &Path) {
        // Recorded first: once the listing is down there is nothing left to
        // record but the sentinel standing in for its directory.
        self.record_location();
        if self.is_synthetic() {
            self.close_search();
            self.leave_recent();
        }
        self.go_to(path);
    }

    /// Show `path` without touching the history.
    ///
    /// For the callers that have already recorded where they were — leaving a
    /// search or Recent has to record the *listing* before taking it down,
    /// because once it is down there is nothing left to record but the
    /// sentinel standing in for its directory.
    pub(super) fn go_to(&mut self, path: &Path) {
        self.columns = vec![Column::new(path.to_path_buf())];
        self.active = 0;
        self.pan.scroll_to(0.0);
        self.dirty = true;
    }

    /// Leave whatever synthetic listing is up and go to `path`, with the
    /// listing left behind Back.
    ///
    /// The same as [`Self::navigate_to`] — every navigation leaves a
    /// synthetic listing now — kept for the callers that say what they mean.
    pub(super) fn leave_synthetic_to(&mut self, path: &Path) {
        self.navigate_to(path);
    }

    /// Open the path entry on the directory being shown, with the whole path
    /// selected — typing replaces it, End keeps it and appends. The trailing
    /// separator is there so the first Tab completes a child rather than
    /// re-completing the folder the user is already in.
    pub(super) fn open_path_entry(&mut self) {
        // A synthetic listing has no location to resolve a typed path
        // against, and the sentinel behind it must never be offered as one.
        if self.is_synthetic() {
            self.refuse(otto_kit::t_owned!("files-recent-no-location"));
            return;
        }
        let mut path = self.current_directory().to_string_lossy().into_owned();
        if !path.ends_with('/') {
            path.push('/');
        }
        self.path_entry = Some(TextInput::editing(
            path,
            view::path_field_style(AppContext::current_theme()),
        ));
        self.dirty = true;
    }

    /// Put the title back. The location is unchanged — an abandoned path is
    /// not a navigation.
    pub(super) fn cancel_path_entry(&mut self) {
        self.path_entry = None;
        self.dirty = true;
    }

    /// The directory the path entry resolves against, and the one Ctrl+L
    /// starts on: the active Miller pane in column view, the single listing
    /// everywhere else.
    pub(super) fn current_directory(&self) -> PathBuf {
        if self.mode == ViewMode::Columns {
            self.columns[self.active].path.clone()
        } else {
            self.current_path()
        }
    }

    /// Resolve what has been typed: `~` for home, a bare name against the
    /// directory on screen, anything else as given.
    pub(super) fn resolve_typed_path(&self, typed: &str) -> Option<PathBuf> {
        let typed = typed.trim();
        if typed.is_empty() {
            return None;
        }
        let expanded = if typed == "~" {
            model::home_dir()?
        } else if let Some(rest) = typed.strip_prefix("~/") {
            model::home_dir()?.join(rest)
        } else if typed.starts_with('/') {
            PathBuf::from(typed)
        } else {
            self.current_directory().join(typed)
        };
        Some(expanded)
    }

    /// Go where the field says. A directory is opened; a file opens its
    /// parent with the file selected, which is what a path pasted out of a
    /// terminal usually means. A path that is not there leaves the field up
    /// with the reason under it — retyping one character is cheaper than
    /// typing the whole thing again.
    pub(super) fn commit_path_entry(&mut self) {
        let typed = self
            .path_entry
            .as_ref()
            .map(|input| input.value().to_string())
            .unwrap_or_default();
        let Some(path) = self.resolve_typed_path(&typed) else {
            self.cancel_path_entry();
            return;
        };
        if path.is_dir() {
            self.path_entry = None;
            self.navigate_to(&path);
            return;
        }
        if path.is_file() {
            let key = Some(path.to_string_lossy().into_owned());
            if let Some(parent) = path.parent() {
                self.path_entry = None;
                self.navigate_to(parent);
                // The parent's listing is read off-thread, so the row to land
                // on does not exist yet — see `pending_pick`.
                self.pending_pick = Some((0, key));
                return;
            }
        }
        self.status = Some(otto_kit::t_owned!(
            "files-no-such-folder",
            path = typed.trim()
        ));
        self.dirty = true;
    }

    /// Tab: extend what has been typed as far as the directory allows —
    /// to the one match, or to the longest prefix every match shares. A
    /// completed directory gains its separator, so Tab walks down a tree
    /// without the user reaching for `/` between levels.
    ///
    /// The read is synchronous, unlike a listing's: it is one directory, on
    /// a keystroke the user is waiting on, and its result is thrown away.
    pub(super) fn complete_path_entry(&mut self) {
        let Some(input) = self.path_entry.as_ref() else {
            return;
        };
        let typed = input.value().to_string();
        let (head, prefix) = match typed.rfind('/') {
            Some(cut) => (&typed[..=cut], &typed[cut + 1..]),
            None => ("", typed.as_str()),
        };
        let Some(dir) = self.resolve_typed_path(if head.is_empty() { "." } else { head }) else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return;
        };
        // A dotfile is only a candidate once the user has typed the dot, the
        // way a shell does it — otherwise every completion in a home
        // directory offers a hundred configuration folders first.
        let mut names: Vec<String> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(prefix))
            .filter(|name| prefix.starts_with('.') || !name.starts_with('.'))
            .collect();
        if names.is_empty() {
            return;
        }
        names.sort();
        let completed = common_prefix(&names);
        if completed.len() < prefix.len() {
            return;
        }
        let mut value = format!("{head}{completed}");
        if names.len() == 1 && dir.join(&completed).is_dir() && !value.ends_with('/') {
            value.push('/');
        }
        if value == typed {
            return;
        }
        let caret = value.chars().count();
        if let Some(input) = self.path_entry.as_mut() {
            input.set_value(value);
            input.state.set_caret(caret, false);
        }
        self.dirty = true;
    }

    // -----------------------------------------------------------------------
    // The command palette — see `specs/file-command-palette.md`
    // -----------------------------------------------------------------------
}
