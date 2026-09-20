//! The command palette's session: its card, list, drag and effects.

use super::*;

impl Browser {
    /// The window as a command provider sees it.
    ///
    /// A description, not a handle: everything here could be written down and
    /// sent to a provider in another process, which is what keeps the seam
    /// worth having.
    pub(super) fn situation(&self) -> command::Situation {
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let visible = self.visible(depth);
        let cursor = self.columns[depth]
            .cursor
            .and_then(|index| visible.get(index).copied());
        command::Situation {
            path: self.current_directory(),
            selection: self
                .selected_entries()
                .into_iter()
                .map(|e| e.path)
                .collect(),
            siblings: visible.iter().map(|entry| entry.name.clone()).collect(),
            cursor_name: cursor.map(|entry| entry.name.clone()),
            cursor_is_dir: cursor.is_some_and(|entry| entry.is_dir),
            trash: self.trash,
            recent: self.recent,
            can_paste: !self.clipboard.is_empty()
                || clipboard::first_available(clipboard::file_mime_preference()).is_some(),
            can_undo: !self.undo.is_empty(),
            can_go_back: !self.back.is_empty(),
            can_go_forward: !self.forward.is_empty(),
            can_go_up: self.current_directory().parent().is_some(),
            has_entries: !visible.is_empty(),
            show_hidden: self.show_hidden,
            view: view_id(self.mode).to_string(),
            sort: sort_id(self.sort).to_string(),
            can_recognise_text: ocr::enabled() && ocr::available(),
            places: self
                .places
                .iter()
                .map(|place| command::PlaceRef {
                    label: place.label.clone(),
                    path: place.path.clone(),
                })
                .collect(),
        }
    }

    /// The situation a palette command is previewed and run against: the
    /// window's, less whatever the dry run's lines were toggled out of it.
    pub(super) fn palette_situation(&self) -> command::Situation {
        let excluded = self
            .palette
            .as_ref()
            .map(|palette| palette.excluded())
            .unwrap_or_default();
        self.situation().excluding(&excluded)
    }

    /// What a palette command would act on, by name and in order: the
    /// selection, or the cursor's entry when nothing is selected. The list
    /// a dry run's lines are threaded onto.
    pub(super) fn palette_targets(&self) -> Vec<String> {
        let situation = self.situation();
        if situation.selection.is_empty() {
            return situation.cursor_name.into_iter().collect();
        }
        situation
            .selection
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect()
    }

    /// Ctrl+P. Asks every provider what it can offer *now* and opens onto the
    /// answer; nothing re-gathers while the palette is up.
    pub(super) fn open_palette(&mut self) {
        let situation = self.situation();
        let commands = self.commands.commands(&situation);
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let column = &self.columns[depth];
        self.palette_selection = Some(SelectionMark {
            depth,
            selection: column.selection.clone(),
            cursor: column.cursor,
            anchor: column.anchor,
        });
        self.palette = Some(palette::Palette::open(
            commands,
            view::palette_field_style(AppContext::current_theme()),
        ));
        // Where it was last left, if anywhere — brought back on screen below
        // in case the window has shrunk since.
        self.palette_offset = self.palette_memory.unwrap_or_default();
        self.palette_scroll = ScrollView::new(Rect::new_empty());
        self.palette_display = None;
        self.palette_caret = None;
        self.palette_drag = None;
        self.clamp_palette();
        self.dirty = true;
    }

    /// Close it without running anything. The browser underneath is exactly as
    /// it was — the palette never changed it by being open.
    pub(super) fn close_palette(&mut self) -> bool {
        let was_open = self.palette.take().is_some();
        // Whatever a previewed argument selected along the way is not what the
        // user asked for: they backed out.
        self.restore_palette_selection();
        self.palette_selection = None;
        if was_open {
            self.dirty = true;
        }
        was_open
    }

    /// Close the palette with what it did left standing — the way out after a
    /// command actually ran.
    pub(super) fn finish_palette(&mut self) -> bool {
        let was_open = self.palette.take().is_some();
        self.palette_selection = None;
        if was_open {
            self.dirty = true;
        }
        was_open
    }

