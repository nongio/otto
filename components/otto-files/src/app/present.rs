//! Presenting: the surfaces beside the window, repaints, decodes and the info window.

use super::*;
use otto_kit::preview::Preview;

impl FilesApp {
    /// Bring every surface beside the window up to date: the columns, the
    /// preview, Quick View and the palette.
    pub(super) fn sync_pane_surfaces(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let Some(surface) = window.surface() else {
            return;
        };
        let parent = surface.wl_surface().clone();
        let mut browser = self.state.lock().unwrap();
        browser.sync_scroll_metrics();
        // Before the frame is built from it: a panel dragged aside in a
        // bigger window, or before the display answered, may otherwise be
        // placed somewhere nothing can reach it.
        browser.clamp_quickview();
        let theme = browser.theme();
        let title = browser.title();
        let frame = browser.frame(&theme, &title);
        let quickview = browser
            .quickview_visible()
            .map(|session| (session, browser.quickview_generation));
        if let Some(panes) = self.pane_surfaces.as_mut() {
            panes.sync(&parent, &frame, quickview);
        }

        // Hand the pointer handler the rect the panel was actually placed at.
        // Doing it here, after the sync, is what keeps the hit test and the
        // paint from disagreeing about where the card is.
        drop(frame);
        browser.quickview_panel = self
            .pane_surfaces
            .as_ref()
            .and_then(pane_surfaces::PaneSurfaces::quickview_resting);
        // The offset that rect was placed with, recorded together with it: a
        // drag reads the two back to work out where the card would rest
        // untouched, and they are only true as a pair.
        browser.quickview_placed_offset = browser
            .quickview_panel
            .and(browser.quickview_visible().map(|session| session.offset));
        // Where the display is, so a drag of the title strip knows how far the
        // card may go. Like the panel's own rect, it is the surface layer that
        // knows and the pointer handler that asks.
        browser.quickview_display = self
            .pane_surfaces
            .as_ref()
            .and_then(pane_surfaces::PaneSurfaces::quickview_display);
        *self.quickview_target.lock().unwrap() = self
            .pane_surfaces
            .as_ref()
            .and_then(pane_surfaces::PaneSurfaces::quickview_target);

        // The palette's card, on its own surface. Synced apart from the rest
        // because painting it needs the browser back mutably — its field
        // renders itself — which the `Frame` above cannot allow while it
        // lives. See `PaneSurfaces::sync_palette`.
        {
            let theme = browser.theme();
            let size = browser.size;
            let palette = browser.palette_frame();
            // Ask where the display is while the card is up, so a drag knows
            // how far it may go. The answer is relative to the window, so it
            // is only worth having for as long as the window stays put.
            browser.palette_display = self
                .pane_surfaces
                .as_mut()
                .filter(|_| palette.is_some())
                .and_then(pane_surfaces::PaneSurfaces::palette_display);
            if let Some(panes) = self.pane_surfaces.as_mut() {
                panes.sync_palette(&parent, &theme, size, palette.as_ref(), |canvas| {
                    browser.paint_palette_field(canvas)
                });
            }
            *self.palette_target.lock().unwrap() = self
                .pane_surfaces
                .as_ref()
                .and_then(pane_surfaces::PaneSurfaces::palette_target);
        }
    }

    pub(super) fn render(&self) {
        if let Some(window) = &self.window {
            window.request_frame();
            Self::wake_for_frame(window);
        }
    }

    /// [`Self::render`] for a frame known to change `area` (in points) and
    /// nothing else.
    pub(super) fn render_damaged(&self, area: Rect) {
        if let Some(window) = &self.window {
            window.request_frame_damaged(&[area]);
            Self::wake_for_frame(window);
        }
    }

    pub(super) fn wake_for_frame(window: &Window) {
        // The runner renders dirty windows at the *top* of a loop iteration,
        // before `on_update`, so a frame requested from `on_update` would sit
        // unrendered until some other event happened to wake the loop. Asking
        // for one more turn is what makes a repaint requested off the input
        // path actually appear — unless a frame is already on its way to the
        // screen: the window cannot paint until the compositor answers, that
        // answer wakes the loop itself, and waking it sooner only spins the
        // loop rebuilding a frame nobody will paint.
        let in_flight = window.surface().is_some_and(|s| s.frame_in_flight());
        if !in_flight {
            AppContext::request_wakeup();
        }
    }

