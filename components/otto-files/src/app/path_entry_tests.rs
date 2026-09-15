use super::*;
use std::path::PathBuf;

/// A real directory, swept up when the test ends. The listing is read off
/// a worker thread and completion reads the filesystem directly, so both
/// need something actually on disk.
struct TempDir(PathBuf);

impl TempDir {
    fn holding(files: &[&str], dirs: &[&str]) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);

        let path = std::env::temp_dir().join(format!(
            "otto-files-path-entry-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        for name in files {
            std::fs::write(path.join(name), b"").expect("temp file");
        }
        for name in dirs {
            std::fs::create_dir_all(path.join(name)).expect("temp subdir");
        }
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A browser over the directory, with the first listing already in.
fn browser_over(files: &[&str], dirs: &[&str]) -> (Browser, TempDir) {
    let dir = TempDir::holding(files, dirs);
    let mut browser = Browser::new(dir.0.clone());
    for _ in 0..500 {
        if browser.columns[0].poll() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(!browser.columns[0].loading(), "listing never arrived");
    (browser, dir)
}

/// What Ctrl+L does: the field opens on where you already are, whole
/// value selected so typing replaces it, and with the separator already
/// there so the first Tab completes a child.
#[test]
fn the_field_opens_on_the_current_directory() {
    let (mut browser, dir) = browser_over(&["a.txt"], &[]);
    browser.open_path_entry();

    let input = browser.path_entry.as_ref().expect("field never opened");
    assert_eq!(input.value(), format!("{}/", dir.0.display()));
    assert_eq!(input.state.selection(), 0..input.value().chars().count());
}

/// Escape puts the title back and leaves the location alone — an
/// abandoned path is not a navigation.
#[test]
fn escape_leaves_the_location_alone() {
    let (mut browser, dir) = browser_over(&["a.txt"], &["sub"]);
    browser.open_path_entry();
    browser
        .path_entry
        .as_mut()
        .unwrap()
        .set_value(dir.0.join("sub").to_string_lossy().into_owned());
    browser.cancel_path_entry();

    assert!(browser.path_entry.is_none());
    assert_eq!(browser.current_path(), dir.0);
}

/// A directory is opened, and the field goes away with it.
#[test]
fn a_typed_directory_is_opened() {
    let (mut browser, dir) = browser_over(&[], &["sub"]);
    browser.open_path_entry();
    browser
        .path_entry
        .as_mut()
        .unwrap()
        .set_value(dir.0.join("sub").to_string_lossy().into_owned());
    browser.commit_path_entry();

    assert!(browser.path_entry.is_none(), "the field stayed up");
    assert_eq!(browser.columns[0].path, dir.0.join("sub"));
}

/// A path to a *file* — what pasting one out of a terminal usually gives
/// you — opens the folder holding it with the file selected. The listing
/// arrives later, so the row is handed to `pending_pick` rather than
/// looked up in a snapshot that does not exist yet.
#[test]
fn a_typed_file_opens_its_parent_with_the_file_selected() {
    let (mut browser, dir) = browser_over(&["report.txt"], &["sub"]);
    browser.navigate_to(&dir.0.join("sub"));
    browser.open_path_entry();
    browser
        .path_entry
        .as_mut()
        .unwrap()
        .set_value(dir.0.join("report.txt").to_string_lossy().into_owned());
    browser.commit_path_entry();

    assert_eq!(browser.columns[0].path, dir.0);
    // The frame loop polls, re-fits the scroll views, then settles the
    // pick — a test has to do all three, in that order, or the listing
    // arrives with nobody to place the selection in it.
    for _ in 0..500 {
        browser.poll();
        browser.sync_scroll_metrics();
        browser.settle_pick();
        if browser.pending_pick.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(
        browser.selected_named(0, "report.txt"),
        "the file the path named was never selected"
    );
}

/// A path that is not there keeps the field up: retyping one character is
/// cheaper than typing the whole path again.
#[test]
fn a_missing_path_keeps_the_field_open() {
    let (mut browser, dir) = browser_over(&[], &[]);
    browser.open_path_entry();
    browser
        .path_entry
        .as_mut()
        .unwrap()
        .set_value(dir.0.join("nowhere").to_string_lossy().into_owned());
    browser.commit_path_entry();

    assert!(browser.path_entry.is_some(), "the field was dismissed");
    assert!(browser.status.is_some(), "nothing said why");
    assert_eq!(browser.current_path(), dir.0);
}

/// Tab against one match completes it whole.
#[test]
fn tab_completes_the_only_match() {
    let (mut browser, dir) = browser_over(&["alpha.txt", "beta.txt"], &[]);
    browser.open_path_entry();
    browser
        .path_entry
        .as_mut()
        .unwrap()
        .set_value(format!("{}/al", dir.0.display()));
    browser.complete_path_entry();

    let input = browser.path_entry.as_ref().unwrap();
    assert_eq!(input.value(), format!("{}/alpha.txt", dir.0.display()));
    assert_eq!(input.state.caret(), input.value().chars().count());
}

/// Tab against several stops at the prefix they share — the shell's
/// behaviour, and the reason completion is worth having at all.
#[test]
fn tab_stops_at_the_prefix_the_matches_share() {
    let (mut browser, dir) = browser_over(&["report-a.txt", "report-b.txt"], &[]);
    browser.open_path_entry();
    browser
        .path_entry
        .as_mut()
        .unwrap()
        .set_value(format!("{}/re", dir.0.display()));
    browser.complete_path_entry();

    assert_eq!(
        browser.path_entry.as_ref().unwrap().value(),
        format!("{}/report-", dir.0.display())
    );
}

/// A completed directory gains its separator, so Tab walks down a tree
/// without the user reaching for `/` between levels.
#[test]
fn a_completed_directory_gains_its_separator() {
    let (mut browser, dir) = browser_over(&[], &["projects"]);
    browser.open_path_entry();
    browser
        .path_entry
        .as_mut()
        .unwrap()
        .set_value(format!("{}/pro", dir.0.display()));
    browser.complete_path_entry();

    assert_eq!(
        browser.path_entry.as_ref().unwrap().value(),
        format!("{}/projects/", dir.0.display())
    );
}

/// A dotfile is only a candidate once the dot has been typed. Otherwise
/// completion in a home directory offers configuration folders first.
#[test]
fn dotfiles_complete_only_once_the_dot_is_typed() {
    let (mut browser, dir) = browser_over(&[".hidden", "visible"], &[]);
    browser.open_path_entry();

    browser
        .path_entry
        .as_mut()
        .unwrap()
        .set_value(format!("{}/", dir.0.display()));
    browser.complete_path_entry();
    assert_eq!(
        browser.path_entry.as_ref().unwrap().value(),
        format!("{}/visible", dir.0.display()),
        "the dotfile was offered unasked"
    );

    browser
        .path_entry
        .as_mut()
        .unwrap()
        .set_value(format!("{}/.h", dir.0.display()));
    browser.complete_path_entry();
    assert_eq!(
        browser.path_entry.as_ref().unwrap().value(),
        format!("{}/.hidden", dir.0.display())
    );
}

/// A bare name resolves against the directory on screen, and `~` against
/// home — the two shorthands anyone typing a path expects to work.
#[test]
fn a_bare_name_resolves_against_the_open_directory() {
    let (browser, dir) = browser_over(&[], &["sub"]);
    assert_eq!(browser.resolve_typed_path("sub"), Some(dir.0.join("sub")));
    assert_eq!(browser.resolve_typed_path("  "), None);
    if let Some(home) = model::home_dir() {
        assert_eq!(browser.resolve_typed_path("~"), Some(home.clone()));
        assert_eq!(
            browser.resolve_typed_path("~/Music"),
            Some(home.join("Music"))
        );
    }
}

/// The row a path lands on has to be *visible*, not merely selected.
///
/// The scroll views take their viewport and content length during
/// render, so anything reached from `poll` is working with metrics from
/// before the listing landed — a reveal against those clamps short of a
/// row that sorted to the bottom. The frame loop re-fits them between
/// the poll and the settle for exactly this reason; this pins that
/// order, since a Ctrl+L onto a file deep in a long directory is the
/// case that shows it.
#[test]
fn a_typed_file_is_scrolled_into_view() {
    let names: Vec<String> = (0..200).map(|i| format!("aaa{i:03}.txt")).collect();
    let mut files: Vec<&str> = names.iter().map(String::as_str).collect();
    files.push("zzz-last.txt");
    let (mut browser, dir) = browser_over(&files, &["sub"]);
    browser.mode = ViewMode::List;
    browser.size = (900.0, 400.0);
    browser.navigate_to(&dir.0.join("sub"));

    browser.open_path_entry();
    browser
        .path_entry
        .as_mut()
        .unwrap()
        .set_value(dir.0.join("zzz-last.txt").to_string_lossy().into_owned());
    browser.commit_path_entry();

    for _ in 0..500 {
        browser.poll();
        browser.sync_scroll_metrics();
        browser.settle_pick();
        if browser.pending_pick.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }

    let depth = browser.active;
    let index = browser.columns[depth].cursor.expect("cursor on the file");
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

#[test]
fn the_shared_prefix_of_nothing_is_nothing() {
    assert_eq!(common_prefix(&[]), "");
    assert_eq!(common_prefix(&["one".into()]), "one");
    assert_eq!(common_prefix(&["ab".into(), "ac".into()]), "a");
    assert_eq!(common_prefix(&["ab".into(), "zz".into()]), "");
    // Whole characters, never half of one.
    assert_eq!(common_prefix(&["éa".into(), "éb".into()]), "é");
}
