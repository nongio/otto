use super::*;

/// A directory of real files, swept up on drop — `move_to_trash` works on
/// the filesystem, so these cannot be faked.
struct Tmp(PathBuf);

impl Tmp {
    fn holding(names: &[&str]) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);

        let path = std::env::temp_dir().join(format!(
            "otto-files-delete-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        for name in names {
            std::fs::write(path.join(name), b"x").expect("temp file");
        }
        Self(path)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Drive the browser until every pane has finished reading, then land
/// whatever the last operation set aside — the two steps the frame loop
/// takes between one input and the next.
fn settle(browser: &mut Browser) {
    for _ in 0..1000 {
        browser.poll();
        if !browser.loading() {
            browser.settle_pick();
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("the listing never landed");
}

fn browser_over(names: &[&str]) -> (Browser, Tmp) {
    // Keeps the deletes out of the developer's own Trash.
    let _ = model::test_data_home();
    let dir = Tmp::holding(names);
    let mut browser = Browser::new(dir.0.clone());
    browser.mode = ViewMode::List;
    settle(&mut browser);
    (browser, dir)
}

/// The selected rows, by name and in view order. The selection itself is
/// keyed by path — see [`Entry::selection_key`] — but what these tests are
/// about is which rows, and they name their fixtures.
fn selected(browser: &Browser) -> Vec<String> {
    let depth = browser.active;
    let selection = &browser.columns[depth].selection;
    browser
        .visible(depth)
        .iter()
        .filter(|e| selection.contains(&e.selection_key()))
        .map(|e| e.name.clone())
        .collect()
}

fn row_of(browser: &Browser, name: &str) -> usize {
    browser
        .visible(browser.active)
        .iter()
        .position(|entry| entry.name == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing"))
}

#[test]
fn the_next_row_takes_the_selection() {
    let (mut browser, _dir) = browser_over(&["a.txt", "b.txt", "c.txt"]);
    browser.select(0, row_of(&browser, "b.txt"));

    browser.move_selected_to_trash();
    settle(&mut browser);

    assert_eq!(selected(&browser), vec!["c.txt".to_string()]);
    assert_eq!(browser.columns[0].cursor, Some(row_of(&browser, "c.txt")));
}

/// Quick View is anchored to the cursor, so a delete has to carry it over
/// to the row that takes the deleted one's place — otherwise the panel
/// sits there previewing a file that is now in the Trash.
#[test]
fn quick_view_follows_the_delete_to_the_next_row() {
    let (mut browser, _dir) = browser_over(&["a.txt", "b.txt", "c.txt"]);
    browser.select(0, row_of(&browser, "b.txt"));
    browser.begin_quickview().expect("a file to preview");

    browser.move_selected_to_trash();
    settle(&mut browser);

    assert!(
        browser.take_quickview_follow(),
        "the host is asked for a fresh decode"
    );
    assert!(browser.quickview.is_some(), "the panel stays up");
    assert_eq!(
        browser.selected_entry().map(|e| e.name),
        Some("c.txt".to_string()),
        "and it is the survivor that gets decoded"
    );
}

/// Deleting the only file leaves no row to stand on, so the panel goes
/// away rather than hanging over an empty pane.
#[test]
fn quick_view_closes_when_the_delete_leaves_nothing() {
    let (mut browser, _dir) = browser_over(&["only.txt"]);
    browser.select(0, 0);
    browser.begin_quickview().expect("a file to preview");

    browser.move_selected_to_trash();
    settle(&mut browser);

    assert!(!browser.take_quickview_follow(), "nothing left to decode");
    assert!(browser.quickview.is_none(), "the panel is dismissed");
}

/// Nothing below the deleted row, so the selection steps back up rather
/// than being left nowhere.
#[test]
fn deleting_the_last_row_falls_back_to_the_one_above() {
    let (mut browser, _dir) = browser_over(&["a.txt", "b.txt", "c.txt"]);
    browser.select(0, row_of(&browser, "c.txt"));

    browser.move_selected_to_trash();
    settle(&mut browser);

    assert_eq!(selected(&browser), vec!["b.txt".to_string()]);
}

/// A multi-selection skips its whole run: the survivor below the *last*
/// deleted row is the one that takes over.
#[test]
fn a_run_of_rows_hands_over_to_the_first_survivor() {
    let (mut browser, _dir) = browser_over(&["a.txt", "b.txt", "c.txt", "d.txt"]);
    browser.select(0, row_of(&browser, "b.txt"));
    browser.extend_select(0, row_of(&browser, "c.txt"));

    browser.move_selected_to_trash();
    settle(&mut browser);

    assert_eq!(selected(&browser), vec!["d.txt".to_string()]);
}

/// The last file in a folder: there is no row left to stand on, so the
/// keyboard goes back to the pane holding the folder itself.
#[test]
fn emptying_a_folder_hands_the_keyboard_to_its_parent() {
    let _ = model::test_data_home();
    let dir = Tmp::holding(&[]);
    std::fs::create_dir_all(dir.0.join("sub")).expect("subdir");
    std::fs::write(dir.0.join("sub/only.txt"), b"x").expect("file");

    let mut browser = Browser::new(dir.0.clone());
    browser.mode = ViewMode::Columns;
    settle(&mut browser);
    browser.select(0, row_of(&browser, "sub"));
    settle(&mut browser);
    assert_eq!(browser.columns.len(), 2, "the child column is up");
    browser.active = 1;
    browser.select(1, 0);

    browser.move_selected_to_trash();
    settle(&mut browser);

    assert_eq!(browser.active, 0, "the parent has the keyboard");
    assert_eq!(selected(&browser), vec!["sub".to_string()]);
}