    /// Move an open Quick View onto whatever the cursor landed on after a
    /// delete.
    ///
    /// A delete cannot do this itself: the successor is only known once the
    /// re-read lands, which is [`Browser::settle_pick`], and that runs deep
    /// inside the draw closure where a decode cannot be spawned. It raises a
    /// flag instead and this drains it on the next pass — one frame of a
    /// stale panel, against a panel that would otherwise sit there showing a
    /// file that is now in the Trash.
    pub(super) fn follow_quickview(&self) {
        let mut browser = self.state.lock().unwrap();
        // Quick View asked for by name, from the palette. Its command ran in
        // the browser, which does not own the decode.
        if browser.take_palette_quickview() {
            self.start_quickview(&mut browser);
            return;
        }
        if browser.take_quickview_follow() {
            self.start_quickview(&mut browser);
        }
    }

    /// Preview the cursor's file, decoding off the UI thread.
    ///
    /// [`quickview::decode`] blocks until the sandboxed worker answers or its
    /// deadline expires — inline, that would stall the frame loop for as long
    /// as the file takes.
    pub(super) fn start_quickview(&self, browser: &mut Browser) {
        let Some((path, generation, anchor)) = browser.begin_quickview() else {
            return;
        };
        let panel = quickview::panel_rect(browser.size.0, browser.size.1);
        let scale = AppContext::scale_factor().max(1) as f32;
        let state = Arc::clone(&self.state);

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        let recognise = ocr::enabled();
        let recogniser = ocr::command();
        tokio::task::spawn_blocking(move || {
            let mut preview = quickview::decode(&path, panel, scale, 1);
            let video = (path.is_file() && otto_media_kit::player::available())
                .then(|| (path.clone(), quickview::video_options(panel, scale, true)));
            // Words already remembered ride in with the picture; otherwise
            // the picture goes up now and the recogniser follows.
            let picture = ocr::is_picture(&path)
                && matches!(&preview, Preview::Pixels { pixels, .. } if pixels.words.is_empty());
            let mut needs_recognising = false;
            if picture && recognise {
                match quickview::remembered_words(&path, panel, scale, 1) {
                    Some(words) => {
                        if let Preview::Pixels { pixels, .. } = &mut preview {
                            pixels.words = words;
                        }
                    }
                    None => needs_recognising = ocr::available(),
                }
            }
            {
                let mut browser = state.lock().unwrap();
                browser.finish_quickview(generation, anchor, name, preview, video);
                if needs_recognising {
                    browser.begin_reading(path.clone());
                    browser.start_quickview_recognising(generation);
                }
            }
            // Wake the UI thread: a window showing "Opening preview…" is not
            // committing frames, so there is no frame callback to notice the
            // decode landed.
            AppContext::request_wakeup();

            if !needs_recognising {
                return;
            }
            // The user may already have moved on; the recogniser is the
            // expensive half, so it is not started for a file nobody is
            // looking at any more.
            {
                let mut browser = state.lock().unwrap();
                if browser.quickview_generation != generation {
                    browser.end_reading(&path);
                    return;
                }
            }
            let words = quickview::recognise(
                &path,
                panel,
                scale,
                1,
                recogniser,
                quickview::Priority::Interactive,
            );
            {
                let mut browser = state.lock().unwrap();
                match words {
                    Some(words) => browser.finish_quickview_words(generation, words),
                    None => browser.finish_quickview_recognising(generation),
                }
                browser.end_reading(&path);
            }
            AppContext::request_wakeup();
        });
    }

