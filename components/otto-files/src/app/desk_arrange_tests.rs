use super::*;
use crate::desk::{Arrange, DeskConfig, Overflow};

/// A folder of `count` files, swept up on drop.
struct Folder(PathBuf);

impl Folder {
    fn holding(count: usize) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "otto-files-desk-arrange-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        for i in 0..count {
            std::fs::write(path.join(format!("file-{i:03}.txt")), b"x").expect("temp file");
        }
        Self(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The desk over `count` files with its icons placed by hand, in a surface
/// of 800 by 500 points, arranged once.
fn desk(count: usize, overflow: Overflow) -> (Browser, Folder) {
    let folder = Folder::holding(count);
    let mut config = DeskConfig::with_folder(folder.0.clone());
    config.arrange = Arrange::Free;
    config.overflow = overflow;
    let mut browser = Browser::listing_the_desk(&config);
    browser.size = (800.0, 500.0);
    for _ in 0..500 {
        if browser.columns[0].poll() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(!browser.columns[0].loading(), "listing never arrived");
    browser.sync_scroll_metrics();
    (browser, folder)
}

fn area(browser: &Browser) -> Rect {
    view::content_viewport(browser.size.0, browser.content_h(), ViewMode::Grid)
}

/// Entry `name`'s rect on the desk.
fn rect_of(browser: &Browser, name: &str) -> Rect {
    let index = browser
        .visible(0)
        .iter()
        .position(|entry| entry.name == name)
        .unwrap_or_else(|| panic!("no {name}"));
    browser.entry_rect(0, index)
}

fn path_of(folder: &Folder, name: &str) -> PathBuf {
    folder.0.join(name)
}

/// Pick `names` up by the first of them and drop them with that icon's
/// top-left corner at `(left, top)`.
fn drag(browser: &mut Browser, folder: &Folder, names: &[&str], left: f32, top: f32) {
    let from = rect_of(browser, names[0]);
    let grab = (from.left + 10.0, from.top + 10.0);
    browser.note_desk_drag(grab.0, grab.1);
    let paths: Vec<PathBuf> = names.iter().map(|name| path_of(folder, name)).collect();
    browser.place_desk_drop(&paths, left + 10.0, top + 10.0);
    browser.sync_scroll_metrics();
}

#[test]
fn icons_start_in_sort_order_on_the_grid() {
    let (browser, _folder) = desk(5, Overflow::Scroll);
    let area = area(&browser);
    for (index, name) in ["file-000.txt", "file-001.txt", "file-004.txt"]
        .iter()
        .zip([0, 1, 4])
        .map(|(name, index)| (index, *name))
    {
        assert_eq!(
            rect_of(&browser, name),
            view::grid_cell_rect(area, index, 0.0)
        );
    }
}

/// A dropped icon lands on the slot nearest where it was let go, leaves a
/// gap behind it, and its file stays where it is.
#[test]
fn a_dropped_icon_lands_on_the_nearest_slot() {
    let (mut browser, folder) = desk(3, Overflow::Scroll);
    let area = area(&browser);
    let target = view::grid_slot_rect(area, 9, 0.0);
    drag(
        &mut browser,
        &folder,
        &["file-000.txt"],
        target.left + 12.0,
        target.top - 15.0,
    );

    assert_eq!(rect_of(&browser, "file-000.txt"), target);
    assert_eq!(
        rect_of(&browser, "file-001.txt"),
        view::grid_cell_rect(area, 1, 0.0)
    );
    assert!(path_of(&folder, "file-000.txt").exists());

    // The slot it left is empty space now, and its new one is it.
    let (x, y) = {
        let first = view::grid_cell_rect(area, 0, 0.0);
        (first.center_x(), first.center_y())
    };
    assert_eq!(browser.entry_at(x, y), None);
    let hit = browser.entry_at(target.center_x(), target.center_y());
    let name = hit.map(|(depth, index)| browser.visible(depth)[index].name.clone());
    assert_eq!(name.as_deref(), Some("file-000.txt"));
}

#[test]
fn a_drop_on_a_taken_slot_snaps_beside_it() {
    let (mut browser, folder) = desk(3, Overflow::Scroll);
    let taken = rect_of(&browser, "file-001.txt");
    drag(
        &mut browser,
        &folder,
        &["file-002.txt"],
        taken.left,
        taken.top,
    );
    let landed = rect_of(&browser, "file-002.txt");
    assert_ne!(landed, taken);
    assert_eq!(rect_of(&browser, "file-001.txt"), taken);
    // Next to it, not across the desk.
    assert!((landed.center_x() - taken.center_x()).abs() <= view::cell_w() + 0.5);
    assert!((landed.center_y() - taken.center_y()).abs() <= view::cell_h() + 0.5);
}

/// Several icons dragged together keep their places around the one picked
/// up.
#[test]
fn a_group_moves_together() {
    let (mut browser, folder) = desk(4, Overflow::Scroll);
    let area = area(&browser);
    let target = view::grid_slot_rect(area, view::grid_columns(area) * 2 + 1, 0.0);
    drag(
        &mut browser,
        &folder,
        &["file-000.txt", "file-001.txt"],
        target.left,
        target.top,
    );
    let first = rect_of(&browser, "file-000.txt");
    let second = rect_of(&browser, "file-001.txt");
    assert_eq!(first, target);
    assert_eq!(second, first.with_offset((view::cell_w(), 0.0)));
}

#[test]
fn clean_up_puts_the_icons_back_in_order() {
    let (mut browser, folder) = desk(3, Overflow::Scroll);
    let area = area(&browser);
    let far = view::grid_slot_rect(area, 10, 0.0);
    drag(&mut browser, &folder, &["file-000.txt"], far.left, far.top);
    assert_eq!(rect_of(&browser, "file-000.txt"), far);

    browser.clean_up_desk();
    browser.sync_scroll_metrics();
    for (index, name) in ["file-000.txt", "file-001.txt", "file-002.txt"]
        .iter()
        .enumerate()
    {
        assert_eq!(
            rect_of(&browser, name),
            view::grid_cell_rect(area, index, 0.0)
        );
    }
}

/// Back on the sorted grid, the icons fill it in order again; their places
/// are kept for when free placement comes back.
#[test]
fn the_sorted_grid_ignores_the_places_and_keeps_them() {
    let (mut browser, folder) = desk(3, Overflow::Scroll);
    let area = area(&browser);
    let far = view::grid_slot_rect(area, 10, 0.0);
    drag(&mut browser, &folder, &["file-000.txt"], far.left, far.top);

    browser.desk_config.as_mut().unwrap().arrange = Arrange::Grid;
    browser.sync_scroll_metrics();
    assert!(browser.recent_sections.slots.is_none());
    assert_eq!(
        rect_of(&browser, "file-000.txt"),
        view::grid_cell_rect(area, 0, 0.0)
    );

    browser.desk_config.as_mut().unwrap().arrange = Arrange::Free;
    browser.sync_scroll_metrics();
    assert_eq!(rect_of(&browser, "file-000.txt"), far);
}

/// A renamed icon stays where it was put.
#[test]
fn a_renamed_icon_keeps_its_place() {
    let (mut browser, folder) = desk(3, Overflow::Scroll);
    let area = area(&browser);
    let far = view::grid_slot_rect(area, 10, 0.0);
    drag(&mut browser, &folder, &["file-000.txt"], far.left, far.top);

    std::fs::rename(path_of(&folder, "file-000.txt"), path_of(&folder, "zz.txt")).unwrap();
    browser.desk_renamed("file-000.txt", "zz.txt");
    browser.columns[0].reload();
    for _ in 0..500 {
        if browser.columns[0].poll() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    browser.sync_scroll_metrics();
    assert_eq!(rect_of(&browser, "zz.txt"), far);
}

/// On a desk that stacks, an icon dropped on the overflow tile takes its
/// slot; the tile gives way to the last free slot, and the items it holds
/// are still the run at the end of the listing.
#[test]
fn the_tile_takes_the_last_free_slot() {
    let (mut browser, folder) = desk(80, Overflow::Stack);
    let area = area(&browser);
    let capacity = view::grid_capacity(area);
    let last = view::grid_slot_rect(area, capacity - 1, 0.0);
    assert_eq!(browser.desk_overflow().expect("a tile").cell(), last);
    drag(
        &mut browser,
        &folder,
        &["file-000.txt"],
        last.left,
        last.top,
    );
    assert_eq!(rect_of(&browser, "file-000.txt"), last);

    // The desk is full, so the only slot left is the one the icon left.
    let overflow = browser.desk_overflow().expect("a tile");
    assert_eq!(overflow.cell(), view::grid_slot_rect(area, 0, 0.0));
    assert_eq!(overflow.tile.first, capacity - 1);
    assert_eq!(overflow.tile.count, 80 - (capacity - 1));
}

/// Up and Down go to the nearest icon in the nearest row that has one,
/// past empty rows.
#[test]
fn down_skips_empty_rows() {
    let (mut browser, folder) = desk(3, Overflow::Scroll);
    let area = area(&browser);
    let columns = view::grid_columns(area);
    let far = view::grid_slot_rect(area, columns * 3 + 1, 0.0);
    drag(&mut browser, &folder, &["file-002.txt"], far.left, far.top);

    let index_of = |browser: &Browser, name: &str| {
        browser
            .visible(0)
            .iter()
            .position(|entry| entry.name == name)
            .unwrap()
    };
    let start = index_of(&browser, "file-000.txt");
    browser.select(0, start);
    let next = browser.desk_free_vertical(true).expect("placed icons");
    assert_eq!(next, index_of(&browser, "file-002.txt"));
    browser.select(0, next);
    assert_eq!(browser.desk_free_vertical(true), Some(next));
    let back = browser.desk_free_vertical(false).unwrap();
    assert!(back == start || back == index_of(&browser, "file-001.txt"));
}
