//! What a pointer event does to the browser, layer by layer: whatever is
//! up over the listing first — a rename, Quick View, a sheet, a field, the
//! footer — then the listing itself, by the kind of event.

use smithay_client_toolkit::seat::pointer::{AxisScroll, PointerEvent};

use super::*;

/// Where a pointer event landed, and the modifiers held, in window points.
#[derive(Clone, Copy)]
pub(super) struct PointerAt {
    x: f32,
    y: f32,
    ctrl: bool,
    shift: bool,
    /// The window's width.
    width: f32,
    /// The file area's bottom, not the window's: every hit test on the
    /// listing stops short of the picker's action row.
    height: f32,
}

/// Something laid over the listing that may take a pointer event for itself.
type Layer = fn(&mut Browser, &PointerEvent, PointerAt) -> Option<After>;

/// What is left to do once the browser has taken an event — work that runs
/// with the browser let go of, since it can call back into it.
pub(super) enum After {
    /// On to the next event of the batch.
    Next,
    /// The rest of the batch is not for the window: a move, a resize, the
    /// palette took the press.
    Stop,
    /// The selection has travelled far enough to be dragged out.
    Drag(DragStart),
    /// A right click: the context menu, with its items.
    Menu(MenuAt),
}

pub(super) struct DragStart {
    pub(super) paths: Vec<PathBuf>,
    pub(super) items: Option<DragItems>,
    pub(super) mode: ViewMode,
    pub(super) serial: u32,
}

pub(super) struct MenuAt {
    pub(super) items: Vec<MenuItem>,
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) serial: u32,
}

impl Browser {
    pub(super) fn on_pointer(
        &mut self,
        event: &PointerEvent,
        mods: Modifiers,
        window: &Window,
    ) -> After {
        let at = PointerAt {
            x: event.position.0 as f32,
            y: event.position.1 as f32,
            ctrl: mods.ctrl,
            shift: mods.shift,
            width: self.size.0,
            height: self.content_h(),
        };
        // Whatever is up over the listing takes the event first, in this
        // order; the first that answers for it keeps it.
        let layers: [Layer; 9] = [
            Self::rename_pointer,
            Self::quickview_pointer,
            Self::preview_video_press,
            Self::confirm_pointer,
            Self::search_strip_pointer,
            Self::path_entry_pointer,
            Self::save_field_pointer,
            Self::footer_pointer,
            Self::path_bar_pointer,
        ];
        for layer in layers {
            if let Some(after) = layer(self, event, at) {
                return after;
            }
        }
        match event.kind {
            PointerEventKind::Motion { .. } => self.pointer_motion(at),
            PointerEventKind::Release { .. } => self.pointer_release(at, window),
            PointerEventKind::Leave { .. } => {
                self.pointer_leave();
                After::Next
            }
            PointerEventKind::Press { serial, button, .. } if button == BTN_RIGHT => {
                After::Menu(MenuAt {
                    items: self.context_menu_items(at.x, at.y),
                    x: at.x,
                    y: at.y,
                    serial,
                })
            }
            PointerEventKind::Press { serial, .. } => self.pointer_press(at, serial, window),
            PointerEventKind::Axis {
                vertical,
                horizontal,
                ..
            } => {
                self.pointer_axis(at, vertical, horizontal);
                After::Next
            }
            _ => After::Next,
        }
    }

    /// An in-place rename owns the pointer while it is up.
    fn rename_pointer(&mut self, event: &PointerEvent, at: PointerAt) -> Option<After> {
        let PointerAt {
            x,
            y,
            shift,
            width,
            height,
            ..
        } = at;
        // An in-place rename owns the pointer while it is up: a click
        // inside places the caret, a click anywhere else commits it
        // the way clicking away from a Finder rename does.
        if let Some(session) = self.rename.as_ref() {
            if let PointerEventKind::Press { .. } = event.kind {
                let (depth, index) = (session.depth, session.index);
                let count = self.visible(depth).len();
                let scroll = self.columns[depth].scroll.offset();
                let rect = match self.mode {
                    ViewMode::List => {
                        view::list_rename_rect(width, self.list_columns, count, scroll, index)
                    }
                    ViewMode::Columns => {
                        let is_dir = self.visible(depth).get(index).is_some_and(|e| e.is_dir);
                        view::miller_rename_rect(
                            height,
                            self.pan.offset(),
                            self.miller_w,
                            depth,
                            count,
                            scroll,
                            index,
                            is_dir,
                        )
                    }
                    ViewMode::Grid => view::grid_rename_rect(width, height, scroll, index),
                };
                if rect.contains(skia_safe::Point::new(x, y)) {
                    if let Some(session) = self.rename.as_mut() {
                        session.input.on_pointer_down(x - rect.left, 1, shift);
                    }
                    self.dirty = true;
                } else {
                    self.commit_rename();
                }
            }
            return Some(After::Next);
        }
        None
    }

