//! The app's lifecycle: startup, the update loop, configure, focus and accessibility.

use super::*;

impl App for FilesApp {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        self.context_menu = Some(ContextMenu::new(Vec::new()));

        // Before the window: a surface builds its own root layer node at
        // construction, and that node is what the scene hangs off.
        AppContext::enable_layer_engine(view::WINDOW_W, view::WINDOW_H);

        let mut window = Window::new(
            otto_kit::t!("files-window-title"),
            view::WINDOW_W as i32,
            view::WINDOW_H as i32,
        )?;
        window.set_min_size(view::MIN_W as u32, view::MIN_H as u32);
        // The window as a whole has no ground of its own: the sidebar, the
        // header and the content area each carry their own material, and two
        // of the three are translucent so the compositor's blur reads through
        // them. Anything painted here would sit between that blur and them.
        window.set_background(skia_safe::Color::TRANSPARENT);

        // Name the window after its desktop entry, so the dock and the app
        // switcher find `otto-files.desktop` — and its file-manager icon —
        // directly. Without an app_id the compositor has to guess from the
        // client's pid and executable name, which lands on the same entry only
        // because the `Exec=` line happens to match. The Trash is its own
        // entry with its own icon, out of the same executable, so guessing
        // would land it on the file manager's.
        if let Some(surface) = window.surface() {
            surface.xdg_window().set_app_id(app_id().to_string());
        }

        // The window keeps the radius and re-sends it when the appearance or
        // the tile decoration changes.
        window.set_frame_corner_radius(view::CORNER);

        // Serving a portal request, this window is that application's file
        // dialog rather than a document window of ours.
        Self::adopt_picker_parent(&window, &self.state.lock().unwrap());

        // otto-kit's materials are translucent by design — they expect a
        // blurred backdrop behind them. Without one the desktop shows through
        // the window rather than being frosted by it.
        //
        // Asked of the window rather than of the style directly: the window
        // drops the blur while it is unfocused and puts it back on the next
        // activate, so the compositor is not running a full-window gaussian
        // for a window nobody is looking at.
        //
        // Under a compositor that offers no blur at all (see
        // `otto_kit::backdrop`) the panels are filled in rather than left
        // translucent.
        let blur = otto_kit::backdrop::blur_available();
        window.set_background_blur(blur);
        self.state.lock().unwrap().blur_available = blur;

        // The panels' scene. `None` only where the engine could not be brought
        // up, which is the case the immediate-mode chrome still covers: the
        // window then draws without its grounds rather than not at all.
        let scene = Arc::new(Mutex::new(window.layer_node().map(scene::Scene::new)));

        // The panels are this client's own layers, so the fade between their
        // translucent and their filled-in forms is the scene's to run — and
        // with it the only moment the blur can be switched without being seen
        // doing it. The window stops following focus and waits to be told; see
        // `scene::FrostState`, which `on_update` drains.
        if let Some(scene) = scene.lock().unwrap().as_ref() {
            window.set_fades_own_material(true);
            self.frost = Some(scene.frost_state());
        }

