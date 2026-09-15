//! The file picker's shell: its selection, save field, filter and answer.

use super::*;

impl Browser {
    /// What the accept button would return, or `None` if it has nothing to
    /// return and must stay disabled.
    ///
    /// Directory mode with nothing picked accepts the directory being
    /// *viewed*, which is how a user says "this folder" without having to
    /// step out of it and select it from its parent.
    pub(super) fn picker_selection(&self) -> Option<Vec<PathBuf>> {
        let session = self.picker.as_ref()?;
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let column = self.columns.get(depth)?;

        let picked: Vec<PathBuf> = self
            .visible(depth)
            .into_iter()
            .filter(|e| column.selection.contains(&e.selection_key()))
            .filter(|e| session.selectable(&e.name, e.is_dir))
            .map(|e| e.path.clone())
            .collect();

        if picked.is_empty() {
            if session.request.directory {
                return Some(vec![column.path.clone()]);
            }
            return None;
        }
        // A single-select request returns exactly one file however many the
        // pointer managed to gather.
        if !session.request.multiple {
            return picked.into_iter().next().map(|p| vec![p]);
        }
        Some(picked)
    }

    // --- The save modes ----------------------------------------------------

    /// The directory a save-mode accept writes into.
    ///
    /// `Save` always uses the directory being *viewed*: the name field says
    /// what to call the file, the listing says where it goes, and a folder
    /// merely selected in that listing is somewhere the user is looking at,
    /// not somewhere they have gone. `SaveFiles` follows the directory-mode
    /// rule instead — the whole request is "which folder", so a selected one
    /// is the answer.
    pub(super) fn save_directory(&self) -> Option<PathBuf> {
        let session = self.picker.as_ref()?;
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let column = self.columns.get(depth)?;

        if session.request.mode == picker::Mode::SaveFiles {
            let picked: Vec<&Entry> = self
                .visible(depth)
                .into_iter()
                .filter(|e| e.is_dir && column.selection.contains(&e.selection_key()))
                .collect();
            if let [only] = picked.as_slice() {
                return Some(only.path.clone());
            }
        }
        Some(column.path.clone())
    }

    /// What the accept button would do in `Save` mode, and why it is disabled
    /// when it is.
    ///
    /// The directory check comes first: a name is never the problem when the
    /// folder cannot be written to at all, and saying "Enter a name" about a
    /// read-only folder would send the user off correcting the wrong thing.
    pub(super) fn save_action(&self) -> picker::SaveAction {
        let Some(dir) = self.save_directory() else {
            return picker::SaveAction::Blocked("files-save-nowhere");
        };
        let name = self
            .save_name
            .as_ref()
            .map(TextInput::value)
            .unwrap_or("")
            .to_string();

        if let Some((cached_dir, cached_name, action)) = self.save_probe.borrow().as_ref() {
            if *cached_dir == dir && *cached_name == name {
                return action.clone();
            }
        }
        let action = probe_save_action(&dir, &name);
        *self.save_probe.borrow_mut() = Some((dir, name, action.clone()));
        action
    }

    /// Ask the filesystem again, ignoring the memo.
    ///
    /// Accept uses this: between the last repaint and the click, someone else
    /// may have created the file the user is about to be told is not there,
    /// and the confirmation exists precisely to catch that.
    pub(super) fn save_action_now(&self) -> picker::SaveAction {
        self.save_probe.borrow_mut().take();
        self.save_action()
    }

    /// Whether the accept button is live, for the frame.
    pub(super) fn picker_accept_enabled(&self) -> bool {
        match self.picker.as_ref().map(|s| s.request.mode) {
            Some(picker::Mode::Save) => {
                !matches!(self.save_action(), picker::SaveAction::Blocked(_))
            }
            Some(picker::Mode::SaveFiles) => self
                .save_directory()
                .is_some_and(|dir| picker::is_writable_dir(&dir)),
            Some(picker::Mode::Open) => self.picker_selection().is_some(),
            None => false,
        }
    }

