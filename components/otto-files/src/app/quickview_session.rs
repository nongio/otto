//! Quick View: opening, following, zooming, panning and closing the panel.

use super::*;

impl Browser {
    /// The rect Quick View grows its panel from, for the active column.
    ///
    /// Surface-local, and empty when there is nothing on screen to grow out of.
    /// Both are usable now that the panel is drawn into this same surface.
    pub(super) fn quickview_anchor(&self) -> Rect {
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let entries = self.visible(depth);
        let column = &self.columns[depth];
        let pane = view::PaneData {
            selection: Some(&column.selection),
            cursor: column.cursor,
            entries,
            scroll: column.scroll.offset(),
            bar: None,
            velocity: 0.0,
            loading: column.awaiting_first_listing(),
            error: None,
        };
        view::quickview_anchor(
            self.size.0,
            self.content_h(),
            self.mode,
            &pane,
            depth,
            self.pan.offset(),
            self.miller_w,
        )
    }

    /// Drain the follow flag a delete raised.
    ///
    /// Returns whether the host still owes a decode. It does not when the
    /// delete emptied the pane outright: there is no row left to preview, so
    /// the panel is dismissed here rather than left up over the gap. The
    /// choice belongs to the browser — it is the one that knows what the
    /// cursor is on — and the host only runs the decode.
    pub(super) fn take_quickview_follow(&mut self) -> bool {
        if !std::mem::take(&mut self.quickview_follow) || self.quickview.is_none() {
            return false;
        }
        if self.selected_entry().is_none() {
            self.close_quickview();
            return false;
        }
        true
    }

    /// Start previewing the cursor's file, replacing whatever is open.
    ///
    /// Returns the path to decode and the generation to tag it with, or `None`
    /// when there is nothing to preview. The decode itself is the caller's to
    /// run off the UI thread — this only moves the state.
    pub(super) fn begin_quickview(&mut self) -> Option<(PathBuf, u64, Rect)> {
        // Recent and search results preview like any other listing. Their
        // rows are real files with real paths, and since the selection is
        // keyed by path — see [`Entry::selection_key`] — the row the panel is
        // anchored to survives the next batch landing under it. Previewing is
        // most of what those two panes are *for*: finding a file you cannot
        // quite name is how you got there.
        let entry = self.selected_entry()?;
        // A directory previews as a listing, which the decoder handles, so
        // nothing is excluded here.
        let anchor = self.quickview_anchor();
        self.quickview_generation += 1;
        self.quickview_pending = true;
        // The panel goes up — or over to this file — on the keystroke, not
        // when the decode lands. An open one drops what it was showing now
        // rather than carrying the last file's preview while the new one is
        // read, and keeps its place and its entrance while it waits.
        match self.quickview.as_mut() {
            Some(session) => session.awaiting(entry.name.clone(), entry.is_dir, anchor),
            None => {
                self.quickview = Some(quickview::Session::waiting(
                    entry.name.clone(),
                    entry.is_dir,
                    anchor,
                    std::time::Instant::now(),
                ));
                self.quickview_closing = None;
            }
        }
        // Clear any stale message so "Opening preview…" is not fighting the
        // last operation's summary.
        self.status = None;
        self.dirty = true;
        Some((entry.path, self.quickview_generation, anchor))
    }

    /// Show a decode that arrived, unless the user has moved on since.
    pub(super) fn finish_quickview(
        &mut self,
        generation: u64,
        anchor: Rect,
        name: String,
        preview: otto_kit::preview::Preview,
        video: Option<(PathBuf, otto_media_kit::Options)>,
    ) {
        if generation != self.quickview_generation {
            return; // Stale: the user arrow-keyed past this file mid-decode.
        }
        self.quickview_pending = false;
        // Re-opening onto the same panel keeps its entrance rather than
        // replaying it, so arrow-keying through a folder does not pulse.
        let (opened_at, expanded) = match &self.quickview {
            Some(session) => (session.opened_at, session.expanded),
            None => (std::time::Instant::now(), false),
        };
        let mut session = quickview::Session::new(preview, name, anchor, opened_at);
        session.expanded = expanded;
        // Only once the decoder has said what the file is: the player is
        // started on the sniffed type, never on the name. It wakes the loop
        // from its own thread on every frame, the way a landing decode does.
        if let Some((path, limits)) = video {
            session.attach_video(&path, limits, AppContext::request_wakeup);
        }
        self.quickview = Some(session);
        // Re-opening cancels whatever was on its way out: two panels in flight
        // at once would cross over each other.
        self.quickview_closing = None;
        self.dirty = true;
    }