        let state = Arc::clone(&self.state);
        window.on_draw(move |canvas| {
            let mut browser = state.lock().unwrap();

            // Drain finished directory reads. This is the UI thread by
            // construction, which is what the poll needs — see
            // `install_frame_loop` for why it cannot live on a worker.
            let t_total = perf::now();
            let t_prep = perf::now();
            browser.poll();

            let theme = browser.theme();
            let title = browser.title();
            // Panes measure themselves against the size this frame is drawn
            // at, so their scroll views are re-fitted before anything reads
            // an offset.
            browser.sync_scroll_metrics();
            // Needs those metrics, so it runs here rather than beside the poll.
            browser.settle_restore();
            // Same reason, and after it: a Back step and a delete never land
            // in the same frame, and both want the metrics that just landed.
            browser.settle_pick();
            browser.settle_empty_ask();
            perf::mark(perf::Stage::Prep, t_prep);
            let t_frame = perf::now();
            let frame = browser.frame(&theme, &title);
            perf::mark(perf::Stage::FrameBuild, t_frame);
            // The panels first, composited by the engine from cached pictures,
            // then the chrome that sits over them.
            let t0 = perf::now();
            if let Some(scene) = scene.lock().unwrap().as_mut() {
                scene.update(&frame);
                perf::mark(perf::Stage::SceneUpdate, t0);
                let t1 = perf::now();
                scene.render(canvas);
                perf::mark(perf::Stage::SceneRender, t1);
            }
            let t2 = perf::now();
            view::draw(canvas, &frame);
            perf::mark(perf::Stage::Chrome, t2);
            perf::mark(perf::Stage::Total, t_total);
            drop(frame);

            // Where each field's caret ended up this frame, filled in as the
            // fields are drawn because only the draw knows where they went.
            let mut rename_caret = None;
            let mut path_caret = None;
            let mut search_caret = None;
            let mut save_caret = None;

            if let Some(session) = browser.rename.as_ref() {
                let (depth, index) = (session.depth, session.index);
                let (width, height) = (browser.size.0, browser.content_h());
                let count = browser.visible(depth).len();
                let scroll = browser.columns[depth].scroll.offset();
                let rect = match browser.mode {
                    ViewMode::List => {
                        view::list_rename_rect(width, browser.list_columns, count, scroll, index)
                    }
                    ViewMode::Columns => {
                        let is_dir = browser.visible(depth).get(index).is_some_and(|e| e.is_dir);
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
                let session = browser.rename.as_mut().unwrap();
                session.input.set_size(rect.width(), rect.height());
                canvas.save();
                canvas.translate((rect.left, rect.top));
                session.input.render_at(canvas, rect.width(), rect.height());
                canvas.restore();
                rename_caret = caret_in_window(&session.input, (rect.left, rect.top));
            }

            // The path entry's value, over the box the header drew for it —
            // the same two-step as the rename and save fields.
            if browser.path_entry.is_some() {
                let rect = view::path_field_rect(browser.size.0);
                let input = browser.path_entry.as_mut().unwrap();
                input.set_size(rect.width(), rect.height());
                canvas.save();
                canvas.translate((rect.left, rect.top));
                input.render_at(canvas, rect.width(), rect.height());
                canvas.restore();
                path_caret = caret_in_window(input, (rect.left, rect.top));
            }

            // The query, over the capsule the header drew for it. Inset past
            // the magnifier, so the text starts clear of the glyph rather than
            // underneath it.
            if browser.search.is_some() {
                let rect = view::search_field_rect(browser.size.0);
                let text_w = rect.width() - view::SEARCH_TEXT_INSET - 8.0;
                let input = browser.search.as_mut().unwrap();
                input.set_size(text_w, rect.height());
                let origin = (rect.left + view::SEARCH_TEXT_INSET, rect.top);
                canvas.save();
                canvas.translate(origin);
                input.render_at(canvas, text_w, rect.height());
                canvas.restore();
                search_caret = caret_in_window(input, origin);
            }

            // The save field's value, over the box the action row drew for
            // it — the same two-step the in-place rename takes, and for the
            // same reason: the text input owns its caret and selection and
            // paints them itself.
            if browser.save_name.is_some() {
                let (width, window_h) = (browser.size.0, browser.size.1);
                let rect = view::footer_name_rect(width, window_h);
                let input = browser.save_name.as_mut().unwrap();
                input.set_size(rect.width(), rect.height());
                canvas.save();
                canvas.translate((rect.left, rect.top));
                input.render_at(canvas, rect.width(), rect.height());
                canvas.restore();
                save_caret = caret_in_window(input, (rect.left, rect.top));
            }

            // Tell the compositor where the text is, so an input method — or
            // the emoji picker, or anything else watching
            // `otto_text_cursor_manager_v1` — can put itself beside the word
            // being typed instead of in the middle of the screen.
            //
            // The `or` chain is [`Browser::focused_input`]'s precedence rather
            // than the order the fields are painted in: the caret to report is
            // the one the keys are going to.
            otto_kit::AppContext::report_text_cursor(
                browser
                    .palette
                    .is_some()
                    .then_some(browser.palette_caret)
                    .flatten()
                    .or(rename_caret)
                    .or(path_caret)
                    .or(save_caret)
                    .or(search_caret),
            );

            // Last of all, because it is modal and dims everything above.
            if let Some(sheet) = browser.confirm.as_ref() {
                let (width, window_h) = (browser.size.0, browser.size.1);
                view::draw_confirm(
                    canvas,
                    &theme,
                    width,
                    window_h,
                    &view::ConfirmData {
                        message: &sheet.message,
                        detail: &sheet.detail,
                        accept_label: &sheet.accept_label,
                        pressed: sheet.pressed,
                    },
                );
            }
        });

        self.pane_surfaces = Some(pane_surfaces::PaneSurfaces::new());

        self.install_dnd(&window);
        self.install_quickview_pointer();
        self.install_palette_pointer();
        self.install_info_window_pointer();
        self.install_pointer(&window, self.context_menu.clone().unwrap());
        self.install_frame_loop(&window);
        AppContext::register_window(window.clone());
        self.window = Some(window);

        // Visible to assistive technologies. Nothing is built until one
        // attaches — see `App::accessibility`.
        if let Some(surface) = self.window.as_ref().and_then(Window::surface_id) {
            AppContext::enable_accessibility(&surface);
        }
        Ok(())
    }

    /// Repaint whatever changed since the last pass, from wherever it changed.
    ///
    /// The frame-callback loop can only carry work that is already on screen:
    /// it sustains itself by committing frames, so a window that is drawing
    /// nothing new stops being called. A directory read or a decode finishing
    /// on another thread wakes the loop instead — see
    /// [`AppContext::request_wakeup`] — and this is where that wakeup turns
    /// into a frame.
    /// What a screen reader reads: the listing of the column that has the
    /// keyboard, as the list it is drawn as.
    ///
    /// The browser moves its own cursor with the arrows, so there is no
    /// traversal ring here — the cursor row *is* the focus, which is what makes
    /// each file read out as the user moves through the directory.
    fn accessibility(
        &mut self,
        _ctx: &AppContext,
        _surface: &wayland_client::backend::ObjectId,
    ) -> Option<A11yTree> {
        let browser = self.state.lock().ok()?;
        let depth = browser.active.min(browser.columns.len().saturating_sub(1));
        let column = browser.columns.get(depth)?;

        let title = column
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| column.path.display().to_string());

        let mut tree = A11yTree::new(title.clone());

        // A directory that could not be read is what the pane shows instead of
        // a listing, so it is what the tree says too.
        if let Some(error) = &column.snapshot.error {
            tree.status(
                FILES_STATUS,
                Rect::from_wh(browser.size.0, browser.size.1),
                error.clone(),
            );
            return Some(tree);
        }

        let entries = browser.visible(depth);
        let cursor = column.cursor;
        let selection = &column.selection;

        // Where each row is, in whichever way this view lays them out. A wrong
        // rectangle is worse than none — mouse review would land on the file
        // next to the one it named — so each mode uses its own geometry, the
        // same functions that draw and hit-test it.
        let area = Rect::from_wh(browser.size.0, browser.size.1);
        let scroll = column.scroll.state.offset();
        let count = entries.len();
        // Only the rows the view shows are described, the same rows the paint
        // walks — the cost of a tree must not grow with the directory any more
        // than the cost of a frame does. Each row carries its place in the
        // whole listing, so a screen reader still reads "12 of 5000".
        let (placement, shown): (Box<dyn Fn(usize) -> Rect>, std::ops::Range<usize>) = match browser
            .mode
        {
            ViewMode::List => {
                let strip = view::RowStrip::list(browser.size.0, count, scroll);
                let band =
                    view::content_viewport(browser.size.0, browser.content_h(), ViewMode::List);
                (
                    Box::new(move |index| strip.rect(index)),
                    strip.visible(band),
                )
            }
            ViewMode::Columns => {
                let pane = view::miller_pane_rect(
                    depth,
                    browser.content_h(),
                    browser.pan.offset(),
                    browser.miller_w,
                );
                let strip = view::RowStrip::miller(pane, count, scroll);
                (
                    Box::new(move |index| strip.rect(index)),
                    strip.visible(pane),
                )
            }
            ViewMode::Grid => {
                let cells =
                    view::content_viewport(browser.size.0, browser.content_h(), ViewMode::Grid);
                let sections = browser.recent_sections.clone();
                let shown = view::grid_visible_range_in(cells, &sections, count, scroll, cells);
                (
                    Box::new(move |index| view::grid_cell_rect_in(cells, &sections, index, scroll)),
                    shown,
                )
            }
        };
        // The keyboard's row is described wherever it is: it is what the focus
        // names, and a focus pointing at an undescribed node reads as nothing.
        let off_screen_cursor = cursor.filter(|c| *c < count && !shown.contains(c));

        tree.region(
            FILES_LIST,
            area,
            Role::List,
            otto_kit::t!("files-window-title"),
            |tree| {
                for index in off_screen_cursor.into_iter().chain(shown.clone()) {
                    let entry = entries[index];
                    let bounds = placement(index);
                    tree.control(row_focus(index), bounds, Role::ListItem, true, |node| {
                        node.set_size_of_set(count);
                        node.set_position_in_set(index + 1);
                        node.set_label(entry.name.clone());
                        // What the Kind column says, plus the size for a file: the
                        // two things that tell one listing row from another when
                        // the names are similar.
                        let mut description = entry.kind_label().to_owned();
                        if let Some(size) = entry.size.filter(|_| !entry.is_dir) {
                            description.push_str(", ");
                            description.push_str(&crate::model::format_size(size));
                        }
                        node.set_description(description);
                        node.set_selected(selection.contains(&entry.selection_key()));
                        node.add_action(Action::Click);
                    });
                }
            },
        );

        // The column view shows a preview of the selected file beside the
        // listing. It is not what the keyboard is on — the listing is — but it
        // is on screen and it is about the file being read out, so it is
        // described where it sits.
        if let Some(preview) = browser
            .preview
            .as_ref()
            .filter(|_| browser.preview_visible())
            .and_then(|state| state.decoded.as_ref().map(|p| (&state.path, p)))
        {
            let (path, decoded) = preview;
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let pane = view::preview_pane_rect(
                browser.columns.len(),
                browser.content_h(),
                browser.pan.offset(),
                browser.miller_w,
            );
            tree.preview(PREVIEW_PANE, pane, &name, decoded);
        }

        // An open preview *is* what the user is looking at, so it takes the
        // focus — and the previewer describes itself, whatever it is showing.
        // See `A11yTree::preview`.
        if let Some(session) = browser.quickview.as_ref().filter(|s| s.closing.is_none()) {
            let panel = browser
                .quickview_panel
                .unwrap_or_else(|| session.panel(area));
            tree.preview(QUICKVIEW, panel, &session.name, &session.preview);
            tree.set_focus(QUICKVIEW);
        } else if let Some(cursor) = cursor.filter(|c| *c < entries.len()) {
            tree.set_focus(row_focus(cursor));
        }

        Some(tree)
    }

