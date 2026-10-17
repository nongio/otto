use super::*;

/// A real directory holding a real subdirectory: the columns are read off the
/// disk by a worker, and the race this is about is that read.
struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Drive the browser until every pane has finished reading.
fn settle(browser: &mut Browser) {
    for _ in 0..2000 {
        browser.poll();
        if !browser.loading() {
            browser.settle_pick();
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("the listing never landed");
}

/// A folder with enough in it that reading it takes a moment — long enough
/// for a key press to land first, as it does in the window.
fn browser_over_a_folder() -> (Browser, Tmp) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let root = std::env::temp_dir().join(format!(
        "otto-files-columns-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(root.join("target")).expect("temp dir");
    for index in 0..64 {
        std::fs::write(root.join("target").join(format!("{index}.txt")), b"x").expect("temp file");
    }
    std::fs::write(root.join("a.txt"), b"x").expect("temp file");

    let mut browser = Browser::new(root.clone());
    browser.mode = ViewMode::Columns;
    settle(&mut browser);
    (browser, Tmp(root))
}

fn row_of(browser: &Browser, depth: usize, name: &str) -> usize {
    browser
        .visible(depth)
        .iter()
        .position(|entry| entry.name == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing"))
}

#[test]
fn right_into_a_folder_still_being_read_takes_its_first_row() {
    let (mut browser, _dir) = browser_over_a_folder();
    let index = row_of(&browser, 0, "target");
    browser.select(0, index);

    // No settle: the pane the press steps into is still being read, which is
    // the ordinary case when the folder was reached from a file's selection.
    assert!(
        browser.loading(),
        "the child column should still be reading"
    );
    browser.move_lateral(1);
    assert_eq!(browser.active, 1);

    settle(&mut browser);
    assert!(browser.columns[1].path.ends_with("target"));
    assert_eq!(
        browser.columns[1].cursor,
        Some(0),
        "the pane the keyboard stepped into should hold the cursor"
    );
}

#[test]
fn a_descent_the_user_left_does_not_move_the_cursor_later() {
    let (mut browser, _dir) = browser_over_a_folder();
    let index = row_of(&browser, 0, "target");
    browser.select(0, index);
    browser.move_lateral(1);
    // Straight back out again, before the listing lands.
    browser.move_lateral(-1);

    settle(&mut browser);
    assert_eq!(browser.active, 0);
    assert_eq!(browser.columns[1].cursor, None);
}

/// Three columns, each with a row in it: the root, `a` inside it and `b`
/// inside that.
fn browser_three_deep() -> (Browser, Tmp) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let root = std::env::temp_dir().join(format!(
        "otto-files-three-deep-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(root.join("a").join("b")).expect("temp dirs");
    std::fs::write(root.join("a").join("b").join("c.txt"), b"x").expect("temp file");

    let mut browser = Browser::new(root.clone());
    browser.mode = ViewMode::Columns;
    browser.size.0 = 2000.0;
    settle(&mut browser);
    let a = row_of(&browser, 0, "a");
    browser.select(0, a);
    settle(&mut browser);
    let b = row_of(&browser, 1, "b");
    browser.select(1, b);
    settle(&mut browser);
    assert_eq!(browser.columns.len(), 3, "three columns should be open");
    (browser, Tmp(root))
}

#[test]
fn dragging_a_divider_resizes_only_its_own_column() {
    let (mut browser, _dir) = browser_three_deep();
    let before: Vec<Rect> = (0..3).map(|depth| browser.entry_rect(depth, 0)).collect();

    let delta = 40.0;
    browser.miller_divider_press(1, 500.0);
    assert!(browser.miller_divider_drag(500.0 + delta));

    let after: Vec<Rect> = (0..3).map(|depth| browser.entry_rect(depth, 0)).collect();
    assert_eq!(after[0], before[0], "the column left of the divider moved");
    assert_eq!(
        after[1].width(),
        before[1].width() + delta,
        "the dragged column should follow the pointer exactly"
    );
    assert_eq!(
        after[2].width(),
        before[2].width(),
        "the column right of the divider changed width"
    );
    assert_eq!(
        after[2].left,
        before[2].left + delta,
        "the column right of the divider should shift by the drag"
    );
}
