//! Pressing and opening entries, and opening new windows.

use super::*;

impl Browser {
    /// How far through the open pulse we are, if one is running.
    pub(super) fn opening_progress(&self) -> Option<(usize, f32)> {
        let (depth, started) = self.opening?;
        let t = started.elapsed().as_secs_f32() / view::OPEN_PULSE.as_secs_f32();
        (t < 1.0).then_some((depth, t))
    }

    /// Drop a pulse that has run its course, so the clock can stop.
    pub(super) fn tick_open_pulse(&mut self) -> bool {
        if self.opening.is_some() && self.opening_progress().is_none() {
            self.opening = None;
            self.dirty = true;
        }
        self.opening.is_some()
    }

    /// A plain press on a row or cell.
    ///
    /// Pressing an entry that is *already* one of several selected leaves the
    /// selection alone: what usually follows is a drag of the whole group, and
    /// narrowing to the one under the pointer would throw the rest away before
    /// the drag could carry them. The narrowing is not abandoned, only deferred
    /// to the release — a press that comes back up without dragging was a click
    /// after all, and a click on one of several selected files does mean "just
    /// this one".
    ///
    /// Any pending narrow from an earlier gesture is dropped first. A deferred
    /// decision that outlives its own gesture is worse than no deferral at all:
    /// it would land on a later, unrelated click and undo it.
    pub(super) fn press_entry(&mut self, depth: usize, index: usize) {
        self.press_pending = None;

        if self.is_in_multiple_selection(depth, index) {
            self.press_pending = Some((depth, index));
            return;
        }
        self.select(depth, index);
        self.note_row_click(depth, index);
    }

    /// The button came up. A press that deferred its narrowing and did not turn
    /// into a drag was a click, so it narrows now.
    pub(super) fn release_entry(&mut self) {
        let Some((depth, index)) = self.press_pending.take() else {
            return;
        };
        self.select(depth, index);
        self.note_row_click(depth, index);
    }

    /// A drag has begun: the group travels whole, so the press that started it
    /// must not narrow to one of them when the button comes up.
    pub(super) fn drag_started(&mut self) {
        self.press_pending = None;
    }

    /// Is `index` one of *several* entries selected in `depth`?
    ///
    /// One selected entry is not a group: pressing it again means the same
    /// thing either way, and deferring would only make the common case answer
    /// late.
    pub(super) fn is_in_multiple_selection(&self, depth: usize, index: usize) -> bool {
        let Some(column) = self.columns.get(depth) else {
            return false;
        };
        if column.selection.len() < 2 {
            return false;
        }
        self.visible(depth)
            .get(index)
            .is_some_and(|entry| column.selection.contains(&entry.selection_key()))
    }

    /// Record a plain click on a row/cell, opening it if this is the second
    /// one to land on the same row within the double-click window — List and
    /// Grid's only mouse way to open a directory, since neither shows a
    /// child eagerly the way Miller does.
    pub(super) fn note_row_click(&mut self, depth: usize, index: usize) {
        let now = std::time::Instant::now();
        let double_click = self.last_row_click.is_some_and(|(d, i, at)| {
            d == depth && i == index && now.duration_since(at) < DOUBLE_CLICK_WINDOW
        });
        if double_click {
            self.last_row_click = None;
            self.open_selection();
        } else {
            self.last_row_click = Some((depth, index, now));
        }
    }

    /// Record a Ctrl+click on a row/cell. The first one toggles the row into
    /// or out of the selection; a second one on the same row within the
    /// double-click window opens it in a *new window* instead of toggling it
    /// back out, so Ctrl+double-click reads as "open this one elsewhere"
    /// rather than as two selection changes.
    ///
    /// The picker never takes this path: it answers one request in one
    /// window, so there Ctrl+click only ever toggles.
    pub(super) fn note_ctrl_row_click(&mut self, depth: usize, index: usize) {
        let now = std::time::Instant::now();
        let double_click = self.picker.is_none()
            && self.last_row_click.is_some_and(|(d, i, at)| {
                d == depth && i == index && now.duration_since(at) < DOUBLE_CLICK_WINDOW
            });
        if double_click {
            self.last_row_click = None;
            self.open_in_new_window(depth, index);
        } else {
            self.toggle_select(depth, index);
            self.last_row_click = Some((depth, index, now));
        }
    }

    /// Open one directory in a second browser window — Ctrl+double-click.
    ///
    /// A window is a process here: the app shell is built around a single
    /// toplevel, so the second window is a second `otto-files` handed the
    /// directory on its command line. Anything that is not a directory falls
    /// back to plain activation, which is all a new window could do with it.
    pub(super) fn open_in_new_window(&mut self, depth: usize, index: usize) {
        let Some(entry) = self.visible(depth).get(index).map(|e| (*e).clone()) else {
            return;
        };
        if !entry.is_dir {
            self.select(depth, index);
            self.open_selection();
            return;
        }

        self.spawn_window(&entry.path);
    }

