//! Pointer handling: the window, the palette, Quick View and the info window.

use super::*;

impl FilesApp {
    /// The palette's card handles its own pointer, for the same reason Quick
    /// View's does: dragged clear of the window, it sits over pixels the
    /// toplevel is never told about.
    ///
    /// Everything is put back into window points before it is acted on, so the
    /// card is hit-tested against the same rects it was painted from and a
    /// click means the same thing whichever surface it arrived on.
    ///
    /// A drag continues to arrive here even once the pointer has left the
    /// card, because a held button holds the pointer to the surface that took
    /// the press. That is what lets the card keep up with a fast drag instead
    /// of being dropped the moment the pointer outruns it.
    pub(super) fn install_palette_pointer(&self) {
        let state = Arc::clone(&self.state);
        let target = Arc::clone(&self.palette_target);

        AppContext::register_pointer_callback(move |events| {
            for event in events {
                use smithay_client_toolkit::seat::pointer::PointerEventKind;
                use wayland_client::Proxy;
                let Some((surface, rect)) = target.lock().unwrap().clone() else {
                    continue;
                };
                if event.surface.id() != surface {
                    continue;
                }
                // Surface-local, as the compositor reports it, shifted back
                // into the window points everything else measures in.
                let x = event.position.0 as f32 + rect.left;
                let y = event.position.1 as f32 + rect.top;
                let mut browser = state.lock().unwrap();
                if browser.palette.is_none() {
                    continue;
                }
                match event.kind {
                    PointerEventKind::Press { serial, .. } => {
                        browser.palette_press(x, y, serial);
                    }
                    PointerEventKind::Motion { .. } if browser.palette_drag.is_some() => {
                        browser.drag_palette_to(x, y);
                    }
                    PointerEventKind::Motion { .. } => {
                        browser.palette_hover(x, y);
                    }
                    // The card stays where it was let go of.
                    PointerEventKind::Release { .. } => {
                        if browser.palette_drag.take().is_some() {
                            browser.palette_dropped();
                        }
                    }
                    PointerEventKind::Axis { vertical, .. } => {
                        browser.palette_wheel(
                            x,
                            y,
                            vertical.absolute as f32,
                            vertical.stop,
                            vertical.discrete != 0,
                        );
                    }
                    _ => {}
                }
                drop(browser);
                AppContext::request_wakeup();
            }
        });
    }

    /// Quick View's panel handles its own pointer, because it is the one
    /// surface of this window that is routinely *outside* it.
    ///
    /// Centred on the display, the card hangs past the toplevel's edges, and
    /// the compositor delivers events over that part to this surface — never
    /// to the toplevel. Hit-testing the button in window coordinates
    /// therefore misses it exactly when the panel is placed correctly.
    ///
    /// The close button and the panel's own scrolling are handled here.
    /// Both are things the pointer does *over* the card, and over the card is
    /// exactly where the toplevel never hears about it. Dismissing on a click
    /// outside the panel stays with the toplevel, where the rest of the
    /// browser's hit-testing already lives — a click outside the card is a
    /// click on the window.
    pub(super) fn install_quickview_pointer(&self) {
        let state = Arc::clone(&self.state);
        let target = Arc::clone(&self.quickview_target);

        AppContext::register_pointer_callback(move |events| {
            for event in events {
                use wayland_client::Proxy;
                let Some((surface, panel)) = target.lock().unwrap().clone() else {
                    continue;
                };
                if event.surface.id() != surface {
                    continue;
                }
                // Surface-local already: the compositor reports positions
                // against the surface the pointer is over. Everything derived
                // from `panel` below is in that same space.
                let point = skia_safe::Point::new(event.position.0 as f32, event.position.1 as f32);
                let over = view::quickview_close_rect(panel)
                    .with_outset((4.0, 4.0))
                    .contains(point);
                let over_expand = view::quickview_expand_rect(panel)
                    .with_outset((4.0, 4.0))
                    .contains(point);

                let mut browser = state.lock().unwrap();
                match event.kind {
                    PointerEventKind::Press { .. } if over => {
                        browser.close_quickview();
                    }
                    PointerEventKind::Press { .. } if over_expand => {
                        browser.toggle_quickview_expand();
                    }
                    PointerEventKind::Press { .. } => {
                        // A scrollbar over a zoomed picture takes the press
                        // before anything else does.
                        browser.quickview_pan_pointer(QuickviewPointer::Press, point, panel);
                    }
                    PointerEventKind::Release { .. } => {
                        browser.quickview_pan_pointer(QuickviewPointer::Release, point, panel);
                    }
                    PointerEventKind::Motion { .. } | PointerEventKind::Enter { .. } => {
                        browser.quickview_focus(point, panel);
                        browser.quickview_pan_pointer(QuickviewPointer::Motion, point, panel);
                        if browser.quickview_close_hovered != over
                            || browser.quickview_expand_hovered != over_expand
                        {
                            browser.quickview_close_hovered = over;
                            browser.quickview_expand_hovered = over_expand;
                            browser.dirty = true;
                        }
                    }
                    PointerEventKind::Leave { .. } => {
                        browser.quickview_focus = None;
                        browser.quickview_pan_pointer(QuickviewPointer::Leave, point, panel);
                        if browser.quickview_close_hovered || browser.quickview_expand_hovered {
                            browser.quickview_close_hovered = false;
                            browser.quickview_expand_hovered = false;
                            browser.dirty = true;
                        }
                    }
                    PointerEventKind::Axis {
                        vertical,
                        horizontal,
                        ..
                    } => {
                        browser.quickview_wheel(
                            horizontal.absolute as f32,
                            vertical.absolute as f32,
                            panel,
                            vertical.stop || horizontal.stop,
                            vertical.discrete != 0 || horizontal.discrete != 0,
                        );
                    }
                }
                drop(browser);
                AppContext::request_wakeup();
            }
        });
    }