    /// A screen reader picked a row: select it, exactly as a click does.
    fn on_accessibility_action(
        &mut self,
        _ctx: &AppContext,
        _surface: &wayland_client::backend::ObjectId,
        request: &ActionRequest,
    ) {
        if !matches!(request.action, Action::Click) {
            return;
        }

        let Ok(mut browser) = self.state.lock() else {
            return;
        };
        let depth = browser.active.min(browser.columns.len().saturating_sub(1));
        let count = browser.visible_len(depth);
        let target = (0..count).find(|index| {
            otto_kit::accessibility::node_id(row_focus(*index)) == request.target_node
        });
        let Some(index) = target else { return };

        browser.press_entry(depth, index);
        drop(browser);
        self.render();
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        // The scene decides when the compositor's backdrop blur may be
        // switched — it owns the fade the switch has to hide under — but the
        // window is what carries the request, so it is applied here.
        if let (Some(frost), Some(window)) = (self.frost.as_ref(), self.window.as_ref()) {
            let fading = frost.is_fading();
            if let Some(frosted) = frost.take_pending() {
                window.set_frost(frosted);
            }
            if fading {
                window.request_frame();
            }
        }

        // The file area's paper is opaque whether or not the window is frosted
        // — see `scene::Scene::sync_materials` — so the compositor need not
        // blur anything behind it. Down to the path bar, which sits on the
        // same paper, less the window's rounded corner at the bottom, where the
        // edge is antialiased and the frost shows through it.
        if let Some(window) = self.window.as_ref() {
            let area = {
                let browser = self.state.lock().unwrap();
                let top = view::header_h();
                let bottom = (browser.content_h() + browser.path_bar_h() - view::corner()).max(top);
                Rect::from_ltrb(view::sidebar_w(), top, browser.size.0, bottom)
            };
            if self.opaque_region != Some(area) {
                self.opaque_region = Some(area);
                window.set_opaque_region(&[area]);
            }
        }

        let (
            repaint,
            preview_target,
            scrolled_only,
            scroll_on_surfaces,
            scroll_area,
            thumb_jobs,
            ocr_job,
        ) = {
            let mut browser = self.state.lock().unwrap();
            let changed = browser.poll();
            // Momentum, the overscroll bounce and the scrollbar's fade all
            // advance here rather than on input, since they keep running after
            // the gesture ends.
            let scrolled = browser.tick_scroll() | std::mem::take(&mut browser.scroll_moved);
            // The palette's list is a pane of its own, and its scroll is none
            // of the window's business: stepped here, shown by its surfaces.
            browser.tick_palette_scroll();
            let elapsed = browser.caret_elapsed();
            let blinking = browser.tick_caret(elapsed);
            let animating = blinking
                | browser.quickview_animating()
                | browser.tick_quickview_animation()
                | browser.tick_quickview_exit()
                | browser.tick_open_pulse();
            // The docked preview column follows the selection wherever it
            // moves — a click, an arrow key, a directory finishing a load
            // that changes what "the selection" resolves to — so this is
            // checked centrally here rather than threaded through every place
            // the selection can change.
            let preview_target = browser.sync_preview_target();
            // What the entries on screen still need a picture for. Same place
            // and same reasoning as the preview target above: everything that
            // can change what is visible — a scroll, a directory landing, a
            // switch of view mode — has already happened by the time this
            // runs.
            let thumb_jobs = browser.sync_thumbnails();
            // And, with nothing else to do, a picture whose text is not yet
            // known to Find.
            let ocr_job = browser.sync_recognition();
            let listing = std::mem::take(&mut browser.listing_dirty);
            let quiet = !changed && !browser.dirty && !animating && preview_target.is_none();
            let scrolled_only = scrolled && quiet;
            // A frame that only changes the file area — a scroll, a thumbnail
            // landing — says so, and the rest of the window is not
            // recomposited behind it.
            let scroll_area = ((scrolled || listing) && quiet)
                .then(|| browser.scroll_damage())
                .flatten();
            let scroll_on_surfaces = browser.scroll_on_surfaces();
            // Taken whatever else asks for the repaint, so it does not ask
            // again on the next pass.
            let dirty = std::mem::take(&mut browser.dirty);
            let repaint =
                changed || scrolled || listing || dirty || animating || preview_target.is_some();
            (
                repaint,
                preview_target,
                scrolled_only,
                scroll_on_surfaces,
                scroll_area,
                thumb_jobs,
                ocr_job,
            )
        };
        if let Some(job) = ocr_job {
            self.start_recognition(job);
        }
        if let Some((path, generation)) = preview_target {
            self.start_preview(path, generation);
        }
        for job in thumb_jobs {
            self.start_thumbnail(job);
        }

        // One lock, taken once. A `self.state.lock()` in an `if` condition
        // holds its guard for the whole `if`, so locking again inside the body
        // deadlocks the update loop — which is exactly what it did.
        let open_now = {
            let mut browser = self.state.lock().unwrap();
            let depth = browser.active;
            let ready = browser.quickview_auto && !browser.visible(depth).is_empty();
            if ready {
                browser.quickview_auto = false;
                browser.columns[depth].cursor = Some(0);
            }
            ready
        };
        if open_now {
            let mut browser = self.state.lock().unwrap();
            self.start_quickview(&mut browser);
        }
        self.follow_quickview();
        self.auto_palette();

        // With the columns in their own surfaces, a scroll is repainted there
        // and the window is left alone — which is the whole point, so the
        // scroll must not also count towards a window repaint.
        let mut repaint = repaint;
        if self.pane_surfaces.is_some() {
            self.sync_pane_surfaces();
            // A scroll the columns present on their own surfaces is theirs to
            // show — a step held back until the last one is on screen too: the
            // compositor's answer wakes this loop, and the next pass takes it.
            // A paint the throttle turned away is retried the same way, with
            // `idle_timeout` keeping the loop turning should no answer come.
            if scrolled_only && scroll_on_surfaces {
                repaint = false;
            }
        }

        if repaint {
            match scroll_area {
                Some(area) => self.render_damaged(area),
                None => self.render(),
            }
        }

        self.sync_info_window();
        self.advance_picker();
    }