    /// Open a second browser window on `path`.
    ///
    /// A window is a process here — the app shell is built around a single
    /// toplevel — so this re-executes this binary with the directory on its
    /// command line. Shared by Ctrl+double-click and by the New Window
    /// shortcut, which differ only in which directory they name.
    pub(super) fn spawn_window(&mut self, path: &Path) {
        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(err) => {
                self.status = Some(otto_kit::t_owned!(
                    "files-new-window-failed",
                    error = err.to_string()
                ));
                self.dirty = true;
                return;
            }
        };
        let spawned = std::process::Command::new(exe)
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        match spawned {
            // Reap it on a thread of its own: the child outlives this call and
            // nothing else here would ever wait on it.
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(err) => {
                self.status = Some(otto_kit::t_owned!(
                    "files-new-window-failed",
                    error = err.to_string()
                ));
                self.dirty = true;
            }
        }
    }

    /// Open a new window on the default location — Ctrl+N.
    ///
    /// The default location, not this window's directory: a new window is a
    /// fresh start, and starting somewhere arbitrary — wherever the window
    /// that happened to have focus was pointed — is what makes a new window
    /// feel like a copy of the old one rather than a new one. Ctrl+double-click
    /// is the gesture for "that directory, in another window".
    pub(super) fn open_new_window(&mut self) {
        let Some(path) = self.new_window_target() else {
            return;
        };
        self.spawn_window(&path);
    }

    /// Where a new window would open, or `None` when one makes no sense.
    ///
    /// Split from the spawning so the choice can be tested without launching
    /// a process: everything interesting about Ctrl+N is which directory it
    /// names, and that is all this answers.
    pub(super) fn new_window_target(&self) -> Option<PathBuf> {
        // The picker answers one request in one window: a second browser
        // window would have nothing to do with the request and no way to
        // answer it.
        if self.picker.is_some() {
            return None;
        }
        Some(Self::default_location())
    }

    /// Where a window with nowhere in particular to be starts.
    ///
    /// The home directory for now. It is a single function rather than a
    /// literal at each call site because this is the thing a preference would
    /// replace — when there is one, it is read here and everywhere that opens
    /// a fresh window follows.
    pub(super) fn default_location() -> PathBuf {
        model::home_dir().unwrap_or_else(|| PathBuf::from("/"))
    }

    /// Descend into the selection: in Miller view the child column already
    /// exists, so this only moves the keyboard into it.
    pub(super) fn open_selection(&mut self) {
        let depth = self.active;
        let Some(index) = self.columns[depth].cursor else {
            return;
        };
        let Some(entry) = self.visible(depth).get(index).map(|e| (*e).clone()) else {
            return;
        };

        // Handing a trashed file to an application would launch it on
        // something the user threw away, and let it be edited in place in the
        // can. Put Back first; the message says so. A trashed *folder* can
        // still be opened — looking inside it is how you decide.
        if self.trash && !entry.is_dir {
            self.refuse(otto_kit::t_owned!("files-trash-cant-open"));
            return;
        }

        // Say that the open was heard, before doing it: handing a file to
        // another process can take long enough that a double-click with no
        // answer reads as one that did not land. In every view — the gesture
        // is the same everywhere, so the answer to it should be too.
        //
        // A directory is the exception, and not because of the waiting: going
        // into one *shows* itself. The listing changes, or in column view a
        // child column arrives beside the one that was clicked, and a ghost of
        // the row on top of that is one answer too many.
        if !entry.is_dir {
            self.opening = Some((depth, std::time::Instant::now()));
            self.dirty = true;
        }

        if entry.is_dir {
            // A folder found by a search, or listed in Recent, is a real
            // folder somewhere on the disk — but there is no hierarchy under a
            // result to descend *into*. Opening it is going there, which means
            // leaving the listing, with the listing left behind Back.
            if self.is_synthetic() {
                self.leave_synthetic_to(&entry.path);
                return;
            }
            match self.mode {
                ViewMode::Columns => {
                    if depth + 1 < self.columns.len() {
                        self.active = depth + 1;
                        if self.columns[self.active].cursor.is_none() {
                            if self.visible(self.active).is_empty() {
                                // Still being read: take the first row when it
                                // lands, in `poll`. The listing is on a worker
                                // and a keyboard descent routinely beats it.
                                self.entering = Some(self.active);
                            } else {
                                // Same path a click takes: if this first entry
                                // is itself a directory, its column shows up
                                // too — every directory on screen keeps the
                                // pane to its right populated, not just the
                                // one last entered.
                                self.select(self.active, 0);
                            }
                        }
                        self.reveal_pane(self.active);
                    }
                }
                ViewMode::List | ViewMode::Grid => {
                    // These show one directory, so descending replaces it.
                    self.record_location();
                    self.columns.truncate(depth + 1);
                    self.columns.push(Column::new(entry.path.clone()));
                    self.active = self.columns.len() - 1;
                }
            }
            self.dirty = true;
            return;
        }

        // A file. In the picker, activating one *is* the accept — double-click
        // and Enter both land here, and both mean "this one".
        if self.picker.is_some() {
            self.picker_accept();
        } else {
            self.open_in_default_app(&entry.path);
        }
    }

    /// Hand a file — or a URL — to whatever the desktop opens it with.
    ///
    /// `xdg-open` rather than resolving the association here: it is the
    /// desktop's own answer to this question, it already knows about
    /// `mimeapps.list`, the portal and the fallbacks, and a file manager that
    /// disagreed with the rest of the session about what opens a `.pdf` would
    /// be the thing that was wrong.
    ///
    /// Detached, like a new window: stdio closed and reaped on a thread of its
    /// own, so the application outlives the browser that started it.
    pub(super) fn open_in_default_app(&mut self, target: impl AsRef<std::ffi::OsStr>) {
        let spawned = std::process::Command::new("xdg-open")
            .arg(target)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        match spawned {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(err) => {
                self.status = Some(otto_kit::t_owned!(
                    "files-open-failed",
                    error = err.to_string()
                ));
                self.dirty = true;
            }
        }
    }

    // --- The picker's half of "activate" -----------------------------------
}
