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
            "otto-files-desk-overflow-{}-{}",
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
fn a_desk_that_scrolls_has_no_overflow_tile() {
    let (browser, _folder) = desk(80, Overflow::Scroll);
    assert!(browser.desk_overflow().is_none());
}

#[test]
fn a_desk_whose_icons_fit_has_no_overflow_tile() {
    let (browser, _folder) = desk(3, Overflow::Stack);
    assert!(capacity(&browser) > 3);
    assert!(browser.desk_overflow().is_none());
}

/// The last cell holds its own item and every one after it, and it is no
/// entry of its own: a press there is the tile's and a drop lands on the
/// desk.
#[test]
fn the_last_cell_is_a_tile_of_the_rest() {
    let (browser, _folder) = desk(80, Overflow::Stack);
    let cells = capacity(&browser);
    assert!(cells > 1 && cells < 80, "{cells}");
    let tile = browser.desk_overflow().expect("a tile").tile;
    assert_eq!(tile.first, cells - 1);
    assert_eq!(tile.first + tile.count, 80);

    let first = browser.entry_rect(0, 0);
    assert_eq!(
        browser.entry_at(first.center_x(), first.center_y()),
        Some((0, 0))
    );
    let (x, y) = centre(browser.entry_rect(0, tile.first));
    assert_eq!(browser.entry_at(x, y), None);
    let hit = browser.drop_target_at(x, y).expect("a target");
    assert_eq!(hit.path(), &browser.columns[0].path);
}

/// Open, the panel answers for the tile's items where it draws them, and
/// the tile's items are nowhere else.
#[test]
fn the_panel_holds_the_tiles_items_where_it_draws_them() {
    let (mut browser, _folder) = desk(80, Overflow::Stack);
    let tile = browser.desk_overflow().unwrap().tile;
    browser.open_overflow_panel();
    let (panel, _) = browser
        .desk_overflow()
        .unwrap()
        .panel
        .expect("the panel is open");

    for index in [tile.first, tile.first + 1, tile.first + 5] {
        let rect = browser.entry_rect(0, index);
        assert!(panel
            .rect
            .contains(Point::new(rect.center_x(), rect.center_y())));
        let (x, y) = centre(rect);
        assert_eq!(browser.entry_at(x, y), Some((0, index)));
        assert!(browser.in_overflow_panel(index));
    }
    assert!(!browser.in_overflow_panel(0));
}

/// The panel is not held inside the desk's grid: it is a surface of its own
/// above the windows, two rows tall, inside the desk's surface.
#[test]
fn the_panel_shows_two_rows_inside_the_surface() {
    let (mut browser, _folder) = desk(80, Overflow::Stack);
    browser.open_overflow_panel();
    let (panel, _) = browser.desk_overflow().unwrap().panel.unwrap();
    assert_eq!(
        panel.rect.height(),
        2.0 * view::cell_h() + crate::desk::PANEL_PAD * 2.0
    );
    let surface = Rect::from_wh(browser.size.0, browser.size.1);
    assert!(surface.contains(panel.rect), "{panel:?} in {surface:?}");
    assert!(panel.max_scroll() > 0.0);
}

/// The keyboard stops at the tile; Return opens the panel and the arrows
/// then walk the tile's items and no others, scrolling the panel to keep
/// them in view; Escape closes it again.
#[test]
fn the_keyboard_reaches_the_tile_and_walks_the_panel() {
    let (mut browser, _folder) = desk(80, Overflow::Stack);
    let tile = browser.desk_overflow().unwrap().tile;

    // The first press lands on the first icon, the next runs to the end.
    browser.move_cursor(100_000, false);
    browser.move_cursor(100_000, false);
    assert_eq!(browser.columns[0].cursor, Some(tile.first));
    assert!(browser.cursor_on_closed_tile());

    browser.open_selection();
    assert!(browser.desk_overflow().unwrap().panel.is_some());
    assert!(!browser.cursor_on_closed_tile());

    browser.move_cursor(100_000, false);
    assert_eq!(browser.columns[0].cursor, Some(79));
    let (panel, scroll) = browser.desk_overflow().unwrap().panel.unwrap();
    assert_eq!(scroll, panel.max_scroll());
    browser.move_cursor(-100_000, false);
    assert_eq!(browser.columns[0].cursor, Some(tile.first));
    let (_, scroll) = browser.desk_overflow().unwrap().panel.unwrap();
    assert_eq!(scroll, 0.0);

    assert!(browser.close_overflow_panel());
    assert!(browser.desk_overflow().unwrap().panel.is_none());
}

/// Closing hands the panel to its exit: still on screen, going back into
/// the tile, and gone once that has run.
#[test]
fn a_closed_panel_goes_back_into_its_tile() {
    let (mut browser, _folder) = desk(80, Overflow::Stack);
    browser.open_overflow_panel();
    let open = browser.overflow_shown().expect("on screen");
    assert!(!open.closing);
    assert!(browser.close_overflow_panel());
    let closing = browser.overflow_shown().expect("still on screen");
    assert!(closing.closing);
    assert_eq!(closing.resting(), open.resting());
    // At its end the panel is the tile's icon again.
    let end = super::desk_overflow::OverflowShown { t: 1.0, ..closing };
    let rect = end.rect();
    assert!((rect.center_x() - closing.anchor.center_x()).abs() < 1.0);
    assert!((rect.center_y() - closing.anchor.center_y()).abs() < 1.0);
    assert_eq!(end.opacity(), 0.0);
    assert!(!browser.close_overflow_panel());
}

/// The panel scrolls the way every list in Files does: a touchpad's
/// stream glides on after the fingers lift.
#[test]
fn the_panel_scrolls_with_momentum() {
    let (mut browser, _folder) = desk(80, Overflow::Stack);
    browser.open_overflow_panel();
    let session = browser.overflow_panel.as_mut().unwrap();
    for _ in 0..5 {
        session.scroll.on_wheel(4.0);
        session.scroll.advance(1.0 / 60.0);
    }
    session.scroll.on_wheel_end();
    let lifted = session.scroll.offset();
    assert!(lifted > 0.0);
    session.scroll.advance(1.0 / 60.0);
    assert!(session.scroll.offset() > lifted, "no glide after the lift");
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

/// A panel whose tile is gone is forgotten, not reopened when a tile forms
/// again.
#[test]
fn a_panel_without_its_tile_is_forgotten() {
    let (mut browser, _folder) = desk(80, Overflow::Stack);
    browser.open_overflow_panel();
    browser.desk_config.as_mut().unwrap().overflow = Overflow::Scroll;
    browser.sync_scroll_metrics();
    browser.desk_config.as_mut().unwrap().overflow = Overflow::Stack;
    assert!(browser.desk_overflow().unwrap().panel.is_none());
}
