//! The Photos view's state on the browser: its cached layout, the pictures'
//! sizes as they are probed, the tile under the pointer, and the arrow keys.

use super::*;
use crate::photos;
use otto_kit::components::scroll::Direction;

/// Everything the Photos layout is computed from. The layout is rebuilt only
/// when one of these moves: packing rows walks every entry, which is not a
/// cost to pay on every frame of a scroll.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct PhotosKey {
    width: u32,
    depth: usize,
    epoch: u64,
    order: Option<model::SortCacheKey>,
    dims: u64,
    today: i64,
    row_h: u32,
}

impl Browser {
    /// Bring [`Self::photos`] in line with the listing, the width and the
    /// sizes probed so far. Outside the Photos view the layout is dropped.
    pub(super) fn rebuild_photos_layout(&mut self) {
        if self.mode != ViewMode::Photos {
            if self.photos_key.is_some() {
                self.photos = view::PhotosLayout::default();
                self.photos_key = None;
                self.photo_hover = None;
            }
            return;
        }
        let depth = self.columns.len() - 1;
        self.ensure_sorted(depth);
        // The info panel is always up beside the wall, which is packed into
        // what it leaves.
        let panel_w = view::PHOTOS_INFO_W;
        let full = view::content_viewport(self.size.0, self.content_h(), ViewMode::Photos);
        let area = Rect::from_ltrb(full.left, full.top, full.right - panel_w, full.bottom);
        let today = photos::today();
        let key = PhotosKey {
            width: area.width().round() as u32,
            depth,
            epoch: self.columns[depth].epoch,
            order: self.columns[depth].sorted.borrow().key,
            dims: self.photo_dims.epoch(),
            today,
            row_h: self.photos_row_h.round() as u32,
        };
        if self.photos_key.as_ref() == Some(&key) {
            return;
        }
        // Whatever moves the rows — the size, the window — keeps what the
        // user is looking at where it was: the selected tile if it is on
        // screen, the top one otherwise.
        let reflow = self
            .photos_key
            .as_ref()
            .is_some_and(|old| old.width != key.width || old.row_h != key.row_h);
        if reflow && self.photos_anchor.is_none() {
            self.photos_anchor = self.photos_on_screen_anchor(depth);
        }
        let entries = self.visible(depth);
        let sections = photos::sections(&entries, today, self.photos_group);
        let aspects: Vec<f32> = entries.iter().map(|e| self.photo_dims.aspect(e)).collect();
        self.photos = view::PhotosLayout::new(sections, &aspects, area.width(), self.photos_row_h)
            .with_panel(panel_w);
        self.photos_key = Some(key);
        if let Some((index, above)) = self.photos_anchor.take() {
            let tile = self.photos.tile_rect(area, index, 0.0);
            if !tile.is_empty() {
                self.scroll_to_after_layout = Some((tile.top - area.top - above).max(0.0));
            }
        }
        self.dirty = true;
    }

    /// The tile to hold still through a reflow, and how far below the top of
    /// the view it is: the cursor's when it is on screen, else the first
    /// visible one.
    fn photos_on_screen_anchor(&self, depth: usize) -> Option<(usize, f32)> {
        let area = self.photos.area(self.size.0, self.content_h());
        let scroll = self.columns[depth].scroll.offset();
        let on_screen = |index: usize| {
            let tile = self.photos.tile_rect(area, index, scroll);
            (!tile.is_empty() && tile.bottom > area.top && tile.top < area.bottom)
                .then_some((index, tile.top - area.top))
        };
        self.columns[depth]
            .cursor
            .and_then(on_screen)
            .or_else(|| on_screen(self.photos.visible_range(area, scroll, area).start))
    }

    /// The next batch of pictures to measure, in the Photos view. The ones on
    /// screen go first, so the rows the user is looking at settle first.
    pub(super) fn sync_photo_dims(&mut self) -> Vec<photos::ProbeJob> {
        if self.mode != ViewMode::Photos || self.photo_dims.is_busy() {
            return Vec::new();
        }
        let depth = self.columns.len() - 1;
        // Held aside while the listing borrows the browser: the store is only
        // written to below, and nothing else reads it.
        let mut dims = std::mem::take(&mut self.photo_dims);
        let entries = self.visible(depth);
        let area = self.photos.area(self.size.0, self.content_h());
        let scroll = self.columns[depth].scroll.offset();
        let shown = self.photos.visible_range(area, scroll, area);
        let shown = shown.start.min(entries.len())..shown.end.min(entries.len());
        let order = entries[shown.clone()]
            .iter()
            .chain(entries[shown.end..].iter())
            .chain(entries[..shown.start].iter())
            .copied();
        let jobs = dims.wanted(order);
        drop(entries);
        self.photo_dims = dims;
        jobs
    }