    /// Put the selection back to what it was when the palette opened.
    pub(super) fn restore_palette_selection(&mut self) {
        let Some(mark) = self.palette_selection.clone() else {
            return;
        };
        if let Some(column) = self.columns.get_mut(mark.depth) {
            column.selection = mark.selection;
            column.cursor = mark.cursor;
            column.anchor = mark.anchor;
        }
    }

    /// Show a previewed argument as it is typed.
    ///
    /// Every keystroke answers afresh from the selection the palette opened
    /// on, so deleting a character widens the selection again rather than
    /// leaving the last narrower answer standing.
    pub(super) fn preview_palette_argument(&mut self) {
        let Some((id, typed)) = self
            .palette
            .as_ref()
            .and_then(|palette| palette.previewed_argument())
            .map(|(id, typed)| (id.to_string(), typed.to_string()))
        else {
            return;
        };
        self.restore_palette_selection();
        if typed.trim().is_empty() {
            if let Some(palette) = self.palette.as_mut() {
                palette.set_preview(Vec::new());
            }
            self.dirty = true;
            return;
        }
        // A provider's command: ask it for the dry run — without whatever
        // has been toggled out — and thread its lines back among the names
        // left out, so those can be brought back.
        if command::Request::new(id.clone(), None)
            .namespace()
            .is_some()
        {
            let situation = self.palette_situation();
            let targets = self.palette_targets();
            let request = command::Request::new(id, Some(typed));
            let preview = self.commands.preview(&request, &situation);
            if let Some(palette) = self.palette.as_mut() {
                let excluded = palette.excluded();
                let (rows, note) = match preview {
                    Some(preview) => (preview.rows, preview.note),
                    None => (Vec::new(), None),
                };
                palette.set_preview(palette::PreviewLine::merge(&targets, &excluded, rows));
                if note.is_some() {
                    palette.set_note(note);
                }
            }
            self.dirty = true;
            return;
        }
        if id == command::id::SELECT_MATCHING {
            // A pattern that matches nothing leaves the restored selection
            // standing; the palette says so in its note rather than as an
            // error, since a half-typed pattern is not wrong.
            let _ = self.select_matching(&typed);
            let note = self.path_bar_note().unwrap_or_else(|| {
                otto_kit::t_owned!("files-nothing-matches", pattern = typed.as_str())
            });
            if let Some(palette) = self.palette.as_mut() {
                palette.set_note(Some(note));
            }
        }
        self.dirty = true;
    }

    /// Fill in what a half-typed path could be completed to.
    ///
    /// The palette itself never touches the disk, so this is the host's half
    /// of the bargain: it is called after every key, and does nothing at all
    /// unless a path argument is open.
    pub(super) fn refresh_palette_completions(&mut self) {
        let Some(mut palette) = self.palette.take() else {
            return;
        };
        if let Some((typed, dirs_only)) = palette.path_argument() {
            let completions = self.path_completions(typed, dirs_only);
            palette.set_completions(completions);
        }
        self.palette = Some(palette);
    }