    /// A hand laid on the touchpad stops whatever is gliding, the way a
    /// finger on a spinning wheel does. Nothing else in the pointer stream
    /// says so: a hold carries no motion and no button.
    fn on_pointer_hold_begin(&mut self, _ctx: &AppContext, _fingers: u32) {
        let mut browser = self.state.lock().unwrap();
        browser.pan.stop();
        browser.gesture_axis = None;
        for column in &mut browser.columns {
            column.scroll.stop();
        }
        drop(browser);
        self.render();
    }

    /// A two-finger pinch zooms the open preview's picture.
    ///
    /// Only the preview: the browser's own views have no zoom, and a pinch
    /// with no panel up is left alone rather than repurposed into something
    /// the gesture does not mean anywhere else.
    fn on_pointer_pinch_begin(&mut self, _ctx: &AppContext, fingers: u32) {
        let mut browser = self.state.lock().unwrap();
        // Where this gesture's scale is measured from. Taken at the start
        // because the protocol reports scale against the start.
        browser.quickview_pinch = (fingers == 2)
            .then(|| browser.quickview.as_ref().map(|s| s.zoom.scale))
            .flatten();
    }

    fn on_pointer_pinch_update(
        &mut self,
        _ctx: &AppContext,
        dx: f64,
        dy: f64,
        scale: f64,
        _rotation: f64,
    ) {
        let mut browser = self.state.lock().unwrap();
        let Some(base) = browser.quickview_pinch else {
            return;
        };
        let moved = browser.quickview_zoom_to(base * scale as f32, (dx as f32, dy as f32));
        drop(browser);
        if moved {
            self.render();
        }
    }

