//! Open With: the browser's side of the chooser.
//!
//! [`crate::open_with::Chooser`] holds the list and its answer; this is where
//! a chooser is opened for the selection, fed keys and clicks from its window,
//! and carried out.

use super::*;
use crate::open_with::{Chooser, Row};
use otto_kit::mime_apps;

/// An open chooser, with its field and what the pointer is doing to it.
pub(super) struct OpenWithSession {
    pub(super) chooser: Chooser,
    pub(super) input: TextInput,
    pub(super) hover: Option<view::OpenWithHit>,
    pub(super) pressed: Option<view::OpenWithHit>,
    /// The list's scroll: momentum, the rubber band past either end, and the
    /// overlay bar, like every other list in the window.
    pub(super) scroll: ScrollView,
    /// The last press on a row, to tell a double-click from two clicks.
    last_row_press: Option<(usize, std::time::Instant)>,
}

/// A key the chooser answers to, already told apart from the keymap.
pub(super) enum OpenWithKey {
    Up,
    Down,
    Enter,
    Escape,
    Edit(TextInputKey),
}

impl Browser {
    /// Open the chooser for the selected files.
    ///
    /// Folders are left out: opening one is going into it, and what opens a
    /// folder elsewhere is the file manager this already is. So is anything
    /// in the Trash, for the reason a double-click there is refused.
    pub(super) fn open_with_selection(&mut self) {
        if self.picker.is_some() {
            return;
        }
        if self.trash {
            self.refuse(otto_kit::t_owned!("files-trash-cant-open"));
            return;
        }
        let paths: Vec<PathBuf> = self
            .selected_entries()
            .into_iter()
            .filter(|entry| !entry.is_dir)
            .map(|entry| entry.path)
            .collect();
        if paths.is_empty() {
            return;
        }
        let associations = mime_apps::Associations::load();
        let chooser = Chooser::new(paths, &associations);
        let input = TextInput::editing(
            String::new(),
            view::search_field_style(AppContext::current_theme()),
        );
        let mut session = OpenWithSession {
            chooser,
            input,
            hover: None,
            pressed: None,
            scroll: ScrollView::new(view::open_with_list_rect(open_with_sheet())),
            last_row_press: None,
        };
        session.fit_scroll();
        self.open_with = Some(session);
        self.open_with_dirty = true;
    }

    pub(super) fn close_open_with(&mut self) {
        if self.open_with.take().is_some() {
            self.open_with_dirty = true;
        }
    }

    /// Open the files with the highlighted application, remembering it as
    /// the type's default if the box is ticked.
    ///
    /// A default that cannot be written does not stop the files opening: the
    /// user still asked for them, and the status line says the choice was not
    /// kept. A launch that fails leaves the chooser up, saying why, so
    /// another application can be picked.
    pub(super) fn open_with_accept(&mut self) {
        let Some(session) = self.open_with.as_mut() else {
            return;
        };
        let Some(app) = session.chooser.selected().cloned() else {
            return;
        };
        let chooser = &mut session.chooser;
        let remember = chooser.always.then(|| chooser.mime.clone()).flatten();

        if let Err(err) = mime_apps::open(&app, &chooser.paths) {
            chooser.error = Some(otto_kit::t_owned!(
                "files-open-failed",
                error = open_error_text(&err)
            ));
            self.open_with_dirty = true;
            return;
        }
        if let Some(mime) = remember {
            if let Err(err) = mime_apps::set_default(&mime, &app.id) {
                self.status = Some(otto_kit::t_owned!(
                    "files-open-with-not-remembered",
                    error = err.to_string()
                ));
                self.dirty = true;
            }
        }
        self.close_open_with();
    }

    /// A key pressed while the chooser has the keyboard.
    pub(super) fn open_with_key(&mut self, key: OpenWithKey, mods: KeyMods) {
        let Some(session) = self.open_with.as_mut() else {
            return;
        };
        match key {
            OpenWithKey::Escape => {
                self.close_open_with();
                return;
            }
            OpenWithKey::Enter => {
                let chooser = &mut session.chooser;
                let Some(index) = chooser.highlight else {
                    return;
                };
                if chooser.activate(index).is_some() {
                    self.open_with_accept();
                    return;
                }
                session.fit_scroll();
            }
            OpenWithKey::Up | OpenWithKey::Down => {
                let delta = if matches!(key, OpenWithKey::Up) {
                    -1
                } else {
                    1
                };
                session.chooser.move_highlight(delta);
                session.reveal_highlight();
            }
            OpenWithKey::Edit(edit) => {
                session.input.on_key(edit, mods);
                let query = session.input.value().to_string();
                if query != session.chooser.query {
                    session.chooser.set_query(&query);
                    session.fit_scroll();
                    session.scroll.scroll_to(0.0);
                }
            }
        }
        self.open_with_dirty = true;
    }

