//! The desk's free placement: each icon on a slot of its own, moved by
//! dragging it within the desk — see `specs/desk.md`, *Arrangement*.
//!
//! The slots come from [`desk_slots::arrange`] once per change of the
//! listing, the grid or the saved cells, and reach every hit test and
//! drawing through [`view::GridSlots`] on the grid's sections. The listing
//! itself is put in slot order, so the keyboard's Left and Right read the
//! desk the way the eye does and the overflow tile's items stay the run at
//! the end of it.

use std::collections::{HashMap, HashSet};

use super::*;
use crate::desk::{Arrange, Overflow};
use crate::desk_slots::{self, Cell};

/// What an arrangement was worked out from: when none of it has moved, the
/// slots are still right.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ArrangeKey {
    epoch: u64,
    sort: SortKey,
    ascending: bool,
    show_hidden: bool,
    columns: usize,
    rows: Option<usize>,
    places: u64,
}

/// A drag that started on the desk: which icon was picked up, and where
/// inside its cell the pointer held it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct DeskDrag {
    grabbed: PathBuf,
    grab: (f32, f32),
}

/// The desk's free placement as the browser keeps it.
#[derive(Debug, Default)]
pub(super) struct DeskPlacement {
    /// The saved cells of [`Self::folder`], read from the state file the
    /// first time free placement is on in it.
    pub(super) places: Option<desk_slots::Places>,
    /// The folder [`Self::places`] belong to: the desk's folder when they
    /// were read.
    pub(super) folder: Option<PathBuf>,
    /// Bumped whenever [`Self::places`] changes.
    places_epoch: u64,
    /// Each icon's slot, by name: the order the listing is put in.
    pub(super) rank: HashMap<String, usize>,
    /// Bumped whenever [`Self::rank`] changes, so the sorted listing knows
    /// to sort again.
    pub(super) rank_epoch: u64,
    /// What the slots on the grid were worked out from.
    arranged: Option<ArrangeKey>,
    /// The drag in progress that started on the desk.
    drag: Option<DeskDrag>,
}

impl Browser {
    /// Whether this is the desk with its icons placed by hand.
    pub(super) fn desk_free(&self) -> bool {
        self.desk
            && self.mode == ViewMode::Grid
            && self
                .desk_config
                .as_ref()
                .is_some_and(|config| config.arrange == Arrange::Free)
    }

    /// The desk's grid in slots: how many across, and how many down when
    /// the panel stacks rather than scrolls.
    fn desk_slot_grid(&self) -> (usize, Option<usize>) {
        let area = view::content_viewport(self.size.0, self.content_h(), ViewMode::Grid);
        let columns = view::grid_columns(area);
        let stacks = self
            .desk_config
            .as_ref()
            .is_some_and(|config| config.overflow == Overflow::Stack);
        (columns, stacks.then(|| view::grid_capacity(area) / columns))
    }

    /// Bring the slots on the grid in line with the listing, the panel and
    /// the saved cells. Cheap when none of them moved, so it runs every
    /// frame.
    pub(super) fn arrange_desk(&mut self) {
        if !self.desk_free() {
            if self.recent_sections.slots.is_some() || !self.desk_placement.rank.is_empty() {
                self.recent_sections.slots = None;
                self.desk_placement.rank.clear();
                self.desk_placement.rank_epoch += 1;
                self.desk_placement.arranged = None;
                self.dirty = true;
            }
            return;
        }
        // Nothing to place until the folder has been read: an empty first
        // frame would forget nothing, but would save nothing useful either.
        if self.columns[0].awaiting_first_listing() {
            return;
        }
        self.desk_places();
        let (columns, rows) = self.desk_slot_grid();
        let key = ArrangeKey {
            epoch: self.columns[0].epoch,
            sort: self.sort,
            ascending: self.ascending,
            show_hidden: self.show_hidden,
            columns,
            rows,
            places: self.desk_placement.places_epoch,
        };
        if self.desk_placement.arranged == Some(key) {
            return;
        }
        let positions = self
            .desk_placement
            .places
            .as_ref()
            .map(|places| places.positions.clone())
            .unwrap_or_default();

        let names: Vec<String> = self
            .visible(0)
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let arrangement = desk_slots::arrange(&refs, &positions, columns, rows);

        // New files keep the slot they were given, and files that went away
        // give theirs up. A folder that could not be read, or has gone, lists
        // nothing, and that is not every file going away. A hidden file is
        // still there, so it keeps its slot for when hidden files show.
        let column = &self.columns[0];
        if column.snapshot.error.is_none() && !column.gone {
            let present: HashSet<&str> = column
                .snapshot
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect();
            let mut next = positions;
            next.retain(|name, _| present.contains(name.as_str()));
            next.extend(arrangement.placed.iter().cloned());
            self.save_desk_positions(next);
        }

        if arrangement.slots != self.desk_placement.rank {
            self.desk_placement.rank = arrangement.slots.clone();
            self.desk_placement.rank_epoch += 1;
        }
        let tile = arrangement.tile.unwrap_or_default();
        let of: Vec<usize> = self
            .visible(0)
            .iter()
            .map(|entry| arrangement.slots.get(&entry.name).copied().unwrap_or(tile))
            .collect();
        let slots = Some(Arc::new(view::GridSlots::new(of)));
        if self.recent_sections.slots != slots {
            self.recent_sections.slots = slots;
            self.dirty = true;
        }
        self.desk_placement.arranged = Some(ArrangeKey {
            places: self.desk_placement.places_epoch,
            ..key
        });
    }