    /// The Get Info window's pointer. Registered once, for whichever panel is
    /// open — see [`FilesApp::info_window`].
    pub(super) fn install_info_window_pointer(&self) {
        let state = Arc::clone(&self.state);
        let info_window = Rc::clone(&self.info_window);

        AppContext::register_pointer_callback(move |events| {
            use wayland_client::Proxy;
            let window = info_window.borrow().clone();
            let Some(window) = window else { return };
            let Some(surface) = window.wl_surface() else {
                return;
            };
            // The window is the card, so surface-local coordinates are the
            // panel's own and nothing has to be converted.
            let sheet = Rect::from_wh(view::INFO_W, view::INFO_H);

            for event in events {
                if event.surface.id() != surface.id() {
                    continue;
                }
                let point = (event.position.0 as f32, event.position.1 as f32);
                let drag = {
                    let mut browser = state.lock().unwrap();
                    info_pointer(&mut browser, &event.kind, sheet, point)
                };
                if drag {
                    if let PointerEventKind::Press { serial, .. } = event.kind {
                        if let Some(seat) = AppContext::seat_state().seats().next() {
                            window.start_move(&seat, serial);
                        }
                    }
                }
                AppContext::request_wakeup();
            }
        });
    }

    /// Take drops: from another application, and from this window's own drags.
    ///
    /// The drag conversation is per-position — see [`otto_kit::dnd`] — so every
    /// enter and every motion answers, even to say no. Answering nothing is how
    /// a target refuses, and a refused drag never delivers.
    pub(super) fn install_dnd(&self, window: &Window) {
        use otto_kit::dnd::{self, DragEvent};
        use wayland_client::Proxy;

        let state = Arc::clone(&self.state);
        let window = window.clone();
        // Enter names the surface; motion and drop do not. A drag over some
        // other surface of ours — Quick View, the info sheet — is not a drop
        // target, so what the enter decided has to be remembered.
        let on_toplevel = std::cell::Cell::new(false);

        dnd::register(move |event| {
            let is_ours = |id: &wayland_client::backend::ObjectId| {
                window.wl_surface().is_some_and(|s| s.id() == *id)
            };

            match event {
                DragEvent::Enter { surface, x, y } => {
                    on_toplevel.set(is_ours(surface));
                    if !on_toplevel.get() {
                        return;
                    }
                    hover_drag(&state, *x as f32, *y as f32);
                }
                DragEvent::Motion { x, y } => {
                    if !on_toplevel.get() {
                        return;
                    }
                    hover_drag(&state, *x as f32, *y as f32);
                }
                DragEvent::Leave => {
                    if !on_toplevel.replace(false) {
                        return;
                    }
                    let mut browser = state.lock().unwrap();
                    browser.dirty |= browser.drop_target.take().is_some();
                    drop(browser);
                    AppContext::request_wakeup();
                }
                DragEvent::Drop { x, y } => {
                    if !on_toplevel.replace(false) {
                        return;
                    }
                    // The negotiated action, not the one we asked for: the
                    // compositor picks, and a source that only offered a copy
                    // must not have its files moved.
                    let move_them = dnd::selected_action().contains(dnd::DndAction::Move);

                    // Our own drag is served from our own payload; see
                    // `dnd::own_files` for why it cannot go over the pipe.
                    let paths = if dnd::dragging() {
                        let paths = dnd::own_files();
                        dnd::finish();
                        paths
                    } else {
                        dnd::receive_files().map(|(paths, _)| paths)
                    };

                    let mut browser = state.lock().unwrap();
                    // Resolved again at the drop's own position rather than
                    // trusting the last motion: the release may land somewhere
                    // no motion reported.
                    browser.drop_target = browser.drop_target_at(*x as f32, *y as f32);
                    match paths {
                        Some(paths) => browser.apply_drop(paths, move_them),
                        None => {
                            browser.drop_target = None;
                            browser.dirty = true;
                        }
                    }
                    drop(browser);
                    AppContext::request_wakeup();
                }
            }
        });
    }