    /// An open Quick View owns the pointer.
    fn quickview_pointer(&mut self, event: &PointerEvent, at: PointerAt) -> Option<After> {
        let PointerAt { x, y, .. } = at;
        // An open preview owns the pointer, the way the sheet does: a
        // click outside dismisses it, the wheel scrolls its content, and
        // nothing reaches the listing underneath.
        if self.quickview.is_some() {
            // Where the panel *is*, which is not where the window's
            // centre is once the compositor has centred it on the
            // display. The fallback is the window-centred rect, and
            // the window height is right for it: the panel floats
            // over the action row rather than beside it.
            let panel = self
                .quickview_panel
                .unwrap_or_else(|| self.quickview_fallback_panel());
            let point = skia_safe::Point::new(x, y);
            let over_close = view::quickview_close_rect(panel)
                .with_outset((4.0, 4.0))
                .contains(point);
            let over_expand = view::quickview_expand_rect(panel)
                .with_outset((4.0, 4.0))
                .contains(point);
            match event.kind {
                PointerEventKind::Press { .. } => {
                    // The buttons first: they sit inside the panel,
                    // so the "click outside dismisses" rule below
                    // would never reach them. Then the pan's
                    // scrollbars, which are inside it too.
                    if over_close || !panel.contains(point) {
                        self.close_quickview();
                    } else if over_expand {
                        self.toggle_quickview_expand();
                    } else {
                        self.quickview_pan_pointer(QuickviewPointer::Press, point, panel);
                    }
                }
                PointerEventKind::Release { .. } => {
                    self.quickview_pan_pointer(QuickviewPointer::Release, point, panel);
                }
                PointerEventKind::Motion { .. } | PointerEventKind::Enter { .. } => {
                    self.quickview_focus(point, panel);
                    self.quickview_pan_pointer(QuickviewPointer::Motion, point, panel);
                    self.sync_quickview_cursor(point, panel);
                    if self.quickview_close_hovered != over_close
                        || self.quickview_expand_hovered != over_expand
                    {
                        self.quickview_close_hovered = over_close;
                        self.quickview_expand_hovered = over_expand;
                        self.dirty = true;
                    }
                }
                PointerEventKind::Leave { .. } => {
                    self.quickview_focus = None;
                    self.quickview_pan_pointer(QuickviewPointer::Leave, point, panel);
                    self.reset_quickview_cursor();
                    if self.quickview_close_hovered || self.quickview_expand_hovered {
                        self.quickview_close_hovered = false;
                        self.quickview_expand_hovered = false;
                        self.dirty = true;
                    }
                }
                PointerEventKind::Axis {
                    vertical,
                    horizontal,
                    ..
                } => {
                    self.quickview_wheel(
                        horizontal.absolute as f32,
                        vertical.absolute as f32,
                        panel,
                        vertical.stop || horizontal.stop,
                        vertical.discrete != 0 || horizontal.discrete != 0,
                    );
                }
            }
            return Some(After::Next);
        }
        None
    }

    /// The preview column's video takes its own presses and scrubs.
    fn preview_video_press(&mut self, event: &PointerEvent, at: PointerAt) -> Option<After> {
        let PointerAt { x, y, .. } = at;
        // The preview column's video takes its own presses and a
        // scrub in progress, before anything decides what a click on
        // the column means.
        let video_pointer = match event.kind {
            PointerEventKind::Press { .. } => Some(quickview::VideoPointer::Press),
            PointerEventKind::Release { .. } => Some(quickview::VideoPointer::Release),
            PointerEventKind::Motion { .. } | PointerEventKind::Enter { .. } => {
                Some(quickview::VideoPointer::Motion)
            }
            PointerEventKind::Leave { .. } => Some(quickview::VideoPointer::Leave),
            PointerEventKind::Axis { .. } => None,
        };
        if let Some(kind) = video_pointer {
            if self.preview_video_pointer(kind, x, y) {
                AppContext::request_wakeup();
                return Some(After::Next);
            }
        }
        None
    }