    /// The localisation key for why the accept button is disabled, shown
    /// beside the name field. `None` while it is enabled — there is then
    /// nothing to explain. The key, not the message: the caller looks it up.
    pub(super) fn save_problem(&self) -> Option<&'static str> {
        match self.picker.as_ref()?.request.mode {
            picker::Mode::Save => match self.save_action() {
                // "Enter a name" is not a complaint about an empty field the
                // user has not filled in yet; it is the placeholder's job.
                // Matched on the key rather than the message, which is
                // whatever language the user reads in.
                picker::SaveAction::Blocked("files-save-enter-a-name") => None,
                picker::SaveAction::Blocked(reason) => Some(reason),
                _ => None,
            },
            picker::Mode::SaveFiles => self
                .save_directory()
                .filter(|dir| !picker::is_writable_dir(dir))
                .map(|_| "files-save-permission-denied"),
            picker::Mode::Open => None,
        }
    }

    /// `Save`: resolve the name field against the directory being viewed.
    pub(super) fn save_accept(&mut self) {
        match self.save_action_now() {
            picker::SaveAction::Blocked(_) => {}
            picker::SaveAction::Descend => {
                // The name names a folder that is already there. Going into it
                // is what the user meant; clearing the field is what stops the
                // next Return from bouncing straight back out of it.
                let Some(dir) = self.save_directory() else {
                    return;
                };
                let name = self
                    .save_name
                    .as_ref()
                    .map(|i| i.value().trim().to_string())
                    .unwrap_or_default();
                self.navigate_to(&dir.join(name));
                if let Some(input) = self.save_name.as_mut() {
                    input.set_value(String::new());
                }
                self.dirty = true;
            }
            picker::SaveAction::Replace => {
                let Some(target) = self.save_target() else {
                    return;
                };
                let name = target
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.confirm = Some(ConfirmSheet {
                    message: otto_kit::t_owned!("files-replace-one", name = name.as_str()),
                    detail: otto_kit::t_owned!("files-replace-one-detail"),
                    accept_label: otto_kit::t_owned!("common-replace"),
                    action: ConfirmAction::Answer(vec![target]),
                    pressed: None,
                });
                self.dirty = true;
            }
            picker::SaveAction::Write => {
                let Some(target) = self.save_target() else {
                    return;
                };
                self.answer_with(vec![target]);
            }
        }
    }

    /// The single path `Save` would answer with.
    pub(super) fn save_target(&self) -> Option<PathBuf> {
        let name = self.save_name.as_ref()?.value().trim();
        if name.is_empty() {
            return None;
        }
        Some(self.save_directory()?.join(name))
    }

    /// `SaveFiles`: one path per name the request carried, all in the chosen
    /// directory.
    ///
    /// Each name is reduced to its final component. The spec's "no name
    /// mangling" is about not inventing `file (1).txt` when something is in
    /// the way; it is not a licence for an application to reach out of the
    /// directory the user chose by sending `../../.bashrc`.
    pub(super) fn save_files_targets(&self) -> Vec<PathBuf> {
        let Some(dir) = self.save_directory() else {
            return Vec::new();
        };
        let Some(session) = self.picker.as_ref() else {
            return Vec::new();
        };
        session
            .request
            .files
            .iter()
            .filter_map(|name| Path::new(name).file_name().map(|n| dir.join(n)))
            .collect()
    }

    pub(super) fn save_files_accept(&mut self) {
        let targets = self.save_files_targets();
        if targets.is_empty() {
            return;
        }
        let clashes: Vec<&PathBuf> = targets
            .iter()
            .filter(|p| existing_kind(p).is_some())
            .collect();
        if clashes.is_empty() {
            self.answer_with(targets);
            return;
        }
        // One sheet for all of them, not one per file: the user is answering
        // a single question about a single batch.
        let message = if clashes.len() == 1 {
            let name = clashes[0]
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            otto_kit::t_owned!("files-replace-one", name = name.as_str())
        } else {
            otto_kit::t_owned!("files-replace-many", count = clashes.len() as i64)
        };
        self.confirm = Some(ConfirmSheet {
            message,
            detail: otto_kit::t_owned!("files-replace-many-detail"),
            accept_label: otto_kit::t_owned!("common-replace"),
            action: ConfirmAction::Answer(targets),
            pressed: None,
        });
        self.dirty = true;
    }

    /// Answer the request with `paths` and let the window go.
    pub(super) fn answer_with(&mut self, paths: Vec<PathBuf>) {
        if let Some(session) = self.picker.as_mut() {
            session.accept(&paths);
        }
        self.dirty = true;
    }

    /// The affirmative answer: carry out whatever the sheet was asking about,
    /// against the paths it was showing.
    pub(super) fn confirm_accept(&mut self) {
        let Some(sheet) = self.confirm.take() else {
            return;
        };
        match sheet.action {
            ConfirmAction::Answer(paths) => self.answer_with(paths),
            ConfirmAction::DeleteForever(paths) => self.destroy(paths),
            ConfirmAction::EmptyTrash => {
                let result = model::empty_trash();
                self.report(&result);
                self.reload_all();
            }
        }
        self.dirty = true;
    }

    /// Dismiss the sheet without answering. The request stays open and the
    /// user is back in the dialog, which is what "Cancel" means here — it
    /// cancels the replacement, not the save.
    pub(super) fn confirm_dismiss(&mut self) {
        if self.confirm.take().is_some() {
            self.dirty = true;
        }
    }

    /// Return the current selection to the application and close the window.
    pub(super) fn picker_accept(&mut self) {
        match self.picker.as_ref().map(|s| s.request.mode) {
            Some(picker::Mode::Save) => self.save_accept(),
            Some(picker::Mode::SaveFiles) => self.save_files_accept(),
            Some(picker::Mode::Open) => {
                let Some(paths) = self.picker_selection() else {
                    return;
                };
                self.answer_with(paths);
            }
            None => {}
        }
    }

    /// Cancel the request. The window closes and the application is told the
    /// user declined — which is a different answer from "it went wrong".
    pub(super) fn picker_cancel(&mut self) {
        if let Some(session) = self.picker.as_mut() {
            session.resolve(picker::Outcome::cancelled());
        }
        self.dirty = true;
    }

    /// Switch to filter `index` and re-filter in place.
    pub(super) fn set_filter(&mut self, index: usize) {
        if let Some(session) = self.picker.as_mut() {
            if index < session.filters.len() && index != session.current_filter {
                session.current_filter = index;
                // The cursor and selection are indices into an order that no
                // longer exists once the filter moves.
                for column in &mut self.columns {
                    column.selection.clear();
                    column.cursor = None;
                    column.anchor = None;
                }
            }
            session.filter_open = false;
            self.dirty = true;
        }
    }

    /// The action row's press half: arm the button under the pointer.
    pub(super) fn footer_press(&mut self, button: view::FooterButton) {
        self.footer_pressed = Some(button);
        self.dirty = true;
    }

    /// The action row's release half: fire only if the pointer is still over
    /// the button that was armed.
    pub(super) fn footer_release(&mut self, over: Option<view::FooterButton>) {
        let armed = self.footer_pressed.take();
        self.dirty = true;
        if armed.is_none() || armed != over {
            return;
        }
        match armed {
            Some(view::FooterButton::Accept) => self.picker_accept(),
            Some(view::FooterButton::Cancel) => self.picker_cancel(),
            Some(view::FooterButton::Filter) => {
                if let Some(session) = self.picker.as_mut() {
                    session.filter_open = !session.filter_open;
                }
            }
            Some(view::FooterButton::FilterOption(index)) => self.set_filter(index),
            None => {}
        }
    }
}