    pub(super) fn install_pointer(&self, window: &Window, context_menu: ContextMenu) {
        let state = Arc::clone(&self.state);
        let window_for_events = window.clone();
        let modifiers = Arc::clone(&self.modifiers);

        window.on_pointer_event(move |events| {
            for event in events {
                let (x, y) = (event.position.0 as f32, event.position.1 as f32);
                let mods = *modifiers.lock().unwrap();
                let (ctrl, shift) = (mods.ctrl, mods.shift);
                let mut browser = state.lock().unwrap();
                // The file area's bottom, not the window's: every hit test
                // below is against the listing, which stops short of the
                // picker's action row. The row's own hit test uses the full
                // window height and runs first.
                let (width, height) = (browser.size.0, browser.content_h());

                // An in-place rename owns the pointer while it is up: a click
                // inside places the caret, a click anywhere else commits it
                // the way clicking away from a Finder rename does.
                if let Some(session) = browser.rename.as_ref() {
                    if let PointerEventKind::Press { .. } = event.kind {
                        let (depth, index) = (session.depth, session.index);
                        let count = browser.visible(depth).len();
                        let scroll = browser.columns[depth].scroll.offset();
                        let rect = match browser.mode {
                            ViewMode::List => view::list_rename_rect(
                                width,
                                browser.list_columns,
                                count,
                                scroll,
                                index,
                            ),
                            ViewMode::Columns => {
                                let is_dir =
                                    browser.visible(depth).get(index).is_some_and(|e| e.is_dir);
                                view::miller_rename_rect(
                                    height,
                                    browser.pan.offset(),
                                    browser.miller_w,
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
                            if let Some(session) = browser.rename.as_mut() {
                                session.input.on_pointer_down(x - rect.left, 1, shift);
                            }
                            browser.dirty = true;
                        } else {
                            browser.commit_rename();
                        }
                    }
                    drop(browser);
                    continue;
                }

                // An open preview owns the pointer, the way the sheet does: a
                // click outside dismisses it, the wheel scrolls its content, and
                // nothing reaches the listing underneath.
                if browser.quickview.is_some() {
                    // Where the panel *is*, which is not where the window's
                    // centre is once the compositor has centred it on the
                    // display. The fallback is the window-centred rect, and
                    // the window height is right for it: the panel floats
                    // over the action row rather than beside it.
                    let panel = browser
                        .quickview_panel
                        .unwrap_or_else(|| browser.quickview_fallback_panel());
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
                                browser.close_quickview();
                            } else if over_expand {
                                browser.toggle_quickview_expand();
                            } else {
                                browser.quickview_pan_pointer(
                                    QuickviewPointer::Press,
                                    point,
                                    panel,
                                );
                            }
                        }
                        PointerEventKind::Release { .. } => {
                            browser.quickview_pan_pointer(QuickviewPointer::Release, point, panel);
                        }
                        PointerEventKind::Motion { .. } | PointerEventKind::Enter { .. } => {
                            browser.quickview_focus(point, panel);
                            browser.quickview_pan_pointer(QuickviewPointer::Motion, point, panel);
                            if browser.quickview_close_hovered != over_close
                                || browser.quickview_expand_hovered != over_expand
                            {
                                browser.quickview_close_hovered = over_close;
                                browser.quickview_expand_hovered = over_expand;
                                browser.dirty = true;
                            }
                        }
                        PointerEventKind::Leave { .. } => {
                            browser.quickview_focus = None;
                            browser.quickview_pan_pointer(QuickviewPointer::Leave, point, panel);
                            if browser.quickview_close_hovered
                                || browser.quickview_expand_hovered
                            {
                                browser.quickview_close_hovered = false;
                                browser.quickview_expand_hovered = false;
                                browser.dirty = true;
                            }
                        }
                        PointerEventKind::Axis {
                            vertical,
                            horizontal,
                            ..
                        } => {
                            browser.quickview_wheel(
                                horizontal.absolute as f32,
                                vertical.absolute as f32,
                                panel,
                                vertical.stop || horizontal.stop,
                                vertical.discrete != 0 || horizontal.discrete != 0,
                            );
                        }
                    }
                    drop(browser);
                    continue;
                }

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
                    if browser.preview_video_pointer(kind, x, y) {
                        drop(browser);
                        AppContext::request_wakeup();
                        continue;
                    }
                }

                // The confirmation sheet is modal: it answers its own two
                // buttons and swallows everything else, so a click meant for
                // it can never land on the listing behind it.
                if browser.confirm.is_some() {
                    let window_h = browser.size.1;
                    let hit = view::confirm_at(x, y, width, window_h);
                    match event.kind {
                        PointerEventKind::Press { button, .. } if button != BTN_RIGHT => {
                            if let Some(sheet) = browser.confirm.as_mut() {
                                sheet.pressed = hit;
                            }
                            browser.dirty = true;
                        }
                        PointerEventKind::Release { button, .. } if button != BTN_RIGHT => {
                            let armed = browser
                                .confirm
                                .as_mut()
                                .and_then(|sheet| sheet.pressed.take());
                            if armed.is_some() && armed == hit {
                                match armed {
                                    Some(view::ConfirmButton::Accept) => {
                                        browser.confirm_accept()
                                    }
                                    Some(view::ConfirmButton::Cancel) => browser.confirm_dismiss(),
                                    None => {}
                                }
                            }
                            browser.dirty = true;
                        }
                        PointerEventKind::Motion { .. } => {
                            AppContext::set_cursor_shape(CursorShape::Default);
                        }
                        _ => {}
                    }
                    drop(browser);
                    continue;
                }

                // The filter strip, while it is open: a click in the field
                // places the caret, a click on a pill changes the scope.
                // Unlike the path entry, clicking away does *not* dismiss it —
                // the results are still on screen, and leaving the field means
                // going to look at what you found.
                if matches!(event.kind, PointerEventKind::Press { button, .. } if button != BTN_RIGHT)
                    && browser.search.is_some()
                {
                    let point = skia_safe::Point::new(x, y);
                    let field = view::search_field_rect(width);
                    if field.contains(point) {
                        if let Some(input) = browser.search.as_mut() {
                            // Clicking into the field is how the keyboard
                            // comes back after a click on the listing sent it
                            // away, so focus is taken before the caret is
                            // placed — an unfocused field ignores keys.
                            input.state.set_focused(true);
                            input.on_pointer_down(
                                x - field.left - view::SEARCH_TEXT_INSET,
                                1,
                                shift,
                            );
                        }
                        browser.dirty = true;
                        drop(browser);
                        continue;
                    }
                    let pills = view::search_scope_rects(width).into_iter().zip([
                        model::SearchScope::Folder,
                        model::SearchScope::Everywhere,
                    ]);
                    if let Some((_, scope)) = pills.into_iter().find(|(r, _)| r.contains(point)) {
                        browser.set_search_scope(scope);
                        drop(browser);
                        continue;
                    }
                    if view::search_band_rect(width).contains(point) {
                        drop(browser);
                        continue;
                    }
                    // Below the strip: the click belongs to the listing, and
                    // so does the keyboard from here on. Without this the
                    // query keeps taking keys after you have clicked a file,
                    // and Space types a space instead of opening Quick View
                    // on what you just selected. The strip stays open — the
                    // results are still what you are looking at — it simply
                    // stops being where the typing goes. Falls through rather
                    // than continuing: the click still has a file to land on.
                    browser.blur_search();
                }

                // A click in the path entry places the caret; a click
                // anywhere else puts the title back, the way clicking away
                // from a location bar dismisses it.
                if browser.path_entry.is_some()
                    && matches!(event.kind, PointerEventKind::Press { button, .. } if button != BTN_RIGHT)
                {
                    let field = view::path_field_rect(width);
                    if field.contains(skia_safe::Point::new(x, y)) {
                        if let Some(input) = browser.path_entry.as_mut() {
                            input.on_pointer_down(x - field.left, 1, shift);
                        }
                        browser.dirty = true;
                        drop(browser);
                        continue;
                    }
                    browser.cancel_path_entry();
                }

                // A click in the name field places the caret in it. The field
                // never loses focus to the listing, so there is no focus to
                // take here — only a caret to move.
                if browser.save_name.is_some()
                    && matches!(event.kind, PointerEventKind::Press { button, .. } if button != BTN_RIGHT)
                {
                    let field = view::footer_name_rect(width, browser.size.1);
                    if field.contains(skia_safe::Point::new(x, y)) {
                        if let Some(input) = browser.save_name.as_mut() {
                            input.on_pointer_down(x - field.left, 1, shift);
                        }
                        browser.dirty = true;
                        drop(browser);
                        continue;
                    }
                }

                // The picker's action row, and the filter menu it opens,
                // take the pointer before the listing does. Their geometry is
                // in *window* coordinates — `height` above is the file area's
                // bottom, which is exactly where this strip begins.
                if browser.picker.is_some() {
                    let window_h = browser.size.1;
                    let (filter_count, menu_open) = browser
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
                            Some(view::FooterButton::FilterOption(_))
                                | Some(view::FooterButton::Filter)
                        );

                    match event.kind {
                        PointerEventKind::Motion { .. } => {
                            if browser.footer_hover != hit {
                                browser.footer_hover = hit;
                                browser.dirty = true;
                            }
                            if hit.is_some() {
                                AppContext::set_cursor_shape(CursorShape::Default);
                                drop(browser);
                                continue;
                            }
                        }
                        PointerEventKind::Leave { .. } => {
                            if browser.footer_hover.take().is_some() {
                                browser.dirty = true;
                            }
                        }
                        PointerEventKind::Press { button, .. } if button != BTN_RIGHT => {
                            if dismissing_menu {
                                if let Some(session) = browser.picker.as_mut() {
                                    session.filter_open = false;
                                }
                                browser.dirty = true;
                                drop(browser);
                                continue;
                            }
                            if let Some(button) = hit {
                                browser.footer_press(button);
                                drop(browser);
                                continue;
                            }
                        }
                        PointerEventKind::Release { button, .. }
                            if button != BTN_RIGHT && browser.footer_pressed.is_some() =>
                        {
                            browser.footer_release(hit);
                            drop(browser);
                            continue;
                        }
                        _ => {}
                    }
                }

                // The path bar takes the pointer before the listing does, for
                // the same reason the action row does: its geometry is in
                // *window* coordinates, and the strip begins exactly where the
                // file area's bottom is.
                if browser.path_bar_h() > 0.0 {
                    let bar = view::path_bar_rect(width, browser.size.1, browser.footer_h());
                    let inside = bar.contains(skia_safe::Point::new(x, y));

                    match event.kind {
                        PointerEventKind::Motion { .. } => {
                            let hit = inside
                                .then(|| browser.path_crumb_hovered(x, y))
                                .flatten();
                            if browser.path_crumb_hover != hit {
                                browser.path_crumb_hover = hit;
                                browser.dirty = true;
                            }
                            if inside {
                                AppContext::set_cursor_shape(CursorShape::Default);
                                drop(browser);
                                continue;
                            }
                        }
                        PointerEventKind::Leave { .. } => {
                            if browser.path_crumb_hover.take().is_some() {
                                browser.dirty = true;
                            }
                        }
                        // A crumb is a step, taken on press — it navigates,
                        // and there is nothing to arm. The resize edge along
                        // the window's bottom still wins: it runs under this
                        // strip, and a grab there is a grab of the window.
                        PointerEventKind::Press { button, .. }
                            if button != BTN_RIGHT
                                && inside
                                && resize::edge_at(
                                    Rect::from_wh(width, browser.size.1),
                                    x,
                                    y,
                                )
                                .is_none() =>
                        {
                            if let Some(path) = browser.path_crumb_target(x, y) {
                                // Going to a result's folder is a navigation,
                                // not an abandoned query: the strip closes
                                // rather than putting the folder the search
                                // began in back, and the search itself stays
                                // behind Back.
                                browser.leave_synthetic_to(&path);
                            }
                            drop(browser);
                            continue;
                        }
                        _ => {}
                    }
                }

                match event.kind {
                    PointerEventKind::Motion { .. } => {
                        // The palette being dragged owns the pointer outright,
                        // the same way a divider does.
                        if browser.palette_drag.is_some() {
                            browser.drag_palette_to(x, y);
                            AppContext::set_cursor_shape(CursorShape::Grabbing);
                            drop(browser);
                            continue;
                        }
                        // A column divider being dragged owns the pointer
                        // outright — nothing else on this move should react.
                        if let Some((boundary, start_x, start_w)) = browser.column_resize {
                            let dx = x - start_x;
                            let new_w = (start_w - dx).clamp(view::COLUMN_MIN_W, 400.0);
                            match boundary {
                                view::ColumnBoundary::Size => browser.list_columns.size = new_w,
                                view::ColumnBoundary::Kind => browser.list_columns.kind = new_w,
                                view::ColumnBoundary::Modified => {
                                    browser.list_columns.modified = new_w
                                }
                            }
                            browser.dirty = true;
                            AppContext::set_cursor_shape(CursorShape::ColResize);
                            drop(browser);
                            continue;
                        }
                        if let Some((depth, start_x, start_w)) = browser.miller_resize {
                            let dx = (x - start_x) / (depth + 1) as f32;
                            browser.miller_w =
                                (start_w + dx).clamp(view::MILLER_MIN_W, view::MILLER_MAX_W);
                            browser.dirty = true;
                            AppContext::set_cursor_shape(CursorShape::ColResize);
                            drop(browser);
                            continue;
                        }

                        // A rubber band owns the gesture while it is out: no
                        // scrollbar, hover or resize affordance should answer
                        // a pointer that is busy drawing a selection.
                        if browser.marquee.is_some() {
                            browser.update_marquee(x, y);
                            drop(browser);
                            continue;
                        }

                        // Far enough from the press to be a drag rather than an
                        // unsteady click. The whole selection goes, and the
                        // compositor owns the pointer from here — nothing else
                        // in this handler will see the rest of the gesture.
                        if let Some((start_x, start_y, serial)) = browser.drag_armed {
                            if (x - start_x).hypot(y - start_y) >= DRAG_THRESHOLD {
                                browser.drag_armed = None;
                                browser.drag_started();
                                let paths = browser.drag_paths();
                                // The picture the cursor carries: the first
                                // file of the selection, and how many are
                                // coming with it.
                                // Every travelling file, where it is on screen
                                // now: the picture starts as the listing and
                                // gathers from there.
                                let items = browser.drag_items(start_x, start_y);
                                // The picture is shaped like the view it came
                                // from: a cell in the grid, a row card in the
                                // list and column views.
                                let mode = browser.mode;
                                let count = paths.len();
                                drop(browser);
                                if let (Some(surface), Some((picture, size, anchor))) =
                                    (window_for_events.wl_surface(), items)
                                {
                                    let theme = AppContext::current_theme();
                                    otto_kit::dnd::start_file_drag_with_icon(
                                        &paths,
                                        DndAction::Copy | DndAction::Move,
                                        &surface,
                                        serial,
                                        (size.0 as i32, size.1 as i32),
                                        anchor,
                                        move |canvas, w, h| match picture {
                                            DragPicture::Entries(items) => view::draw_drag_image(
                                                canvas, &theme, mode, &items, anchor, count,
                                            ),
                                            DragPicture::Preview(draw) => draw(canvas, w, h),
                                        },
                                    );
                                }
                                continue;
                            }
                        }

                        AppContext::set_cursor_shape(browser.hover_shape(x, y));

                        // A scrollbar drag follows the pointer wherever it
                        // goes, so the dragged pane is asked first and the
                        // hovered one only styles its bar.
                        browser.sync_scroll_metrics();
                        let hovered = browser.pane_under(x, y);
                        let mut moved = browser.pan.on_pointer_drag(x, y);
                        if browser.mode == ViewMode::Columns {
                            moved |= browser.pan.on_pointer_move(x, y);
                        } else {
                            browser.pan.on_pointer_leave();
                        }
                        for (depth, column) in browser.columns.iter_mut().enumerate() {
                            moved |= column.scroll.on_pointer_drag(x, y);
                            if depth == hovered {
                                // Hovering the scrollbar keeps it up and
                                // widens it, the way the settings app does.
                                moved |= column.scroll.on_pointer_move(x, y);
                            } else {
                                column.scroll.on_pointer_leave();
                            }
                        }
                        browser.dirty |= moved;

                        // The traffic lights reveal their glyphs while the
                        // pointer is over the group.
                        let control = view::control_at(x, y, browser.size.0);
                        browser.dirty |= browser.controls.on_motion(control);
                    }
                    PointerEventKind::Release { .. } => {
                        // The palette stays where it was let go of.
                        if browser.palette_drag.take().is_some() {
                            browser.palette_dropped();
                            drop(browser);
                            continue;
                        }
                        // A press that came up without travelling was a click —
                        // including one that left its narrowing until now.
                        browser.drag_armed = None;
                        // The band goes away with the button that drew it; what
                        // it caught stays selected.
                        browser.dirty |= browser.marquee.take().is_some();
                        browser.release_entry();
                        browser.column_resize = None;
                        browser.miller_resize = None;
                        browser.pan.on_pointer_up();
                        for column in &mut browser.columns {
                            column.scroll.on_pointer_up();
                        }

                        // A nav arrow steps on release, and only over the half
                        // the press landed on: a press dragged off it is a
                        // cancelled click, and only clears the fill.
                        if let Some(armed) = browser.nav_pressed.take() {
                            browser.dirty = true;
                            if browser.nav_button_at(x, y) == Some(armed) {
                                match armed {
                                    view::NavButton::Back => browser.go_back(),
                                    view::NavButton::Forward => browser.go_forward(),
                                }
                            }
                        }

                        if let Some(armed) = browser.trash_pressed.take() {
                            browser.dirty = true;
                            if browser.trash_action_at(x, y) == Some(armed) {
                                match armed {
                                    view::TrashAction::PutBack => browser.put_back_selection(),
                                    view::TrashAction::Empty => browser.ask_empty_trash(),
                                }
                            }
                        }

                        // A control fires on release, and only over the dot
                        // the press landed on.
                        let control = view::control_at(x, y, browser.size.0);
                        browser.dirty |= browser.controls.pressed().is_some();
                        match browser.controls.on_release(control) {
                            // In the picker, closing the window *is*
                            // cancelling: exiting here would leave the
                            // requesting application waiting on a reply that
                            // no longer has a sender.
                            Some(WindowControl::Close) => {
                                if browser.picker.is_some() {
                                    browser.picker_cancel();
                                } else {
                                    std::process::exit(0)
                                }
                            }
                            Some(WindowControl::Minimize) => window_for_events.minimize(),
                            Some(WindowControl::Zoom) => window_for_events.toggle_maximized(),
                            None => {}
                        }
                    }
                    PointerEventKind::Leave { .. } => {
                        browser.column_resize = None;
                        browser.miller_resize = None;
                        browser.pan.on_pointer_leave();
                        for column in &mut browser.columns {
                            column.scroll.on_pointer_leave();
                        }
                        // Nothing in the header is hovered once the pointer is
                        // off the window, or the glyphs stay drawn on it.
                        browser.controls.on_leave();
                        // Same for a held arrow: the release will never come.
                        browser.nav_pressed = None;
                        browser.dirty = true;
                    }
                    PointerEventKind::Press { serial, button, .. } if button == BTN_RIGHT => {
                        let items = browser.context_menu_items(x, y);
                        drop(browser);

                        let Some(parent_xdg) = window_for_events
                            .surface()
                            .map(|s| s.xdg_window().xdg_surface().clone())
                        else {
                            continue;
                        };
                        let Ok(positioner) = XdgPositioner::new(AppContext::xdg_shell_state())
                        else {
                            continue;
                        };

                        context_menu.state().borrow_mut().set_items(items);
                        let theme = AppContext::current_theme();
                        context_menu
                            .clone()
                            .with_style(ContextMenuStyle::default().with_theme(theme));

                        let (menu_w, menu_h) = context_menu.get_size_at_depth(0);
                        positioner.set_size(menu_w as i32, menu_h as i32);
                        positioner.set_anchor_rect(x as i32, y as i32, 1, 1);
                        positioner.set_anchor(xdg_positioner::Anchor::TopLeft);
                        positioner.set_gravity(xdg_positioner::Gravity::BottomRight);
                        positioner.set_constraint_adjustment(
                            xdg_positioner::ConstraintAdjustment::SlideX
                                | xdg_positioner::ConstraintAdjustment::SlideY
                                | xdg_positioner::ConstraintAdjustment::FlipX
                                | xdg_positioner::ConstraintAdjustment::FlipY,
                        );

                        let click_state = Arc::clone(&state);
                        context_menu.clone().on_item_click(move |action_id| {
                            let mut browser = click_state.lock().unwrap();
                            match action_id {
                                "open" => browser.open_selection(),
                                "get_info" => browser.open_info(),
                                "rename" => browser.start_rename(),
                                "cut" => browser.copy_selection(true, serial),
                                "copy" => browser.copy_selection(false, serial),
                                "paste" => browser.paste(),
                                "trash" => browser.move_selected_to_trash(),
                                "put_back" => browser.put_back_selection(),
                                "delete_forever" => browser.ask_delete_forever(),
                                "empty_trash" => browser.ask_empty_trash(),
                                "new_folder" => browser.new_folder(),
                                "new_folder_with_selection" => {
                                    // No name from a menu: the folder takes the
                                    // default one and lands in rename, ready to
                                    // be typed over.
                                    if let Err(error) = browser.new_folder_with_selection("") {
                                        browser.status = Some(error);
                                        browser.dirty = true;
                                    }
                                }
                                // A provider's command, by its namespaced id.
                                id if id.contains(':') => browser.run_menu_command(id, serial),
                                _ => {}
                            }
                        });

                        context_menu.show(&parent_xdg, &positioner, serial);
                        continue;
                    }
                    PointerEventKind::Press { serial, .. } => {
                        // The palette is over everything, so it is asked
                        // first. A click on a row picks it; a click anywhere
                        // outside the card dismisses the palette and stops
                        // there, rather than also selecting whatever file
                        // happened to be underneath.
                        if browser.palette.is_some() {
                            browser.palette_press(x, y, serial);
                            return;
                        }

                        // The whole window, not the file area: the border being
                        // grabbed is the window's, and in the picker the file
                        // area stops short of the bottom by the action row.
                        // Measuring against `height` there would put the
                        // "bottom edge" across the middle of that row.
                        if let Some(edge) =
                            resize::edge_at(Rect::from_wh(width, browser.size.1), x, y)
                        {
                            if let Some(seat) = AppContext::seat_state().seats().next() {
                                window_for_events.start_resize(&seat, serial, edge);
                            }
                            return;
                        }

                        // Arming rather than acting: the control fires on
                        // release, over the same dot.
                        let control = view::control_at(x, y, browser.size.0);
                        if browser.controls.on_press(control) {
                            browser.dirty = true;
                            return;
                        }

                        // Dragging the header moves the window, in every view. The
                        // sidebar's first place reaches into the header band, so a
                        // click there belongs to the place, not to the drag.
                        if view::is_drag_area(x, y, width)
                            && view::place_at(x, y, browser.places.len()).is_none()
                        {
                            if let Some(seat) = AppContext::seat_state().seats().next() {
                                // A double click on the header zooms the
                                // window instead of moving it.
                                window_for_events.titlebar_press(&seat, serial, x, y);
                            }
                            return;
                        }

                        // A press on a scrollbar thumb grabs it and selects
                        // nothing: the bar sits over the rows it scrolls, so
                        // it has to win the click.
                        browser.sync_scroll_metrics();
                        // The stack's bar lies along the bottom of every
                        // pane, crossing the foot of each pane's own gutter,
                        // so it is asked first where the two overlap.
                        let depth = browser.pane_under(x, y);
                        let panning = browser.mode == ViewMode::Columns;
                        if (panning && browser.pan.on_pointer_down(x, y))
                            || browser.columns[depth].scroll.on_pointer_down(x, y)
                        {
                            browser.dirty = true;
                            drop(browser);
                            continue;
                        }

                        // A press on a row — or on the preview column's picture,
                        // which is one file drawn large — might be the start of
                        // a drag. Armed here and decided on motion: the
                        // selection below still happens, so a press that never
                        // travels is an ordinary click and a second one still
                        // opens the directory.
                        if browser.dnd_enabled()
                            && (browser.entry_at(x, y).is_some()
                                || browser.preview_grab_at(x, y).is_some())
                        {
                            browser.drag_armed = Some((x, y, serial));
                        }

                        if let Some(action) = browser.trash_action_at(x, y) {
                            // Armed and decided on release, the way a nav
                            // arrow is: both of these destroy or move files,
                            // and a press dragged off the button is a
                            // cancelled click rather than a confirmation.
                            browser.trash_pressed = Some(action);
                            browser.dirty = true;
                        } else if let Some(button) = browser.nav_button_at(x, y) {
                            // Armed, not acted on: the step happens on
                            // release, over the same half, so the arrow can
                            // sit visibly pressed in the meantime.
                            browser.nav_pressed = Some(button);
                            browser.dirty = true;
                        } else if !browser.trash && view::switcher_at(x, y, width).is_some() {
                            if let Some(mode) = view::switcher_at(x, y, width) {
                                browser.set_mode(mode);
                            }
                        } else if let Some(index) = view::place_at(x, y, browser.places.len()) {
                            // Picking a place is leaving whatever synthetic
                            // listing was up: results are not a place, and a
                            // window still calling itself a search while
                            // showing a folder refuses half its own menu.
                            browser.active_place = Some(index);
                            browser.dirty = true;
                            if browser.places[index].recent {
                                browser.enter_recent();
                            } else {
                                let path = browser.places[index].path.clone();
                                browser.leave_synthetic_to(&path);
                            }
                        } else if browser.mode == ViewMode::Grid {
                            let depth = browser.columns.len() - 1;
                            let count = browser.visible(depth).len();
                            let scroll = browser.columns[depth].scroll.offset();
                            let area = view::content_viewport(width, height, ViewMode::Grid);
                            let sections = browser.recent_sections.clone();
                            if let Some(index) =
                                view::grid_cell_at_in(area, &sections, x, y, count, scroll)
                            {
                                if ctrl {
                                    browser.note_ctrl_row_click(depth, index);
                                } else if shift {
                                    browser.extend_select(depth, index);
                                } else {
                                    browser.press_entry(depth, index);
                                }
                            } else if hit_content(area, x, y) {
                                // Nothing under the press: it is the corner of
                                // a rubber band. A band that never travels is
                                // an empty one, which is how a plain click on
                                // nothing comes to mean nothing selected.
                                if !ctrl && !shift {
                                    browser.clear_pane_selection(depth);
                                }
                                browser.begin_marquee(depth, x, y, ctrl || shift);
                            }
                        } else if browser.mode == ViewMode::List
                            && view::column_boundary_at(x, y, width, browser.list_columns).is_some()
                        {
                            let boundary =
                                view::column_boundary_at(x, y, width, browser.list_columns)
                                    .unwrap();
                            let now = std::time::Instant::now();
                            let double_click =
                                browser.last_boundary_click.is_some_and(|(last, at)| {
                                    last == boundary && now.duration_since(at) < DOUBLE_CLICK_WINDOW
                                });
                            if double_click && boundary == view::ColumnBoundary::Size {
                                let depth = browser.columns.len() - 1;
                                let longest = view::widest_name(
                                    browser.visible(depth).iter().map(|e| e.name.as_str()),
                                );
                                browser.list_columns.size =
                                    view::fit_size_column(width, browser.list_columns, longest);
                                browser.last_boundary_click = None;
                                browser.dirty = true;
                            } else {
                                let start = match boundary {
                                    view::ColumnBoundary::Size => browser.list_columns.size,
                                    view::ColumnBoundary::Kind => browser.list_columns.kind,
                                    view::ColumnBoundary::Modified => browser.list_columns.modified,
                                };
                                browser.column_resize = Some((boundary, x, start));
                                browser.last_boundary_click = Some((boundary, now));
                            }
                        } else if browser.mode == ViewMode::List {
                            if let Some(key) = view::column_at(x, y, width, browser.list_columns) {
                                if browser.sort == key {
                                    browser.ascending = !browser.ascending;
                                } else {
                                    browser.sort = key;
                                    // A fresh key reads best in its natural
                                    // direction: names from A, dates from now.
                                    browser.ascending = key != SortKey::Modified;
                                }
                                browser.sort_pinned = true;
                                browser.dirty = true;
                            } else {
                                let depth = browser.columns.len() - 1;
                                let count = browser.visible(depth).len();
                                let scroll = browser.columns[depth].scroll.offset();
                                if let Some(index) =
                                    view::row_at(x, y, width, height, count, scroll)
                                {
                                    if ctrl {
                                        browser.note_ctrl_row_click(depth, index);
                                    } else if shift {
                                        browser.extend_select(depth, index);
                                    } else {
                                        browser.press_entry(depth, index);
                                    }
                                } else if !ctrl && !shift {
                                    let area =
                                        view::content_viewport(width, height, ViewMode::List);
                                    if hit_content(area, x, y) {
                                        browser.clear_pane_selection(depth);
                                    }
                                }
                            }
                        } else if browser.mode == ViewMode::Columns
                            && view::miller_boundary_at(
                                x,
                                y,
                                width,
                                height,
                                browser.pan.offset(),
                                browser.columns.len(),
                                browser.miller_w,
                            )
                            .is_some()
                        {
                            let depth = view::miller_boundary_at(
                                x,
                                y,
                                width,
                                height,
                                browser.pan.offset(),
                                browser.columns.len(),
                                browser.miller_w,
                            )
                            .unwrap();
                            let now = std::time::Instant::now();
                            let double_click =
                                browser.last_miller_click.is_some_and(|(last, at)| {
                                    last == depth && now.duration_since(at) < DOUBLE_CLICK_WINDOW
                                });
                            if double_click {
                                let entries = browser.visible(depth);
                                let longest =
                                    view::widest_name(entries.iter().map(|e| e.name.as_str()));
                                let has_dirs = entries.iter().any(|e| e.is_dir);
                                browser.miller_w = view::fit_miller_width(longest, has_dirs);
                                browser.last_miller_click = None;
                                browser.dirty = true;
                            } else {
                                browser.miller_resize = Some((depth, x, browser.miller_w));
                                browser.last_miller_click = Some((depth, now));
                            }
                        } else {
                            let counts = browser.counts();
                            let hit = view::miller_at(
                                x,
                                y,
                                width,
                                height,
                                &browser.columns,
                                &counts,
                                browser.pan.offset(),
                                browser.miller_w,
                            );
                            if let Some((depth, Some(index))) = hit {
                                if ctrl {
                                    browser.note_ctrl_row_click(depth, index);
                                } else if shift {
                                    browser.extend_select(depth, index);
                                } else {
                                    // A directory here already opened on the
                                    // single click — Miller shows its child
                                    // eagerly. A *file* did not, and a double
                                    // click is how one is opened in the other
                                    // two views, so it is how one is opened
                                    // here too.
                                    browser.press_entry(depth, index);
                                }
                            } else if let Some((depth, None)) = hit {
                                // Inside a pane, below its last row. The pane
                                // takes the keyboard either way; without a
                                // modifier the click also means "nothing".
                                if ctrl || shift {
                                    browser.active = depth;
                                    browser.dirty = true;
                                } else {
                                    browser.clear_pane_selection(depth);
                                }
                            }
                        }
                    }
                    PointerEventKind::Axis {
                        vertical,
                        horizontal,
                        ..
                    } => {
                        let dy = vertical.absolute as f32;
                        let dx = horizontal.absolute as f32;
                        // Both axes clamp, band and fling themselves, so the
                        // metrics have to be current before either is fed.
                        browser.sync_scroll_metrics();

                        let stop = vertical.stop || horizontal.stop;
                        let discrete = vertical.discrete != 0 || horizontal.discrete != 0;
                        // One gesture belongs to one axis, chosen by its first
                        // delta and kept until it lifts.
                        let leading = if browser.mode == ViewMode::Columns && dx.abs() > dy.abs() {
                            Axis::Horizontal
                        } else {
                            Axis::Vertical
                        };
                        let axis = *browser.gesture_axis.get_or_insert(leading);
                        let (delta, scroll) = match axis {
                            // The stack pans as a whole …
                            Axis::Horizontal => (dx, &mut browser.pan),
                            // … while a vertical scroll belongs to the pane
                            // under the pointer.
                            Axis::Vertical => {
                                let depth = browser.pane_under(x, y);
                                (dy, &mut browser.columns[depth].scroll)
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
                            browser.gesture_axis = None;
                        }
                        browser.scroll_moved |= moved;
                    }
                    _ => {}
                }

                drop(browser);
            }
            // Nothing is presented from here. What the batch changed is on
            // the browser — `dirty`, a scroll moved — and the update loop,
            // which runs straight after, decides what that repaints: the
            // whole window, the file area alone, or only surfaces.
        });
    }
}