    /// The confirmation sheet is modal and swallows everything.
    fn confirm_pointer(&mut self, event: &PointerEvent, at: PointerAt) -> Option<After> {
        let PointerAt { x, y, width, .. } = at;
        // The confirmation sheet is modal: it answers its own two
        // buttons and swallows everything else, so a click meant for
        // it can never land on the listing behind it.
        if self.confirm.is_some() {
            let window_h = self.size.1;
            let hit = view::confirm_at(x, y, width, window_h);
            match event.kind {
                PointerEventKind::Press { button, .. } if button != BTN_RIGHT => {
                    if let Some(sheet) = self.confirm.as_mut() {
                        sheet.pressed = hit;
                    }
                    self.dirty = true;
                }
                PointerEventKind::Release { button, .. } if button != BTN_RIGHT => {
                    let armed = self.confirm.as_mut().and_then(|sheet| sheet.pressed.take());
                    if armed.is_some() && armed == hit {
                        match armed {
                            Some(view::ConfirmButton::Accept) => self.confirm_accept(),
                            Some(view::ConfirmButton::Cancel) => self.confirm_dismiss(),
                            None => {}
                        }
                    }
                    self.dirty = true;
                }
                PointerEventKind::Motion { .. } => {
                    AppContext::set_cursor_shape(CursorShape::Default);
                }
                _ => {}
            }
            return Some(After::Next);
        }
        None
    }

    /// The filter strip, while it is open.
    fn search_strip_pointer(&mut self, event: &PointerEvent, at: PointerAt) -> Option<After> {
        let PointerAt {
            x, y, shift, width, ..
        } = at;
        // The filter strip, while it is open: a click in the field
        // places the caret, a click on a pill changes the scope.
        // Unlike the path entry, clicking away does *not* dismiss it —
        // the results are still on screen, and leaving the field means
        // going to look at what you found.
        if matches!(event.kind, PointerEventKind::Press { button, .. } if button != BTN_RIGHT)
            && self.search.is_some()
        {
            let point = skia_safe::Point::new(x, y);
            let field = view::search_field_rect(width);
            if field.contains(point) {
                if let Some(input) = self.search.as_mut() {
                    // Clicking into the field is how the keyboard
                    // comes back after a click on the listing sent it
                    // away, so focus is taken before the caret is
                    // placed — an unfocused field ignores keys.
                    input.state.set_focused(true);
                    input.on_pointer_down(x - field.left - view::SEARCH_TEXT_INSET, 1, shift);
                }
                self.dirty = true;
                return Some(After::Next);
            }
            let pills = view::search_scope_rects(width)
                .into_iter()
                .zip([model::SearchScope::Folder, model::SearchScope::Everywhere]);
            if let Some((_, scope)) = pills.into_iter().find(|(r, _)| r.contains(point)) {
                self.set_search_scope(scope);
                return Some(After::Next);
            }
            if view::search_band_rect(width).contains(point) {
                return Some(After::Next);
            }
            // Below the strip: the click belongs to the listing, and
            // so does the keyboard from here on. Without this the
            // query keeps taking keys after you have clicked a file,
            // and Space types a space instead of opening Quick View
            // on what you just selected. The strip stays open — the
            // results are still what you are looking at — it simply
            // stops being where the typing goes. Falls through rather
            // than continuing: the click still has a file to land on.
            self.blur_search();
        }
        None
    }

    /// The path entry places its caret, or goes away.
    fn path_entry_pointer(&mut self, event: &PointerEvent, at: PointerAt) -> Option<After> {
        let PointerAt {
            x, y, shift, width, ..
        } = at;
        // A click in the path entry places the caret; a click
        // anywhere else puts the title back, the way clicking away
        // from a location bar dismisses it.
        if self.path_entry.is_some()
            && matches!(event.kind, PointerEventKind::Press { button, .. } if button != BTN_RIGHT)
        {
            let field = view::path_field_rect(width);
            if field.contains(skia_safe::Point::new(x, y)) {
                if let Some(input) = self.path_entry.as_mut() {
                    input.on_pointer_down(x - field.left, 1, shift);
                }
                self.dirty = true;
                return Some(After::Next);
            }
            self.cancel_path_entry();
        }
        None
    }