    /// Turn an open PDF's page, decoding off the UI thread like any other
    /// preview. Returns whether there was a page to turn to — when there is
    /// not, the keystroke was never the preview's and the caller goes on to
    /// do what it does to the listing.
    pub(super) fn turn_quickview_page(&self, browser: &mut Browser, delta: i32) -> bool {
        let Some((path, generation, page, anchor)) = browser.turn_quickview_page(delta) else {
            return false;
        };
        let panel = quickview::panel_rect(browser.size.0, browser.size.1);
        let scale = AppContext::scale_factor().max(1) as f32;
        let state = Arc::clone(&self.state);

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        // No video: what is being turned is a document's page, and a
        // paginated preview is never a player.
        tokio::task::spawn_blocking(move || {
            let preview = quickview::decode(&path, panel, scale, page);
            state
                .lock()
                .unwrap()
                .finish_quickview(generation, anchor, name, preview, None);
            AppContext::request_wakeup();
        });
        true
    }

    /// Decode the docked preview column's target, off the UI thread — the
    /// same worker path Quick View's overlay uses, just landing in
    /// [`Browser::finish_preview`] instead of [`Browser::finish_quickview`].
    pub(super) fn start_preview(&self, path: PathBuf, generation: u64) {
        let panel = {
            let browser = self.state.lock().unwrap();
            Rect::from_wh(view::PREVIEW_W, browser.size.1)
        };
        let scale = AppContext::scale_factor().max(1) as f32;
        let state = Arc::clone(&self.state);

        // Paused, and only where a worker exists: the column is a glance that
        // follows the selection, and the click that starts it is the user
        // asking for sound. It opens on the first frame, not on black.
        let video = (path.is_file() && otto_media_kit::player::available())
            .then(|| quickview::video_options(panel, scale, false));
        tokio::task::spawn_blocking(move || {
            let preview = quickview::decode(&path, panel, scale, 1);
            state
                .lock()
                .unwrap()
                .finish_preview(generation, preview, video);
            AppContext::request_wakeup();
        });
    }

    /// Fetch one thumbnail off the UI thread.
    ///
    /// The shared cache makes most of these a single file read; the rest end
    /// in the sandboxed decoder, which is why this is never run inline. The
    /// result is recorded whatever it is — a miss is worth remembering, or the
    /// same file is asked for again on the very next frame.
    pub(super) fn start_thumbnail(&self, job: thumbnails::Job) {
        let state = Arc::clone(&self.state);
        tokio::task::spawn_blocking(move || {
            let found = thumbnails::fetch(&job);
            let mut browser = state.lock().unwrap();
            let before = browser.thumbs.epoch();
            browser.thumbs.finish(job.path, job.modified, found);
            // A picture landing changes what the panes draw; a miss changes
            // only what will be asked for next, and repainting for it would
            // render the same pixels again. The store's epoch is what tells
            // the two apart. In column view the rows are the columns' own
            // surfaces, which see the epoch move for themselves: the window
            // has nothing to repaint. In the list and the grid only the file
            // area has.
            browser.listing_dirty |=
                browser.thumbs.epoch() != before && browser.mode != ViewMode::Columns;
            drop(browser);
            // Same reason the preview decodes wake the loop: a window that has
            // stopped committing frames has no frame callback to notice a
            // thumbnail landed.
            AppContext::request_wakeup();
        });
    }

    /// Keep repainting while a directory read is outstanding.
    ///
    /// Two constraints force this shape. A worker thread cannot ask for a
    /// repaint at all — `AppContext::request_frame` dispatches through a
    /// thread-local and is a silent no-op anywhere but the UI thread. And
    /// asking from *inside* the draw does not reliably schedule another frame:
    /// measured, a request made during a draw is honoured only when some other
    /// event also triggers a render, so a read finishing after the last input
    /// is never shown. The frame callback runs on the UI thread and after the
    /// frame is acknowledged, which is the one place both hold.
    ///
    /// The loop sustains itself only while something is loading, so an idle
    /// window costs nothing.
    pub(super) fn install_frame_loop(&self, window: &Window) {
        use wayland_client::Proxy;

        let Some(surface) = window.wl_surface() else {
            return;
        };
        let state = Arc::clone(&self.state);
        let window = window.clone();

        AppContext::register_frame_callback(surface.id(), move || {
            let repaint = {
                let mut browser = state.lock().unwrap();
                // A read that lands *here* is not in the frame just drawn, and
                // clears `loading` — so without the `changed` arm the column
                // that finished would sit empty until the next input event.
                let changed = browser.poll();
                // Also while a Quick View call is outstanding, so its result
                // paints without waiting for the next keystroke.
                // …and while the preview's entrance is still running, which is
                // animated in this process now that the panel lives in this
                // window's own surface.
                let opening = browser.quickview_animating();
                let preview_pending = browser.preview.as_ref().is_some_and(|p| p.pending);
                changed
                    || browser.loading()
                    || browser.quickview_pending
                    || browser.quickview_recognising
                    || opening
                    || preview_pending
                    // …and while thumbnails are being fetched, so they appear
                    // as they land rather than at the next keystroke.
                    || browser.thumbs.is_busy()
            };
            if repaint {
                window.request_frame();
            }
        });
    }

