use super::*;

/// A real directory: the entries a paste chooses between are read off the
/// disk, and `is_dir` is what the choice turns on.
struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

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

/// A folder holding one subfolder and two files, shown in column view.
fn browser_over_a_mixed_folder() -> (Browser, Tmp) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let root = std::env::temp_dir().join(format!(
        "otto-files-paste-target-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(root.join("target")).expect("temp dir");
    std::fs::write(root.join("a.txt"), b"x").expect("temp file");
    std::fs::write(root.join("b.txt"), b"x").expect("temp file");

    let mut browser = Browser::new(root.clone());
    browser.mode = ViewMode::Columns;
    settle(&mut browser);
    (browser, Tmp(root))
}

fn row_of(browser: &Browser, name: &str) -> usize {
    browser
        .visible(browser.active)
        .iter()
        .position(|entry| entry.name == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing"))
}

/// A clipboard holding one path that is not in the listing under test.
fn clipboard_of(paths: &[PathBuf]) -> model::Clipboard {
    model::Clipboard {
        paths: paths.to_vec(),
        cut: false,
    }
}

#[test]
fn a_selected_folder_is_where_the_paste_goes() {
    let (mut browser, dir) = browser_over_a_mixed_folder();
    let index = row_of(&browser, "target");
    browser.select(0, index);

    let clip = clipboard_of(&[PathBuf::from("/elsewhere/photo.png")]);
    assert_eq!(browser.paste_destination(&clip), dir.0.join("target"));
}

/// List and icon view show one directory at a time, and that directory is
/// where a paste goes however the selection happens to be sitting.
#[test]
fn a_selected_folder_outside_column_view_does_not_take_the_paste() {
    for mode in [ViewMode::List, ViewMode::Grid] {
        let (mut browser, dir) = browser_over_a_mixed_folder();
        browser.mode = mode;
        let index = row_of(&browser, "target");
        browser.select(0, index);

        let clip = clipboard_of(&[PathBuf::from("/elsewhere/photo.png")]);
        assert_eq!(browser.paste_destination(&clip), dir.0, "{mode:?}");
    }
}

#[test]
fn a_selected_file_leaves_the_paste_in_the_folder_on_screen() {
    let (mut browser, dir) = browser_over_a_mixed_folder();
    let index = row_of(&browser, "a.txt");
    browser.select(0, index);

    let clip = clipboard_of(&[PathBuf::from("/elsewhere/photo.png")]);
    assert_eq!(browser.paste_destination(&clip), dir.0);
}

#[test]
fn several_selected_entries_leave_the_paste_in_the_folder_on_screen() {
    let (mut browser, dir) = browser_over_a_mixed_folder();
    let first = row_of(&browser, "a.txt");
    browser.select(0, first);
    let last = row_of(&browser, "target");
    browser.extend_select(0, last);
    assert!(browser.selected_entries().len() > 1);

    let clip = clipboard_of(&[PathBuf::from("/elsewhere/photo.png")]);
    assert_eq!(browser.paste_destination(&clip), dir.0);
}

#[test]
fn nothing_selected_leaves_the_paste_in_the_folder_on_screen() {
    let (browser, dir) = browser_over_a_mixed_folder();
    let clip = clipboard_of(&[PathBuf::from("/elsewhere/photo.png")]);
    assert_eq!(browser.paste_destination(&clip), dir.0);
}

/// Copying a folder leaves it selected, and the copy belongs beside it: a
/// folder cannot be put inside itself.
#[test]
fn a_copied_folder_still_selected_is_duplicated_beside_itself() {
    let (mut browser, dir) = browser_over_a_mixed_folder();
    let index = row_of(&browser, "target");
    browser.select(0, index);

    let clip = clipboard_of(&[dir.0.join("target")]);
    assert_eq!(browser.paste_destination(&clip), dir.0);
}
