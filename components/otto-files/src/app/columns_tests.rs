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