    fn on_pointer_pinch_end(&mut self, _ctx: &AppContext, _cancelled: bool) {
        // Nothing to settle: every update already left the zoom clamped and
        // snapped, so the fingers lifting only ends the gesture.
        self.state.lock().unwrap().quickview_pinch = None;
    }

    /// While something is gliding the app needs a steady clock, not just the
    /// next input event.
    fn idle_timeout(&self) -> Option<std::time::Duration> {
        let browser = self.state.lock().unwrap();
        let animating = browser.scroll_animating()
            || browser.quickview_animating()
            // An animated preview has a frame due on its own clock, with
            // nothing else on screen moving to ask for one.
            || browser.quickview_frames_running()
            || browser.opening.is_some()
            // The panel materials' fade runs on this client's own engine, and
            // an engine only advances when it is ticked.
            || self.frost.as_ref().is_some_and(|frost| frost.is_fading())
            // A blinking caret needs the same steady clock, and for the same
            // reason: nothing else is going to ask for the next frame.
            || browser.has_focused_input()
            // A surface paint held back for a frame that has not been answered.
            || self
                .pane_surfaces
                .as_ref()
                .is_some_and(pane_surfaces::PaneSurfaces::pending);
        animating.then_some(IDLE_TICK)
    }

    fn on_configure(&mut self, _ctx: &AppContext, configure: WindowConfigure, _serial: u32) {
        // Whether the window is tiled arrives on the configure — otto-kit has
        // already read it by the time this runs — and the chrome follows the
        // decoration a tile wears: the lights' size and row, the header and
        // sidebar under them, and the frame's corners, which the window
        // itself re-sends.
        if let Some(window) = self.window.as_ref() {
            if view::set_decoration_variant(window.decoration_variant()) {
                self.state.lock().unwrap().dirty = true;
            }
        }
        let mut browser = self.state.lock().unwrap();
        if let (Some(w), Some(h)) = (configure.new_size.0, configure.new_size.1) {
            browser.size = (w.get() as f32, h.get() as f32);
            browser.dirty = true;
        }
        // Focus arrives here, and the chrome is drawn dimmer without it.
        if browser.focused != configure.is_activated() {
            browser.focused = configure.is_activated();
            browser.dirty = true;
        }
        drop(browser);
        self.render();
    }

