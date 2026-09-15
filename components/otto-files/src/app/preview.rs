//! The docked preview column, and the thumbnails the listings ask for.

use super::*;

impl Browser {
    /// Whether the preview pane has something to show: exactly one *file*
    /// selected in the active column, in Miller view — a folder is where you
    /// already are one click away from browsing, so previewing it as if it
    /// were a document's content does not earn the pane, and List/Grid show
    /// one directory at a time with nothing for a trailing pane to sit beside.
    ///
    /// No width check: the pane is a member of the horizontally-panned Miller
    /// stack (see [`Self::preview_width`], [`Self::reveal_preview`]), not
    /// something carved out of the listing's own space, so there is nothing
    /// for a narrow window to run short of — it is simply off-screen until
    /// panned into view, the same as any column would be.
    pub(super) fn preview_visible(&self) -> bool {
        if self.mode != ViewMode::Columns {
            return false;
        }
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let single_selected = self
            .columns
            .get(depth)
            .is_some_and(|c| c.selection.len() == 1);
        single_selected && self.selected_entry().is_some_and(|e| !e.is_dir)
    }

    pub(super) fn preview_width(&self) -> f32 {
        if self.preview_visible() {
            view::PREVIEW_W
        } else {
            0.0
        }
    }

    /// Bring the preview pane's state in line with the current selection.
    /// Returns the path and generation to decode when the target changed and
    /// nothing is already in flight for it — the caller runs the decode off
    /// the UI thread and reports back through [`Self::finish_preview`].
    pub(super) fn sync_preview_target(&mut self) -> Option<(PathBuf, u64)> {
        if !self.preview_visible() {
            if self.preview.take().is_some() {
                self.dirty = true;
            }
            return None;
        }
        let entry = self.selected_entry()?;
        if let Some(pane) = &self.preview {
            if pane.path == entry.path {
                return None; // Already showing (or decoding) this one.
            }
        }
        self.preview_generation_seed += 1;
        let generation = self.preview_generation_seed;
        self.preview = Some(PreviewPaneState {
            path: entry.path.clone(),
            generation,
            pending: true,
            decoded: None,
            video: None,
        });
        self.dirty = true;
        Some((entry.path, generation))
    }

    /// The cursor for a pointer resting at `(x, y)` with no button down.
    ///
    /// The order is the press handler's order, and it has to stay that way:
    /// a cursor that promises one thing while the click does another is worse
    /// than no cursor at all. The window's own border is resolved first —
    /// `resize::edge_at` is the first thing a press asks — and only what falls
    /// outside it can be a column divider.
    ///
    /// That distinction is not academic. The last Miller pane's right edge sits
    /// exactly on the window's right border whenever the stack is panned fully
    /// over, which is most of the time once a preview column is up, so the two
    /// bands overlap on every window.
    pub(super) fn hover_shape(&self, x: f32, y: f32) -> CursorShape {
        if let Some(edge) = resize::edge_at(Rect::from_wh(self.size.0, self.size.1), x, y) {
            return edge.cursor();
        }
        let (width, height) = (self.size.0, self.content_h());
        let over_divider = match self.mode {
            ViewMode::List => view::column_boundary_at(x, y, width, self.list_columns).is_some(),
            ViewMode::Columns => view::miller_boundary_at(
                x,
                y,
                width,
                height,
                self.pan.offset(),
                self.columns.len(),
                self.miller_w,
            )
            .is_some(),
            ViewMode::Grid => false,
        };
        if over_divider {
            CursorShape::ColResize
        } else {
            CursorShape::Default
        }
    }

    /// The preview column's picture, where it is on screen right now — if
    /// there is one, and `(x, y)` is on it.
    ///
    /// The caption below the picture is deliberately not part of the target:
    /// it is a label, and a label is a thing to read rather than a handle to
    /// pick the file up by.
    pub(super) fn preview_grab_at(&self, x: f32, y: f32) -> Option<Rect> {
        if !self.preview_visible() {
            return None;
        }
        let (width, height) = (self.size.0, self.content_h());
        let panel =
            view::preview_pane_rect(self.columns.len(), height, self.pan.offset(), self.miller_w);
        let lines = preview_info(&self.selected_entry()?).len();
        let stage = view::preview_stage_rect(panel, lines);
        // Clipped to the file area: the stack is panned, so a preview column
        // half off the left of the window must not be grabbable under the
        // sidebar.
        let mut visible = stage;
        if !visible.intersect(view::content_viewport(width, height, ViewMode::Columns)) {
            return None;
        }
        visible
            .contains(skia_safe::Point::new(x, y))
            // The picture may start off the left edge; the drag image is the
            // whole picture, so the anchor is measured from the *stage*.
            .then_some(stage)
    }

