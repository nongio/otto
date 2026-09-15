use super::*;
use std::path::PathBuf;
use std::time::Duration;

/// A real directory of empty files, swept up when the test ends.
///
/// The listing comes off a worker thread reading the filesystem, so
/// type-ahead can only be exercised against something actually on disk.
pub(super) struct TempDir(pub(super) PathBuf);

impl TempDir {
    pub(super) fn holding(names: &[&str]) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);

        let path = std::env::temp_dir().join(format!(
            "otto-files-typeahead-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        for name in names {
            std::fs::write(path.join(name), b"").expect("temp file");
        }
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A browser over `names`, with the first listing already in.
pub(super) fn browser_over(names: &[&str]) -> (Browser, TempDir) {
    let dir = TempDir::holding(names);
    let mut browser = Browser::new(dir.0.clone());
    // The frame loop is what normally polls the loader; a test has to.
    for _ in 0..500 {
        if browser.columns[0].poll() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(!browser.columns[0].loading(), "listing never arrived");
    (browser, dir)
}

/// New Folder creates the directory, then selects it with its name ready
/// to be typed over. The listing it has to find the folder in is read off
/// the main thread, so the selection cannot happen in the same call — it
/// waits for the re-read, which is exactly what this pins.
#[test]
fn new_folder_lands_in_rename_mode() {
    let (mut browser, _dir) = browser_over(&["a.txt"]);
    browser.mode = ViewMode::List;
    browser.new_folder();
    assert!(
        browser.rename.is_none(),
        "nothing to rename before the re-read"
    );

    for _ in 0..500 {
        if browser.poll() && browser.rename.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }

    let session = browser.rename.as_ref().expect("rename field never opened");
    let name = session
        .original
        .file_name()
        .expect("named folder")
        .to_string_lossy()
        .to_string();
    assert!(browser.selected_named(browser.active, &name));
    assert_eq!(session.input.value(), name);
}

/// A folder that sorts past the fold has to be scrolled to, or the rename
/// field opens off screen and the user types into something they cannot
/// see. "untitled folder" sorts last among a pile of folders named `aaa*`,
/// which is exactly the case that hides it.
#[test]
fn new_folder_scrolls_the_view_to_it() {
    let dir = TempDir::holding(&[]);
    for i in 0..200 {
        std::fs::create_dir_all(dir.0.join(format!("aaa{i:03}"))).expect("temp subdir");
    }
    let mut browser = Browser::new(dir.0.clone());
    for _ in 0..500 {
        if browser.columns[0].poll() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    browser.mode = ViewMode::List;
    browser.size = (900.0, 400.0);
    assert_eq!(browser.columns[0].scroll.offset(), 0.0, "starts at the top");

    browser.new_folder();
    for _ in 0..500 {
        if browser.poll() && browser.rename.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(browser.rename.is_some(), "rename field never opened");

    let depth = browser.active;
    let index = browser.columns[depth]
        .cursor
        .expect("cursor on the new folder");
    assert!(index > 0, "the folder sorted past the top");
    let offset = browser.columns[depth].scroll.offset();
    assert!(offset > 0.0, "view never scrolled: offset {offset}");
    let (top, item_h) = view::item_span(browser.size.0, browser.content_h(), browser.mode, index);
    let viewport = view::pane_viewport(
        browser.size.0,
        browser.content_h(),
        browser.mode,
        depth,
        browser.pan.offset(),
        browser.miller_w,
    );
    assert!(
        top >= offset && top + item_h <= offset + viewport.height(),
        "row {top}..{} outside the viewport at {offset}",
        top + item_h
    );
}

/// Ctrl+O is bound to the same call a double-click makes, so on a folder
/// it descends. Worth pinning because Return is *not* this — it renames —
/// and it would be an easy mistake to give opening back to Return and
/// leave the chord doing nothing.
#[test]
fn opening_a_folder_descends_into_it() {
    let dir = TempDir::holding(&["a.txt"]);
    let sub = dir.0.join("sub");
    std::fs::create_dir_all(sub.join("inner")).expect("child dir");

    let mut browser = Browser::new(dir.0.clone());
    for _ in 0..500 {
        if browser.columns[0].poll() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    // List view: descending replaces the column, so the deepest path is
    // the plainest statement of where the window ended up.
    browser.mode = ViewMode::List;
    let index = browser
        .visible(0)
        .iter()
        .position(|e| e.name == "sub")
        .expect("the folder is listed");
    browser.select(0, index);

    assert_eq!(browser.current_path(), dir.0);
    browser.open_selection();
    assert_eq!(browser.current_path(), sub);
}

/// Ctrl+N opens a new window at the default location, not wherever this
/// window happens to be pointed — a new window is a fresh start. Where the
/// window is browsing must not move the target.
#[test]
fn a_new_window_opens_at_the_default_location() {
    let (mut browser, dir) = browser_over(&["one.txt", "two.txt"]);
    let default = Browser::default_location();
    assert_eq!(browser.new_window_target(), Some(default.clone()));

    // Navigating somewhere else leaves it alone. `dir` is a temporary
    // directory, so it is never the default location, and a target that
    // followed the window would show up here.
    let child = dir.0.join("sub");
    std::fs::create_dir_all(&child).expect("child dir");
    browser.navigate_to(&child);
    for _ in 0..500 {
        if browser.columns.last_mut().is_some_and(|c| c.poll()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(browser.new_window_target(), Some(default));
    assert_ne!(
        browser.new_window_target().as_deref(),
        Some(child.as_path())
    );
}

fn at_cursor(browser: &Browser) -> Option<String> {
    let index = browser.columns[browser.active].cursor?;
    Some(browser.visible(browser.active)[index].name.clone())
}

/// Names chosen so that a prefix, a second character and a repeat all
/// have somewhere different to land.
const NAMES: &[&str] = &[
    "Alpha.txt",
    "apple.txt",
    "Banana.txt",
    "beta.txt",
    "Photo.png",
];

#[test]
fn a_character_selects_the_first_entry_starting_with_it() {
    let (mut browser, _dir) = browser_over(NAMES);
    browser.typeahead('b');
    // Matching ignores case, so a lowercase key reaches a capitalised name.
    assert_eq!(at_cursor(&browser).as_deref(), Some("Banana.txt"));
}

#[test]
fn a_second_character_narrows_the_same_word() {
    let (mut browser, _dir) = browser_over(NAMES);
    browser.typeahead('a');
    assert_eq!(at_cursor(&browser).as_deref(), Some("Alpha.txt"));
    browser.typeahead('p');
    assert_eq!(at_cursor(&browser).as_deref(), Some("apple.txt"));
}

#[test]
fn repeating_one_character_cycles_and_wraps() {
    let (mut browser, _dir) = browser_over(NAMES);
    browser.typeahead('b');
    assert_eq!(at_cursor(&browser).as_deref(), Some("Banana.txt"));
    browser.typeahead('b');
    assert_eq!(at_cursor(&browser).as_deref(), Some("beta.txt"));
    // Past the last match it comes back round rather than stopping dead.
    browser.typeahead('b');
    assert_eq!(at_cursor(&browser).as_deref(), Some("Banana.txt"));
}

#[test]
fn a_second_of_silence_starts_a_new_word() {
    let (mut browser, _dir) = browser_over(NAMES);
    browser.typeahead('a');
    assert_eq!(at_cursor(&browser).as_deref(), Some("Alpha.txt"));
    // Backdated rather than slept through: the expiry is a second, and no
    // test should take one.
    let (buffer, _) = browser.typeahead.take().expect("buffer");
    browser.typeahead = Some((buffer, std::time::Instant::now() - Duration::from_secs(2)));
    browser.typeahead('p');
    assert_eq!(at_cursor(&browser).as_deref(), Some("Photo.png"));
}

#[test]
fn a_name_that_matches_nothing_leaves_the_cursor_alone() {
    let (mut browser, _dir) = browser_over(NAMES);
    browser.typeahead('b');
    browser.typeahead('z');
    assert_eq!(at_cursor(&browser).as_deref(), Some("Banana.txt"));
    // The miss stays in the buffer: it is part of the word being typed.
    assert_eq!(
        browser.typeahead.as_ref().map(|(b, _)| b.as_str()),
        Some("bz")
    );
}

#[test]
fn an_empty_directory_is_not_a_panic() {
    let (mut browser, _dir) = browser_over(&[]);
    browser.typeahead('a');
    assert_eq!(at_cursor(&browser), None);
}