    /// The authoritative modifier state, sent before the key event it
    /// belongs to and again whenever the window takes focus.
    fn on_modifiers(&mut self, _ctx: &AppContext, modifiers: Modifiers) {
        *self.modifiers.lock().unwrap() = modifiers;
    }

    /// Quick View is a preview of what the window has selected, so it belongs
    /// to the window's focus: once the keyboard goes somewhere else the panel
    /// is a card floating over a background window with nothing to preview.
    ///
    /// This is also how expose reaches us. The panel is a subsurface, not a
    /// popup, so the compositor's `dismiss_all_popups` on the way into Show All
    /// cannot take it down — but Otto drops keyboard focus entering expose, and
    /// that lands here.
    ///
    /// Only the browser's own toplevel counts. A leave on the Get Info panel is
    /// focus moving between two of our windows, not away from the browser.
    fn on_keyboard_leave(&mut self, _ctx: &AppContext, surface: &wl_surface::WlSurface) {
        use wayland_client::Proxy;
        let ours = self
            .window
            .as_ref()
            .and_then(|window| window.wl_surface())
            .is_some_and(|main| main.id() == surface.id());
        if !ours {
            return;
        }
        // Both go with the keyboard: a panel that is nothing but a place to
        // type has no reason to stay up once the typing would land elsewhere.
        let mut browser = self.state.lock().unwrap();
        let changed = browser.close_quickview() | browser.close_palette();
        drop(browser);
        if changed {
            self.render();
        }
    }

