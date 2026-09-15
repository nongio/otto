use super::*;

struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "otto-files-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        Self(path)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

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

/// Throw `name` away and open the Trash window on it, with that row
/// selected. Returns the window and the path the file came from.
fn trashed(tag: &str) -> (Browser, Tmp, PathBuf) {
    let _ = model::test_data_home();
    let dir = Tmp::new(tag);
    // Named after the test. The can is shared with every other test in
    // this binary, and two of them trashing a file of the same name race
    // for it: `first_free_name` checks and then moves, so both can pick
    // the plain name and the second's sidecar wins.
    let origin = dir.0.join(format!("{tag}.txt"));
    std::fs::write(&origin, b"body").unwrap();

    let result = model::move_to_trash(std::slice::from_ref(&origin));
    assert_eq!(result.trashed, 1, "{:?}", result.errors);
    let Some(model::Change::Trashed { to, .. }) = result.changes.first() else {
        panic!("nothing was trashed");
    };
    let name = to.file_name().unwrap().to_string_lossy().into_owned();

    let mut browser = Browser::listing_the_trash();
    settle(&mut browser);
    let index = browser
        .visible(0)
        .iter()
        .position(|e| e.name == name)
        .expect("the trashed file is listed");
    browser.select(0, index);
    (browser, dir, origin)
}

#[test]
fn put_back_returns_the_row_to_where_it_came_from() {
    let (mut browser, _dir, origin) = trashed("trashwin-putback");

    browser.put_back_selection();
    settle(&mut browser);

    assert!(origin.exists(), "the file is back: {:?}", browser.status);
    assert_eq!(std::fs::read_to_string(&origin).unwrap(), "body");
}

/// The origin is what the Trash's third column reads out, in place of the
/// Kind the browser shows there.
#[test]
fn a_row_carries_where_it_came_from() {
    let (browser, _dir, origin) = trashed("trashwin-origin");

    let entry = browser.selected_entry().expect("a selected row");
    assert_eq!(
        entry.origin.as_deref(),
        Some(origin.as_path()),
        "the sidecar was read back"
    );
}

/// Opening would hand a thrown-away file to an application, and editing
/// it there would edit it inside the can.
#[test]
fn a_trashed_file_cannot_be_opened() {
    let (mut browser, _dir, _origin) = trashed("trashwin-open");

    browser.open_selection();

    assert!(browser.opening.is_none(), "nothing was launched");
    assert!(browser.status.is_some(), "and it says why");
}

/// Renaming would rewrite the name the sidecar is keyed on, and Put Back
/// would then have nothing to read.
#[test]
fn a_trashed_file_cannot_be_renamed() {
    let (mut browser, _dir, _origin) = trashed("trashwin-rename");

    browser.start_rename();

    assert!(browser.rename.is_none(), "no rename field opened");
    assert!(browser.status.is_some(), "and it says why");
}

/// A paste would put files in the can with no sidecar — rows that could
/// never be put back.
#[test]
fn nothing_can_be_pasted_into_the_trash() {
    let (mut browser, dir, _origin) = trashed("trashwin-paste");
    let stray = dir.0.join("stray.txt");
    std::fs::write(&stray, b"x").unwrap();
    browser.clipboard = model::Clipboard {
        paths: vec![stray],
        cut: false,
    };

    browser.paste();
    settle(&mut browser);

    assert!(
        !browser.visible(0).iter().any(|e| e.name == "stray.txt"),
        "the can took nothing"
    );
}

/// Delete in the Trash destroys, so it asks first — every time, because
/// nothing can put the file back afterwards.
#[test]
fn delete_asks_before_it_destroys() {
    let (mut browser, _dir, _origin) = trashed("trashwin-delete");
    let victim = browser.selected_entry().expect("a row").path;

    browser.delete_key();

    let sheet = browser.confirm.as_ref().expect("a confirmation went up");
    assert!(
        matches!(&sheet.action, ConfirmAction::DeleteForever(paths) if paths == std::slice::from_ref(&victim)),
        "it is about the selected row"
    );
    assert!(victim.exists(), "and nothing has happened yet");
}

/// Emptying is the same question about the whole can.
#[test]
fn emptying_asks_before_it_destroys() {
    let (mut browser, _dir, _origin) = trashed("trashwin-empty");

    browser.ask_empty_trash();

    let sheet = browser.confirm.as_ref().expect("a confirmation went up");
    assert!(matches!(sheet.action, ConfirmAction::EmptyTrash));
    assert!(
        !browser.visible(0).is_empty(),
        "and nothing has happened yet"
    );
}

/// The browser's Delete is a trip to the Trash, which is undoable and so
/// asks nothing. Only the Trash's own Delete puts a question up.
#[test]
fn the_browser_still_deletes_without_asking() {
    let _ = model::test_data_home();
    let dir = Tmp::new("trashwin-browser-delete");
    std::fs::write(dir.0.join("a.txt"), b"x").unwrap();

    let mut browser = Browser::new(dir.0.clone());
    browser.mode = ViewMode::List;
    settle(&mut browser);
    browser.select(0, 0);

    browser.delete_key();

    assert!(browser.confirm.is_none(), "no question for a trip to Trash");
    assert!(!dir.0.join("a.txt").exists());
}