    /// The save field places its caret.
    fn save_field_pointer(&mut self, event: &PointerEvent, at: PointerAt) -> Option<After> {
        let PointerAt {
            x, y, shift, width, ..
        } = at;
        // A click in the name field places the caret in it. The field
        // never loses focus to the listing, so there is no focus to
        // take here — only a caret to move.
        if self.save_name.is_some()
            && matches!(event.kind, PointerEventKind::Press { button, .. } if button != BTN_RIGHT)
        {
            let field = view::footer_name_rect(width, self.size.1);
            if field.contains(skia_safe::Point::new(x, y)) {
                if let Some(input) = self.save_name.as_mut() {
                    input.on_pointer_down(x - field.left, 1, shift);
                }
                self.dirty = true;
                return Some(After::Next);
            }
        }
        None
    }

    /// The picker's action row and its filter menu.
    fn footer_pointer(&mut self, event: &PointerEvent, at: PointerAt) -> Option<After> {
        let PointerAt { x, y, width, .. } = at;
        // The picker's action row, and the filter menu it opens,
        // take the pointer before the listing does. Their geometry is
        // in *window* coordinates — `height` above is the file area's
        // bottom, which is exactly where this strip begins.
        if self.picker.is_some() {
            let window_h = self.size.1;
            let (filter_count, menu_open) = self
                .picker
                .as_ref()
                .map(|p| (p.filters.len(), p.filter_open))
                .unwrap_or((0, false));
            let hit = view::footer_at(x, y, width, window_h, filter_count, menu_open);
            // A click anywhere outside the open menu closes it, the
            // way clicking away from any menu does — including a
            // click on the listing, which is then swallowed.
            let dismissing_menu = menu_open
                && !matches!(
                    hit,
                    Some(view::FooterButton::FilterOption(_)) | Some(view::FooterButton::Filter)
                );

            match event.kind {
                PointerEventKind::Motion { .. } => {
                    if self.footer_hover != hit {
                        self.footer_hover = hit;
                        self.dirty = true;
                    }
                    if hit.is_some() {
                        AppContext::set_cursor_shape(CursorShape::Default);
                        return Some(After::Next);
                    }
                }
                PointerEventKind::Leave { .. } => {
                    if self.footer_hover.take().is_some() {
                        self.dirty = true;
                    }
                }
                PointerEventKind::Press { button, .. } if button != BTN_RIGHT => {
                    if dismissing_menu {
                        if let Some(session) = self.picker.as_mut() {
                            session.filter_open = false;
                        }
                        self.dirty = true;
                        return Some(After::Next);
                    }
                    if let Some(button) = hit {
                        self.footer_press(button);
                        return Some(After::Next);
                    }
                }
                PointerEventKind::Release { button, .. }
                    if button != BTN_RIGHT && self.footer_pressed.is_some() =>
                {
                    self.footer_release(hit);
                    return Some(After::Next);
                }
                _ => {}
            }
        }
        None
    }

    /// The path bar and its crumbs.
    fn path_bar_pointer(&mut self, event: &PointerEvent, at: PointerAt) -> Option<After> {
        let PointerAt { x, y, width, .. } = at;
        // The path bar takes the pointer before the listing does, for
        // the same reason the action row does: its geometry is in
        // *window* coordinates, and the strip begins exactly where the
        // file area's bottom is.
        if self.path_bar_h() > 0.0 {
            let bar = view::path_bar_rect(width, self.size.1, self.footer_h());
            let inside = bar.contains(skia_safe::Point::new(x, y));

            match event.kind {
                PointerEventKind::Motion { .. } => {
                    let hit = inside.then(|| self.path_crumb_hovered(x, y)).flatten();
                    if self.path_crumb_hover != hit {
                        self.path_crumb_hover = hit;
                        self.dirty = true;
                    }
                    if inside {
                        AppContext::set_cursor_shape(CursorShape::Default);
                        return Some(After::Next);
                    }
                }
                PointerEventKind::Leave { .. } => {
                    if self.path_crumb_hover.take().is_some() {
                        self.dirty = true;
                    }
                }
                // A crumb is a step, taken on press — it navigates,
                // and there is nothing to arm. The resize edge along
                // the window's bottom still wins: it runs under this
                // strip, and a grab there is a grab of the window.
                PointerEventKind::Press { button, .. }
                    if button != BTN_RIGHT
                        && inside
                        && resize::edge_at(Rect::from_wh(width, self.size.1), x, y).is_none() =>
                {
                    if let Some(path) = self.path_crumb_target(x, y) {
                        // Going to a result's folder is a navigation,
                        // not an abandoned query: the strip closes
                        // rather than putting the folder the search
                        // began in back, and the search itself stays
                        // behind Back.
                        self.leave_synthetic_to(&path);
                    }
                    return Some(After::Next);
                }
                _ => {}
            }
        }
        None
    }