    fn on_key_event(
        &mut self,
        _ctx: &AppContext,
        event: &KeyEvent,
        key_state: wl_keyboard::KeyState,
        serial: u32,
    ) {
        self.handle_key(event, key_state, serial);
    }
}

impl FilesApp {
    /// Open the palette on its own, for looking at. See
    /// [`Browser::palette_auto`] — never on in a real session.
    pub(super) fn auto_palette(&self) {
        let mut browser = self.state.lock().unwrap();
        let depth = browser.active;
        if browser.palette_auto.is_none() || browser.visible(depth).is_empty() {
            return;
        }
        // Scripts describe themselves in the background, and the palette
        // offers what has answered when it opens — so a query typed this
        // early would miss them; a script's run lands later too, and the
        // Undo it leaves behind is only offered once it has. Wait for a
        // later pass; a person cannot press Ctrl+P that soon.
        if !browser.commands.settled() {
            return;
        }
        // One segment per pass, so the wait above holds between them as well.
        let query = browser.palette_auto.take().unwrap_or_default();
        let (segment, rest) = match query.split_once('|') {
            Some((segment, rest)) => (segment.to_owned(), Some(rest.to_owned())),
            None => (query, None),
        };
        browser.palette_auto = rest;
        if browser.columns[depth].cursor.is_none() {
            browser.columns[depth].cursor = Some(0);
        }
        let mods = KeyMods {
            shift: false,
            ctrl: false,
        };
        // A value other than a bare "1" is typed in, so a screenshot can be
        // taken of the palette part-way through a query rather than at rest.
        // A tab is a Tab press, so argument mode can be looked at too; a
        // newline is Return, which runs the command; and `|` opens the palette
        // again for another go — `$'select m\t*\n|rename\tHoliday {n}'`
        // selects everything and then shows the rename's dry run on it.
        if browser.palette.is_none() {
            browser.open_palette();
        }
        let keys = if segment == "1" { "" } else { segment.as_str() };
        for ch in keys.chars() {
            let key = match ch {
                '\t' => palette::Key::Tab,
                '\n' => palette::Key::Enter,
                '\u{1b}' => palette::Key::Escape,
                '\u{2193}' => palette::Key::Down,
                '\u{2191}' => palette::Key::Up,
                ch => palette::Key::Edit(TextInputKey::Char(ch)),
            };
            let Some(outcome) = browser
                .palette
                .as_mut()
                .map(|palette| palette.on_key(key, mods))
            else {
                break;
            };
            browser.settle_palette(outcome, 0);
        }
        browser.dirty = true;
    }

