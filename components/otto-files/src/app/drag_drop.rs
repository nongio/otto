//! Dragging entries out, and dropping onto the browser.

use super::*;

impl Browser {
    /// Whether this window does drag and drop at all.
    ///
    /// The picker does not: it is a transient serving someone else's request,
    /// and file management belongs to the browser — see
    /// [`specs/file-picker.md`]. Dropping files into the directory it happens
    /// to be showing would be exactly that.
    pub(super) fn dnd_enabled(&self) -> bool {
        self.picker.is_none()
    }

    /// Where a drag at `(x, y)` would put its files, if anywhere.
    ///
    /// Everything resolves to a directory. A hit on a *file* row is not a
    /// target of its own — the files go beside it, into the directory it is
    /// in — which is why a miss falls through to the pane rather than
    /// rejecting the drop.
    pub(super) fn drop_target_at(&self, x: f32, y: f32) -> Option<DropTarget> {
        if !self.dnd_enabled() {
            return None;
        }
        if let Some(index) = view::place_at(x, y, self.places.len()) {
            // Recent is a listing, not a folder. There is nowhere for a drop
            // on it to put anything, so it takes none.
            if self.places[index].recent {
                return None;
            }
            return Some(DropTarget::Place {
                index,
                path: self.places[index].path.clone(),
            });
        }
        // The rest of the sidebar takes nothing: it is chrome, not a place.
        if x < view::sidebar_w() {
            return None;
        }

        if let Some((depth, index)) = self.entry_at(x, y) {
            if let Some(entry) = self.visible(depth).get(index) {
                if entry.is_dir && !Self::is_a_move_home(&entry.path) {
                    return Some(DropTarget::Entry {
                        depth,
                        index,
                        path: entry.path.clone(),
                    });
                }
            }
        }

        let content = view::content_viewport(self.size.0, self.content_h(), self.mode);
        if x < content.left || x >= content.right || y < content.top || y >= content.bottom {
            return None;
        }
        let depth = self.pane_under(x, y);
        let path = self.columns[depth].path.clone();
        if Self::is_a_move_home(&path) {
            return None;
        }
        Some(DropTarget::Pane { depth, path })
    }

    /// Would dropping our own drag on `dest` move the files exactly where they
    /// already are?
    ///
    /// Such a drop has nothing to do, and a target that lights up to say it
    /// will do nothing is worse than no target at all: the answer is a
    /// dismissal — no outline, a cursor that says the drop is refused, and the
    /// icon flying home to where it was picked up.
    ///
    /// A *copy* onto the same directory is a real request — that is how a
    /// duplicate is made — so this only speaks for a move. The test is
    /// "not a copy" rather than "is a move" so it stays put once we refuse:
    /// refusing clears the negotiated action, and an `is_move` test would then
    /// accept again on the next motion and flicker between the two.
    pub(super) fn is_a_move_home(dest: &Path) -> bool {
        use otto_kit::dnd;

        // Someone else's drag: we do not know where those files live, and the
        // source is the one that would have to answer this anyway. Asked first
        // because it reads our own payload, while `selected_action` needs a
        // live `AppContext` there is no reason to require of a target test.
        let Some(paths) = dnd::own_files() else {
            return false;
        };
        if paths.is_empty() || paths.iter().any(|path| path.parent() != Some(dest)) {
            return false;
        }
        !dnd::selected_action().contains(DndAction::Copy)
    }

    /// Put `paths` into the current drop target, moving them or copying them.
    ///
    /// Runs on the UI thread, like [`Browser::paste`], and inherits the same
    /// caveat: fine for a deliberate gesture on a known set of files, and due
    /// to move to the worker pool with the rest of the file operations.
    pub(super) fn apply_drop(&mut self, paths: Vec<PathBuf>, move_them: bool) {
        let Some(target) = self.drop_target.take() else {
            return;
        };
        self.dirty = true;
        let dest = target.path().clone();

        // A drop onto the Trash window is a delete, not a move into a
        // directory that happens to be the trash: pasting there would leave
        // items with no sidecar, which can never be put back. This is the
        // one thing the Trash window accepts being dropped on it, and it is
        // what a bin is for.
        if self.trash {
            let result = model::move_to_trash(&paths);
            self.report(&result);
            self.record_undo(otto_kit::t!("files-undo-delete"), result.changes);
            self.reload_all();
            return;
        }

        // Files dragged back into the directory they already live in: a move
        // there is a no-op, and doing it through `paste` would rename them
        // out of the way of themselves. A *copy* onto the same directory is a
        // real request — that is how a duplicate is made — so it is kept.
        let paths: Vec<PathBuf> = paths
            .into_iter()
            .filter(|path| !move_them || path.parent() != Some(dest.as_path()))
            .collect();
        if paths.is_empty() {
            return;
        }

        // Keep Both for the same reason the paste path does: with no conflict
        // sheet to ask with, the only safe default is the one that cannot
        // destroy anything. A directory dropped into itself is refused by
        // `paste` itself, with a message.
        let result = model::paste(
            &model::Clipboard {
                paths,
                cut: move_them,
            },
            &dest,
            model::OnConflict::KeepBoth,
        );

        let summary = result.summary();
        self.status = (!summary.is_empty()).then_some(summary);
        Self::play_op_sound(&result);
        self.record_undo(
            if move_them {
                otto_kit::t!("files-undo-move")
            } else {
                otto_kit::t!("files-undo-copy")
            },
            result.changes,
        );
        self.reload_all();
    }