    /// What `typed` could be completed to, as whole paths.
    ///
    /// The same rules the location bar completes by: a bare name resolves
    /// against the directory on screen, and a dotfile is only a candidate once
    /// the dot has been typed. The read is synchronous — one directory, on a
    /// keystroke somebody is waiting on.
    pub(super) fn path_completions(
        &self,
        typed: &str,
        dirs_only: bool,
    ) -> Vec<palette::Completion> {
        let (head, prefix) = match typed.rfind('/') {
            Some(cut) => (&typed[..=cut], &typed[cut + 1..]),
            None => ("", typed),
        };
        let Some(dir) = self.resolve_typed_path(if head.is_empty() { "." } else { head }) else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut found: Vec<(String, bool)> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let is_dir = entry.file_type().map(|k| k.is_dir()).unwrap_or(false);
                (name, is_dir)
            })
            .filter(|(name, is_dir)| {
                name.starts_with(prefix)
                    && (prefix.starts_with('.') || !name.starts_with('.'))
                    && (!dirs_only || *is_dir)
            })
            .collect();
        found.sort_by(|a, b| model::natural_cmp(&a.0, &b.0));
        found.truncate(PALETTE_COMPLETION_LIMIT);
        found
            .into_iter()
            .map(|(name, is_dir)| {
                // A completed directory gains its separator, so Tab walks down
                // a tree without anyone reaching for `/` between levels.
                let value = if is_dir {
                    format!("{head}{name}/")
                } else {
                    format!("{head}{name}")
                };
                palette::Completion::new(value, name)
            })
            .collect()
    }

    /// The card, where it is now — the resting rect moved by whatever drag has
    /// been applied to it.
    pub(super) fn palette_card(&mut self) -> Rect {
        let rows = self.palette_layout();
        let view_rows = Self::palette_view_rows(&rows);
        let message = self.palette_message().is_some();
        view::palette_rect(self.size.0, &view_rows, message).with_offset(self.palette_offset)
    }

    /// The band a drag takes hold of: the card's top, where the field is.
    ///
    /// The field rather than a bar of its own — the palette is one card with a
    /// line of text across the top of it, and a strip above that line would be
    /// chrome for its own sake. The rows below it are for picking.
    pub(super) fn palette_grip_at(&mut self, x: f32, y: f32) -> bool {
        let card = self.palette_card();
        Rect::from_ltrb(
            card.left,
            card.top,
            card.right,
            card.top + view::PALETTE_FIELD_H,
        )
        .contains(skia_safe::Point::new(x, y))
    }

    /// A press at `(x, y)` in window points, while the palette is up.
    ///
    /// The top band takes hold of the card; a row is picked, the way Return
    /// picks the highlighted one; the rest of the card is inert; and a press
    /// anywhere else lets the palette go, spending the click on that rather
    /// than on whatever file was underneath.
    ///
    /// One method for both doors the press can come through — the palette's
    /// own catcher surface, and the toplevel when the card is painted into the
    /// window — since both arrive in the same coordinates.
    pub(super) fn palette_press(&mut self, x: f32, y: f32, serial: u32) {
        if self.palette_grip_at(x, y) {
            let card = self.palette_card();
            self.palette_drag = Some((x - card.left, y - card.top));
            self.dirty = true;
            return;
        }
        if let Some(row) = self.palette_row_under(x, y) {
            // A dry-run line is toggled, not picked: the file leaves the run
            // or comes back, and the dry run is made again without it.
            let toggled = self
                .palette
                .as_mut()
                .is_some_and(|palette| palette.toggle_row(row));
            if toggled {
                self.preview_palette_argument();
                self.dirty = true;
                return;
            }
            let picked = self
                .palette
                .as_mut()
                .is_some_and(|palette| palette.highlight_row(row));
            if picked {
                let mods = KeyMods {
                    shift: false,
                    ctrl: false,
                };
                if let Some(outcome) = self
                    .palette
                    .as_mut()
                    .map(|palette| palette.on_key(palette::Key::Enter, mods))
                {
                    self.settle_palette(outcome, serial);
                }
            }
            self.dirty = true;
            return;
        }
        let card = self.palette_card();
        if !card.contains(skia_safe::Point::new(x, y)) {
            self.close_palette();
        }
        self.dirty = true;
    }

    /// The pointer moving over the list, with no button down: the row under it
    /// takes the highlight, so what Return would do is always what the pointer
    /// is resting on.
    pub(super) fn palette_hover(&mut self, x: f32, y: f32) {
        let Some(row) = self.palette_row_under(x, y) else {
            return;
        };
        let moved = self.palette.as_mut().is_some_and(|palette| {
            palette.highlighted() != Some(row) && palette.highlight_row(row)
        });
        self.dirty |= moved;
    }

    /// A scroll over the card. The list flings and springs like a column's:
    /// a touchpad's stream carries momentum and stretches past the ends, a
    /// notched wheel steps.
    pub(super) fn palette_wheel(&mut self, x: f32, y: f32, dy: f32, stop: bool, discrete: bool) {
        let card = self.palette_card();
        if !card.contains(skia_safe::Point::new(x, y)) {
            return;
        }
        let _ = self.palette_layout();
        let scroll = &mut self.palette_scroll;
        if stop {
            scroll.on_wheel_end();
        } else if discrete {
            scroll.on_wheel_discrete(dy);
        } else {
            scroll.on_wheel(dy);
        }
    }

    /// Advance the palette list's glide by one tick. Returns whether it moved.
    pub(super) fn tick_palette_scroll(&mut self) -> bool {
        self.palette_scroll.is_animating() && self.palette_scroll.tick()
    }

    /// Everything the palette's own surface needs, with its text owned.
    ///
    /// Owned rather than borrowed because the surface's draw closure needs the
    /// palette back, mutably, to render its text field into the same canvas.
    pub(super) fn palette_frame(&mut self) -> Option<pane_surfaces::PaletteFrame> {
        self.palette.as_ref()?;
        self.clamp_palette();
        let rows = self.palette_layout();
        let message = self.palette_message();
        let prompt = self.palette.as_ref().and_then(|p| p.prompt());
        let view_rows = Self::palette_view_rows(&rows);
        let resting = view::palette_rect(self.size.0, &view_rows, message.is_some());
        drop(view_rows);
        Some(pane_surfaces::PaletteFrame {
            resting,
            card: resting.with_offset(self.palette_offset),
            prompt,
            message,
            rows,
            scroll: self.palette_scroll.state,
            velocity: self.palette_scroll.velocity(),
            field_key: self.palette_field_key(),
        })
    }

    /// What the palette's field would paint, as a key: its text, caret,
    /// selection, focus, placeholder and whether the caret is in its blink.
    pub(super) fn palette_field_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let Some(palette) = self.palette.as_ref() else {
            return 0;
        };
        let input = palette.input();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        input.value().hash(&mut hasher);
        input.state.caret().hash(&mut hasher);
        input.state.selection().hash(&mut hasher);
        input.state.focused().hash(&mut hasher);
        input.caret_visible().hash(&mut hasher);
        palette.placeholder().hash(&mut hasher);
        hasher.finish()
    }

    /// Paint the palette's text field, in window points.
    ///
    /// Split out of the card's drawing because the field renders itself and so
    /// needs the palette borrowed mutably, which the card's owned frame
    /// deliberately does not hold. Records where the caret ended up, since on
    /// its own surface this runs outside the window's draw and so outside the
    /// chain that reports it.
    pub(super) fn paint_palette_field(&mut self, canvas: &skia_safe::Canvas) {
        let width = self.size.0;
        let field = view::palette_field_rect(width);
        let prompt = self.palette.as_ref().and_then(|p| p.prompt());
        // The prompt is not part of the field, so the field starts after it:
        // what is typed is the argument, and the prefix cannot be edited.
        let lead = prompt
            .as_deref()
            .map(|prompt| {
                otto_kit::typography::styles::BODY_EMPHASIZED
                    .font()
                    .measure_str(prompt, None)
                    .0
                    + 8.0
            })
            .unwrap_or(0.0);
        let placeholder = self
            .palette
            .as_ref()
            .map(|p| p.placeholder())
            .unwrap_or_default();
        let offset = self.palette_offset;
        let Some(palette) = self.palette.as_mut() else {
            return;
        };
        let input = palette.input_mut();
        input.state.placeholder = placeholder;
        input.set_size(field.width() - lead, field.height());
        let origin = (field.left + lead, field.top);
        canvas.save();
        canvas.translate(origin);
        input.render_at(canvas, field.width() - lead, field.height());
        canvas.restore();
        // Reported in window points like every other caret, which stays
        // meaningful once the card is dragged clear of the window: the offset
        // says where it went.
        self.palette_caret = caret_in_window(
            self.palette.as_ref().expect("checked above").input(),
            (origin.0 + offset.0, origin.1 + offset.1),
        );
    }

    /// Follow the pointer, keeping the card on screen.
    ///
    /// On its own surface the card may leave the window — that is the point of
    /// the surface — so the window's edges are no longer the limit. The
    /// *display's* are: a panel dragged off the screen is one nobody can find
    /// the way back to, and it holds the keyboard while it is gone.
    ///
    /// The client is never told where its own window sits, so where the
    /// display is has to be asked for; see
    /// [`pane_surfaces::PaneSurfaces::palette_display`]. Until the answer
    /// arrives the window stands in for it, which is the old behaviour and
    /// wrong only in being too strict.
    pub(super) fn drag_palette_to(&mut self, x: f32, y: f32) {
        let Some((grab_x, grab_y)) = self.palette_drag else {
            return;
        };
        let card = self.palette_card();
        // Where the card would be with no drag applied — the offset is
        // measured from there, so it has to be taken back out first.
        let resting = (
            card.left - self.palette_offset.0,
            card.top - self.palette_offset.1,
        );
        let bounds = self.palette_bounds();
        let left = (x - grab_x).clamp(bounds.left, (bounds.right - card.width()).max(bounds.left));
        // Never above the top edge, and never so far down that the field is
        // off the bottom: the list may hang past it, but the line being typed
        // may not.
        let top = (y - grab_y).clamp(
            bounds.top,
            (bounds.bottom - view::PALETTE_FIELD_H).max(bounds.top),
        );
        self.palette_offset = (left - resting.0, top - resting.1);
        self.dirty = true;
    }

    /// Keep the card inside the bounds it may be dragged around: a remembered
    /// position from a larger window, or a display answer that has just
    /// arrived, may otherwise leave it out of reach.
    pub(super) fn clamp_palette(&mut self) {
        if self.palette.is_none() {
            return;
        }
        let card = self.palette_card();
        let resting = (
            card.left - self.palette_offset.0,
            card.top - self.palette_offset.1,
        );
        let bounds = self.palette_bounds();
        let left = card
            .left
            .clamp(bounds.left, (bounds.right - card.width()).max(bounds.left));
        let top = card.top.clamp(
            bounds.top,
            (bounds.bottom - view::PALETTE_FIELD_H).max(bounds.top),
        );
        let offset = (left - resting.0, top - resting.1);
        if offset != self.palette_offset {
            self.palette_offset = offset;
            self.dirty = true;
        }
    }

    /// The card has been let go of: it stays where it is, and the next open
    /// finds it there.
    pub(super) fn palette_dropped(&mut self) {
        self.palette_memory = Some(self.palette_offset);
        if self.palette_memory_on_disk {
            let mut remembered = remembered::Remembered::load();
            remembered.palette.offset = Some(self.palette_offset);
            remembered.save();
        }
        self.dirty = true;
    }

    /// Take on what was remembered from the last run, and remember on disk
    /// from here on.
    pub fn remember(&mut self, remembered: &remembered::Remembered) {
        self.palette_memory = remembered.palette.offset;
        self.palette_memory_on_disk = true;
    }

    /// How far the palette may be dragged, in window points: the display when
    /// the compositor has said where it is, and the window until then.
    pub(super) fn palette_bounds(&self) -> Rect {
        self.palette_display
            .unwrap_or_else(|| Rect::from_wh(self.size.0, self.size.1))
    }

    /// The palette's rows as the view wants them, scrolled so the highlight is
    /// on screen.
    pub(super) fn palette_rows(&self) -> Vec<PaletteRowData> {
        let Some(palette) = self.palette.as_ref() else {
            return Vec::new();
        };
        let highlight = palette.highlighted();
        let resting = palette.resting();
        palette
            .rows()
            .iter()
            .enumerate()
            .map(|(index, row)| match row {
                palette::Row::Heading(group) => PaletteRowData {
                    kind: view::PaletteRowKind::Heading,
                    title: group.label(),
                    badge: None,
                    subtitle: None,
                    shortcut: None,
                    highlighted: false,
                },
                palette::Row::Command(command) => {
                    let command = &palette.commands()[*command];
                    PaletteRowData {
                        kind: view::PaletteRowKind::Item,
                        title: command.title.clone(),
                        // What the command will go on to ask for, so a row
                        // that leads somewhere rather than acting says so
                        // before it is picked.
                        badge: command.arg.as_ref().map(|arg| format!("› {}", arg.label)),
                        // The group is a heading when the list is resting, so
                        // it only becomes a badge once ranking has thrown the
                        // headings away.
                        subtitle: (!resting).then(|| command.group.label()),
                        shortcut: command.shortcut.clone(),
                        highlighted: highlight == Some(index),
                    }
                }
                palette::Row::Completion(completion) => {
                    let completion = &palette.completions()[*completion];
                    PaletteRowData {
                        kind: view::PaletteRowKind::Item,
                        title: completion.title.clone(),
                        badge: None,
                        subtitle: completion.subtitle.clone(),
                        shortcut: None,
                        highlighted: highlight == Some(index),
                    }
                }
                palette::Row::Preview(line) => {
                    let line = &palette.preview()[*line];
                    PaletteRowData {
                        kind: view::PaletteRowKind::Preview {
                            conflict: line.conflict,
                            excluded: line.excluded,
                        },
                        // A line in the run that names no outcome — "these
                        // go in" — is just the name; there is no arrow to
                        // draw. A line toggled out keeps its shape.
                        title: if line.to.is_empty() && !line.excluded {
                            line.from.clone()
                        } else {
                            line.to.clone()
                        },
                        badge: None,
                        subtitle: (!line.to.is_empty() || line.excluded).then(|| line.from.clone()),
                        shortcut: None,
                        highlighted: highlight == Some(index),
                    }
                }
            })
            .collect()
    }

    /// Size the list's scroll view to the rows it holds, and return those
    /// rows. Everything that measures the list — painting, hit-testing,
    /// revealing the highlight — goes through here, so they cannot disagree.
    pub(super) fn palette_layout(&mut self) -> Vec<PaletteRowData> {
        let rows = self.palette_rows();
        let message = self.palette_message().is_some();
        let view_rows = Self::palette_view_rows(&rows);
        let viewport = view::palette_list_rect(self.size.0, &view_rows, message);
        let content = view::palette_content_h(&view_rows);
        drop(view_rows);
        if self.palette_scroll.state.viewport() != viewport {
            self.palette_scroll.set_viewport(viewport);
        }
        if self.palette_scroll.state.content_length() != content {
            self.palette_scroll.set_content_length(content);
        }
        rows
    }

    /// The rows as the view measures them — text borrowed, nothing else.
    pub(super) fn palette_view_rows(rows: &[PaletteRowData]) -> Vec<view::PaletteRow<'_>> {
        rows.iter()
            .map(|row| view::PaletteRow {
                kind: row.kind,
                title: &row.title,
                badge: row.badge.as_deref(),
                subtitle: row.subtitle.as_deref(),
                shortcut: row.shortcut.as_deref(),
                highlighted: row.highlighted,
            })
            .collect()
    }

    /// Scroll the list by the least that brings the highlighted row into
    /// view. After every key, so the arrows walk a long list a row at a time.
    pub(super) fn palette_reveal_highlight(&mut self) {
        let Some(highlight) = self.palette.as_ref().and_then(|p| p.highlighted()) else {
            return;
        };
        let rows = self.palette_layout();
        let view_rows = Self::palette_view_rows(&rows);
        if highlight >= view_rows.len() {
            return;
        }
        let row = view::palette_row_rect(self.size.0, &view_rows, highlight);
        let viewport = self.palette_scroll.state.viewport();
        let offset = view::palette_reveal(row, viewport, self.palette_scroll.offset());
        if offset != self.palette_scroll.offset() {
            self.palette_scroll.scroll_to(offset);
        }
    }

    /// Which row of the list is under `(x, y)` in window points, through the
    /// list's scroll: the rows are hit-tested where they lie, shifted by how
    /// far they have been scrolled.
    pub(super) fn palette_row_under(&mut self, x: f32, y: f32) -> Option<usize> {
        let rows = self.palette_layout();
        let view_rows = Self::palette_view_rows(&rows);
        let (x, y) = (x - self.palette_offset.0, y - self.palette_offset.1);
        let viewport = self.palette_scroll.state.viewport();
        if !viewport.contains(skia_safe::Point::new(x, y)) {
            return None;
        }
        view::palette_row_at(x, y + self.palette_scroll.offset(), self.size.0, &view_rows)
    }

    /// What to show in place of the list: a refusal, or that nothing matches.
    pub(super) fn palette_message(&self) -> Option<String> {
        let palette = self.palette.as_ref()?;
        if let Some(error) = palette.error() {
            return Some(error.to_string());
        }
        // What the argument being typed is doing right now, when the host
        // has said — the live answer to a previewed pattern.
        if let Some(note) = palette.note() {
            return Some(note.to_string());
        }
        // Only about the *command* list. An argument with nothing under it —
        // a name, a pattern, anything free-text — has no completions by
        // nature, and saying "no command matches" about it is both wrong and
        // alarming.
        (palette.prompt().is_none() && palette.rows().is_empty() && !palette.resting())
            .then(|| otto_kit::t_owned!("files-palette-no-matches"))
    }

    /// Act on what the palette made of an event, whether it came from a key or
    /// from a click on a row.
    ///
    /// The one place a palette outcome is turned into something happening, so
    /// the pointer and the keyboard cannot drift apart over what picking a row
    /// means.
    pub(super) fn settle_palette(&mut self, outcome: palette::Outcome, serial: u32) {
        match outcome {
            palette::Outcome::Close => {
                self.close_palette();
            }
            palette::Outcome::Clipboard(text) => {
                clipboard::set_text(&text, serial);
                self.dirty = true;
            }
            palette::Outcome::Run(request) => match self.run_request(&request, serial) {
                Ok(followup) => {
                    // Closed only once the command was carried out: a refusal
                    // keeps the panel up with the text still there to fix.
                    self.finish_palette();
                    self.palette_peek = followup == Followup::Peek;
                }
                Err(error) => {
                    if let Some(palette) = self.palette.as_mut() {
                        palette.set_error(error);
                    }
                    self.dirty = true;
                }
            },
            palette::Outcome::Changed => {
                self.refresh_palette_completions();
                self.preview_palette_argument();
                self.palette_reveal_highlight();
                self.dirty = true;
            }
            palette::Outcome::Ignored => {}
        }
    }

    /// Whether a palette command asked for Peek. Drained by the host,
    /// which owns the decode — see [`FilesApp::follow_peek`].
    pub(super) fn take_palette_peek(&mut self) -> bool {
        std::mem::take(&mut self.palette_peek)
    }

    /// Take on board what a provider's command did: the status line, the undo
    /// entry, the re-read. The one place a provider's outcome touches the
    /// window, so a provider in another process would go through the same
    /// door.
    pub(super) fn apply_effect(&mut self, effect: command::Effect) {
        if let Some(status) = effect.status {
            self.status = Some(status);
        }
        // A command is heard by what it did, not by who did it: a script that
        // makes a file sounds like any other file arriving.
        Self::play_op_sound(&super::file_ops::sounds_like(&effect.changes));
        if let Some(label) = effect.undo_label {
            self.record_undo(label, effect.changes);
        }
        if effect.reload {
            self.reload_all();
        }
        self.dirty = true;
    }

    /// Carry out a request the palette produced.
    ///
    /// The host answers for its own namespace because it is the only thing
    /// holding the window; anything else goes to the provider that owns it.
    /// An `Err` keeps the palette open with the reason shown, so the text is
    /// still there to fix.
    pub(super) fn run_request(
        &mut self,
        request: &command::Request,
        serial: u32,
    ) -> Result<Followup, String> {
        use command::id;

        if request.namespace().is_some() {
            let situation = self.palette_situation();
            let effect = self
                .commands
                .run(request, &situation)
                .unwrap_or_else(|| Err(format!("{} has no provider", request.id)))?;
            self.apply_effect(effect);
            return Ok(Followup::Nothing);
        }

        let arg = request.arg.as_deref().unwrap_or_default().trim();
        match request.id.as_str() {
            id::GO_BACK => self.go_back(),
            id::GO_FORWARD => self.go_forward(),
            id::GO_UP => self.go_up(),
            id::GO_HOME => {
                let home = model::home_dir()
                    .ok_or_else(|| otto_kit::t_owned!("files-no-such-folder", path = "~"))?;
                self.navigate_to(&home);
            }
            id::GO_TO_PATH => {
                let path = self
                    .resolve_typed_path(arg)
                    .filter(|path| path.exists())
                    .ok_or_else(|| otto_kit::t_owned!("files-no-such-folder", path = arg))?;
                if path.is_dir() {
                    self.navigate_to(&path);
                } else if let Some(parent) = path.parent() {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                    self.navigate_to(parent);
                    self.pending_pick = Some((0, name));
                }
            }
            id::GO_TO_PLACE => {
                let path = PathBuf::from(arg);
                // The Recent place is a sentinel, not a folder.
                if crate::recent::is_sentinel(&path) {
                    self.enter_recent();
                    return Ok(Followup::Nothing);
                }
                if !path.is_dir() {
                    return Err(otto_kit::t_owned!("files-no-such-folder", path = arg));
                }
                self.navigate_to(&path);
            }
            id::OPEN => self.open_cursor_entry(),
            id::GET_INFO => self.open_info(),
            id::RENAME => self.rename_cursor_to(arg)?,
            id::NEW_FOLDER => self.new_folder_named(arg)?,
            id::TRASH => self.move_selected_to_trash(),
            id::PUT_BACK => self.put_back_selection(),
            id::DELETE_FOREVER => self.ask_delete_forever(),
            id::EMPTY_TRASH => self.ask_empty_trash(),
            id::CUT => self.copy_selection(true, serial),
            id::COPY => self.copy_selection(false, serial),
            id::PASTE => self.paste(),
            id::SELECT_ALL => self.select_all(),
            id::SELECT_MATCHING => self.select_matching(arg)?,
            id::MOVE_TO => self.move_selection_to(arg)?,
            id::NEW_FOLDER_WITH_SELECTION => self.new_folder_with_selection(arg)?,
            id::UNDO => self.undo_last(),
            // The three views by name share their ids with the values Change
            // View takes, so one arm answers for both.
            id::VIEW_LIST | id::VIEW_GRID | id::VIEW_COLUMNS => {
                let mode = view_from_id(&request.id)
                    .ok_or_else(|| otto_kit::t_owned!("files-palette-no-matches"))?;
                self.set_mode(mode);
            }
            id::CHANGE_VIEW => {
                let mode = view_from_id(arg)
                    .ok_or_else(|| otto_kit::t_owned!("files-palette-no-matches"))?;
                self.set_mode(mode);
            }
            id::SORT_BY => {
                let key = sort_from_id(arg)
                    .ok_or_else(|| otto_kit::t_owned!("files-palette-no-matches"))?;
                self.set_sort(key);
            }
            id::TOGGLE_HIDDEN => {
                self.show_hidden = !self.show_hidden;
                self.dirty = true;
            }
            // The preview is the host window's, not the browser's: it owns the
            // decode. Handed back rather than run here.
            id::QUICK_LOOK => return Ok(Followup::Peek),
            // The strip is opened either way; a query given with the command
            // is typed into it, so one keystroke can start a search outright.
            id::SEARCH => {
                self.toggle_search();
                if !arg.is_empty() {
                    if let Some(input) = self.search.as_mut() {
                        input.set_value(arg);
                    }
                    // Typing into the strip does not search — Return does —
                    // and a query given with the command is a Return already
                    // pressed.
                    self.run_search();
                }
            }
            id::RECENT => self.enter_recent(),
            id::RECOGNISE_TEXT => self.recognise_selection()?,
            other => return Err(format!("Unknown command: {other}")),
        }
        Ok(Followup::Nothing)
    }
}