    /// Move the picker on when its request is done with, and notice when the
    /// portal withdraws the one on screen.
    ///
    /// A no-op in the browser, and on every pass where the window is still
    /// serving a live request — which is nearly all of them.
    pub(super) fn advance_picker(&mut self) {
        let Some(queue) = self.picker_queue.clone() else {
            return;
        };

        {
            let mut browser = self.state.lock().unwrap();
            let Some(session) = browser.picker.as_mut() else {
                return;
            };
            // Withdrawn by the portal — the requesting application went away,
            // or gave up. The window goes immediately; there is nobody left
            // to answer.
            if !session.answered() && queue.take_withdrawn(&session.request.handle) {
                session.resolve(picker::Outcome::ended());
            }
            if !session.answered() {
                return;
            }
        }

        // The request has been answered. Serve the next one in the same
        // window, or leave — an idle picker holds no window and no Wayland
        // connection, and the bus starts a fresh one when it is next needed.
        match queue.next_session() {
            Some(session) => {
                let start = session.request.starting_directory(None);
                let mut browser = self.state.lock().unwrap();
                let size = browser.size;
                *browser = Browser::for_picker(session, start);
                browser.size = size;
                browser.dirty = true;
                // The next request comes from a different application, so the
                // window belongs to a different parent now.
                if let Some(window) = self.window.as_ref() {
                    Self::adopt_picker_parent(window, &browser);
                }
                drop(browser);
                self.render();
            }
            None => AppContext::request_exit(),
        }
    }

    /// Tell the compositor whose dialog this window is.
    ///
    /// The portal request carries `parent_window` — a handle the requesting
    /// application exported with xdg-foreign — and importing it makes this
    /// window a child of that one. A window with a parent is a dialog: it
    /// stacks with its parent, and on a tiling workspace Otto floats it
    /// instead of laying it out as a tile.
    ///
    /// The handle may be empty (the application never exported one) or the
    /// compositor may not offer xdg-foreign, so the dialog hint goes out in
    /// either case: it says the same thing without naming a parent, and it is
    /// what keeps the picker out of the tree when the import fails.
    ///
    /// A no-op in the browser and the Trash, which are windows in their own
    /// right.
    pub(super) fn adopt_picker_parent(window: &Window, browser: &Browser) {
        let Some(session) = browser.picker.as_ref() else {
            return;
        };
        // Without a parent the compositor has nothing to attach the picker
        // to, and only a *modal* hint keeps it out of a tiling layout (a bare
        // dialog hint is what GTK gives every window). A picker is modal to
        // its requester in practice anyway, so ask for it.
        let parented = window.set_parent_handle(&session.request.parent_window);
        if !parented {
            tracing::debug!("no foreign parent for this pick; asking for modal instead");
        }
        window.set_modal(session.request.modal || !parented);
    }
}