    /// The pointer moved over the listing.
    fn pointer_motion(&mut self, at: PointerAt) -> After {
        let PointerAt { x, y, .. } = at;
        // The palette being dragged owns the pointer outright,
        // the same way a divider does.
        if self.palette_drag.is_some() {
            self.drag_palette_to(x, y);
            AppContext::set_cursor_shape(CursorShape::Grabbing);
            return After::Next;
        }
        // A column divider being dragged owns the pointer
        // outright — nothing else on this move should react.
        if let Some((boundary, start_x, start_w)) = self.column_resize {
            let dx = x - start_x;
            let new_w = (start_w - dx).clamp(view::COLUMN_MIN_W, 400.0);
            match boundary {
                view::ColumnBoundary::Size => self.list_columns.size = new_w,
                view::ColumnBoundary::Kind => self.list_columns.kind = new_w,
                view::ColumnBoundary::Modified => self.list_columns.modified = new_w,
            }
            self.dirty = true;
            AppContext::set_cursor_shape(CursorShape::ColResize);
            return After::Next;
        }
        if let Some((depth, start_x, start_w)) = self.miller_resize {
            let dx = (x - start_x) / (depth + 1) as f32;
            self.miller_w = (start_w + dx).clamp(view::MILLER_MIN_W, view::MILLER_MAX_W);
            self.dirty = true;
            AppContext::set_cursor_shape(CursorShape::ColResize);
            return After::Next;
        }

        // A rubber band owns the gesture while it is out: no
        // scrollbar, hover or resize affordance should answer
        // a pointer that is busy drawing a selection.
        if self.marquee.is_some() {
            self.update_marquee(x, y);
            return After::Next;
        }

        // Far enough from the press to be a drag rather than an
        // unsteady click. The whole selection goes, and the
        // compositor owns the pointer from here — nothing else
        // in this handler will see the rest of the gesture.
        if let Some((start_x, start_y, serial)) = self.drag_armed {
            if (x - start_x).hypot(y - start_y) >= DRAG_THRESHOLD {
                self.drag_armed = None;
                self.drag_started();
                return After::Drag(DragStart {
                    paths: self.drag_paths(),
                    items: self.drag_items(start_x, start_y),
                    mode: self.mode,
                    serial,
                });
            }
        }

        AppContext::set_cursor_shape(self.hover_shape(x, y));

        // A scrollbar drag follows the pointer wherever it
        // goes, so the dragged pane is asked first and the
        // hovered one only styles its bar.
        self.sync_scroll_metrics();
        let hovered = self.pane_under(x, y);
        let mut moved = self.pan.on_pointer_drag(x, y);
        if self.mode == ViewMode::Columns {
            moved |= self.pan.on_pointer_move(x, y);
        } else {
            self.pan.on_pointer_leave();
        }
        for (depth, column) in self.columns.iter_mut().enumerate() {
            moved |= column.scroll.on_pointer_drag(x, y);
            if depth == hovered {
                // Hovering the scrollbar keeps it up and
                // widens it, the way the settings app does.
                moved |= column.scroll.on_pointer_move(x, y);
            } else {
                column.scroll.on_pointer_leave();
            }
        }
        self.dirty |= moved;

        // The traffic lights reveal their glyphs while the
        // pointer is over the group.
        let control = view::control_at(x, y, self.size.0);
        self.dirty |= self.controls.on_motion(control);
        After::Next
    }