    /// A pointer event on the chooser's window. Returns true when a press
    /// landed on the strip the window is dragged by, so the caller can start
    /// the move.
    pub(super) fn open_with_pointer(&mut self, kind: &PointerEventKind, x: f32, y: f32) -> bool {
        let Some(session) = self.open_with.as_mut() else {
            return false;
        };
        let sheet = open_with_sheet();
        let hit = {
            let chooser = &session.chooser;
            view::open_with_hit(
                sheet,
                x,
                y,
                chooser.rows().len(),
                session.scroll.offset(),
                chooser.can_remember(),
            )
        };

        match kind {
            PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                // A thumb drag follows the pointer wherever it goes; otherwise
                // the bar only needs to know it is being hovered.
                if session.scroll.on_pointer_drag(x, y) {
                    self.open_with_dirty = true;
                    return false;
                }
                session.scroll.on_pointer_move(x, y);
                if session.hover != hit {
                    session.hover = hit;
                    self.open_with_dirty = true;
                }
            }
            PointerEventKind::Leave { .. } => {
                session.scroll.on_pointer_leave();
                session.hover = None;
                session.pressed = None;
                self.open_with_dirty = true;
            }
            PointerEventKind::Release { .. } => {
                session.scroll.on_pointer_up();
                let pressed = session.pressed.take();
                self.open_with_dirty = true;
                // A button answers on release, over the button it was pressed
                // on, so a press can still be taken back by sliding off.
                match (pressed, hit) {
                    (Some(view::OpenWithHit::Open), Some(view::OpenWithHit::Open)) => {
                        self.open_with_accept();
                    }
                    (Some(view::OpenWithHit::Cancel), Some(view::OpenWithHit::Cancel)) => {
                        self.close_open_with();
                    }
                    _ => {}
                }
            }
            PointerEventKind::Press { .. } => {
                self.open_with_dirty = true;
                // The bar's thumb sits over the rows' right edge, and a press
                // on it is a drag of the list, not a pick.
                if session.scroll.on_pointer_down(x, y) {
                    return false;
                }
                match hit {
                    Some(view::OpenWithHit::Close) => self.close_open_with(),
                    Some(view::OpenWithHit::Titlebar) => return true,
                    Some(view::OpenWithHit::Always) => session.chooser.toggle_always(),
                    Some(button @ (view::OpenWithHit::Open | view::OpenWithHit::Cancel)) => {
                        session.pressed = Some(button);
                    }
                    Some(view::OpenWithHit::Search) => {
                        let field = view::open_with_search_rect(sheet);
                        session.input.on_pointer_down(
                            x - field.left - view::OPEN_WITH_SEARCH_INSET,
                            1,
                            false,
                        );
                    }
                    Some(view::OpenWithHit::Row(index)) => {
                        let now = std::time::Instant::now();
                        let double = session.last_row_press.is_some_and(|(row, at)| {
                            row == index && now.duration_since(at) < DOUBLE_CLICK_WINDOW
                        });
                        session.last_row_press = Some((index, now));
                        let is_app =
                            matches!(session.chooser.rows().get(index), Some(Row::App { .. }));
                        if !is_app {
                            session.chooser.activate(index);
                            session.fit_scroll();
                        } else if double {
                            session.chooser.highlight = Some(index);
                            self.open_with_accept();
                        } else {
                            session.chooser.highlight = Some(index);
                        }
                    }
                    None => {}
                }
            }
            // A touchpad's stream carries momentum and stretches past the
            // ends; a notched wheel steps.
            PointerEventKind::Axis { vertical, .. } => {
                let scroll = &mut session.scroll;
                if vertical.stop {
                    scroll.on_wheel_end();
                } else if vertical.discrete != 0 {
                    scroll.on_wheel_discrete(vertical.absolute as f32);
                } else {
                    scroll.on_wheel(vertical.absolute as f32);
                }
                self.open_with_dirty = true;
            }
        }
        false
    }
}

/// Why an application did not start, in the interface's language where the
/// reason is ours to word.
pub(super) fn open_error_text(err: &mime_apps::OpenError) -> String {
    match err {
        mime_apps::OpenError::Spawn(err) => err.to_string(),
        mime_apps::OpenError::NoCommand | mime_apps::OpenError::BadCommand(_) => {
            otto_kit::t_owned!("files-open-app-broken")
        }
    }
}

/// The chooser's own window, in its own coordinates: the window is the card.
pub(super) fn open_with_sheet() -> skia_safe::Rect {
    skia_safe::Rect::from_wh(view::OPEN_WITH_W, view::OPEN_WITH_H)
}

impl OpenWithSession {
    /// Tell the scroll view how long the list is now. After anything that
    /// changes which rows there are: a query, the heading folding.
    fn fit_scroll(&mut self) {
        let content = view::open_with_content_h(self.chooser.rows().len());
        if self.scroll.state.content_length() != content {
            self.scroll.set_content_length(content);
        }
    }

    /// Scroll the list by the least that brings the highlighted row into view.
    fn reveal_highlight(&mut self) {
        if let Some(index) = self.chooser.highlight {
            let offset = view::open_with_reveal(open_with_sheet(), index, self.scroll.offset());
            if offset != self.scroll.offset() {
                self.scroll.scroll_to(offset);
            }
        }
    }
}

impl Browser {
    /// Advance the chooser list's glide, bounce and bar fade by one tick.
    pub(super) fn tick_open_with_scroll(&mut self) {
        let Some(session) = self.open_with.as_mut() else {
            return;
        };
        if session.scroll.is_animating() {
            session.scroll.tick();
            self.open_with_dirty = true;
        }
    }

    /// Whether the chooser's list still has motion to run.
    pub(super) fn open_with_scrolling(&self) -> bool {
        self.open_with
            .as_ref()
            .is_some_and(|session| session.scroll.is_animating())
    }
}
