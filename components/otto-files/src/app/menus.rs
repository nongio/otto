//! The context menu.

use super::*;

impl Browser {
    /// Point the browser at what a right-click at `(x, y)` should act on,
    /// and build the menu for it.
    ///
    /// A hit not already part of the selection replaces it, the way a plain
    /// click does; a hit that's already selected leaves a multi-selection
    /// alone, so the menu acts on the whole group. An empty-space click just
    /// moves the keyboard focus to that pane, for New Folder and Paste.
    pub(super) fn context_menu_items(&mut self, x: f32, y: f32) -> Vec<MenuItem> {
        match self.entry_at(x, y) {
            Some((depth, index)) => {
                let already_selected = self
                    .visible(depth)
                    .get(index)
                    .is_some_and(|e| self.columns[depth].selection.contains(&e.selection_key()));
                if already_selected {
                    self.active = depth;
                } else {
                    self.select(depth, index);
                }
            }
            None => self.active = self.pane_under(x, y),
        }
        // The pane with the keyboard may have changed, and it is drawn so.
        self.dirty = true;

        let entries = self.selected_entries();
        let mut items = Vec::new();

        // The Trash's own menu. Nothing it shares with the browser applies:
        // a trashed file cannot be opened, renamed, copied or trashed again,
        // and pasting into the trash would be a way of putting files there
        // without a sidecar saying where they came from.
        if self.trash {
            if !entries.is_empty() {
                items.push(
                    MenuItem::action(otto_kit::t!("files-put-back")).with_action_id("put_back"),
                );
                if let [only] = entries.as_slice() {
                    let _ = only;
                    items.push(
                        MenuItem::action(otto_kit::t!("files-get-info")).with_action_id("get_info"),
                    );
                }
                let label = if entries.len() == 1 {
                    otto_kit::t_owned!("files-delete-immediately")
                } else {
                    otto_kit::t_owned!(
                        "files-delete-count-immediately",
                        count = entries.len() as f64
                    )
                };
                items.push(MenuItem::separator());
                items.push(MenuItem::action(label).with_action_id("delete_forever"));
            }
            if !self.visible(0).is_empty() {
                if !items.is_empty() {
                    items.push(MenuItem::separator());
                }
                items.push(
                    MenuItem::action(otto_kit::t!("files-empty-trash"))
                        .with_action_id("empty_trash"),
                );
            }
            return items;
        }

        // Open, and Open With for files. A folder has no "with": opening one
        // is going into it, here.
        if let [_] = entries.as_slice() {
            items.push(MenuItem::action(otto_kit::t!("common-open")).with_action_id("open"));
        }
        if self.picker.is_none() && !entries.is_empty() && entries.iter().all(|e| !e.is_dir) {
            items.push(
                MenuItem::action(otto_kit::t!("files-open-with")).with_action_id("open_with"),
            );
        }
        if let [_] = entries.as_slice() {
            items.push(MenuItem::separator());
            items.push(MenuItem::action(otto_kit::t!("files-get-info")).with_action_id("get_info"));
            items.push(MenuItem::action(otto_kit::t!("common-rename")).with_action_id("rename"));
        }

        if entries.is_empty() {
            items.push(
                MenuItem::action(otto_kit::t!("files-new-folder")).with_action_id("new_folder"),
            );
            let can_paste = !self.clipboard.is_empty()
                || clipboard::first_available(clipboard::file_mime_preference()).is_some();
            if can_paste {
                items.push(MenuItem::separator());
                items.push(MenuItem::action(otto_kit::t!("common-paste")).with_action_id("paste"));
            }
        } else {
            if !items.is_empty() {
                items.push(MenuItem::separator());
            }
            let folder_label = if entries.len() == 1 {
                otto_kit::t_owned!("files-new-folder-with-selection")
            } else {
                otto_kit::t_owned!("files-new-folder-with-count", count = entries.len() as f64)
            };
            items.push(MenuItem::action(folder_label).with_action_id("new_folder_with_selection"));
            items.push(MenuItem::separator());
            items.push(MenuItem::action(otto_kit::t!("common-cut")).with_action_id("cut"));
            items.push(MenuItem::action(otto_kit::t!("common-copy")).with_action_id("copy"));
            if crate::stash::available() {
                items.push(MenuItem::separator());
                items.push(
                    MenuItem::action(otto_kit::t!("files-add-to-stash"))
                        .with_action_id(command::id::ADD_TO_STASH),
                );
            }
            items.push(MenuItem::separator());
            let label = if entries.len() == 1 {
                otto_kit::t_owned!("files-move-to-trash")
            } else {
                otto_kit::t_owned!("files-move-count-to-trash", count = entries.len() as f64)
            };
            items.push(MenuItem::action(label).with_action_id("trash"));
        }

        // What the providers offer for this situation — scripts, the pattern
        // rename — after the window's own items. The menu asks the same
        // registry the palette does, so a script that shows up in one shows
        // up in the other; picking one goes through the palette, which is
        // where a command's argument and its dry run live.
        let situation = self.situation();
        let provided: Vec<command::Command> = self
            .commands
            .commands(&situation)
            .into_iter()
            .filter(|command| command.namespace().is_some())
            .collect();
        if !provided.is_empty() {
            if !items.is_empty() {
                items.push(MenuItem::separator());
            }
            for command in provided {
                let label = if command.arg.is_some() {
                    format!("{}…", command.title)
                } else {
                    command.title.clone()
                };
                items.push(MenuItem::action(label).with_action_id(command.id));
            }
        }

        items
    }

    /// Carry out a provider command picked from a menu.
    ///
    /// One that takes an argument opens the palette in its field, initial
    /// value and dry run included, so the menu is a shortcut into the same
    /// interaction rather than a second one; one that does not runs at once.
    pub(super) fn run_menu_command(&mut self, id: &str, serial: u32) {
        let takes_arg = self
            .commands
            .commands(&self.situation())
            .iter()
            .any(|command| command.id == id && command.arg.is_some());
        if takes_arg {
            self.open_palette();
            let opened = self
                .palette
                .as_mut()
                .is_some_and(|palette| palette.open_on(id));
            if opened {
                self.preview_palette_argument();
                self.palette_reveal_highlight();
            } else {
                self.close_palette();
            }
            return;
        }
        match self.run_request(&command::Request::new(id, None), serial) {
            Ok(_) => {}
            Err(error) => self.status = Some(error),
        }
        self.dirty = true;
    }
}