    /// A button came up over the listing.
    fn pointer_release(&mut self, at: PointerAt, window: &Window) -> After {
        let PointerAt { x, y, .. } = at;
        // The palette stays where it was let go of.
        if self.palette_drag.take().is_some() {
            self.palette_dropped();
            return After::Next;
        }
        // A press that came up without travelling was a click —
        // including one that left its narrowing until now.
        self.drag_armed = None;
        // The band goes away with the button that drew it; what
        // it caught stays selected.
        self.dirty |= self.marquee.take().is_some();
        self.release_entry();
        self.column_resize = None;
        self.miller_resize = None;
        self.pan.on_pointer_up();
        for column in &mut self.columns {
            column.scroll.on_pointer_up();
        }

        // A nav arrow steps on release, and only over the half
        // the press landed on: a press dragged off it is a
        // cancelled click, and only clears the fill.
        if let Some(armed) = self.nav_pressed.take() {
            self.dirty = true;
            if self.nav_button_at(x, y) == Some(armed) {
                match armed {
                    view::NavButton::Back => self.go_back(),
                    view::NavButton::Forward => self.go_forward(),
                }
            }
        }

        if let Some(armed) = self.trash_pressed.take() {
            self.dirty = true;
            if self.trash_action_at(x, y) == Some(armed) {
                match armed {
                    view::TrashAction::PutBack => self.put_back_selection(),
                    view::TrashAction::Empty => self.ask_empty_trash(),
                }
            }
        }

        // A control fires on release, and only over the dot
        // the press landed on.
        let control = view::control_at(x, y, self.size.0);
        self.dirty |= self.controls.pressed().is_some();
        match self.controls.on_release(control) {
            // In the picker, closing the window *is*
            // cancelling: exiting here would leave the
            // requesting application waiting on a reply that
            // no longer has a sender.
            Some(WindowControl::Close) => {
                if self.picker.is_some() {
                    self.picker_cancel();
                } else {
                    std::process::exit(0)
                }
            }
            Some(WindowControl::Minimize) => window.minimize(),
            Some(WindowControl::Zoom) => window.toggle_maximized(),
            None => {}
        }
        After::Next
    }

    /// The pointer left the window.
    fn pointer_leave(&mut self) {
        self.column_resize = None;
        self.miller_resize = None;
        self.pan.on_pointer_leave();
        for column in &mut self.columns {
            column.scroll.on_pointer_leave();
        }
        // Nothing in the header is hovered once the pointer is
        // off the window, or the glyphs stay drawn on it.
        self.controls.on_leave();
        // Same for a held arrow: the release will never come.
        self.nav_pressed = None;
        self.dirty = true;
    }