    /// Bring the Get Info panel's window into line with the browser's state:
    /// open one when there is something to show, take it away when there is
    /// not, and repaint it when what it shows changes.
    ///
    /// The panel is a window of its own rather than a sheet drawn over this
    /// one. It is dragged around, it stays put while the browser goes on
    /// being used behind it, and it wants the shadow and the stacking every
    /// other window gets — all of which the compositor already does for a
    /// toplevel and none of which is worth rebuilding inside this window.
    /// It carries the browser's own `app_id`, so it lands under the same dock
    /// icon instead of adding one of its own.
    pub(super) fn sync_info_window(&mut self) {
        let (wanted, dirty) = {
            let mut browser = self.state.lock().unwrap();
            (
                browser.info.is_some(),
                std::mem::take(&mut browser.info_dirty),
            )
        };
        // Never with the cell borrowed: creating a window talks to the
        // compositor, which dispatches events — the panel's own pointer
        // callback among them — and that callback reads this same cell.
        let open = self.info_window.borrow().is_some();
        match (wanted, open) {
            (true, false) => {
                let window = self.create_info_window();
                *self.info_window.borrow_mut() = window;
            }
            (false, true) => {
                let window = self.info_window.borrow_mut().take();
                if let Some(window) = window {
                    window.close();
                }
            }
            _ => {}
        }
        if dirty {
            let window = self.info_window.borrow().clone();
            if let Some(window) = window {
                window.request_frame();
                AppContext::request_wakeup();
            }
        }
    }

    pub(super) fn create_info_window(&self) -> Option<Window> {
        let mut window = Window::new(
            otto_kit::t!("files-info-window-title"),
            view::INFO_W as i32,
            view::INFO_H as i32,
        )
        .ok()?;
        // The layout inside is fixed, so the window does not resize.
        window.set_min_size(view::INFO_W as u32, view::INFO_H as u32);
        window.set_max_size(view::INFO_W as u32, view::INFO_H as u32);
        // The card paints its own ground, and the compositor rounds and
        // shadows the surface around it.
        window.set_background(skia_safe::Color::TRANSPARENT);
        if let Some(surface) = window.surface() {
            // The window's own app_id: this is another window of the file
            // manager, not another application, and the dock and the app
            // switcher should both read it that way.
            surface.xdg_window().set_app_id(app_id().to_string());
        }
        if let Some(style) = window.surface_style() {
            style.set_corner_radius(otto_kit::corners::radius(14.0) as f64);
        }

        let state = Arc::clone(&self.state);
        window.on_draw(move |canvas| {
            let browser = state.lock().unwrap();
            let Some(info) = browser.info.as_ref() else {
                return;
            };
            let theme = AppContext::current_theme();
            view::draw_info(
                canvas,
                &theme,
                Rect::from_wh(view::INFO_W, view::INFO_H),
                info,
                browser.info_text,
                browser.info_error.as_deref(),
                browser.info_close_hovered,
                false,
            );
        });

        // A close asked for from outside — the app switcher, a keyboard
        // shortcut, the dock's Quit — closes the panel. Without this the
        // runner would take it for the application's own close request and
        // end the process, which is a surprising way for Get Info to go away.
        let state = Arc::clone(&self.state);
        window.on_close_request(move || state.lock().unwrap().close_info());

        Some(window)
    }
}
