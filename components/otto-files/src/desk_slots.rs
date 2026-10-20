//! The desk's free placement: where each icon sits on an invisible grid of
//! cell-sized slots, and the state file that remembers it — see
//! `specs/desk.md`, *Arrangement*.
//!
//! A slot is a column and a row of the grid the panel holds. Positions are
//! kept by folder, then by file name, in `$XDG_STATE_HOME/otto/desk.toml`,
//! never in the folder itself, so pointing the desk at another folder
//! neither wipes nor borrows this one's layout:
//!
//! ```toml
//! [folders."/home/me/Desktop".positions]
//! "notes.txt" = [0, 0]
//! "Projects" = [3, 1]
//! ```
//!
//! A file from before positions were kept per folder has a bare
//! `[positions]` table. The first folder read with no layout of its own
//! takes it up, and the next save writes it under that folder.
//!
//! [`arrange`] turns the saved cells and the listing into a slot per icon.
//! It never moves a saved cell: one that no longer fits the panel is
//! reflowed for as long as it does not, and comes back when it does.

// Rust guideline compliant 2026-02-21

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

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

/// Where the desk's icons were put in one folder, by file name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Places {
    #[serde(default)]
    pub positions: HashMap<String, Cell>,
}

/// The whole state file: every folder's layout.
#[derive(Debug, Default, Serialize, Deserialize)]
struct StateFile {
    /// The one layout of a file from before folders were told apart.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    positions: HashMap<String, Cell>,
    #[serde(default)]
    folders: BTreeMap<String, Places>,
}

/// The key `folder`'s layout is kept under.
fn folder_key(folder: &Path) -> String {
    folder.to_string_lossy().into_owned()
}

impl Places {
    /// Read `folder`'s layout from the state file, or nothing where there is
    /// none or the file cannot be read.
    pub fn load(folder: &Path) -> Self {
        state_path().map_or_else(Self::default, |path| Self::load_from(&path, folder))
    }

    fn load_from(path: &Path, folder: &Path) -> Self {
        let mut state = read_state(path).unwrap_or_default();
        match state.folders.remove(&folder_key(folder)) {
            Some(places) => places,
            None => Self {
                positions: state.positions,
            },
        }
    }

    /// Write this as `folder`'s layout, keeping every other folder's. A
    /// failure is a warning: the icons stay where they are for this session
    /// either way.
    pub fn save(&self, folder: &Path) {
        let Some(path) = state_path() else {
            return;
        };
        if let Err(err) = self.save_to(&path, folder) {
            tracing::warn!(
                name: "desk.places.write_failed",
                path = %path.display(),
                %err,
                "cannot save the desk's positions"
            );
        }
    }

    fn save_to(&self, path: &Path, folder: &Path) -> std::io::Result<()> {
        // A file that does not parse is not written over: it may be a newer
        // Otto's, or a hand edit gone wrong, and either is worth keeping.
        let mut state = read_state(path).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "the state file cannot be read",
            )
        })?;
        // Whichever folder read the old single layout took it up.
        state.positions.clear();
        let key = folder_key(folder);
        if self.positions.is_empty() {
            state.folders.remove(&key);
        } else {
            state.folders.insert(key, self.clone());
        }
        let text = toml::to_string(&state).map_err(std::io::Error::other)?;
        otto_kit::fs::write_atomically(path, text.as_bytes())
    }
}

/// The state file at `path`: empty where there is none, `None` where it
/// cannot be read or does not parse.
fn read_state(path: &Path) -> Option<StateFile> {
    match std::fs::read_to_string(path) {
        Ok(text) => match toml::from_str(&text) {
            Ok(state) => Some(state),
            Err(err) => {
                tracing::warn!(
                    name: "desk.places.invalid",
                    path = %path.display(),
                    %err,
                    "ignoring the desk's saved positions"
                );
                None
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Some(StateFile::default()),
        Err(err) => {
            tracing::warn!(
                name: "desk.places.unreadable",
                path = %path.display(),
                %err,
                "cannot read the desk's saved positions"
            );
            None
        }
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

    /// A state file of its own, swept up on drop.
    struct StateDir(PathBuf);

    impl StateDir {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "otto-files-desk-state-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self) -> PathBuf {
            self.0.join("desk.toml")
        }
    }

    impl Drop for StateDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn each_folder_keeps_its_own_layout() {
        let dir = StateDir::new();
        let (desktop, stuff) = (Path::new("/home/me/Desktop"), Path::new("/home/me/Stuff"));
        let mine = Places {
            positions: saved(&[("a", 1, 0)]),
        };
        let theirs = Places {
            positions: saved(&[("b", 2, 2)]),
        };
        mine.save_to(&dir.file(), desktop).unwrap();
        theirs.save_to(&dir.file(), stuff).unwrap();
        assert_eq!(Places::load_from(&dir.file(), desktop), mine);
        assert_eq!(Places::load_from(&dir.file(), stuff), theirs);
        assert_eq!(
            Places::load_from(&dir.file(), Path::new("/elsewhere")),
            Places::default()
        );

        // Cleaning one folder up leaves the other alone.
        Places::default().save_to(&dir.file(), stuff).unwrap();
        assert_eq!(Places::load_from(&dir.file(), desktop), mine);
        assert_eq!(Places::load_from(&dir.file(), stuff), Places::default());
    }

    #[test]
    fn an_old_single_layout_is_taken_up_by_the_first_folder_read() {
        let dir = StateDir::new();
        std::fs::write(dir.file(), "[positions]\n\"notes.txt\" = [3, 1]\n").unwrap();
        let desktop = Path::new("/home/me/Desktop");
        let places = Places::load_from(&dir.file(), desktop);
        assert_eq!(places.positions, saved(&[("notes.txt", 3, 1)]));

        places.save_to(&dir.file(), desktop).unwrap();
        let text = std::fs::read_to_string(dir.file()).unwrap();
        assert!(!text.starts_with("[positions]"), "{text}");
        assert_eq!(Places::load_from(&dir.file(), desktop), places);
        // Moved, not copied: another folder does not inherit it as well.
        assert_eq!(
            Places::load_from(&dir.file(), Path::new("/home/me/Stuff")),
            Places::default()
        );
    }

    #[test]
    fn a_state_file_that_does_not_parse_is_not_written_over() {
        let dir = StateDir::new();
        std::fs::write(dir.file(), "not [ toml").unwrap();
        let places = Places {
            positions: saved(&[("a", 0, 0)]),
        };
        assert!(places.save_to(&dir.file(), Path::new("/d")).is_err());
        assert_eq!(std::fs::read_to_string(dir.file()).unwrap(), "not [ toml");
    }
}