    /// A left press over the listing.
    fn pointer_press(&mut self, at: PointerAt, serial: u32, window: &Window) -> After {
        let PointerAt {
            x,
            y,
            ctrl,
            shift,
            width,
            height,
            ..
        } = at;
        // The palette is over everything, so it is asked
        // first. A click on a row picks it; a click anywhere
        // outside the card dismisses the palette and stops
        // there, rather than also selecting whatever file
        // happened to be underneath.
        if self.palette.is_some() {
            self.palette_press(x, y, serial);
            return After::Stop;
        }

        // The whole window, not the file area: the border being
        // grabbed is the window's, and in the picker the file
        // area stops short of the bottom by the action row.
        // Measuring against `height` there would put the
        // "bottom edge" across the middle of that row.
        if let Some(edge) = resize::edge_at(Rect::from_wh(width, self.size.1), x, y) {
            if let Some(seat) = AppContext::seat_state().seats().next() {
                window.start_resize(&seat, serial, edge);
            }
            return After::Stop;
        }

        // Arming rather than acting: the control fires on
        // release, over the same dot.
        let control = view::control_at(x, y, self.size.0);
        if self.controls.on_press(control) {
            self.dirty = true;
            return After::Stop;
        }

        // Dragging the header moves the window, in every view. The
        // sidebar's first place reaches into the header band, so a
        // click there belongs to the place, not to the drag.
        if view::is_drag_area(x, y, width) && view::place_at(x, y, self.places.len()).is_none() {
            if let Some(seat) = AppContext::seat_state().seats().next() {
                // A double click on the header zooms the
                // window instead of moving it.
                window.titlebar_press(&seat, serial, x, y);
            }
            return After::Stop;
        }

        // A press on a scrollbar thumb grabs it and selects
        // nothing: the bar sits over the rows it scrolls, so
        // it has to win the click.
        self.sync_scroll_metrics();
        // The stack's bar lies along the bottom of every
        // pane, crossing the foot of each pane's own gutter,
        // so it is asked first where the two overlap.
        let depth = self.pane_under(x, y);
        let panning = self.mode == ViewMode::Columns;
        if (panning && self.pan.on_pointer_down(x, y))
            || self.columns[depth].scroll.on_pointer_down(x, y)
        {
            self.dirty = true;
            return After::Next;
        }

        // A press on a row — or on the preview column's picture,
        // which is one file drawn large — might be the start of
        // a drag. Armed here and decided on motion: the
        // selection below still happens, so a press that never
        // travels is an ordinary click and a second one still
        // opens the directory.
        if self.dnd_enabled()
            && (self.entry_at(x, y).is_some() || self.preview_grab_at(x, y).is_some())
        {
            self.drag_armed = Some((x, y, serial));
        }

        if let Some(action) = self.trash_action_at(x, y) {
            // Armed and decided on release, the way a nav
            // arrow is: both of these destroy or move files,
            // and a press dragged off the button is a
            // cancelled click rather than a confirmation.
            self.trash_pressed = Some(action);
            self.dirty = true;
        } else if let Some(button) = self.nav_button_at(x, y) {
            // Armed, not acted on: the step happens on
            // release, over the same half, so the arrow can
            // sit visibly pressed in the meantime.
            self.nav_pressed = Some(button);
            self.dirty = true;
        } else if !self.trash && view::switcher_at(x, y, width).is_some() {
            if let Some(mode) = view::switcher_at(x, y, width) {
                self.set_mode(mode);
            }
        } else if let Some(index) = view::place_at(x, y, self.places.len()) {
            // Picking a place is leaving whatever synthetic
            // listing was up: results are not a place, and a
            // window still calling itself a search while
            // showing a folder refuses half its own menu.
            self.active_place = Some(index);
            self.dirty = true;
            if self.places[index].recent {
                self.enter_recent();
            } else {
                let path = self.places[index].path.clone();
                self.leave_synthetic_to(&path);
            }
        } else if self.mode == ViewMode::Grid {
            let depth = self.columns.len() - 1;
            let count = self.visible(depth).len();
            let scroll = self.columns[depth].scroll.offset();
            let area = view::content_viewport(width, height, ViewMode::Grid);
            let sections = self.recent_sections.clone();
            if let Some(index) = view::grid_cell_at_in(area, &sections, x, y, count, scroll) {
                if ctrl {
                    self.note_ctrl_row_click(depth, index);
                } else if shift {
                    self.extend_select(depth, index);
                } else {
                    self.press_entry(depth, index);
                }
            } else if hit_content(area, x, y) {
                // Nothing under the press: it is the corner of
                // a rubber band. A band that never travels is
                // an empty one, which is how a plain click on
                // nothing comes to mean nothing selected.
                if !ctrl && !shift {
                    self.clear_pane_selection(depth);
                }
                self.begin_marquee(depth, x, y, ctrl || shift);
            }
        } else if self.mode == ViewMode::List
            && view::column_boundary_at(x, y, width, self.list_columns).is_some()
        {
            let boundary = view::column_boundary_at(x, y, width, self.list_columns).unwrap();
            let now = std::time::Instant::now();
            let double_click = self.last_boundary_click.is_some_and(|(last, at)| {
                last == boundary && now.duration_since(at) < DOUBLE_CLICK_WINDOW
            });
            if double_click && boundary == view::ColumnBoundary::Size {
                let depth = self.columns.len() - 1;
                let longest =
                    view::widest_name(self.visible(depth).iter().map(|e| e.name.as_str()));
                self.list_columns.size = view::fit_size_column(width, self.list_columns, longest);
                self.last_boundary_click = None;
                self.dirty = true;
            } else {
                let start = match boundary {
                    view::ColumnBoundary::Size => self.list_columns.size,
                    view::ColumnBoundary::Kind => self.list_columns.kind,
                    view::ColumnBoundary::Modified => self.list_columns.modified,
                };
                self.column_resize = Some((boundary, x, start));
                self.last_boundary_click = Some((boundary, now));
            }
        } else if self.mode == ViewMode::List {
            if let Some(key) = view::column_at(x, y, width, self.list_columns) {
                if self.sort == key {
                    self.ascending = !self.ascending;
                } else {
                    self.sort = key;
                    // A fresh key reads best in its natural
                    // direction: names from A, dates from now.
                    self.ascending = key != SortKey::Modified;
                }
                self.sort_pinned = true;
                self.dirty = true;
            } else {
                let depth = self.columns.len() - 1;
                let count = self.visible(depth).len();
                let scroll = self.columns[depth].scroll.offset();
                if let Some(index) = view::row_at(x, y, width, height, count, scroll) {
                    if ctrl {
                        self.note_ctrl_row_click(depth, index);
                    } else if shift {
                        self.extend_select(depth, index);
                    } else {
                        self.press_entry(depth, index);
                    }
                } else if !ctrl && !shift {
                    let area = view::content_viewport(width, height, ViewMode::List);
                    if hit_content(area, x, y) {
                        self.clear_pane_selection(depth);
                    }
                }
            }
        } else if self.mode == ViewMode::Columns
            && view::miller_boundary_at(
                x,
                y,
                width,
                height,
                self.pan.offset(),
                self.columns.len(),
                self.miller_w,
            )
            .is_some()
        {
            let depth = view::miller_boundary_at(
                x,
                y,
                width,
                height,
                self.pan.offset(),
                self.columns.len(),
                self.miller_w,
            )
            .unwrap();
            let now = std::time::Instant::now();
            let double_click = self.last_miller_click.is_some_and(|(last, at)| {
                last == depth && now.duration_since(at) < DOUBLE_CLICK_WINDOW
            });
            if double_click {
                let entries = self.visible(depth);
                let longest = view::widest_name(entries.iter().map(|e| e.name.as_str()));
                let has_dirs = entries.iter().any(|e| e.is_dir);
                self.miller_w = view::fit_miller_width(longest, has_dirs);
                self.last_miller_click = None;
                self.dirty = true;
            } else {
                self.miller_resize = Some((depth, x, self.miller_w));
                self.last_miller_click = Some((depth, now));
            }
        } else {
            let counts = self.counts();
            let hit = view::miller_at(
                x,
                y,
                width,
                height,
                &self.columns,
                &counts,
                self.pan.offset(),
                self.miller_w,
            );
            if let Some((depth, Some(index))) = hit {
                if ctrl {
                    self.note_ctrl_row_click(depth, index);
                } else if shift {
                    self.extend_select(depth, index);
                } else {
                    // A directory here already opened on the
                    // single click — Miller shows its child
                    // eagerly. A *file* did not, and a double
                    // click is how one is opened in the other
                    // two views, so it is how one is opened
                    // here too.
                    self.press_entry(depth, index);
                }
            } else if let Some((depth, None)) = hit {
                // Inside a pane, below its last row. The pane
                // takes the keyboard either way; without a
                // modifier the click also means "nothing".
                if ctrl || shift {
                    self.active = depth;
                    self.dirty = true;
                } else {
                    self.clear_pane_selection(depth);
                }
            }
        }
        After::Next
    }

