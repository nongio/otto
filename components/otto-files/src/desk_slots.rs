//! The desk's free placement: where each icon sits on an invisible grid of
//! cell-sized slots, and the state file that remembers it — see
//! `specs/desk.md`, *Arrangement*.
//!
//! A slot is a column and a row of the grid the panel holds. Positions are
//! kept by file name in `$XDG_STATE_HOME/otto/desk.toml`, never in the
//! folder itself:
//!
//! ```toml
//! [positions]
//! "notes.txt" = [0, 0]
//! "Projects" = [3, 1]
//! ```
//!
//! [`arrange`] turns the saved cells and the listing into a slot per icon.
//! It never moves a saved cell: one that no longer fits the panel is
//! reflowed for as long as it does not, and comes back when it does.

// Rust guideline compliant 2026-02-21

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A slot on the desk's grid: a column and a row, from the top-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "[u32; 2]", into = "[u32; 2]")]
pub struct Cell {
    pub col: u32,
    pub row: u32,
}

impl Cell {
    pub fn new(col: u32, row: u32) -> Self {
        Self { col, row }
    }

    /// The slot's index in grid order, `columns` across.
    pub fn slot(self, columns: usize) -> usize {
        self.row as usize * columns + self.col as usize
    }

    /// The cell at `slot` in grid order, `columns` across.
    pub fn of_slot(slot: usize, columns: usize) -> Self {
        let columns = columns.max(1);
        Self::new((slot % columns) as u32, (slot / columns) as u32)
    }

    /// Whether the cell is on a grid `columns` across and, when it is
    /// bounded, `rows` down.
    pub fn fits(self, columns: usize, rows: Option<usize>) -> bool {
        (self.col as usize) < columns && rows.is_none_or(|rows| (self.row as usize) < rows)
    }
}

impl From<[u32; 2]> for Cell {
    fn from([col, row]: [u32; 2]) -> Self {
        Self::new(col, row)
    }
}

impl From<Cell> for [u32; 2] {
    fn from(cell: Cell) -> Self {
        [cell.col, cell.row]
    }
}

/// Where the desk's icons were put, by file name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Places {
    #[serde(default)]
    pub positions: HashMap<String, Cell>,
}

impl Places {
    /// Read the state file, or nothing where there is none or it cannot be
    /// read.
    pub fn load() -> Self {
        let Some(path) = state_path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|err| {
                tracing::warn!(
                    name: "desk.places.invalid",
                    path = %path.display(),
                    %err,
                    "ignoring the desk's saved positions"
                );
                Self::default()
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(err) => {
                tracing::warn!(
                    name: "desk.places.unreadable",
                    path = %path.display(),
                    %err,
                    "cannot read the desk's saved positions"
                );
                Self::default()
            }
        }
    }

    /// Write the state file. A failure is a warning: the icons stay where
    /// they are for this session either way.
    pub fn save(&self) {
        let Some(path) = state_path() else {
            return;
        };
        if let Err(err) = self.write(&path) {
            tracing::warn!(
                name: "desk.places.write_failed",
                path = %path.display(),
                %err,
                "cannot save the desk's positions"
            );
        }
    }

    fn write(&self, path: &std::path::Path) -> std::io::Result<()> {
        let text = toml::to_string(self).map_err(std::io::Error::other)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temp = path.with_extension(format!("toml.tmp.{}", std::process::id()));
        std::fs::write(&temp, text)?;
        std::fs::rename(&temp, path)
    }
}

/// Where the state file is, honouring `XDG_STATE_HOME`. `OTTO_DESK_STATE`
/// overrides the whole path.
#[cfg(not(test))]
fn state_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("OTTO_DESK_STATE") {
        return Some(PathBuf::from(path));
    }
    Some(otto_kit::xdg::state_home()?.join("otto").join("desk.toml"))
}

/// Tests never read or write the person's own positions.
#[cfg(test)]
fn state_path() -> Option<PathBuf> {
    None
}

/// Every icon's slot, as [`arrange`] works it out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Arrangement {
    /// The slot, in grid order, of each icon that has one, by name.
    pub slots: HashMap<String, usize>,
    /// The overflow tile's slot, when some icons have none.
    pub tile: Option<usize>,
    /// Icons that had no saved cell and were given one: new files. They
    /// keep it, so a slot freed later does not pull them across the desk.
    pub placed: Vec<(String, Cell)>,
}