    /// What to draw under the cursor: the first selected entry's name and icon
    /// chain, and how many entries are travelling in total.
    ///
    /// The pictures a drag from `(x, y)` carries, laid out where they are on
    /// screen right now — plus the box that holds them, and where the grab sits
    /// inside it.
    ///
    /// The drag image is one surface, not one per file, so the whole spread has
    /// to fit in a single box: its size is the bounding box of the entries at
    /// the moment the drag begins, which is what lets them start where they
    /// were and gather from there. The nearest [`view::DRAG_ITEMS_MAX`] to the
    /// grab are the ones shown; the badge still counts them all.
    pub(super) fn drag_items(&self, x: f32, y: f32) -> Option<DragItems> {
        // The preview column first: it is a picture of one file, and pressing
        // it picks that file up carrying the picture rather than a row the
        // pointer is nowhere near.
        if let Some(stage) = self.preview_grab_at(x, y) {
            let picture = self.preview_drag_picture()?;
            return Some((
                DragPicture::Preview(Box::new(picture)),
                (stage.width(), stage.height()),
                (x - stage.left, y - stage.top),
            ));
        }

        let (depth, grabbed) = self.entry_at(x, y)?;
        let names: Vec<String> = self
            .selected_entries()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        if names.is_empty() {
            return None;
        }

        // The selected rows, nearest the grabbed one first, capped — and then
        // put back in listing order so the pile stacks the way the eye saw them.
        let entries = self.visible(depth);
        let mut chosen: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| names.contains(&entry.name))
            .map(|(index, _)| index)
            .collect();
        chosen.sort_by_key(|index| index.abs_diff(grabbed));
        chosen.truncate(view::DRAG_ITEMS_MAX);
        chosen.sort_unstable();
        let rects: Vec<Rect> = chosen
            .iter()
            .map(|index| self.entry_rect(depth, *index))
            .collect();
        let (image_w, image_h) = view::drag_image_size(self.mode);

        // The box: every picture's start, and the grab point itself, have to be
        // inside it.
        let left = rects.iter().fold(x, |acc, r| acc.min(r.left));
        let top = rects.iter().fold(y, |acc, r| acc.min(r.top));
        let right = rects
            .iter()
            .fold(x, |acc, r| acc.max(r.left + image_w))
            .max(left + image_w);
        let bottom = rects
            .iter()
            .fold(y, |acc, r| acc.max(r.top + image_h))
            .max(top + image_h);

        let items = chosen
            .iter()
            .zip(&rects)
            .filter_map(|(index, rect)| {
                let entry = entries.get(*index)?;
                Some(view::DragItem {
                    entry: (*entry).clone(),
                    thumb: self.thumbs.image(&entry.path, entry.modified).cloned(),
                    start: (rect.left - left, rect.top - top),
                })
            })
            .collect();

        // The badge hangs off the cursor's bottom right, and anything drawn
        // past the surface's edge is simply not there.
        let right = right.max(x + 8.0 + view::drag_badge_width(names.len()));
        let bottom = bottom.max(y + 30.0);

        Some((
            DragPicture::Entries(items),
            (right - left, bottom - top),
            (x - left, y - top),
        ))
    }

    /// The paths a drag started now would carry: the whole selection, so
    /// dragging one of several selected files takes all of them.
    pub(super) fn drag_paths(&self) -> Vec<PathBuf> {
        self.selected_entries()
            .into_iter()
            .map(|entry| entry.path)
            .collect()
    }
}
