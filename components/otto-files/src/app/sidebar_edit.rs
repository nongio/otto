//! Editing the sidebar from the sidebar: folders dropped between its rows
//! join it, the user's own rows are dragged into a new order, and a
//! right-click on a row takes it out.
//!
//! Sorting is a gesture inside the window, not drag and drop: nothing leaves
//! it, so there is no payload to offer anyone and no compositor to ask.
//!
//! Nothing here keeps a list of its own. Every edit is written to
//! `files.toml` through [`places_config`] and the rows are read back from
//! it, so the file stays the one answer to what the sidebar holds — the one a
//! person editing it by hand, Settings and every other open window all see.

use super::*;

use crate::places_config::{self, Builtin, SidebarConfig};

/// The prefix of the menu ids that show a hidden built-in row again; the
/// row's config id follows it.
pub(super) const SHOW_PREFIX: &str = "sidebar_show_";

/// A press on a sidebar row, and where it has got to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PlaceDrag {
    /// The row pressed.
    pub index: usize,
    /// Where the press landed, to tell a drag from an unsteady click.
    pub start_y: f32,
    /// Where the pointer is now, for the ghost to follow.
    pub y: f32,
    /// Past the drag threshold: the row is being sorted, and the press is no
    /// longer a click.
    pub moving: bool,
}

impl Browser {
    /// A press on sidebar row `index` at height `y`, which may be about to
    /// be dragged to a new place in the list. Any row: the built-in ones are
    /// the user's to arrange too.
    pub(super) fn press_place(&mut self, index: usize, y: f32) {
        self.place_drag = (index < self.places.len()).then_some(PlaceDrag {
            index,
            start_y: y,
            y,
            moving: false,
        });
    }

    /// Follow a sorting drag to height `y`. Whether one is under way, in
    /// which case it owns the pointer.
    pub(super) fn drag_place_to(&mut self, y: f32) -> bool {
        let Some(drag) = self.place_drag.as_mut() else {
            return false;
        };
        if !drag.moving {
            if (y - drag.start_y).abs() < DRAG_THRESHOLD {
                return false;
            }
            drag.moving = true;
        }
        drag.y = y;
        // The ghost and the cursor follow every move; the rows standing aside
        // are stepped by the frame clock, in `tick_place_shifts`.
        self.dirty = true;
        AppContext::set_cursor_shape(CursorShape::Grabbing);
        true
    }

    /// Where among the other rows the row being sorted is over now.
    fn sort_position(&self, drag: &PlaceDrag) -> usize {
        view::place_sort_position(drag.index, drag.y - drag.start_y, self.places.len())
    }

    /// The gap, counted in the rows shown, a sort let go of now would land
    /// in; `None` when that is where the row already is.
    pub(super) fn sort_slot(&self, drag: &PlaceDrag) -> Option<usize> {
        let position = self.sort_position(drag);
        match position.cmp(&drag.index) {
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Less => Some(position),
            // Past its own place, the gap is counted with the row still in.
            std::cmp::Ordering::Greater => Some(position + 1),
        }
    }

    /// Whether sidebar rows are still easing to where a sort wants them, and
    /// so need the frame clock.
    pub(super) fn place_shifts_animating(&self) -> bool {
        self.place_shifts_at.is_some()
    }

    /// Step each row toward where the sort in progress wants it — or back
    /// home once there is none. Whether any is still on its way.
    pub(super) fn tick_place_shifts(&mut self) -> bool {
        // How long a row takes to get most of the way there.
        const EASE: f32 = 0.06;
        let count = self.places.len();
        let targets: Vec<f32> = match self.place_drag.filter(|drag| drag.moving) {
            Some(drag) => {
                let position = self.sort_position(&drag);
                (0..count)
                    .map(|i| view::place_sort_shift(i, drag.index, position))
                    .collect()
            }
            None => vec![0.0; count],
        };
        self.place_shifts.resize(count, 0.0);
        let now = std::time::Instant::now();
        let dt = self
            .place_shifts_at
            .map_or(0.016, |at| now.duration_since(at).as_secs_f32().min(0.1));
        let step = 1.0 - (-dt / EASE).exp();
        let mut moving = false;
        for (shift, target) in self.place_shifts.iter_mut().zip(targets) {
            let gap = target - *shift;
            if gap.abs() < 0.5 {
                if *shift != target {
                    *shift = target;
                    self.dirty = true;
                }
                continue;
            }
            *shift += gap * step;
            self.dirty = true;
            moving = true;
        }
        self.place_shifts_at = moving.then_some(now);
        moving
    }

