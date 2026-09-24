//! Keyboard handling.

use super::*;

impl FilesApp {
    pub(super) fn handle_key(
        &mut self,
        event: &KeyEvent,
        key_state: wl_keyboard::KeyState,
        serial: u32,
    ) {
        use smithay_client_toolkit::seat::keyboard::Keysym;

        // A modifier key on its own is not a shortcut and not type-ahead:
        // the state it changed already arrived in `on_modifiers`.
        if matches!(
            event.keysym,
            Keysym::Control_L
                | Keysym::Control_R
                | Keysym::Shift_L
                | Keysym::Shift_R
                | Keysym::Alt_L
                | Keysym::Alt_R
                | Keysym::Super_L
                | Keysym::Super_R
        ) {
            return;
        }
        if key_state != wl_keyboard::KeyState::Pressed {
            return;
        }
        let mods = *self.modifiers.lock().unwrap();
        let (ctrl, shift) = (mods.ctrl, mods.shift);

        // The Open With chooser is a window of its own, and while it has the
        // keyboard every key is its: the field, the list and its buttons.
        let chooser_focused = {
            use wayland_client::Proxy;
            let window = self.open_with_window.borrow().clone();
            window
                .and_then(|window| window.wl_surface())
                .is_some_and(|surface| AppContext::keyboard_focus() == Some(surface.id()))
        };
        if chooser_focused {
            use open_with::OpenWithKey;
            let key = match event.keysym {
                Keysym::Up => Some(OpenWithKey::Up),
                Keysym::Down => Some(OpenWithKey::Down),
                Keysym::Return | Keysym::KP_Enter => Some(OpenWithKey::Enter),
                Keysym::Escape => Some(OpenWithKey::Escape),
                Keysym::Left => Some(OpenWithKey::Edit(TextInputKey::Left)),
                Keysym::Right => Some(OpenWithKey::Edit(TextInputKey::Right)),
                Keysym::Home => Some(OpenWithKey::Edit(TextInputKey::Home)),
                Keysym::End => Some(OpenWithKey::Edit(TextInputKey::End)),
                Keysym::BackSpace => Some(OpenWithKey::Edit(TextInputKey::Backspace)),
                Keysym::Delete => Some(OpenWithKey::Edit(TextInputKey::Delete)),
                Keysym::a if ctrl => Some(OpenWithKey::Edit(TextInputKey::SelectAll)),
                Keysym::c if ctrl => Some(OpenWithKey::Edit(TextInputKey::Copy)),
                Keysym::x if ctrl => Some(OpenWithKey::Edit(TextInputKey::Cut)),
                Keysym::v if ctrl => {
                    clipboard::text().map(|text| OpenWithKey::Edit(TextInputKey::Paste(text)))
                }
                _ if ctrl => None,
                _ => event
                    .utf8
                    .as_ref()
                    .and_then(|s| s.chars().next())
                    .filter(|ch| !ch.is_control())
                    .map(|ch| OpenWithKey::Edit(TextInputKey::Char(ch))),
            };
            if let Some(key) = key {
                self.state
                    .lock()
                    .unwrap()
                    .open_with_key(key, KeyMods { shift, ctrl });
            }
            return;
        }

        {
            let mut browser = self.state.lock().unwrap();

            // The confirmation sheet is modal, so it takes the keyboard
            // whole: Return is the answer it is asking for and Escape backs
            // out of it. Nothing else reaches the window behind it.
            if browser.confirm.is_some() {
                match event.keysym {
                    Keysym::Return | Keysym::KP_Enter => browser.confirm_accept(),
                    Keysym::Escape => browser.confirm_dismiss(),
                    _ => {}
                }
                drop(browser);
                return;
            }

            // The palette takes the keyboard whole while it is up. It is not
            // modal over the compositor — the window can still be moved,
            // resized and closed — but no key reaches the listing behind it.
            if browser.palette.is_some() {
                // A second Ctrl+P closes it, the way the chord that opens Get
                // Info closes it again.
                if event.keysym == Keysym::p && ctrl {
                    browser.close_palette();
                    drop(browser);
                    return;
                }
                let key = match event.keysym {
                    Keysym::Escape => Some(palette::Key::Escape),
                    Keysym::Return | Keysym::KP_Enter => Some(palette::Key::Enter),
                    Keysym::Tab | Keysym::ISO_Left_Tab => Some(palette::Key::Tab),
                    Keysym::Up => Some(palette::Key::Up),
                    Keysym::Down => Some(palette::Key::Down),
                    Keysym::Home => Some(palette::Key::Home),
                    Keysym::End => Some(palette::Key::End),
                    Keysym::Left => Some(palette::Key::Edit(TextInputKey::Left)),
                    Keysym::Right => Some(palette::Key::Edit(TextInputKey::Right)),
                    Keysym::BackSpace => Some(palette::Key::Edit(TextInputKey::Backspace)),
                    Keysym::Delete => Some(palette::Key::Edit(TextInputKey::Delete)),
                    Keysym::a if ctrl => Some(palette::Key::Edit(TextInputKey::SelectAll)),
                    Keysym::c if ctrl => Some(palette::Key::Edit(TextInputKey::Copy)),
                    Keysym::x if ctrl => Some(palette::Key::Edit(TextInputKey::Cut)),
                    Keysym::v if ctrl => {
                        clipboard::text().map(|text| palette::Key::Edit(TextInputKey::Paste(text)))
                    }
                    _ => event
                        .utf8
                        .as_ref()
                        .and_then(|s| s.chars().next())
                        .map(|ch| palette::Key::Edit(TextInputKey::Char(ch))),
                };
                if let Some(key) = key {
                    let mods = KeyMods { shift, ctrl };
                    if let Some(outcome) = browser
                        .palette
                        .as_mut()
                        .map(|palette| palette.on_key(key, mods))
                    {
                        browser.settle_palette(outcome, serial);
                    }
                }
                drop(browser);
                return;
            }

            // An in-place rename owns the keyboard outright: every key is
            // text-field input, not a browser shortcut.
            if browser.rename.is_some() {
                let key = match event.keysym {
                    Keysym::Return | Keysym::KP_Enter => Some(TextInputKey::Enter),
                    Keysym::Escape => Some(TextInputKey::Escape),
                    Keysym::Left => Some(TextInputKey::Left),
                    Keysym::Right => Some(TextInputKey::Right),
                    Keysym::Home => Some(TextInputKey::Home),
                    Keysym::End => Some(TextInputKey::End),
                    Keysym::BackSpace => Some(TextInputKey::Backspace),
                    Keysym::Delete => Some(TextInputKey::Delete),
                    Keysym::a if ctrl => Some(TextInputKey::SelectAll),
                    // Cut, copy and paste edit the *name* here, not the
                    // selection in the listing: the field owns the keyboard.
                    Keysym::c if ctrl => Some(TextInputKey::Copy),
                    Keysym::x if ctrl => Some(TextInputKey::Cut),
                    Keysym::v if ctrl => clipboard::text().map(TextInputKey::Paste),
                    _ => event
                        .utf8
                        .as_ref()
                        .and_then(|s| s.chars().next())
                        .map(TextInputKey::Char),
                };
                if let Some(key) = key {
                    let mods = KeyMods { shift, ctrl };
                    let response = browser
                        .rename
                        .as_mut()
                        .map(|session| session.input.on_key(key, mods));
                    match response {
                        Some(TextInputResponse::Commit) => browser.commit_rename(),
                        Some(TextInputResponse::Cancel) => browser.cancel_rename(),
                        Some(TextInputResponse::Clipboard(text)) => {
                            clipboard::set_text(&text, serial);
                            browser.dirty = true;
                        }
                        Some(_) => browser.dirty = true,
                        None => {}
                    }
                }
                drop(browser);
                return;
            }

            // The path entry owns the keyboard the same way a rename does,
            // with two keys of its own: Tab completes against the directory
            // being typed, and Return goes there.
            if browser.path_entry.is_some() {
                let key = match event.keysym {
                    Keysym::Return | Keysym::KP_Enter => {
                        browser.commit_path_entry();
                        drop(browser);
                        return;
                    }
                    Keysym::Escape => {
                        browser.cancel_path_entry();
                        drop(browser);
                        return;
                    }
                    Keysym::Tab | Keysym::ISO_Left_Tab => {
                        browser.complete_path_entry();
                        drop(browser);
                        return;
                    }
                    // A second Ctrl+L closes it, the way the chord that opens
                    // Get Info closes it again.
                    Keysym::l if ctrl => {
                        browser.cancel_path_entry();
                        drop(browser);
                        return;
                    }
                    Keysym::Left => Some(TextInputKey::Left),
                    Keysym::Right => Some(TextInputKey::Right),
                    Keysym::Home => Some(TextInputKey::Home),
                    Keysym::End => Some(TextInputKey::End),
                    Keysym::BackSpace => Some(TextInputKey::Backspace),
                    Keysym::Delete => Some(TextInputKey::Delete),
                    Keysym::a if ctrl => Some(TextInputKey::SelectAll),
                    Keysym::c if ctrl => Some(TextInputKey::Copy),
                    Keysym::x if ctrl => Some(TextInputKey::Cut),
                    Keysym::v if ctrl => clipboard::text().map(TextInputKey::Paste),
                    _ => event
                        .utf8
                        .as_ref()
                        .and_then(|s| s.chars().next())
                        .map(TextInputKey::Char),
                };
                if let Some(key) = key {
                    let mods = KeyMods { shift, ctrl };
                    let response = browser
                        .path_entry
                        .as_mut()
                        .map(|input| input.on_key(key, mods));
                    match response {
                        Some(TextInputResponse::Clipboard(text)) => {
                            clipboard::set_text(&text, serial);
                            browser.dirty = true;
                        }
                        Some(_) => browser.dirty = true,
                        None => {}
                    }
                }
                drop(browser);
                return;
            }

            // The search field holds the caret while it is open, but it is
            // deliberately *leaky* — unlike the path entry, which owns the
            // keyboard whole. Vertical motion and Return go to the listing
            // rather than to the query, so the results are one key away and
            // nobody has to Tab out of the field to reach what they found.
            //
            // Focused, not merely open: clicking a result hands the keyboard
            // back to the listing, and from there Space has to preview the
            // file rather than put a space in a query nobody is typing. What
            // still works on a blurred strip works further down — Escape
            // clears the search in the unwinding chain, Ctrl+F puts the caret
            // back.
            if browser
                .search
                .as_ref()
                .is_some_and(|input| input.state.focused())
            {
                let editing = match event.keysym {
                    Keysym::Escape => {
                        browser.clear_search();
                        drop(browser);
                        return;
                    }
                    Keysym::f if ctrl => {
                        browser.clear_search();
                        drop(browser);
                        return;
                    }
                    // Vertical motion is how you leave the query for the
                    // results, so it takes the keyboard with it rather than
                    // reaching over from the field: one Down both moves the
                    // selection onto a file and makes that file the thing the
                    // keyboard is talking to, so the next Space previews it
                    // instead of typing into a query you have stopped writing.
                    // The strip stays open and the text stays in it — Ctrl+F
                    // or a click in the field comes back.
                    Keysym::Up
                    | Keysym::Down
                    | Keysym::Page_Up
                    | Keysym::Page_Down
                    | Keysym::Tab => {
                        browser.blur_search();
                        None
                    }
                    // Return is what runs the query. Typing only writes it:
                    // a search of the whole index on every keystroke spends a
                    // round trip apiece to answer questions half-asked, and
                    // the answer to three letters of a word is mostly noise
                    // that the fourth letter throws away.
                    //
                    // The caret stays in the field, so a query that found the
                    // wrong thing can be edited and asked again without
                    // reaching for anything.
                    Keysym::Return | Keysym::KP_Enter if browser.query().is_some() => {
                        browser.run_search();
                        drop(browser);
                        return;
                    }
                    // With nothing typed there is no query to run, so Return
                    // means what it means everywhere else: open what is
                    // selected.
                    Keysym::Return | Keysym::KP_Enter => None,
                    Keysym::Left => Some(TextInputKey::Left),
                    Keysym::Right => Some(TextInputKey::Right),
                    Keysym::Home => Some(TextInputKey::Home),
                    Keysym::End => Some(TextInputKey::End),
                    Keysym::BackSpace => Some(TextInputKey::Backspace),
                    Keysym::Delete => Some(TextInputKey::Delete),
                    Keysym::a if ctrl => Some(TextInputKey::SelectAll),
                    Keysym::c if ctrl => Some(TextInputKey::Copy),
                    Keysym::x if ctrl => Some(TextInputKey::Cut),
                    Keysym::v if ctrl => clipboard::text().map(TextInputKey::Paste),
                    // Every other chord belongs to the window, not the field.
                    _ if ctrl => None,
                    _ => event
                        .utf8
                        .as_ref()
                        .and_then(|s| s.chars().next())
                        .map(TextInputKey::Char),
                };
                if let Some(key) = editing {
                    let mods = KeyMods { shift, ctrl };
                    let response = browser.search.as_mut().map(|input| input.on_key(key, mods));
                    match response {
                        Some(TextInputResponse::Clipboard(text)) => {
                            clipboard::set_text(&text, serial);
                            browser.dirty = true;
                        }
                        Some(TextInputResponse::Changed) => {
                            // Typing does not search — Return does. An
                            // *emptied* field is not a half-written query
                            // though, it is a cancel, and puts back the
                            // listing the search was opened in.
                            if browser.query().is_none() {
                                browser.run_search();
                            } else {
                                browser.dirty = true;
                            }
                        }
                        Some(_) => browser.dirty = true,
                        None => {}
                    }
                    drop(browser);
                    return;
                }
            }

            // In Save mode the name field holds the keyboard focus: what the
            // user is doing is naming a file, so printable keys are the name
            // rather than type-ahead, and Backspace edits it rather than
            // walking up a directory.
            //
            // What the field does *not* take is vertical motion. Up, Down and
            // the page keys still drive the listing, so a user can pick the
            // file they mean to overwrite without ever leaving the field —
            // which is the whole point of it holding focus.
            if browser.save_name.is_some() {
                let editing = match event.keysym {
                    Keysym::Return | Keysym::KP_Enter => {
                        browser.picker_accept();
                        drop(browser);
                        return;
                    }
                    Keysym::Escape => {
                        browser.picker_cancel();
                        drop(browser);
                        return;
                    }
                    Keysym::Up
                    | Keysym::Down
                    | Keysym::Page_Up
                    | Keysym::Page_Down
                    | Keysym::Tab => None,
                    Keysym::Left => Some(TextInputKey::Left),
                    Keysym::Right => Some(TextInputKey::Right),
                    Keysym::Home => Some(TextInputKey::Home),
                    Keysym::End => Some(TextInputKey::End),
                    Keysym::BackSpace => Some(TextInputKey::Backspace),
                    Keysym::Delete => Some(TextInputKey::Delete),
                    Keysym::a if ctrl => Some(TextInputKey::SelectAll),
                    Keysym::c if ctrl => Some(TextInputKey::Copy),
                    Keysym::x if ctrl => Some(TextInputKey::Cut),
                    Keysym::v if ctrl => clipboard::text().map(TextInputKey::Paste),
                    // A chord that is not the field's own is the window's:
                    // Ctrl+W and friends still reach the shortcuts below.
                    _ if ctrl => None,
                    _ => event
                        .utf8
                        .as_ref()
                        .and_then(|s| s.chars().next())
                        .map(TextInputKey::Char),
                };
                if let Some(key) = editing {
                    let mods = KeyMods { shift, ctrl };
                    let response = browser
                        .save_name
                        .as_mut()
                        .map(|input| input.on_key(key, mods));
                    if let Some(TextInputResponse::Clipboard(text)) = response {
                        clipboard::set_text(&text, serial);
                    }
                    browser.dirty = true;
                    drop(browser);
                    return;
                }
            }

            // Set by the type-ahead arm below: every other key ends the
            // word being typed, the way a second of silence does.
            let mut typing = false;
            // Whether the keystroke was spent turning a page of the open
            // preview. It stops the follow below from re-decoding the file at
            // page one and undoing the turn.
            let mut paginated = false;

            match event.keysym {
                // History and hierarchy, on the chords every file manager
                // binds them to. These come first: unmodified, the same keys
                // move the cursor.
                Keysym::Left if mods.alt => browser.go_back(),
                Keysym::Right if mods.alt => browser.go_forward(),
                Keysym::Up if mods.alt => browser.go_up(),
                Keysym::Home if mods.alt => {
                    if let Some(home) = model::home_dir() {
                        browser.navigate_to(&home);
                    }
                }
                Keysym::Down => {
                    let step = browser.row_step();
                    browser.move_cursor(step, shift)
                }
                Keysym::Up => {
                    let step = browser.row_step();
                    browser.move_cursor(-step, shift)
                }
                // The grid is two-dimensional: sideways is the next cell, not
                // a move in or out of a directory the way it is in the list
                // and Miller views.
                Keysym::Right if browser.mode == ViewMode::Grid => browser.move_cursor(1, shift),
                Keysym::Left if browser.mode == ViewMode::Grid => browser.move_cursor(-1, shift),
                Keysym::Right => browser.move_lateral(1),
                Keysym::Left => browser.move_lateral(-1),
                Keysym::Return | Keysym::KP_Enter => {
                    // In the picker, Return means "this one" — descend into a
                    // directory or accept a file. Renaming is file management,
                    // which the picker does not do.
                    if browser.picker.is_some() {
                        browser.open_selection();
                    } else {
                        browser.start_rename();
                    }
                }
                // The Linux convention, alongside Return: both rename.
                Keysym::F2 => browser.start_rename(),
                // Move to Trash. Plain Delete is the spec's binding; the
                // modified forms are there because the chord people reach for
                // is Cmd+Delete, and on a keyboard whose big key is Backspace
                // that arrives as Ctrl+BackSpace. Plain Backspace is *not*
                // one of them — it goes up a directory, and always has.
                //
                // Shift is excluded deliberately: Shift+Delete is spelled for
                // permanent deletion, which is not built. Trashing instead
                // would be the wrong answer to a keystroke that means "and I
                // mean it", so the chord does nothing until it does the right
                // thing.
                Keysym::Delete | Keysym::KP_Delete if !shift && browser.picker.is_none() => {
                    browser.delete_key()
                }
                Keysym::BackSpace if ctrl && !shift && browser.picker.is_none() => {
                    browser.delete_key()
                }
                Keysym::BackSpace => browser.go_up(),
                Keysym::Home => browser.move_cursor(-100_000, shift),
                Keysym::End => browser.move_cursor(100_000, shift),
                // The page keys belong to the content when the content has
                // pages: with a PDF open, Page Down is the next page rather
                // than the next screenful of file names. Everywhere else they
                // mean what they always meant, including on the last page of
                // a PDF — a key that stops working at the end would be worse
                // than one that hands the listing back.
                Keysym::Page_Down => {
                    paginated = browser.turn_peek_page(1);
                    if !paginated {
                        let step = browser.row_step();
                        browser.move_cursor(15 * step, shift)
                    }
                }
                Keysym::Page_Up => {
                    paginated = browser.turn_peek_page(-1);
                    if !paginated {
                        let step = browser.row_step();
                        browser.move_cursor(-15 * step, shift)
                    }
                }
                // Select-all only means something when the request asked for
                // more than one file.
                // With a picture up, select-all takes its words first; a
                // picture without any hands the key back to the listing.
                Keysym::a if ctrl && browser.select_all_peek_words() => {}
                Keysym::a if ctrl => {
                    let multiple = browser.picker.as_ref().is_none_or(|p| p.request.multiple);
                    if multiple {
                        browser.select_all();
                    }
                }
                // Words selected on a previewed picture are copied as text,
                // in either host: copying text is not file management.
                Keysym::c if ctrl && browser.copy_peek_selection(serial) => {}
                // Cut, copy and paste are file management: browser only.
                Keysym::c if ctrl && browser.picker.is_none() => {
                    browser.copy_selection(false, serial)
                }
                Keysym::x if ctrl && browser.picker.is_none() => {
                    browser.copy_selection(true, serial)
                }
                Keysym::v if ctrl && browser.picker.is_none() => browser.paste(),
                // Undo is file management too: a picker is a chooser, and has
                // nothing of its own to take back.
                Keysym::z if ctrl && browser.picker.is_none() => browser.undo_last(),
                // Space toggles: the second press dismisses what the first
                // opened, which is the gesture people already have.
                Keysym::space => {
                    if !browser.close_peek() {
                        self.start_peek(&mut browser);
                    }
                }
                // Escape unwinds one layer at a time: the preview, then the
                // filter menu, then — in the picker — the request itself, and
                // with nothing else up it stops a running operation.
                Keysym::Escape => {
                    let menu_open = browser.picker.as_ref().is_some_and(|p| p.filter_open);
                    // The panel is not modal, so Escape does not belong to it
                    // outright — it takes its turn in the same unwinding order
                    // as everything else that is up.
                    if browser.open_with.is_some() {
                        browser.close_open_with();
                    } else if browser.info.is_some() {
                        browser.close_info();
                    } else if browser.clear_peek_selection() {
                        // A stray drag does not cost the preview.
                    } else if browser.searching {
                        // The field can be closed with results still up. That
                        // is still a search, and Escape's job is to put back
                        // what was on screen before it.
                        browser.clear_search();
                    } else if browser.close_peek() {
                        // The preview took it.
                    } else if menu_open {
                        if let Some(session) = browser.picker.as_mut() {
                            session.filter_open = false;
                        }
                        browser.dirty = true;
                    } else if browser.picker.is_some() {
                        browser.picker_cancel();
                    } else if browser.job.is_some() {
                        // What it has copied so far stands, and stays
                        // undoable. It simply does no more.
                        browser.cancel_job();
                    } else {
                        browser.clear_selection();
                    }
                }
                // A second press closes it, the way Space does for Quick
                // View: the shortcut that opened the panel is the one already
                // under the user's fingers.
                Keysym::i if ctrl => {
                    if browser.info.is_some() {
                        browser.close_info();
                    } else {
                        browser.open_info();
                    }
                }
                Keysym::h if ctrl => {
                    browser.show_hidden = !browser.show_hidden;
                    browser.dirty = true;
                }
                Keysym::n if ctrl => browser.open_new_window(),
                // Every command the window has, by name — Ctrl+P everywhere
                // but the picker, which is answering a request rather than
                // managing files and has none of them.
                Keysym::p if ctrl && browser.picker.is_none() => browser.open_palette(),
                // The location, made editable — Ctrl+L everywhere but the
                // picker, whose header is a toolbar with no title to replace.
                Keysym::l if ctrl && browser.picker.is_none() => browser.open_path_entry(),
                Keysym::f if ctrl && browser.picker.is_none() && !browser.trash => {
                    browser.toggle_search()
                }
                // What a double-click does, from the keyboard: descend into a
                // directory, or activate a file. Return is not free for this —
                // it renames, the way it does on the desktop this follows — so
                // opening needs a chord of its own.
                Keysym::o if ctrl => browser.open_cursor_entry(),
                Keysym::g if ctrl && browser.picker.is_none() && !browser.trash => {
                    browser.add_selection_to_stash()
                }
                Keysym::_1 if ctrl => {
                    browser.set_mode(ViewMode::List);
                }
                Keysym::_2 if ctrl => {
                    browser.set_mode(ViewMode::Grid);
                }
                Keysym::_3 if ctrl => {
                    browser.set_mode(ViewMode::Columns);
                }
                // Anything else printable is type-ahead. It comes last so
                // that every shortcut above keeps the key it already had.
                _ => {
                    // Only an unmodified key is a letter of a name. A chord
                    // this app does not bind is still a chord — it belongs to
                    // whoever does bind it, not to the buffer.
                    let chord = ctrl || mods.alt || mods.logo;
                    if let Some(ch) = event
                        .utf8
                        .as_deref()
                        .filter(|_| !chord)
                        .and_then(|text| text.chars().next())
                        .filter(|ch| !ch.is_control() && *ch != ' ')
                    {
                        browser.typeahead(ch);
                        typing = true;
                    }
                }
            }
            if !typing {
                browser.typeahead = None;
            }

            // The preview follows the cursor: arrow-keying through a folder
            // re-decodes in place rather than dismissing. The generation on
            // each decode is what keeps a slow file from landing late.
            let moved = matches!(
                event.keysym,
                Keysym::Down
                    | Keysym::Up
                    | Keysym::Left
                    | Keysym::Right
                    | Keysym::Home
                    | Keysym::End
                    | Keysym::Page_Down
                    | Keysym::Page_Up
            );
            if moved && !paginated && browser.peek.is_some() {
                self.start_peek(&mut browser);
            }
        }
    }
}