    /// The saved cells of the folder the desk shows, read from the state
    /// file when that folder's are not in hand yet: the first time, and
    /// after `desk.folder` moved the desk to another folder.
    fn desk_places(&mut self) -> &mut desk_slots::Places {
        let folder = &self.columns[0].path;
        let placement = &mut self.desk_placement;
        if placement.folder.as_ref() != Some(folder) || placement.places.is_none() {
            placement.places = Some(desk_slots::Places::load(folder));
            placement.folder = Some(folder.clone());
            placement.places_epoch += 1;
        }
        placement.places.get_or_insert_with(Default::default)
    }

    /// Keep `positions` as the saved cells, writing the state file when they
    /// changed.
    fn save_desk_positions(&mut self, positions: HashMap<String, Cell>) {
        let folder = self.columns[0].path.clone();
        let places = self.desk_places();
        if places.positions == positions {
            return;
        }
        places.positions = positions;
        places.save(&folder);
        self.desk_placement.places_epoch += 1;
    }

    /// Remember where a drag from `(x, y)` on the desk picked its icon up.
    pub(super) fn note_desk_drag(&mut self, x: f32, y: f32) {
        self.desk_placement.drag = None;
        if !self.desk_free() {
            return;
        }
        let Some((depth, index)) = self.entry_at(x, y) else {
            return;
        };
        let Some(entry) = self
            .visible(depth)
            .get(index)
            .map(|entry| entry.path.clone())
        else {
            return;
        };
        let cell = self.entry_rect(depth, index);
        self.desk_placement.drag = Some(DeskDrag {
            grabbed: entry,
            grab: (x - cell.left, y - cell.top),
        });
    }

    /// The slot the desk's own drag, over `(x, y)`, would put the icon it
    /// picked up on. `None` when this is not such a drag.
    pub(super) fn desk_drop_slot(&self, paths: &[PathBuf], x: f32, y: f32) -> Option<usize> {
        let drag = self.desk_placement.drag.as_ref()?;
        if !self.desk_free() || !paths.contains(&drag.grabbed) {
            return None;
        }
        let (columns, _) = self.desk_slot_grid();
        let moves = self.desk_moves(paths, x, y)?;
        let grabbed = drag.grabbed.file_name()?.to_str()?;
        moves
            .iter()
            .find(|(name, _)| name == grabbed)
            .map(|(_, cell)| cell.slot(columns))
    }