    /// Expand the open panel to fill its display, or bring it back.
    pub(super) fn toggle_quickview_expand(&mut self) {
        if let Some(session) = self.quickview.as_mut() {
            session.toggle_expanded();
            self.dirty = true;
        }
    }

    /// Where the panel rests when the surface layer has not said: centred in
    /// the window, at whichever size the session asks for.
    pub(super) fn quickview_fallback_panel(&self) -> Rect {
        let expanded = self.quickview.as_ref().is_some_and(|s| s.expanded);
        quickview::resting_in(Rect::from_wh(self.size.0, self.size.1), expanded)
    }

    /// Remember where the pointer is over the Quick View panel, and which
    /// panel rect that position was measured against.
    ///
    /// A pinch carries no position of its own — only how far its focal point
    /// has drifted since it began — so the only way to zoom about the fingers
    /// is to have kept the last place the pointer was seen.
    pub(super) fn quickview_focus(&mut self, point: skia_safe::Point, panel: Rect) {
        self.quickview_focus = Some((point, panel));
    }

    /// Zoom the open preview to `scale`, about the focal point: wherever the
    /// pointer last was, carried along by `drift` — the focal point's travel
    /// since the pinch began. Returns whether anything moved.
    pub(super) fn quickview_zoom_to(&mut self, scale: f32, drift: (f32, f32)) -> bool {
        // With no remembered pointer — a pinch that began before the panel
        // ever saw one — the panel's own centre is the honest focal point.
        let (focus, panel) = match self.quickview_focus {
            Some((point, panel)) => ((point.x + drift.0, point.y + drift.1), panel),
            None => {
                let panel = self
                    .quickview_panel
                    .unwrap_or_else(|| self.quickview_fallback_panel());
                let content = view::quickview_content_rect(panel);
                ((content.center_x(), content.center_y()), panel)
            }
        };
        let content = view::quickview_content_rect(panel);
        let Some(session) = self.quickview.as_mut() else {
            return false;
        };
        let moved = session.zoom_to(scale, focus, content);
        self.dirty |= moved;
        moved
    }

    /// Feed a two-finger scroll to the open preview: a pan when there is a
    /// zoomed image to drag, and the scroll it has always been otherwise.
    ///
    /// One entry point for both handlers — the toplevel's and the panel's own
    /// surface — because the choice between panning and scrolling has to come
    /// out the same whichever of the two the compositor happened to deliver
    /// the event to.
    pub(super) fn quickview_wheel(
        &mut self,
        dx: f32,
        dy: f32,
        panel: Rect,
        stop: bool,
        discrete: bool,
    ) {
        let content = view::quickview_content_rect(panel);
        let pannable = self
            .quickview
            .as_ref()
            .is_some_and(|session| session.pannable(content));
        let Some(session) = self.quickview.as_mut() else {
            return;
        };
        if pannable {
            // A picture is scrolled, not dragged: the deltas go to the pan's
            // own scroll views, which amplify them the way every other scroll
            // view in the toolkit does, keep gliding when the fingers lift,
            // and resist the ends.
            session.pan_wheel(dx, dy, content, stop, discrete);
        } else {
            // A gesture that ended moved nothing on its own.
            if stop {
                return;
            }
            // The content's box, not the card's: the rows are laid out below
            // the title strip, so scrolling has to measure against the same
            // rect the preview was drawn into.
            let rows = (dy / otto_kit::preview::ROW_HEIGHT).round() as i32;
            session.scroll_by(rows, content);
        }
        self.dirty = true;
    }