    /// The next folders whose cards need their pictures found, in the Photos
    /// view. The folders lead the listing, so these are on screen first.
    pub(super) fn sync_folder_previews(&mut self) -> Vec<photos::FolderJob> {
        if self.mode != ViewMode::Photos || self.folder_previews.is_busy() {
            return Vec::new();
        }
        let depth = self.columns.len() - 1;
        let mut previews = std::mem::take(&mut self.folder_previews);
        let jobs = previews.wanted(self.visible(depth));
        self.folder_previews = previews;
        jobs
    }

    /// A batch of folder cards' pictures landed.
    pub(super) fn finish_folder_previews(&mut self, results: Vec<photos::FolderResult>) {
        self.folder_previews.finish(results);
        self.listing_dirty = true;
    }

    /// A batch of sizes landed: the rows are packed again on the next frame.
    pub(super) fn finish_photo_dims(&mut self, results: Vec<photos::ProbeResult>) {
        self.photo_dims.finish(results);
        self.dirty = true;
    }

    /// An arrow key in the Photos view: to the neighbouring tile the layout
    /// names, or onto the first or last tile when nothing has the cursor.
    pub(super) fn move_photo_cursor(&mut self, direction: Direction, extend: bool) {
        let depth = self.active.min(self.columns.len() - 1);
        let count = self.visible_len(depth);
        if count == 0 {
            return;
        }
        let next = match self.columns[depth].cursor {
            Some(cursor) => match self.photos.neighbor(cursor, direction) {
                Some(next) if next < count => next,
                _ => return,
            },
            None => match direction {
                Direction::Down | Direction::Right => 0,
                Direction::Up | Direction::Left => count - 1,
            },
        };
        self.move_cursor_to(next, extend);
    }

    /// A press on the Photos header controls: the slider takes it and starts
    /// a drag, and the grouping button asks for its menu. `None` when the
    /// press is on neither.
    pub(super) fn photos_controls_press(
        &mut self,
        x: f32,
        y: f32,
        serial: u32,
    ) -> Option<listing_pointer::After> {
        if self.mode == ViewMode::Grid {
            return self.zoom_slider_press(x, y);
        }
        if self.mode != ViewMode::Photos || self.trash || self.picker.is_some() {
            return None;
        }
        let point = skia_safe::Point::new(x, y);
        if self.photos.has_panel() {
            let panel = view::photos_info_rect(self.size.0, self.content_h());
            if panel.contains(point) {
                self.photos_info_press(panel, x, y, serial);
                return Some(listing_pointer::After::Stop);
            }
        }
        if let Some(after) = self.zoom_slider_press(x, y) {
            return Some(after);
        }
        let rect = view::photos_group_rect(self.size.0);
        if rect.contains(skia_safe::Point::new(x, y)) {
            self.photos_group_open = true;
            self.dirty = true;
            return Some(listing_pointer::After::GroupMenu { rect, serial });
        }
        None
    }

    /// The size slider's range and step in this view: the Photos view's row
    /// height, or the icon view's icon size. `None` in the views with no size
    /// to set, and in the picker, the Trash and the desk — which takes its
    /// icon size from its own config.
    pub(super) fn zoom_range(&self) -> Option<(f32, f32, f32)> {
        if self.trash || self.desk || self.picker.is_some() {
            return None;
        }
        match self.mode {
            ViewMode::Photos => Some((
                view::PHOTOS_ROW_MIN,
                view::PHOTOS_ROW_MAX,
                view::PHOTOS_ROW_STEP,
            )),
            ViewMode::Grid => Some((
                view::GRID_ICON_MIN,
                view::GRID_ICON_MAX,
                view::GRID_ICON_STEP,
            )),
            ViewMode::List | ViewMode::Columns => None,
        }
    }

    /// What the size slider is set to in this view.
    pub(super) fn zoom_value(&self) -> f32 {
        match self.mode {
            ViewMode::Grid => self.grid_icon,
            _ => self.photos_row_h,
        }
    }

    /// Set this view's size, the way its slider does.
    pub(super) fn set_zoom(&mut self, value: f32) {
        match self.mode {
            ViewMode::Photos => self.set_photos_row_h(value),
            ViewMode::Grid => self.set_grid_icon(value),
            ViewMode::List | ViewMode::Columns => {}
        }
    }

