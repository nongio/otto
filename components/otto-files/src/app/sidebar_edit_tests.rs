use super::*;

use crate::places_config::Builtin;

/// A row dragged past the threshold is lifted, and the line follows the
/// nearest gap — anywhere in the list, but never the two beside where the row
/// already is.
#[test]
fn sorting_a_row_shows_the_gap_it_would_land_in() {
    let (mut browser, dir) = browser_with_sidebar();
    let b = view::place_rect(3);
    browser.press_place(3, b.center_y());

    // An unsteady click is still a click.
    assert!(!browser.drag_place_to(b.center_y() + 2.0));
    assert_eq!(browser.dragging_place_frame().map(|drag| drag.index), None);

    // Up over `a`'s top half: the gap before it.
    assert!(browser.drag_place_to(view::place_rect(2).top + 4.0));
    assert_eq!(
        browser.dragging_place_frame().map(|drag| drag.index),
        Some(3)
    );
    assert_eq!(browser.drop_target, Some(DropTarget::PlaceGap { slot: 2 }));

    // Up among the built-ins: the row can go above them too.
    assert!(browser.drag_place_to(view::place_rect(0).top));
    assert_eq!(browser.drop_target, Some(DropTarget::PlaceGap { slot: 0 }));

    // Back over itself: nowhere to go, no line.
    assert!(browser.drag_place_to(b.center_y() + 8.0));
    assert_eq!(browser.drop_target, None);

    // Let go there: the gesture ends, and nothing is written.
    assert!(browser.release_place_drag(b.center_y() + 8.0));
    assert_eq!(browser.dragging_place_frame().map(|drag| drag.index), None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The built-in rows are the user's to arrange as well. A sidebar with a
/// single folder of the user's own has nothing else it could sort.
#[test]
fn a_built_in_row_sorts_too() {
    let (mut browser, dir) = browser_with_sidebar();
    browser.press_place(1, view::place_rect(1).center_y());
    assert!(browser.drag_place_to(view::place_rect(3).bottom + 10.0));
    assert_eq!(
        browser.dragging_place_frame().map(|drag| drag.index),
        Some(1)
    );
    assert_eq!(browser.drop_target, Some(DropTarget::PlaceGap { slot: 4 }));
    browser.cancel_place_drag();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The order a sort writes: the moved row at its gap, everything else as
/// shown, and rows the config orders but this window does not show kept in
/// their place.
#[test]
fn a_sort_writes_the_whole_order() {
    use crate::places_config::PlaceKey;
    let (browser, dir) = browser_with_sidebar();
    let home = PlaceKey::Builtin(Builtin::Home);
    let docs = PlaceKey::Builtin(Builtin::Documents);
    let a = PlaceKey::Path(dir.join("a"));
    let b = PlaceKey::Path(dir.join("b"));

    // Documents dragged below b.
    assert_eq!(
        browser.order_with(std::slice::from_ref(&docs), 4, Vec::new()),
        [home.clone(), a.clone(), b.clone(), docs.clone()]
    );
    // b dragged above everything.
    assert_eq!(
        browser.order_with(std::slice::from_ref(&b), 0, Vec::new()),
        [b.clone(), home.clone(), docs.clone(), a.clone()]
    );
    // Music is hidden but was second in the order; it stays second.
    let music = PlaceKey::Builtin(Builtin::Music);
    assert_eq!(
        browser.order_with(
            std::slice::from_ref(&a),
            0,
            vec![home.clone(), music.clone(), docs.clone()]
        ),
        [a, music, home, docs, b]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Leaving the window drops the gesture and the line with it.
#[test]
fn a_sort_let_go_outside_the_window_leaves_the_row() {
    let (mut browser, dir) = browser_with_sidebar();
    browser.press_place(2, view::place_rect(2).center_y());
    assert!(browser.drag_place_to(view::place_rect(3).bottom + 40.0));
    assert_eq!(browser.drop_target, Some(DropTarget::PlaceGap { slot: 4 }));
    browser.cancel_place_drag();
    assert_eq!(browser.drop_target, None);
    assert_eq!(browser.dragging_place_frame().map(|drag| drag.index), None);
    let _ = std::fs::remove_dir_all(&dir);
}

fn place(label: &str, path: &Path, builtin: Option<Builtin>) -> Place {
    Place {
        label: label.to_string(),
        path: path.to_path_buf(),
        icon: "folder".to_string(),
        recent: builtin == Some(Builtin::Recent),
        builtin,
    }
}

/// A browser whose sidebar is Home, Documents, and two folders of the user's
/// own — written straight in, so nothing is read from or written to the
/// machine's real config.
fn browser_with_sidebar() -> (Browser, PathBuf) {
    // One folder per call: the tests run in parallel, and each sweeps its
    // own away when it is done.
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "otto-files-sidebar-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    for name in ["docs", "a", "b"] {
        std::fs::create_dir_all(dir.join(name)).expect("temp dir");
    }
    let mut browser = Browser::new(dir.clone());
    browser.places = vec![
        place("Home", &dir, Some(Builtin::Home)),
        place("Documents", &dir.join("docs"), Some(Builtin::Documents)),
        place("a", &dir.join("a"), None),
        place("b", &dir.join("b"), None),
    ];
    (browser, dir)
}

fn x() -> f32 {
    view::place_rect(0).center_x()
}

/// Between two rows, below the last, and above the first are all gaps; the
/// middle of a row is the row.
#[test]
fn the_gaps_between_rows_are_where_folders_are_added() {
    let count = 4;
    let row = |i| view::place_rect(i);
    assert_eq!(view::place_gap_at(x(), row(0).top + 1.0, count), Some(0));
    assert_eq!(view::place_gap_at(x(), row(1).bottom + 2.0, count), Some(2));
    assert_eq!(view::place_gap_at(x(), row(2).bottom - 1.0, count), Some(3));
    assert_eq!(
        view::place_gap_at(x(), row(3).bottom + 200.0, count),
        Some(4)
    );
    for i in 0..count {
        assert_eq!(view::place_gap_at(x(), row(i).center_y(), count), None);
    }
    // The heading above the rows, and the file area, are not the sidebar.
    assert_eq!(view::place_gap_at(x(), row(0).top - 20.0, count), None);
    assert_eq!(
        view::place_gap_at(view::sidebar_w() + 10.0, row(1).bottom + 2.0, count),
        None
    );
}

/// A folder dropped in a gap goes in that gap, among the built-ins too.
#[test]
fn a_folder_lands_in_the_gap_it_was_dropped_in() {
    let (browser, dir) = browser_with_sidebar();
    let between_home_and_docs = view::place_rect(0).bottom + 2.0;
    assert_eq!(
        browser.drop_target_at(x(), between_home_and_docs),
        Some(DropTarget::PlaceGap { slot: 1 })
    );
    let between_a_and_b = view::place_rect(2).bottom + 2.0;
    assert_eq!(
        browser.drop_target_at(x(), between_a_and_b),
        Some(DropTarget::PlaceGap { slot: 3 })
    );
    let below = view::place_rect(3).bottom + 50.0;
    assert_eq!(
        browser.drop_target_at(x(), below),
        Some(DropTarget::PlaceGap { slot: 4 })
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Dropping on a row itself still files into its folder.
#[test]
fn the_middle_of_a_row_still_takes_files() {
    let (browser, dir) = browser_with_sidebar();
    assert_eq!(
        browser.drop_target_at(x(), view::place_rect(1).center_y()),
        Some(DropTarget::Place {
            index: 1,
            path: dir.join("docs"),
        })
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The insertion line sits in the gap it names, between the rows.
#[test]
fn the_insertion_line_sits_between_the_rows() {
    let line = view::place_gap_rect(2);
    assert!(line.top >= view::place_rect(1).bottom);
    assert!(line.bottom <= view::place_rect(2).top);
    assert!(view::place_gap_rect(0).bottom <= view::place_rect(0).top);
}

/// The sidebar read again keeps the clicked row lit on the folder it led to,
/// even when rows above it came or went.
#[test]
fn a_reloaded_sidebar_keeps_the_lit_row_on_its_folder() {
    let (mut browser, dir) = browser_with_sidebar();
    browser.active_place = Some(3);
    let mut next = browser.places.clone();
    next.remove(1);
    browser.set_places(next);
    assert_eq!(browser.active_place, Some(2));
    assert_eq!(browser.places[2].path, dir.join("b"));

    let mut next = browser.places.clone();
    next.remove(2);
    browser.set_places(next);
    assert_eq!(browser.active_place, None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Only folders that are not already built-in rows are offered.
#[test]
fn adding_the_selection_needs_folders_that_are_not_built_in() {
    let (mut browser, dir) = browser_with_sidebar();
    for _ in 0..500 {
        if browser.columns[0].poll() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let index_of = |browser: &Browser, name: &str| {
        browser
            .visible(0)
            .iter()
            .position(|entry| entry.name == name)
            .expect("listed")
    };
    let docs = index_of(&browser, "docs");
    browser.select(0, docs);
    assert!(browser.selection_for_sidebar().is_none());

    std::fs::create_dir_all(dir.join("c")).unwrap();
    browser.reload_all();
    for _ in 0..500 {
        if browser.columns[0].poll() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let c = index_of(&browser, "c");
    browser.select(0, c);
    assert_eq!(browser.selection_for_sidebar(), Some(vec![dir.join("c")]));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The ghost is the row carried by the pointer's travel, and is held from
/// leaving the top of the list.
#[test]
fn the_ghost_follows_the_pointer_along_the_list() {
    let (mut browser, dir) = browser_with_sidebar();
    let start = view::place_rect(3).center_y();
    browser.press_place(3, start);
    assert!(browser.drag_place_to(start - 40.0));
    let drag = browser.dragging_place_frame().expect("sorting");
    assert_eq!(drag.dy, -40.0);
    let ghost = view::place_ghost_rect(drag);
    assert_eq!(ghost.top, view::place_rect(3).top - 40.0);
    assert_eq!(ghost.left, view::place_rect(3).left);

    // Far above the list: it stops at the top.
    assert!(browser.drag_place_to(-500.0));
    let ghost = view::place_ghost_rect(browser.dragging_place_frame().unwrap());
    assert!(ghost.top >= view::place_rect(0).top - 6.0);
    browser.cancel_place_drag();
    let _ = std::fs::remove_dir_all(&dir);
}
