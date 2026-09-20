//! Pointer handling: the window, the palette, Peek and the info window.

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

    /// Peek's panel handles its own pointer, because it is the one
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
    pub(super) fn install_peek_pointer(&self) {
        let state = Arc::clone(&self.state);
        let target = Arc::clone(&self.peek_target);

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
                let over = view::peek_close_rect(panel)
                    .with_outset((4.0, 4.0))
                    .contains(point);
                let over_expand = view::peek_expand_rect(panel)
                    .with_outset((4.0, 4.0))
                    .contains(point);

                let mut browser = state.lock().unwrap();
                match event.kind {
                    PointerEventKind::Press { .. } if over => {
                        browser.close_peek();
                    }
                    PointerEventKind::Press { .. } if over_expand => {
                        browser.toggle_peek_expand();
                    }
                    PointerEventKind::Press { .. } => {
                        // The title strip takes hold of the card; failing
                        // that, a scrollbar over a zoomed picture takes the
                        // press before anything else does.
                        if !browser.peek_grip(point, panel) {
                            browser.peek_pan_pointer(PeekPointer::Press, point, panel);
                        }
                    }
                    PointerEventKind::Release { .. } => {
                        browser.end_peek_drag();
                        browser.peek_pan_pointer(PeekPointer::Release, point, panel);
                    }
                    PointerEventKind::Motion { .. } if browser.peek_dragging() => {
                        browser.drag_peek_to(point);
                    }
                    PointerEventKind::Motion { .. } | PointerEventKind::Enter { .. } => {
                        browser.peek_focus(point, panel);
                        browser.peek_pan_pointer(PeekPointer::Motion, point, panel);
                        browser.sync_peek_cursor(point, panel);
                        if browser.peek_close_hovered != over
                            || browser.peek_expand_hovered != over_expand
                        {
                            browser.peek_close_hovered = over;
                            browser.peek_expand_hovered = over_expand;
                            browser.dirty = true;
                        }
                    }
                    PointerEventKind::Leave { .. } => {
                        browser.end_peek_drag();
                        browser.peek_focus = None;
                        browser.peek_pan_pointer(PeekPointer::Leave, point, panel);
                        browser.reset_peek_cursor();
                        if browser.peek_close_hovered || browser.peek_expand_hovered {
                            browser.peek_close_hovered = false;
                            browser.peek_expand_hovered = false;
                            browser.dirty = true;
                        }
                    }
                    PointerEventKind::Axis {
                        vertical,
                        horizontal,
                        ..
                    } => {
                        browser.peek_wheel(
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
        // other surface of ours — Peek, the info sheet — is not a drop
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
                let mods = *modifiers.lock().unwrap();
                let after = state
                    .lock()
                    .unwrap()
                    .on_pointer(event, mods, &window_for_events);
                match after {
                    After::Next => {}
                    After::Stop => return,
                    After::Drag(drag) => start_drag(&window_for_events, drag),
                    After::Menu(menu) => {
                        show_context_menu(&window_for_events, &context_menu, &state, menu)
                    }
                }
            }
            // Nothing is presented from here. What the batch changed is on
            // the browser — `dirty`, a scroll moved — and the update loop,
            // which runs straight after, decides what that repaints: the
            // whole window, the file area alone, or only surfaces.
        });
    }
}

/// Hand the travelling selection to the compositor, which owns the pointer
/// from here: nothing in the window sees the rest of the gesture.
fn start_drag(window: &Window, drag: DragStart) {
    let DragStart {
        paths,
        items,
        mode,
        serial,
    } = drag;
    let count = paths.len();
    if let (Some(surface), Some((picture, size, anchor))) = (window.wl_surface(), items) {
        let theme = AppContext::current_theme();
        otto_kit::dnd::start_file_drag_with_icon(
            &paths,
            DndAction::Copy | DndAction::Move,
            &surface,
            serial,
            (size.0 as i32, size.1 as i32),
            anchor,
            move |canvas, w, h| match picture {
                DragPicture::Entries(items) => {
                    view::draw_drag_image(canvas, &theme, mode, &items, anchor, count)
                }
                DragPicture::Preview(draw) => draw(canvas, w, h),
            },
        );
    }
}

/// Open the context menu at the press, acting on the browser when an item is
/// picked.
fn show_context_menu(
    window: &Window,
    context_menu: &ContextMenu,
    state: &Arc<Mutex<Browser>>,
    menu: MenuAt,
) {
    let MenuAt {
        items,
        x,
        y,
        serial,
    } = menu;
    let Some(parent_xdg) = window
        .surface()
        .map(|s| s.xdg_window().xdg_surface().clone())
    else {
        return;
    };
    let Ok(positioner) = XdgPositioner::new(AppContext::xdg_shell_state()) else {
        return;
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

    let click_state = Arc::clone(state);
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
}