    /// Ctrl+= and Ctrl+-: a step larger or smaller, remembered at once.
    pub(super) fn step_zoom(&mut self, steps: f32) {
        let Some((_, _, step)) = self.zoom_range() else {
            return;
        };
        self.set_zoom(self.zoom_value() + steps * step);
        self.remember_view_controls();
    }

    /// The slider's track in this view, when the window has room for it.
    fn zoom_track(&self) -> Option<Rect> {
        self.zoom_range()?;
        view::zoom_slider_rect(self.size.0, self.mode == ViewMode::Photos)
    }

    /// A press on the size slider: it takes the press and starts a drag.
    fn zoom_slider_press(&mut self, x: f32, y: f32) -> Option<listing_pointer::After> {
        use otto_kit::components::slider::SliderResponse;

        let track = self.zoom_track()?;
        let (min, max, _) = self.zoom_range()?;
        match self.photos_slider.on_pointer_down(
            track,
            min,
            max,
            Some(1.0),
            self.zoom_value(),
            x,
            y,
        ) {
            SliderResponse::Ignored => None,
            SliderResponse::Changed(value) => {
                self.set_zoom(value);
                Some(listing_pointer::After::Stop)
            }
            SliderResponse::Redraw => {
                self.dirty = true;
                Some(listing_pointer::After::Stop)
            }
        }
    }

    /// The pointer moved while the slider's knob is held. Returns whether the
    /// slider has the pointer.
    pub(super) fn photos_slider_drag(&mut self, x: f32) -> bool {
        use otto_kit::components::slider::SliderResponse;

        if !self.photos_slider.is_dragging() {
            return false;
        }
        let (Some(track), Some((min, max, _))) = (self.zoom_track(), self.zoom_range()) else {
            return true;
        };
        if let SliderResponse::Changed(value) =
            self.photos_slider
                .on_pointer_drag(track, min, max, Some(1.0), self.zoom_value(), x)
        {
            self.set_zoom(value);
        }
        true
    }

    /// Set the icon view's icon size, within the slider's range.
    ///
    /// Like the Photos wall, the icon at the top of the view stays where it
    /// is while every cell around it grows or shrinks.
    pub(super) fn set_grid_icon(&mut self, icon: f32) {
        let icon = icon.clamp(view::GRID_ICON_MIN, view::GRID_ICON_MAX);
        if (icon - self.grid_icon).abs() < 0.5 {
            return;
        }
        let depth = self.columns.len() - 1;
        let area = view::content_viewport(self.size.0, self.content_h(), ViewMode::Grid);
        let scroll = self.columns[depth].scroll.offset();
        let count = self.visible_len(depth);
        let first =
            view::grid_visible_range_in(area, &self.recent_sections, count, scroll, area).start;
        let before = view::grid_cell_rect_in(area, &self.recent_sections, first, scroll);

        self.grid_icon = icon;
        view::set_grid_icon(icon);
        if count > 0 && !before.is_empty() {
            let after = view::grid_cell_rect_in(area, &self.recent_sections, first, 0.0);
            self.scroll_to_after_layout = Some((after.top - before.top).max(0.0));
        }
        self.dirty = true;
    }

    /// The button came up: a slider drag ends, and the size it settled on is
    /// remembered.
    pub(super) fn photos_slider_release(&mut self) {
        if self.photos_slider.on_pointer_up()
            != otto_kit::components::slider::SliderResponse::Ignored
        {
            self.dirty = true;
            self.remember_view_controls();
        }
    }

    /// Set the row height the Photos wall aims at, within the slider's range.
    ///
    /// The picture at the top of the view stays at the top: every row moves
    /// when the size does, and a wall that kept its scroll offset instead
    /// would slide somewhere unrelated under the pointer.
    pub(super) fn set_photos_row_h(&mut self, row_h: f32) {
        let row_h = row_h.clamp(view::PHOTOS_ROW_MIN, view::PHOTOS_ROW_MAX);
        if (row_h - self.photos_row_h).abs() < 0.5 {
            return;
        }
        self.photos_row_h = row_h;
        self.dirty = true;
    }

    /// A pinch on the wall ended: the size it left is remembered.
    pub(super) fn finish_zoom_pinch(&mut self) {
        self.remember_view_controls();
    }

    /// Group the Photos wall another way.
    pub(super) fn set_photos_group(&mut self, group: photos::Grouping) {
        self.photos_group_open = false;
        self.dirty = true;
        if group != self.photos_group {
            self.photos_group = group;
            self.remember_view_controls();
        }
    }

