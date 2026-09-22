//! Scroll metrics and stepping, for the panes and the column stack.

use super::*;

impl Browser {
    /// Give every pane's scroll view its viewport and content height.
    ///
    /// Re-measured rather than cached, and before both drawing and scrolling:
    /// the viewport moves with the window and the pan, and the content height
    /// changes whenever a directory finishes loading, is re-sorted, or has
    /// hidden files toggled. A scroll view with stale metrics clamps to the
    /// wrong end.
    pub(super) fn sync_scroll_metrics(&mut self) {
        self.rebuild_recent_sections();
        let (width, height) = (self.size.0, self.content_h());
        let mode = self.mode;
        let miller_w = self.miller_w;
        let depth_count = self.columns.len();
        let counts = self.counts();

        // The stack's own metrics first: the panes are laid out from its
        // offset, so a stale pan would place every pane viewport wrong. The
        // preview pane, when showing, is one more thing the stack must have
        // room to pan to — it is folded into the same content length as the
        // real columns rather than carved out of the viewport.
        self.pan
            .set_viewport(view::content_viewport(width, height, mode));
        self.pan.set_content_length(view::miller_content_width(
            depth_count,
            miller_w,
            self.preview_width(),
        ));
        let pan = self.pan.offset();

        for (depth, &count) in counts.iter().enumerate() {
            let viewport = view::pane_viewport(width, height, mode, depth, pan, miller_w);
            // The day headings are part of the content: measured without them
            // the grid is short by their height and the last row is unreachable.
            let content =
                view::pane_content_height_in(width, height, mode, count, &self.recent_sections);
            // A column being re-read has no entries *yet*, and telling its
            // scroll view how long *that* is would clamp the offset to the top
            // — permanently, since the offset is not restored when the listing
            // lands. Anything that reloads in place (a delete, a paste, a drop,
            // a rename) would scroll the pane away from what the user was
            // looking at. The carried length stands until the real one is
            // known.
            //
            // Gated on the read being in flight, not on the measurement coming
            // out zero: an empty listing does not measure as zero in every
            // view. A grid counts its padding and a Miller pane its row inset
            // whether or not there are rows, so those two clamped to the top
            // anyway. Only List happens to measure an empty pane as nothing.
            let loading = self.columns[depth].loading();
            let scroll = &mut self.columns[depth].scroll;
            scroll.state.set_viewport(viewport);
            if !loading {
                scroll.set_content_length(content);
            }
        }
    }

    /// Pan the stack the shortest distance that brings pane `depth` fully into
    /// view — what every navigation into a new column does.
    ///
    /// A no-op outside Miller view, where there is one pane and nothing to
    /// pan. The metrics are re-synced first because the target is clamped to
    /// the stack's content width, which grows and shrinks with the path.
    pub(super) fn reveal_pane(&mut self, depth: usize) {
        if self.mode != ViewMode::Columns {
            return;
        }
        self.sync_scroll_metrics();
        let target = view::miller_pan_for(depth, self.size.0, self.pan.offset(), self.miller_w);
        if self.pan.scroll_to(target) {
            self.dirty = true;
        }
    }

    /// Which pane the pointer is over — the one the wheel and the scrollbar
    /// belong to. Only Miller view has more than one.
    pub(super) fn pane_under(&self, x: f32, y: f32) -> usize {
        if self.mode != ViewMode::Columns {
            return self.columns.len() - 1;
        }
        let counts = self.counts();
        view::miller_at(
            x,
            y,
            self.size.0,
            self.content_h(),
            &self.columns,
            &counts,
            self.pan.offset(),
            self.miller_w,
        )
        .map(|(depth, _)| depth)
        .unwrap_or(self.active)
    }

    /// Whether any pane still has motion to run — momentum, an overscroll
    /// bounce, or a scrollbar fading out.
    pub(super) fn scroll_animating(&self) -> bool {
        self.pan.is_animating()
            || self.columns.iter().any(|c| c.scroll.is_animating())
            || self.palette_scroll.is_animating()
            || self.open_with_scrolling()
            || self.peek_pan_animating()
    }

    /// Advance every pane's scrolling by one tick. Returns whether anything
    /// moved and therefore needs a repaint.
    pub(super) fn tick_scroll(&mut self) -> bool {
        let mut moved = false;
        if self.pan.is_animating() {
            moved |= self.pan.tick();
        }
        for column in &mut self.columns {
            if column.scroll.is_animating() {
                moved |= column.scroll.tick();
            }
        }
        // The palette's list glides on its own tick: see
        // `tick_palette_scroll`. The open preview's picture pans on scroll views of its own, and
        // they fling and spring like any other.
        moved |= self.tick_peek_pan();
        moved
    }
}