/// Give each of `names` a slot on a grid `columns` across and, when the
/// panel stacks, `rows` down.
///
/// A saved cell that fits is kept; two names saved on one cell go to the
/// first of them in `names`. Everything else takes the first free slot, in
/// the order of `names`. On a bounded grid with more names than slots, the
/// last free slot is the overflow tile's, and the names left over have no
/// slot: they are in the tile. A saved cell that does not fit is reflowed
/// without being forgotten, so only names with no saved cell at all come
/// back in [`Arrangement::placed`].
pub fn arrange(
    names: &[&str],
    saved: &HashMap<String, Cell>,
    columns: usize,
    rows: Option<usize>,
) -> Arrangement {
    let columns = columns.max(1);
    let capacity = rows.map(|rows| rows * columns);
    let mut slots: HashMap<String, usize> = HashMap::new();
    let mut taken: HashSet<usize> = HashSet::new();
    for name in names {
        if let Some(cell) = saved.get(*name).filter(|cell| cell.fits(columns, rows)) {
            let slot = cell.slot(columns);
            if taken.insert(slot) {
                slots.insert((*name).to_owned(), slot);
            }
        }
    }

    let mut tile = None;
    if let Some(capacity) = capacity.filter(|capacity| names.len() > *capacity) {
        // The tile takes the last free slot. With none free, it takes the
        // last slot outright, and that slot's icon joins the overflow.
        let last_free = (0..capacity).rev().find(|slot| !taken.contains(slot));
        let slot = match last_free {
            Some(slot) => slot,
            None if capacity > 0 => {
                let last = capacity - 1;
                slots.retain(|_, slot| *slot != last);
                last
            }
            None => return Arrangement::default(),
        };
        taken.insert(slot);
        tile = Some(slot);
    }

    let mut placed = Vec::new();
    let mut next = 0;
    for name in names {
        if slots.contains_key(*name) {
            continue;
        }
        while taken.contains(&next) {
            next += 1;
        }
        if capacity.is_some_and(|capacity| next >= capacity) {
            break;
        }
        taken.insert(next);
        slots.insert((*name).to_owned(), next);
        if !saved.contains_key(*name) {
            placed.push(((*name).to_owned(), Cell::of_slot(next, columns)));
        }
    }
    Arrangement {
        slots,
        tile,
        placed,
    }
}

