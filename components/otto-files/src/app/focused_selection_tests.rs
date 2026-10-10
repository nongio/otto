use super::*;

/// A folder holding `names`, swept up on drop.
struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn browser_over(names: &[&str]) -> (Browser, Tmp) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let dir = std::env::temp_dir().join(format!(
        "otto-files-focused-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    // A name ending in a slash is a folder.
    for name in names {
        match name.strip_suffix('/') {
            Some(folder) => std::fs::create_dir(dir.join(folder)).expect("temp dir"),
            None => std::fs::write(dir.join(name), b"x").expect("temp file"),
        }
    }
    let mut browser = Browser::new(dir.clone());
    browser.mode = ViewMode::List;
    for _ in 0..1000 {
        browser.poll();
        if !browser.loading() {
            return (browser, Tmp(dir));
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("the listing never landed");
}

fn row_of(browser: &Browser, name: &str) -> usize {
    browser
        .visible(0)
        .iter()
        .position(|entry| entry.name == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing"))
}

#[test]
fn the_focused_window_answers_with_its_selected_paths() {
    let (mut browser, dir) = browser_over(&["a.txt", "b.txt", "c.txt"]);
    browser.select(0, row_of(&browser, "a.txt"));
    browser.extend_select(0, row_of(&browser, "b.txt"));

    let expected: Vec<String> = ["a.txt", "b.txt"]
        .iter()
        .map(|name| dir.0.join(name).to_string_lossy().into_owned())
        .collect();
    assert_eq!(browser.focused_selection(true), Some(expected));
    assert_eq!(
        browser.focused_selection(false),
        None,
        "a window without the keyboard does not answer"
    );
}

#[test]
fn the_folder_being_viewed_is_not_a_selection() {
    let (browser, _dir) = browser_over(&["a.txt"]);
    assert_eq!(
        browser.focused_selection(true),
        Some(Vec::new()),
        "the focused window answers, with nothing selected"
    );
}

/// Ctrl+O on a folder goes into it here, as a double-click does, and hands
/// nothing to another application.
#[test]
fn ctrl_o_on_a_folder_opens_it_in_this_window() {
    for mode in [ViewMode::List, ViewMode::Grid, ViewMode::Photos] {
        let (mut browser, dir) = browser_over(&["notes.txt", "Trips/"]);
        browser.set_mode(mode);
        browser.select(0, row_of(&browser, "Trips"));

        browser.open_cursor_entry();

        assert_eq!(
            browser.columns.last().map(|c| c.path.clone()),
            Some(dir.0.join("Trips")),
            "{mode:?}: went into the folder"
        );
        assert!(browser.opening.is_none(), "{mode:?}: nothing was launched");
    }
}

/// The Photos info panel's turn buttons change the photo's orientation, and
/// Ctrl+Z puts it back.
#[test]
fn a_photo_turned_from_the_info_panel_is_undone() {
    use crate::orient::{orientation, Turn};
    let (mut browser, dir) = browser_over(&["shot.jpg"]);
    let shot = dir.0.join("shot.jpg");
    // A JPEG with no EXIF: start of image, an empty scan, end of image.
    std::fs::write(&shot, [0xFF, 0xD8, 0xFF, 0xDA, 0, 2, 0xFF, 0xD9]).unwrap();
    browser.set_mode(ViewMode::Photos);
    browser.select(0, row_of(&browser, "shot.jpg"));

    // Asked for, then carried out off the UI thread: nothing is written
    // until the worker runs.
    browser.turn_selected_photo(Turn::Right);
    browser.turn_selected_photo(Turn::FlipHorizontal);
    assert_eq!(orientation(&shot).unwrap(), 1, "not on the UI thread");
    browser.run_photo_turns();
    assert_eq!(
        orientation(&shot).unwrap(),
        5,
        "a quarter turn, then a flip"
    );

    browser.undo_last();
    assert_eq!(orientation(&shot).unwrap(), 6, "the flip taken back");
    browser.undo_last();
    assert_eq!(orientation(&shot).unwrap(), 1, "and the turn");

    browser.turn_selected_photo(Turn::Right);
    browser.run_photo_turns();
    assert_eq!(orientation(&shot).unwrap(), 6, "a quarter turn clockwise");
    browser.turn_selected_photo(Turn::FlipHorizontal);
    browser.run_photo_turns();
    assert_eq!(orientation(&shot).unwrap(), 5);

    browser.undo_last();
    assert_eq!(orientation(&shot).unwrap(), 6, "the flip taken back");
    browser.undo_last();
    assert_eq!(orientation(&shot).unwrap(), 1, "and the turn");
}