    /// The drag image for a file picked up by its preview: the same picture
    /// the column is showing.
    pub(super) fn preview_drag_picture(
        &self,
    ) -> Option<impl Fn(&skia_safe::Canvas, f32, f32) + Send + Sync + 'static> {
        let entry = self.selected_entry()?;
        let data = view::PreviewData {
            name: entry.name.as_str(),
            icon_chain: entry.icon_chain(),
            decoded: self.preview.as_ref().and_then(|p| p.decoded.as_ref()),
            video: None,
            video_on_surface: false,
            first_row: 0,
            info: preview_info(&entry),
        };
        Some(view::preview_drag_picture(
            &data,
            AppContext::current_theme(),
        ))
    }

    /// Pan the stack so the preview pane — sitting right after the last real
    /// column, the same trailing position a freshly opened directory column
    /// would occupy — is fully in view.
    ///
    /// **Not** called when the preview's target changes. Arrowing down a
    /// listing changes it on every keystroke, and a stack that panned each time
    /// would slide out from under the column being read: the preview is a thing
    /// offered at the edge of the view, not a place the browser goes. Kept for
    /// a caller that means to go there deliberately.
    #[cfg(test)]
    pub(super) fn reveal_preview(&mut self) {
        if !self.preview_visible() {
            return;
        }
        self.sync_scroll_metrics();
        let target = view::preview_pan_for(
            self.columns.len(),
            self.size.0,
            self.pan.offset(),
            self.miller_w,
        );
        if self.pan.scroll_to(target) {
            self.dirty = true;
        }
    }

    /// Ask the thumbnail store what the visible entries still need.
    ///
    /// Called once per update, the same place the preview column's target is
    /// synced. Returns the jobs the host is to run off the UI thread; an empty
    /// vector — the usual answer — means everything on screen is already
    /// settled.
    ///
    /// Only the pane the user is looking at is considered. In Miller view the
    /// parent columns are 16-pixel rows of icons where a thumbnail buys almost
    /// nothing, and fetching for every column at once would spend the whole
    /// in-flight budget on panes the eye is not on.
    pub(super) fn sync_thumbnails(&mut self) -> Vec<thumbnails::Job> {
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let entries = self.visible(depth);
        if entries.is_empty() {
            return Vec::new();
        }

        let area = view::content_viewport(self.size.0, self.content_h(), self.mode);
        let scroll = self.columns[depth].scroll.offset();
        let range = match self.mode {
            view::ViewMode::Grid => view::grid_visible_range_in(
                area,
                &self.recent_sections,
                entries.len(),
                scroll,
                area,
            ),
            view::ViewMode::List => {
                view::RowStrip::list(self.size.0, entries.len(), scroll).visible(area)
            }
            // A Miller pane's rows sit a little way down the pane and are
            // panned sideways with the stack, so its own strip and its own
            // viewport are what describe them — a list's strip would be off by
            // the inset and would not know the pane can be panned off screen
            // entirely.
            view::ViewMode::Columns => {
                let full = view::miller_pane_rect(
                    depth,
                    self.content_h(),
                    self.pan.offset(),
                    self.miller_w,
                );
                let band = view::pane_viewport(
                    self.size.0,
                    self.content_h(),
                    view::ViewMode::Columns,
                    depth,
                    self.pan.offset(),
                    self.miller_w,
                );
                view::RowStrip::miller(full, entries.len(), scroll).visible(band)
            }
        };

        // The box a thumbnail will be drawn in decides how much detail to ask
        // for: a grid cell is worth a real picture, a list row is 16 points of
        // it.
        let box_edge = match self.mode {
            view::ViewMode::Grid => view::GRID_ICON,
            view::ViewMode::List | view::ViewMode::Columns => view::ICON_SIZE,
        };
        let scale = AppContext::scale_factor().max(1) as f32;
        let size = thumbcache::Size::for_box(box_edge, scale);

        let requests = entries
            .get(range.clone())
            .unwrap_or_default()
            .iter()
            // A directory has no picture of its own, and asking for one means
            // a sandboxed worker per folder in a folder of folders.
            .filter(|entry| !entry.is_dir)
            .map(|entry| thumbnails::Request {
                path: entry.path.clone(),
                modified: entry.modified,
                may_generate: entry.kind.thumbnailable(),
            })
            .collect::<Vec<_>>();
        self.thumbs.wanted(requests, size)
    }

    /// Show a preview decode that arrived, unless the selection has moved on
    /// since — the same staleness guard Quick View uses.
    pub(super) fn finish_preview(
        &mut self,
        generation: u64,
        preview: otto_kit::preview::Preview,
        video: Option<otto_media_kit::Options>,
    ) {
        let Some(pane) = &mut self.preview else {
            return;
        };
        if pane.generation != generation {
            return;
        }
        pane.pending = false;
        pane.video = video.and_then(|options| {
            quickview::Video::open(&preview, &pane.path, options, AppContext::request_wakeup)
        });
        pane.decoded = Some(preview);
        self.dirty = true;
    }

    /// The pointer over the preview column's video, if there is one and it
    /// wants the event. The stage is where the picture is; the caption
    /// under it is the listing's business.
    pub(super) fn preview_video_pointer(
        &mut self,
        kind: quickview::VideoPointer,
        x: f32,
        y: f32,
    ) -> bool {
        if !self.preview_visible() {
            return false;
        }
        let Some(entry) = self.selected_entry() else {
            return false;
        };
        let pane = view::preview_pane_rect(
            self.columns.len(),
            self.content_h(),
            self.pan.offset(),
            self.miller_w,
        );
        let stage = view::preview_stage_rect(pane, preview_info(&entry).len());
        let Some(video) = self.preview.as_mut().and_then(|p| p.video.as_mut()) else {
            return false;
        };
        // The controls are drawn in the aspect box, not the whole stage, so
        // the hit test has to measure against the same rect — otherwise the
        // play button answers for a band of empty column above it.
        let content = view::preview_video_box(stage, video.snapshot().aspect());
        let handled = video.pointer(kind, x, y, content);
        self.dirty |= handled;
        handled
    }
}
