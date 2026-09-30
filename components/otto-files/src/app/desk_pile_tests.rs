use super::*;
use crate::desk::{DeskConfig, Overflow};
use skia_safe::Point;

/// A folder of `count` files, swept up on drop.
struct Folder(PathBuf);

impl Folder {
    fn holding(count: usize) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "otto-files-desk-pile-{}-{}",
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

/// The desk over `count` files, in a surface of 800 by 500 points, with
/// `overflow` as configured.
fn desk(count: usize, overflow: Overflow) -> (Browser, Folder) {
    let folder = Folder::holding(count);
    let mut config = DeskConfig::with_folder(folder.0.clone());
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
    assert_eq!(browser.visible_len(0), count);
    (browser, folder)
}

/// How many cells the test desk's grid holds.
fn capacity(browser: &Browser) -> usize {
    view::grid_capacity(view::content_viewport(
        browser.size.0,
        browser.content_h(),
        ViewMode::Grid,
    ))
}

fn centre(rect: Rect) -> (f32, f32) {
    (rect.center_x(), rect.center_y())
}

#[test]
fn a_desk_that_scrolls_has_no_pile() {
    let (browser, _folder) = desk(80, Overflow::Scroll);
    assert!(browser.desk_pile().is_none());
}

#[test]
fn a_desk_whose_icons_fit_has_no_pile() {
    let (browser, _folder) = desk(3, Overflow::Stack);
    assert!(capacity(&browser) > 3);
    assert!(browser.desk_pile().is_none());
}

/// The last cell holds its own item and every one after it, and it is no
/// entry of its own: a press there is the pile's and a drop lands on the
/// desk.
#[test]
fn the_last_cell_is_a_pile_of_the_rest() {
    let (browser, _folder) = desk(80, Overflow::Stack);
    let cells = capacity(&browser);
    assert!(cells > 1 && cells < 80, "{cells}");
    let pile = browser.desk_pile().expect("a pile").pile;
    assert_eq!(pile.first, cells - 1);
    assert_eq!(pile.first + pile.count, 80);

    let first = browser.entry_rect(0, 0);
    assert_eq!(
        browser.entry_at(first.center_x(), first.center_y()),
        Some((0, 0))
    );
    let (x, y) = centre(browser.entry_rect(0, pile.first));
    assert_eq!(browser.entry_at(x, y), None);
    let hit = browser.drop_target_at(x, y).expect("a target");
    assert_eq!(hit.path(), &browser.columns[0].path);
}

/// Open, the fan answers for the pile's items where it draws them, and the
/// pile's items are nowhere else.
#[test]
fn the_fan_holds_the_piles_items_where_it_draws_them() {
    let (mut browser, _folder) = desk(80, Overflow::Stack);
    let pile = browser.desk_pile().unwrap().pile;
    browser.open_desk_fan();
    let (fan, _) = browser.desk_pile().unwrap().fan.expect("the fan is open");

    for index in [pile.first, pile.first + 1, pile.first + 5] {
        let rect = browser.entry_rect(0, index);
        assert!(fan
            .rect
            .contains(Point::new(rect.center_x(), rect.center_y())));
        let (x, y) = centre(rect);
        assert_eq!(browser.entry_at(x, y), Some((0, index)));
    }
}

/// The keyboard stops at the pile; Return opens the fan and the arrows then
/// walk the pile's items and no others; Escape closes it again.
#[test]
fn the_keyboard_reaches_the_pile_and_walks_the_fan() {
    let (mut browser, _folder) = desk(80, Overflow::Stack);
    let pile = browser.desk_pile().unwrap().pile;

    // The first press lands on the first icon, the next runs to the end.
    browser.move_cursor(100_000, false);
    browser.move_cursor(100_000, false);
    assert_eq!(browser.columns[0].cursor, Some(pile.first));
    assert!(browser.cursor_on_closed_pile());

    browser.open_selection();
    assert!(browser.desk_pile().unwrap().fan.is_some());
    assert!(!browser.cursor_on_closed_pile());

    browser.move_cursor(100_000, false);
    assert_eq!(browser.columns[0].cursor, Some(79));
    browser.move_cursor(-100_000, false);
    assert_eq!(browser.columns[0].cursor, Some(pile.first));

    assert!(browser.close_desk_fan());
    assert!(browser.desk_pile().unwrap().fan.is_none());
}

/// The grid never scrolls while it stacks: all it holds fits.
#[test]
fn a_desk_that_stacks_does_not_scroll() {
    let (mut browser, _folder) = desk(80, Overflow::Stack);
    browser.sync_scroll_metrics();
    let scroll = &browser.columns[0].scroll;
    assert!(scroll.state.content_length() <= scroll.state.viewport().height() + 0.5);

    let (mut scrolling, _folder) = desk(80, Overflow::Scroll);
    scrolling.sync_scroll_metrics();
    let scroll = &scrolling.columns[0].scroll;
    assert!(scroll.state.content_length() > scroll.state.viewport().height());
}

/// A fan whose pile is gone is forgotten, not reopened when a pile forms
/// again.
#[test]
fn a_fan_without_its_pile_is_forgotten() {
    let (mut browser, _folder) = desk(80, Overflow::Stack);
    browser.open_desk_fan();
    browser.desk_config.as_mut().unwrap().overflow = Overflow::Scroll;
    browser.sync_scroll_metrics();
    browser.desk_config.as_mut().unwrap().overflow = Overflow::Stack;
    assert!(browser.desk_pile().unwrap().fan.is_none());
}