    /// The button came up. A sorting drag puts its row in the gap it was let
    /// go over; whether there was one, in which case the release was its.
    pub(super) fn release_place_drag(&mut self, y: f32) -> bool {
        let Some(mut drag) = self.place_drag.take() else {
            return false;
        };
        if !drag.moving {
            return false;
        }
        drag.y = y;
        self.dirty = true;
        if let Some(slot) = self.sort_slot(&drag) {
            if let Some(key) = self.places.get(drag.index).map(Place::key) {
                let order = self.order_with(&[key], slot, SidebarConfig::load().order);
                let home = places_config::home_dir();
                self.edit_sidebar(|text| places_config::with_order(text, &order, home.as_deref()));
            }
        }
        true
    }

    /// A sorting drag let go of outside the window: the row stays put, and
    /// the others ease back around it.
    pub(super) fn cancel_place_drag(&mut self) {
        if self.place_drag.take().is_some_and(|drag| drag.moving) {
            self.dirty = true;
        }
    }

    /// The row being sorted and how far it has been carried, for the frame
    /// to draw it faded and its ghost under the pointer.
    pub(super) fn dragging_place_frame(&self) -> Option<view::PlaceDragFrame> {
        self.place_drag
            .filter(|drag| drag.moving)
            .map(|drag| view::PlaceDragFrame {
                index: drag.index,
                dy: drag.y - drag.start_y,
            })
    }

    /// The sidebar's order once `moving` sits at gap `slot` of the rows shown.
    ///
    /// Rows the config's order names but this window does not show — a
    /// hidden one, a folder whose disk is not mounted — keep their place in
    /// it, so showing or mounting them again puts them back where they were.
    pub(super) fn order_with(
        &self,
        moving: &[places_config::PlaceKey],
        slot: usize,
        stored: Vec<places_config::PlaceKey>,
    ) -> Vec<places_config::PlaceKey> {
        let shown: Vec<places_config::PlaceKey> = self.places.iter().map(Place::key).collect();
        let before = shown[..slot.min(shown.len())]
            .iter()
            .filter(|key| moving.contains(key))
            .count();
        let mut order: Vec<places_config::PlaceKey> = shown
            .iter()
            .filter(|key| !moving.contains(key))
            .cloned()
            .collect();
        let at = (slot - before).min(order.len());
        order.splice(at..at, moving.iter().cloned());
        for (index, key) in stored.into_iter().enumerate() {
            if !order.contains(&key) {
                order.insert(index.min(order.len()), key);
            }
        }
        order
    }

    /// The gap a drag at `(x, y)` would add folders at, if it would add any.
    pub(super) fn sidebar_gap_at(&self, x: f32, y: f32) -> Option<usize> {
        if self.trash || self.desk {
            return None;
        }
        let slot = view::place_gap_at(x, y, self.places.len())?;
        // Our own drag says what it carries. Files alone, or folders already
        // built in, would add nothing, and the row's edge is still the row.
        // Someone else's drag only says at the drop, so it is taken on trust.
        if let Some(paths) = otto_kit::dnd::own_files() {
            if !paths.iter().any(|path| self.would_add(path)) {
                return None;
            }
        }
        Some(slot)
    }

    /// Whether adding `path` would change the sidebar: a folder, and not one
    /// of the built-in rows under another name. One of the user's own rows
    /// does change it — dropping it elsewhere moves it.
    fn would_add(&self, path: &Path) -> bool {
        path.is_dir()
            && !self
                .places
                .iter()
                .any(|place| place.builtin.is_some() && place.path == path)
    }

    /// Add the folders among `paths` to the sidebar, as the row at `slot`.
    pub(super) fn add_to_sidebar(&mut self, paths: &[PathBuf], slot: usize) {
        let paths: Vec<PathBuf> = paths
            .iter()
            .filter(|path| self.would_add(path))
            .cloned()
            .collect();
        if paths.is_empty() {
            return;
        }
        // New folders get an entry; ones already listed keep theirs, label
        // and icon included. Where any of them sits is the order's business.
        let new: Vec<PathBuf> = paths
            .iter()
            .filter(|path| !self.places.iter().any(|place| &place.path == *path))
            .cloned()
            .collect();
        let keys: Vec<places_config::PlaceKey> = paths
            .into_iter()
            .map(places_config::PlaceKey::Path)
            .collect();
        let order = self.order_with(&keys, slot, SidebarConfig::load().order);
        let home = places_config::home_dir();
        self.edit_sidebar(|text| {
            let text = places_config::with_places_added(text, &new, None, home.as_deref())?;
            places_config::with_order(&text, &order, home.as_deref())
        });
    }