    /// Where each of `paths` lands when the desk's own drag is dropped at
    /// `(x, y)`: the picked-up icon on the free slot nearest the pointer,
    /// and the others keeping their places around it as far as free slots
    /// allow.
    fn desk_moves(&self, paths: &[PathBuf], x: f32, y: f32) -> Option<Vec<(String, Cell)>> {
        let drag = self.desk_placement.drag.as_ref()?;
        let slots = self.recent_sections.slots.as_ref()?;
        let area = view::content_viewport(self.size.0, self.content_h(), ViewMode::Grid);
        let scroll = self.columns[0].scroll.offset();
        let (columns, rows) = self.desk_slot_grid();

        let dragged: HashSet<&Path> = paths.iter().map(PathBuf::as_path).collect();
        let entries = self.visible(0);
        let mut taken: HashSet<usize> = HashSet::new();
        let mut moving: Vec<(String, Cell, bool)> = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let Some(slot) = slots.slot(index) else {
                continue;
            };
            if dragged.contains(entry.path.as_path()) {
                let grabbed = entry.path == drag.grabbed;
                moving.push((entry.name.clone(), Cell::of_slot(slot, columns), grabbed));
            } else {
                taken.insert(slot);
            }
        }
        // The overflow tile is not an icon of its own: it gives way, and
        // takes whichever slot is left last.
        if let Some(overflow) = self.desk_overflow() {
            if let Some(slot) = slots.slot(overflow.tile.first) {
                taken.remove(&slot);
            }
        }
        let from = moving.iter().find(|(_, _, grabbed)| *grabbed)?.1;
        let to = view::grid_slot_nearest(area, x - drag.grab.0, y - drag.grab.1, scroll);
        let (dcol, drow) = (
            to.col as i64 - from.col as i64,
            to.row as i64 - from.row as i64,
        );
        // The picked-up icon first, then the rest in grid order.
        moving.sort_by_key(|(_, cell, grabbed)| (!*grabbed, cell.slot(columns)));
        let mut moves = Vec::with_capacity(moving.len());
        for (name, cell, _) in moving {
            let want = Cell::new(
                (cell.col as i64 + dcol).max(0) as u32,
                (cell.row as i64 + drow).max(0) as u32,
            );
            let Some(cell) = desk_slots::nearest_free(want, &taken, columns, rows) else {
                break;
            };
            taken.insert(cell.slot(columns));
            moves.push((name, cell));
        }
        Some(moves)
    }

    /// Drop the desk's own drag at `(x, y)`: move the icons, leaving the
    /// files alone.
    pub(super) fn place_desk_drop(&mut self, paths: &[PathBuf], x: f32, y: f32) {
        let moves = self.desk_moves(paths, x, y);
        self.desk_placement.drag = None;
        let Some(moves) = moves else {
            return;
        };
        let mut positions = self
            .desk_placement
            .places
            .as_ref()
            .map(|places| places.positions.clone())
            .unwrap_or_default();
        positions.extend(moves);
        self.save_desk_positions(positions);
        self.dirty = true;
    }

    /// Keep a renamed icon where it was: its place is saved by name.
    pub(super) fn desk_renamed(&mut self, old: &str, new: &str) {
        let Some(places) = self.desk_placement.places.as_ref() else {
            return;
        };
        let Some(cell) = places.positions.get(old).copied() else {
            return;
        };
        let mut positions = places.positions.clone();
        positions.remove(old);
        positions.insert(new.to_owned(), cell);
        self.save_desk_positions(positions);
    }

    /// Clean Up: every icon back in sort order, filling the grid from the
    /// top-left corner.
    pub(super) fn clean_up_desk(&mut self) {
        if !self.desk_free() {
            return;
        }
        self.desk_placement.rank.clear();
        self.desk_placement.rank_epoch += 1;
        self.desk_placement.arranged = None;
        self.save_desk_positions(HashMap::new());
        self.dirty = true;
    }

    /// Switch the desk between the sorted grid and free placement, writing
    /// it to the config. The desk follows its own write like any other edit
    /// of the file, and applies it at once so the menu's answer shows.
    pub(super) fn set_desk_arrange(&mut self, arrange: Arrange) {
        if let Err(err) = crate::desk::save_arrange(arrange) {
            tracing::warn!(name: "desk.arrange.save_failed", %err, "cannot save the arrangement");
        }
        if let Some(config) = self.desk_config.as_mut() {
            config.arrange = arrange;
        }
        self.dirty = true;
    }

    /// Where Up or Down takes the keyboard on a desk with placed icons: the
    /// nearest icon in the nearest row that has one that way. `None` when
    /// the desk's icons are not placed, or the overflow panel has the
    /// keyboard.
    pub(super) fn desk_free_vertical(&self, down: bool) -> Option<usize> {
        if !self.desk_free() || self.overflow_panel.is_some() {
            return None;
        }
        let slots = self.recent_sections.slots.as_ref()?;
        let (columns, _) = self.desk_slot_grid();
        let last = self
            .desk_cursor_range()
            .map_or(self.visible_len(0), |range| range.end() + 1);
        let Some(cursor) = self.columns[self.active].cursor else {
            return Some(if down { 0 } else { last.saturating_sub(1) });
        };
        let from = Cell::of_slot(slots.slot(cursor)?, columns);
        let found = (0..last)
            .filter_map(|index| Some((index, Cell::of_slot(slots.slot(index)?, columns))))
            .filter(|(_, cell)| {
                if down {
                    cell.row > from.row
                } else {
                    cell.row < from.row
                }
            })
            .min_by_key(|(_, cell)| (cell.row.abs_diff(from.row), cell.col.abs_diff(from.col)))
            .map(|(index, _)| index);
        Some(found.unwrap_or(cursor))
    }
}