/// The free cell nearest `want` on a grid `columns` across and, when it is
/// bounded, `rows` down, by straight-line distance. Ties go to the earlier
/// slot in grid order. `None` when every cell is taken.
pub fn nearest_free(
    want: Cell,
    taken: &HashSet<usize>,
    columns: usize,
    rows: Option<usize>,
) -> Option<Cell> {
    let columns = columns.max(1);
    let want = Cell::new(
        want.col.min(columns as u32 - 1),
        match rows {
            Some(rows) => want.row.min((rows as u32).saturating_sub(1)),
            None => want.row,
        },
    );
    if rows == Some(0) {
        return None;
    }
    // An unbounded grid always has a free cell within as many rows below
    // `want` as there are taken cells.
    let last_row = match rows {
        Some(rows) => rows as u32 - 1,
        None => want.row + taken.len() as u32 + 1,
    };
    let distance = |cell: Cell| {
        let dx = cell.col as i64 - want.col as i64;
        let dy = cell.row as i64 - want.row as i64;
        dx * dx + dy * dy
    };
    (0..=last_row)
        .flat_map(|row| (0..columns as u32).map(move |col| Cell::new(col, row)))
        .filter(|cell| !taken.contains(&cell.slot(columns)))
        .min_by_key(|cell| (distance(*cell), cell.slot(columns)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(cells: &[(&str, u32, u32)]) -> HashMap<String, Cell> {
        cells
            .iter()
            .map(|(name, col, row)| ((*name).to_owned(), Cell::new(*col, *row)))
            .collect()
    }

    #[test]
    fn new_names_fill_the_grid_in_order_and_are_reported() {
        let arrangement = arrange(&["a", "b", "c"], &HashMap::new(), 4, None);
        assert_eq!(arrangement.slots["a"], 0);
        assert_eq!(arrangement.slots["b"], 1);
        assert_eq!(arrangement.slots["c"], 2);
        assert_eq!(arrangement.tile, None);
        assert_eq!(arrangement.placed.len(), 3);
    }

    #[test]
    fn saved_cells_stay_and_new_names_take_the_gaps() {
        let saved = saved(&[("a", 2, 1), ("b", 0, 0)]);
        let arrangement = arrange(&["a", "b", "c", "d"], &saved, 4, None);
        assert_eq!(arrangement.slots["a"], 6);
        assert_eq!(arrangement.slots["b"], 0);
        assert_eq!(arrangement.slots["c"], 1);
        assert_eq!(arrangement.slots["d"], 2);
        assert_eq!(
            arrangement.placed,
            vec![("c".into(), Cell::new(1, 0)), ("d".into(), Cell::new(2, 0))]
        );
    }

    #[test]
    fn a_cell_saved_twice_goes_to_the_first_name() {
        let saved = saved(&[("a", 1, 0), ("b", 1, 0)]);
        let arrangement = arrange(&["a", "b"], &saved, 4, None);
        assert_eq!(arrangement.slots["a"], 1);
        assert_eq!(arrangement.slots["b"], 0);
        // `b` still has its own saved cell, so it is not re-saved.
        assert!(arrangement.placed.is_empty());
    }

    #[test]
    fn a_cell_past_the_edge_reflows_without_being_forgotten() {
        let saved = saved(&[("a", 5, 0)]);
        let arrangement = arrange(&["a"], &saved, 3, None);
        assert_eq!(arrangement.slots["a"], 0);
        assert!(arrangement.placed.is_empty());
        let wider = arrange(&["a"], &saved, 6, None);
        assert_eq!(wider.slots["a"], 5);
    }

    #[test]
    fn a_full_stack_puts_the_tile_on_the_last_free_slot() {
        // Two rows of three; `a` sits on the very last cell.
        let saved = saved(&[("a", 2, 1)]);
        let names = ["a", "b", "c", "d", "e", "f", "g", "h"];
        let arrangement = arrange(&names, &saved, 3, Some(2));
        assert_eq!(arrangement.slots["a"], 5);
        assert_eq!(arrangement.tile, Some(4));
        for (name, slot) in [("b", 0), ("c", 1), ("d", 2), ("e", 3)] {
            assert_eq!(arrangement.slots[name], slot);
        }
        assert!(!arrangement.slots.contains_key("f"));
        assert!(!arrangement.slots.contains_key("h"));
    }

    #[test]
    fn a_stack_with_every_slot_saved_gives_up_the_last_one() {
        let saved = saved(&[("a", 0, 0), ("b", 1, 0), ("c", 0, 1), ("d", 1, 1)]);
        let arrangement = arrange(&["a", "b", "c", "d", "e"], &saved, 2, Some(2));
        assert_eq!(arrangement.tile, Some(3));
        assert!(!arrangement.slots.contains_key("d"));
        assert!(!arrangement.slots.contains_key("e"));
        assert_eq!(arrangement.slots.len(), 3);
    }

    #[test]
    fn a_stack_that_fits_has_no_tile() {
        let arrangement = arrange(&["a", "b"], &HashMap::new(), 2, Some(1));
        assert_eq!(arrangement.tile, None);
        assert_eq!(arrangement.slots.len(), 2);
    }

    #[test]
    fn nearest_free_snaps_to_the_closest_open_cell() {
        let taken: HashSet<usize> = [5].into_iter().collect();
        // Wanting the taken (1, 1) on a 4-wide grid: its neighbours tie, and
        // the earliest in grid order wins.
        assert_eq!(
            nearest_free(Cell::new(1, 1), &taken, 4, Some(3)),
            Some(Cell::new(1, 0))
        );
        assert_eq!(
            nearest_free(Cell::new(2, 2), &taken, 4, Some(3)),
            Some(Cell::new(2, 2))
        );
    }

    #[test]
    fn nearest_free_clamps_to_the_grid() {
        let taken = HashSet::new();
        assert_eq!(
            nearest_free(Cell::new(9, 9), &taken, 4, Some(3)),
            Some(Cell::new(3, 2))
        );
        assert_eq!(
            nearest_free(Cell::new(9, 9), &taken, 4, None),
            Some(Cell::new(3, 9))
        );
    }

    #[test]
    fn nearest_free_on_a_full_grid_is_none() {
        let taken: HashSet<usize> = (0..4).collect();
        assert_eq!(nearest_free(Cell::new(0, 0), &taken, 2, Some(2)), None);
    }

    #[test]
    fn places_round_trip_as_pairs() {
        let places = Places {
            positions: saved(&[("notes.txt", 3, 1)]),
        };
        let text = toml::to_string(&places).unwrap();
        assert!(text.contains("[3, 1]"), "{text}");
        assert_eq!(toml::from_str::<Places>(&text).unwrap(), places);
    }
}