    /// Whether the info panel needs the preview column's decode: one file
    /// is selected in the Photos view.
    pub(super) fn photos_info_wants_preview(&self) -> bool {
        if self.mode != ViewMode::Photos {
            return false;
        }
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        self.columns[depth].selection.len() == 1 && self.selected_entry().is_some_and(|e| !e.is_dir)
    }

    /// What the info panel shows: the one selected file or folder, how many
    /// are selected, or with nothing selected the folder being shown.
    pub(super) fn photos_info_data(&self) -> Option<view::PhotosInfoData<'_>> {
        if self.mode != ViewMode::Photos {
            return None;
        }
        let depth = self.active.min(self.columns.len() - 1);
        let column = &self.columns[depth];
        let entries = self.visible(depth);
        let selected = |e: &&Entry| column.selection.contains(&e.selection_key());
        match column.selection.len() {
            0 => Some(view::PhotosInfoData::Here {
                name: column
                    .path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("/"),
                summary: self.photos_subtitle(depth),
            }),
            1 => {
                let entry = column
                    .cursor
                    .and_then(|i| entries.get(i).copied())
                    .filter(selected)
                    .or_else(|| entries.iter().copied().find(selected))?;
                if entry.is_dir {
                    return Some(view::PhotosInfoData::Folder {
                        entry,
                        items: self.folder_previews.count(entry),
                    });
                }
                let pane = self.preview.as_ref().filter(|p| p.path == entry.path);
                let picture = photos::is_photo(entry);
                Some(view::PhotosInfoData::One {
                    entry,
                    decoded: pane.and_then(|p| p.decoded.as_ref()),
                    dims: picture.then(|| self.photo_dims.size(entry)).flatten(),
                    swatches: pane
                        .filter(|_| picture)
                        .map(|p| p.palette.as_slice())
                        .unwrap_or_default(),
                    copied: self.photos_copied.map(|(i, _)| i),
                    hovered: self.photo_swatch_hover,
                    camera: pane.and_then(|p| p.camera.as_ref()),
                    turnable: picture && crate::orient::supported(&entry.path),
                    tool_hover: self.photo_tool_hover,
                })
            }
            count => Some(view::PhotosInfoData::Many {
                count,
                bytes: entries
                    .iter()
                    .copied()
                    .filter(selected)
                    .filter_map(|e| e.size)
                    .sum(),
            }),
        }
    }

    /// A press inside the info panel: a swatch copies its colour, and a turn
    /// or flip button turns the picture.
    fn photos_info_press(&mut self, panel: Rect, x: f32, y: f32, serial: u32) {
        let tool = self
            .photos_info_data()
            .and_then(|data| view::photos_info_tool_at(panel, &data, x, y));
        if let Some(index) = tool {
            self.turn_selected_photo(view::PHOTOS_TOOLS[index]);
            return;
        }
        let swatch = self
            .photos_info_data()
            .and_then(|data| view::photos_info_swatch_at(panel, &data, x, y));
        if let Some(hex) = swatch.and_then(|index| self.pick_swatch(index)) {
            otto_kit::clipboard::set_text(&hex, serial);
        }
    }

    /// The swatch at `index` was clicked: its colour as the text to copy,
    /// and "Copied" under it for a moment.
    pub(super) fn pick_swatch(&mut self, index: usize) -> Option<String> {
        let colour = self.preview.as_ref()?.palette.get(index).copied()?;
        self.photos_copied = Some((index, std::time::Instant::now()));
        self.dirty = true;
        Some(view::hex_colour(colour))
    }

    /// Let the "Copied" under a swatch go after a moment. Returns whether it
    /// is still showing, so the frame loop keeps running to take it down.
    pub(super) fn tick_photos_copied(&mut self) -> bool {
        const SHOWN: std::time::Duration = std::time::Duration::from_millis(1400);
        match self.photos_copied {
            Some((_, at)) if at.elapsed() >= SHOWN => {
                self.photos_copied = None;
                self.dirty = true;
                false
            }
            Some(_) => true,
            None => false,
        }
    }

    /// Take on the size and grouping remembered from the last run.
    pub(super) fn remember_photos_from(&mut self, remembered: &remembered::Remembered) {
        if let Some(row_h) = remembered.photos.row_height {
            self.photos_row_h = row_h.clamp(view::PHOTOS_ROW_MIN, view::PHOTOS_ROW_MAX);
        }
        if let Some(group) = remembered
            .photos
            .group
            .as_deref()
            .and_then(photos::Grouping::from_id)
        {
            self.photos_group = group;
        }
        if let Some(size) = remembered.icons.size {
            self.grid_icon = size.clamp(view::GRID_ICON_MIN, view::GRID_ICON_MAX);
            // The desk sizes its icons from its own config.
            if !view::is_desk() {
                view::set_grid_icon(self.grid_icon);
            }
        }
    }

    /// Write the Photos size and grouping and the icon size to the state
    /// file — only once the window is remembering on disk at all, which a
    /// test's never is.
    fn remember_view_controls(&self) {
        if !self.palette_memory_on_disk {
            return;
        }
        let mut remembered = remembered::Remembered::load();
        remembered.photos.row_height = Some(self.photos_row_h.round());
        remembered.photos.group = Some(self.photos_group.id().to_string());
        remembered.icons.size = Some(self.grid_icon.round());
        remembered.save();
    }

    /// Follow the pointer over the wall, so the tile under it shows its name.
    pub(super) fn track_photo_hover(&mut self, x: f32, y: f32) {
        if self.mode != ViewMode::Photos {
            return;
        }
        let hovered = self.entry_at(x, y).map(|(_, index)| index);
        if hovered != self.photo_hover {
            self.photo_hover = hovered;
            self.listing_dirty = true;
        }
        // The info panel's swatches light up under the pointer too; only
        // arriving on one or leaving it repaints.
        let swatch = self.photos_swatch_at(x, y);
        if swatch != self.photo_swatch_hover {
            self.photo_swatch_hover = swatch;
            self.dirty = true;
        }
        let tool = self.photos_tool_at(x, y);
        if tool != self.photo_tool_hover {
            self.photo_tool_hover = tool;
            self.dirty = true;
        }
    }

    /// The info panel's turn or flip button under `(x, y)`, if any.
    pub(super) fn photos_tool_at(&self, x: f32, y: f32) -> Option<usize> {
        if self.mode != ViewMode::Photos || !self.photos.has_panel() {
            return None;
        }
        let panel = view::photos_info_rect(self.size.0, self.content_h());
        if !panel.contains(skia_safe::Point::new(x, y)) {
            return None;
        }
        self.photos_info_data()
            .and_then(|data| view::photos_info_tool_at(panel, &data, x, y))
    }

    /// Turn or flip the one selected photograph, by its EXIF orientation:
    /// lossless, and undone with Ctrl+Z like any other change to a file.
    pub(super) fn turn_selected_photo(&mut self, turn: crate::orient::Turn) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        if self.trash || !photos::is_photo(&entry) || !crate::orient::supported(&entry.path) {
            return;
        }
        match crate::orient::apply(&entry.path, turn) {
            Ok((from, to)) => {
                let label = match turn {
                    crate::orient::Turn::Left | crate::orient::Turn::Right => {
                        otto_kit::t!("files-undo-rotate")
                    }
                    _ => otto_kit::t!("files-undo-flip"),
                };
                self.record_undo(
                    label,
                    vec![model::Change::Oriented {
                        path: entry.path.clone(),
                        from,
                        to,
                    }],
                );
                // Decoded again the way up it now goes; the listing re-read
                // brings the new mtime, which re-measures it on the wall and
                // asks for a new thumbnail.
                self.preview = None;
                self.reload_all();
            }
            Err(err) => self.refuse(otto_kit::t_owned!(
                "files-turn-failed",
                name = entry.name.clone(),
                error = err.to_string()
            )),
        }
        self.dirty = true;
    }

    /// The info panel's swatch under `(x, y)`, if any.
    pub(super) fn photos_swatch_at(&self, x: f32, y: f32) -> Option<usize> {
        if self.mode != ViewMode::Photos || !self.photos.has_panel() {
            return None;
        }
        let panel = view::photos_info_rect(self.size.0, self.content_h());
        if !panel.contains(skia_safe::Point::new(x, y)) {
            return None;
        }
        self.photos_info_data()
            .and_then(|data| view::photos_info_swatch_at(panel, &data, x, y))
    }

    /// The Photos header line: "36 images, 2 folders".
    pub(super) fn photos_subtitle(&self, depth: usize) -> String {
        let entries = self.visible(depth);
        let images = entries.iter().filter(|e| photos::is_photo(e)).count();
        let folders = entries.iter().filter(|e| e.is_dir).count();
        let images = otto_kit::t_owned!("files-photos-images", count = images as i64);
        if folders == 0 {
            return images;
        }
        let folders = otto_kit::t_owned!("files-photos-folders", count = folders as i64);
        otto_kit::t_owned!(
            "files-photos-summary",
            images = images.as_str(),
            folders = folders.as_str()
        )
    }
}