    /// Route a pointer event over the panel to the pan's scrollbars.
    ///
    /// Returns whether the bar took the press: the panel dismisses on a click
    /// outside and the close dot on a click inside, and a bar dragged over a
    /// zoomed picture must do neither.
    pub(super) fn quickview_pan_pointer(
        &mut self,
        kind: QuickviewPointer,
        point: skia_safe::Point,
        panel: Rect,
    ) -> bool {
        let content = view::quickview_content_rect(panel);
        let Some(session) = self.quickview.as_mut() else {
            return false;
        };
        // A video's controls come before the pan: the two never share a
        // panel, and the player wants the press wherever on the picture it
        // lands.
        let video = match kind {
            QuickviewPointer::Press => quickview::VideoPointer::Press,
            QuickviewPointer::Motion => quickview::VideoPointer::Motion,
            QuickviewPointer::Release => quickview::VideoPointer::Release,
            QuickviewPointer::Leave => quickview::VideoPointer::Leave,
        };
        if let Some(handled) = session.video_pointer(video, point.x, point.y, content) {
            self.dirty |= handled;
            return handled;
        }
        let (handled, moved) = match kind {
            QuickviewPointer::Press => {
                let hit = session.pan_pointer_down(point.x, point.y, content);
                (hit, hit)
            }
            QuickviewPointer::Motion => {
                (false, session.pan_pointer_move(point.x, point.y, content))
            }
            QuickviewPointer::Release => {
                session.pan_pointer_up();
                (false, false)
            }
            QuickviewPointer::Leave => {
                session.pan_pointer_up();
                session.pan_pointer_leave();
                (false, false)
            }
        };
        self.dirty |= moved;
        handled
    }

    /// Advance the open preview's pan by one frame. Returns whether it moved.
    pub(super) fn tick_quickview_pan(&mut self) -> bool {
        let panel = self
            .quickview_panel
            .unwrap_or_else(|| self.quickview_fallback_panel());
        let content = view::quickview_content_rect(panel);
        let Some(session) = self.quickview.as_mut() else {
            return false;
        };
        let moved = session.tick_pan(content);
        self.dirty |= moved;
        moved
    }

    /// Advance an animated preview — a GIF, an animated WEBP — to the frame
    /// its clock has reached. Returns whether the picture changed.
    pub(super) fn tick_quickview_animation(&mut self) -> bool {
        let Some(session) = self.quickview.as_mut() else {
            return false;
        };
        let moved = session.tick_animation();
        self.dirty |= moved;
        moved
    }

    /// Whether the open preview is an animation, which needs the steady clock
    /// for as long as it is open.
    pub(super) fn quickview_frames_running(&self) -> bool {
        self.quickview
            .as_ref()
            .is_some_and(quickview::Session::frames_running)
    }

    /// Whether the open preview's pan still has frames to run.
    pub(super) fn quickview_pan_animating(&self) -> bool {
        self.quickview
            .as_ref()
            .is_some_and(quickview::Session::pan_animating)
    }

    /// Dismiss the preview. Returns whether one was open.
    pub(super) fn close_quickview(&mut self) -> bool {
        // Bumping the generation orphans an in-flight decode, so a slow file
        // cannot re-open a panel the user has already dismissed.
        self.quickview_generation += 1;
        self.quickview_pending = false;
        // Where the file is *now*, not where it was when the panel opened: the
        // selection may have arrow-keyed on, or the list scrolled, and the
        // point of the exit is to say which file this was.
        let anchor = self.quickview_anchor();
        let Some(mut session) = self.quickview.take() else {
            return false;
        };
        if !anchor.is_empty() {
            session.anchor = anchor;
        }
        session.closing = Some(std::time::Instant::now());
        // Moved aside rather than left in `quickview`, so everything that asks
        // "is a preview open" — the key handling, the pointer routing — sees a
        // closed window from this moment, while the panel is still on screen
        // finishing its exit.
        self.quickview_closing = Some(session);
        self.dirty = true;
        true
    }

    /// The panel on screen, whichever direction it is going.
    pub(super) fn quickview_visible(&self) -> Option<&quickview::Session> {
        self.quickview.as_ref().or(self.quickview_closing.as_ref())
    }

    /// Whether a panel still has frames to run — arriving or leaving.
    pub(super) fn quickview_animating(&self) -> bool {
        self.quickview_visible()
            .is_some_and(quickview::Session::animating)
    }

    /// Retire a finished exit. Returns whether anything changed.
    pub(super) fn tick_quickview_exit(&mut self) -> bool {
        let done = self
            .quickview_closing
            .as_ref()
            .is_some_and(|session| !session.animating());
        if done {
            self.quickview_closing = None;
        }
        done
    }
}