    /// The selected folders, if adding them to the sidebar would add
    /// anything.
    pub(super) fn selection_for_sidebar(&self) -> Option<Vec<PathBuf>> {
        if self.trash || self.desk {
            return None;
        }
        let entries = self.selected_entries();
        if entries.is_empty() || entries.iter().any(|entry| !entry.is_dir) {
            return None;
        }
        let paths: Vec<PathBuf> = entries.into_iter().map(|entry| entry.path).collect();
        paths
            .iter()
            .any(|path| self.would_add(path))
            .then_some(paths)
    }

    /// Add the selected folders after the last row.
    pub(super) fn add_selection_to_sidebar(&mut self) {
        if let Some(paths) = self.selection_for_sidebar() {
            self.add_to_sidebar(&paths, self.places.len());
        }
    }

    /// Take row `index` out of the sidebar: one of the user's own folders
    /// leaves the config, and a built-in row is hidden.
    pub(super) fn remove_place(&mut self, index: usize) {
        let Some(place) = self.places.get(index).cloned() else {
            return;
        };
        match place.builtin {
            Some(builtin) => {
                self.edit_sidebar(|text| places_config::with_builtin_hidden(text, builtin, true));
            }
            None => {
                let home = places_config::home_dir();
                self.edit_sidebar(|text| {
                    places_config::with_place_removed(text, &place.path, home.as_deref())
                });
            }
        }
    }

    /// The row the last right-click on the sidebar landed on, removed.
    pub(super) fn remove_menu_place(&mut self) {
        if let Some(index) = self.menu_place.take() {
            self.remove_place(index);
        }
    }

    /// Bring back the built-in row the config calls `id`.
    pub(super) fn show_builtin(&mut self, id: &str) {
        if let Some(builtin) = Builtin::from_id(id) {
            self.edit_sidebar(|text| places_config::with_builtin_hidden(text, builtin, false));
        }
    }

    /// The built-in rows hidden now that would be listed if shown — a hidden
    /// Music with no Music folder behind it is nothing to offer back.
    fn hidden_places() -> Vec<(Builtin, Place)> {
        let config = SidebarConfig::load();
        model::places_with(&config.hiding_nothing())
            .into_iter()
            .filter_map(|place| {
                let builtin = place.builtin.filter(|builtin| config.hides(*builtin))?;
                Some((builtin, place))
            })
            .collect()
    }

    /// The menu for a right-click on the sidebar: removing the row under the
    /// pointer, and bringing back whatever built-in rows were hidden — the
    /// one edit with no row left to click.
    pub(super) fn sidebar_menu_items(&mut self, x: f32, y: f32) -> Vec<MenuItem> {
        self.menu_place = view::place_at(x, y, self.places.len());
        let mut items = Vec::new();
        if self.menu_place.is_some() {
            items.push(
                MenuItem::action(otto_kit::t!("files-remove-from-sidebar"))
                    .with_action_id("sidebar_remove"),
            );
        }
        let hidden = Self::hidden_places();
        if !hidden.is_empty() && !items.is_empty() {
            items.push(MenuItem::separator());
        }
        for (builtin, place) in hidden {
            items.push(
                MenuItem::action(otto_kit::t_owned!(
                    "files-sidebar-show",
                    place = place.label
                ))
                .with_action_id(format!("{SHOW_PREFIX}{}", builtin.id())),
            );
        }
        items
    }

    /// Write one edit to the sidebar config, then show what it says now.
    fn edit_sidebar(&mut self, edit: impl FnOnce(&str) -> Result<String, toml_edit::TomlError>) {
        if let Err(err) = places_config::edit_file(edit) {
            tracing::warn!(%err, "cannot update the sidebar config");
            self.status = Some(otto_kit::t_owned!(
                "files-sidebar-save-failed",
                error = err.to_string()
            ));
        }
        self.reload_places();
        self.dirty = true;
    }

    /// Read the sidebar again, keeping the lit row on the folder it led to.
    ///
    /// Called after this window's own edits and whenever `files.toml`
    /// changes, which is how an edit in one window, in Settings or in an
    /// editor reaches every window already open.
    pub(super) fn reload_places(&mut self) {
        if self.trash || self.desk {
            return;
        }
        self.set_places(model::places());
    }

    /// Show `next` as the sidebar.
    pub(super) fn set_places(&mut self, next: Vec<Place>) {
        if next == self.places {
            return;
        }
        let lit = self
            .active_place
            .and_then(|index| self.places.get(index))
            .map(|place| place.path.clone());
        self.active_place = lit.and_then(|path| next.iter().position(|place| place.path == path));
        self.menu_place = None;
        // A row being sorted may not be at its index any more.
        self.place_drag = None;
        // The rows a sort moved aside are already where the new order puts
        // them, so they arrive by standing still.
        self.place_shifts.clear();
        self.place_shifts_at = None;
        self.places = next;
        // A drag still hovering points at a gap or a row that has moved.
        self.drop_target = None;
        self.dirty = true;
    }
}