    /// A wheel or touchpad scroll over the listing.
    fn pointer_axis(&mut self, at: PointerAt, vertical: AxisScroll, horizontal: AxisScroll) {
        let PointerAt { x, y, .. } = at;
        let dy = vertical.absolute as f32;
        let dx = horizontal.absolute as f32;
        // Both axes clamp, band and fling themselves, so the
        // metrics have to be current before either is fed.
        self.sync_scroll_metrics();

        let stop = vertical.stop || horizontal.stop;
        let discrete = vertical.discrete != 0 || horizontal.discrete != 0;
        // One gesture belongs to one axis, chosen by its first
        // delta and kept until it lifts.
        let leading = if self.mode == ViewMode::Columns && dx.abs() > dy.abs() {
            Axis::Horizontal
        } else {
            Axis::Vertical
        };
        let axis = *self.gesture_axis.get_or_insert(leading);
        let (delta, scroll) = match axis {
            // The stack pans as a whole …
            Axis::Horizontal => (dx, &mut self.pan),
            // … while a vertical scroll belongs to the pane
            // under the pointer.
            Axis::Vertical => {
                let depth = self.pane_under(x, y);
                (dy, &mut self.columns[depth].scroll)
            }
        };

        // A notched wheel reports discrete steps and moves
        // exactly one step per click; a touchpad reports a
        // continuous stream, which is what momentum and
        // rubber banding are for.
        let moved = if stop {
            // Fingers off the touchpad: what the gesture
            // was carrying becomes a fling, and anything
            // pulled past an end springs back.
            scroll.on_wheel_end();
            true
        } else if discrete {
            scroll.on_wheel_discrete(delta)
        } else {
            scroll.on_wheel(delta)
        };
        if stop || discrete {
            // Nothing is in flight to keep an axis for: the
            // next delta picks afresh.
            self.gesture_axis = None;
        }
        self.scroll_moved |= moved;
    }
}
