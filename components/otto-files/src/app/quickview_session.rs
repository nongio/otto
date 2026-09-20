//! Quick View: opening, following, zooming, panning and closing the panel.

use super::*;
use otto_kit::preview::Word;

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

    /// Turn a paginated preview by `delta` pages, in place.
    ///
    /// Returns what the caller must decode — the same file, at another page —
    /// or `None` when the open preview has no pages to turn, which is the
    /// signal to let the keystroke go on meaning what it means everywhere
    /// else. Unlike [`Browser::begin_quickview`] the panel is *not* emptied
    /// while the decode runs: the page on screen is the right file and a good
    /// answer to the keystroke until the next one lands, where the waiting
    /// line would only be a flash of nothing.
    pub(super) fn turn_quickview_page(&mut self, delta: i32) -> Option<(PathBuf, u64, u32, Rect)> {
        let page = self.quickview.as_ref()?.page_turn(delta)?;
        let path = self.selected_entry()?.path;
        self.quickview_generation += 1;
        self.quickview_pending = true;
        self.dirty = true;
        Some((
            path,
            self.quickview_generation,
            page,
            self.quickview_anchor(),
        ))
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

    /// Attach words recognised after the picture was shown, unless the user
    /// has moved on since. The picture on screen is the same decode the words
    /// were made on, so they land on it directly.
    pub(super) fn finish_quickview_words(&mut self, generation: u64, words: Vec<Word>) {
        if generation != self.quickview_generation {
            return;
        }
        self.quickview_recognising = false;
        if let Some(session) = self.quickview.as_mut() {
            tracing::debug!(words = words.len(), name = %session.name, "recognised");
            session.attach_words(words);
            self.dirty = true;
        }
    }

    /// The recogniser finished without words to attach. The badge goes away
    /// with it: there is nothing on this picture to select, and a badge that
    /// stayed would promise otherwise.
    pub(super) fn finish_quickview_recognising(&mut self, generation: u64) {
        if generation == self.quickview_generation {
            self.quickview_recognising = false;
            if let Some(session) = self.quickview.as_mut() {
                tracing::debug!(name = %session.name, "no text recognised");
                session.stop_recognising();
                self.dirty = true;
            }
        }
    }

    /// The recogniser has started on the picture that is up, so the panel can
    /// say so.
    pub(super) fn start_quickview_recognising(&mut self, generation: u64) {
        if generation != self.quickview_generation {
            return;
        }
        self.quickview_recognising = true;
        if let Some(session) = self.quickview.as_mut() {
            tracing::debug!(name = %session.name, "recognising");
            session.start_recognising(std::time::Instant::now());
            self.dirty = true;
        }
    }

    /// Copy the words selected on the previewed picture. Returns whether
    /// there were any — when there were not, the shortcut goes on meaning
    /// what it means to the listing.
    pub(super) fn copy_quickview_selection(&mut self, serial: u32) -> bool {
        let Some(text) = self
            .quickview
            .as_ref()
            .and_then(quickview::Session::selected_text)
        else {
            return false;
        };
        clipboard::set_text(&text, serial);
        true
    }

    /// Select every recognised word on the previewed picture. Returns whether
    /// there were any.
    pub(super) fn select_all_quickview_words(&mut self) -> bool {
        let Some(session) = self.quickview.as_mut() else {
            return false;
        };
        let selected = session.select_all_words();
        self.dirty |= selected;
        selected
    }

    /// Drop the word selection on the previewed picture. Returns whether
    /// there was one — Escape takes that turn before it closes the panel.
    pub(super) fn clear_quickview_selection(&mut self) -> bool {
        let Some(session) = self.quickview.as_mut() else {
            return false;
        };
        let cleared = session.clear_selection();
        self.dirty |= cleared;
        cleared
    }

    /// Whether the pointer at `point` is over a recognised word, so the
    /// cursor can say the picture has text to select.
    pub(super) fn quickview_over_word(&self, point: skia_safe::Point, panel: Rect) -> bool {
        let content = view::quickview_content_rect(panel);
        self.quickview
            .as_ref()
            .is_some_and(|session| session.word_at(point.x, point.y, content).is_some())
    }

    /// Show the text beam over a word and the arrow elsewhere, changing the
    /// shape only when the pointer crosses between the two.
    pub(super) fn sync_quickview_cursor(&mut self, point: skia_safe::Point, panel: Rect) {
        let over_word = self.quickview_over_word(point, panel);
        if over_word != self.quickview_text_cursor {
            self.quickview_text_cursor = over_word;
            AppContext::set_cursor_shape(if over_word {
                CursorShape::Text
            } else {
                CursorShape::Default
            });
        }
    }

    /// The pointer left the panel: back to the arrow if it was the beam.
    pub(super) fn reset_quickview_cursor(&mut self) {
        if self.quickview_text_cursor {
            self.quickview_text_cursor = false;
            AppContext::set_cursor_shape(CursorShape::Default);
        }
    }

    /// Expand the open panel to fill its display, or bring it back.
    pub(super) fn toggle_quickview_expand(&mut self) {
        if let Some(session) = self.quickview.as_mut() {
            session.toggle_expanded();
            // Back to the middle, both ways. An expanded panel takes nearly
            // the whole display, so a card that had been dragged aside would
            // have nowhere to be dragged aside *to* — and coming back out of
            // expanded to a remembered corner reads as the button having
            // moved the panel rather than resized it.
            session.offset = (0.0, 0.0);
            self.dirty = true;
        }
    }

    /// Where the panel rests when the surface layer has not said: centred in
    /// the window, at whichever size the session asks for.
    pub(super) fn quickview_fallback_panel(&self) -> Rect {
        let Some(session) = self.quickview_visible() else {
            return quickview::resting_in(Rect::from_wh(self.size.0, self.size.1), false);
        };
        quickview::resting_in(Rect::from_wh(self.size.0, self.size.1), session.expanded)
            .with_offset(session.offset)
    }

    /// Take hold of the panel by its title strip. Returns whether the press
    /// started a drag, so the caller can stop looking for other meanings.
    ///
    /// `panel` is the card in whichever space the press arrived in — the
    /// panel's own surface, or the window — and the grab is stored inside the
    /// card, which means the same thing in both.
    /// A second press in the same strip inside the double-click window fills
    /// the display instead, which is what the expand button does — the strip
    /// is the panel's titlebar, and a titlebar answers a double-click that
    /// way.
    pub(super) fn quickview_grip(&mut self, point: skia_safe::Point, panel: Rect) -> bool {
        if self.quickview.is_none() || !view::quickview_grip_rect(panel).contains(point) {
            return false;
        }
        let now = std::time::Instant::now();
        let doubled = self
            .last_quickview_title_click
            .is_some_and(|at| now.duration_since(at) < DOUBLE_CLICK_WINDOW);
        if doubled {
            // Cleared, so a third press starts a fresh pair rather than
            // toggling again on the way past.
            self.last_quickview_title_click = None;
            self.toggle_quickview_expand();
            return true;
        }
        self.last_quickview_title_click = Some(now);
        // The press point as reported, and where the card was in the window
        // when it was reported. Everything the drag needs is fixed at this
        // moment; see [`Browser::drag_quickview_to`].
        let card = self
            .quickview_panel
            .unwrap_or_else(|| self.quickview_fallback_panel());
        self.quickview_drag = Some(((point.x, point.y), (card.left, card.top)));
        true
    }

    /// Follow the pointer, in the frame the press was reported in.
    ///
    /// A pointer holding a button is in a *grab*, and a grab keeps the focus
    /// it was taken with — surface and position both. Coordinates therefore
    /// go on being measured against wherever the panel's surface was when the
    /// press landed, however far the card has been moved since, and the frame
    /// the pointer is reported in stays still for the length of the drag.
    ///
    /// So the drag needs nothing from the render path: where the pointer has
    /// travelled since the press, added to where the card was at the press,
    /// is where the card should be now. Reading a rect back per frame and
    /// moving relative to *that* is what makes a drag run away — the pointer
    /// reports far more often than the window paints, and the card's own
    /// movement feeds back into the next measurement.
    pub(super) fn drag_quickview_to(&mut self, point: skia_safe::Point) {
        let Some(((grab_x, grab_y), (origin_x, origin_y))) = self.quickview_drag else {
            return;
        };
        let (resting, _) = self.quickview_placement();
        let wanted = (origin_x + point.x - grab_x, origin_y + point.y - grab_y);
        self.place_quickview(resting, wanted);
    }

    /// Keep a dragged panel where it can still be reached. The window can be
    /// resized, and the display answered for, after the card has been put
    /// somewhere that was on screen at the time and is not any more.
    pub(super) fn clamp_quickview(&mut self) {
        let Some(offset) = self
            .quickview
            .as_ref()
            .map(|session| session.offset)
            .filter(|offset| *offset != (0.0, 0.0))
        else {
            return;
        };
        // Where the panel is *asking* to be, which mid-drag is not where it
        // was last placed: the clamp trims what the drag wants, it does not
        // undo it.
        let (resting, _) = self.quickview_placement();
        let wanted = (resting.left + offset.0, resting.top + offset.1);
        self.place_quickview(resting, wanted);
    }

    /// Where the card would rest with no drag applied, in window points, and
    /// the offset it was last placed with.
    fn quickview_placement(&self) -> (Rect, (f32, f32)) {
        let offset = self
            .quickview_placed_offset
            .or_else(|| self.quickview.as_ref().map(|session| session.offset))
            .unwrap_or((0.0, 0.0));
        let placed = self
            .quickview_panel
            .unwrap_or_else(|| self.quickview_fallback_panel());
        (placed.with_offset((-offset.0, -offset.1)), offset)
    }

    /// Put the card's top-left at `wanted`, as far as the display allows.
    /// `resting` is where it would be with no drag applied, so the offset is
    /// the difference between the two.
    fn place_quickview(&mut self, resting: Rect, wanted: (f32, f32)) {
        let bounds = self.quickview_bounds();
        let left = wanted.0.clamp(
            bounds.left,
            (bounds.right - resting.width()).max(bounds.left),
        );
        // Never above the top edge, and never so far down that the title
        // strip — the only thing that can bring it back — has gone off the
        // bottom.
        let top = wanted.1.clamp(
            bounds.top,
            (bounds.bottom - quickview::TITLEBAR_H).max(bounds.top),
        );
        let offset = (left - resting.left, top - resting.top);
        let Some(session) = self.quickview.as_mut() else {
            return;
        };
        if session.offset == offset {
            return;
        }
        session.offset = offset;
        self.dirty = true;
    }

    /// The panel has been let go of. Returns whether it was being dragged.
    pub(super) fn end_quickview_drag(&mut self) -> bool {
        self.quickview_drag.take().is_some()
    }

    /// Whether the panel is being dragged by its title strip.
    pub(super) fn quickview_dragging(&self) -> bool {
        self.quickview_drag.is_some()
    }

    /// Where the panel may be dragged: the display when the compositor has
    /// said where it is, and the window until it has.
    fn quickview_bounds(&self) -> Rect {
        self.quickview_display
            .unwrap_or_else(|| Rect::from_wh(self.size.0, self.size.1))
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
                // Not on a bar: a press on a word starts selecting, and a
                // press anywhere else on the picture clears a selection.
                let selected = !hit && session.select_pointer_down(point.x, point.y, content);
                (hit || session.selecting, hit || selected)
            }
            QuickviewPointer::Motion => {
                let panned = session.pan_pointer_move(point.x, point.y, content);
                let selected = session.select_pointer_move(point.x, point.y, content);
                (false, panned || selected)
            }
            QuickviewPointer::Release => {
                session.pan_pointer_up();
                session.select_pointer_up();
                (false, false)
            }
            QuickviewPointer::Leave => {
                session.pan_pointer_up();
                session.pan_pointer_leave();
                // A drag that leaves the panel keeps its selection: what was
                // reached is kept, and the button coming up outside ends it.
                session.select_pointer_up();
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
        self.quickview_recognising = false;
        // A panel dismissed mid-drag is no longer being dragged; the offset
        // itself stays, so the exit flies home from where the card actually
        // is rather than from where it would have rested.
        self.quickview_drag = None;
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
