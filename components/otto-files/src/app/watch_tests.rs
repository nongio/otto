
use super::*;

/// A real directory, swept up on drop. Watching is about the filesystem
/// changing, so these tests need one.
struct TempDir(PathBuf);

impl TempDir {
    fn holding(names: &[&str]) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);

        let path = std::env::temp_dir().join(format!(
            "otto-files-watch-app-{}-{}",
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

fn browser_over(names: &[&str]) -> (Browser, TempDir) {
    let dir = TempDir::holding(names);
    let mut browser = Browser::new(dir.0.clone());
    browser.mode = ViewMode::List;
    assert!(settle(&mut browser, |b| !b.loading()));
    (browser, dir)
}

fn at_cursor(browser: &Browser) -> Option<String> {
    let index = browser.columns[browser.active].cursor?;
    Some(browser.visible(browser.active)[index].name.clone())
}

/// Drive the browser's own poll until `done`, or give up. This is the
/// frame loop's job in a running window; a test has to do it by hand, and
/// a watch-driven refresh takes a debounce plus a worker read to land.
fn settle(browser: &mut Browser, done: impl Fn(&Browser) -> bool) -> bool {
    for _ in 0..400 {
        browser.poll();
        if done(browser) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    false
}

/// The whole point of watching: a file another application creates shows
/// up without the user asking for a refresh.
#[test]
fn a_file_created_underneath_appears_on_its_own() {
    let (mut browser, dir) = browser_over(&["one.txt"]);
    std::fs::write(dir.0.join("two.txt"), b"x").expect("write");

    let landed = settle(&mut browser, |b| {
        b.visible(0).iter().any(|e| e.name == "two.txt")
    });
    assert!(landed, "the new file never appeared");
}

/// A refresh must not disturb what the user is doing: the selection is
/// held by key and survives, and the cursor is put back on it even though
/// the new file sorts above it and moved every index down one.
#[test]
fn a_refresh_keeps_the_selection_and_the_cursor_together() {
    let (mut browser, dir) = browser_over(&["m.txt", "z.txt"]);
    browser.mode = ViewMode::List;
    let index = browser
        .visible(0)
        .iter()
        .position(|e| e.name == "z.txt")
        .expect("listed");
    browser.select(0, index);

    std::fs::write(dir.0.join("a.txt"), b"x").expect("write");
    let landed = settle(&mut browser, |b| {
        b.visible(0).iter().any(|e| e.name == "a.txt")
    });
    assert!(landed, "the new file never appeared");

    assert!(browser.selected_named(0, "z.txt"));
    assert_eq!(at_cursor(&browser).as_deref(), Some("z.txt"));
}

/// A refresh must not move the view. The listing is replaced in place,
/// so the scroll view — its offset and its measurements — is never rebuilt
/// out from under whoever is reading it.
#[test]
fn a_refresh_leaves_the_scroll_where_it_was() {
    let names: Vec<String> = (0..200).map(|i| format!("f{i:03}.txt")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let (mut browser, dir) = browser_over(&refs);

    // Measurements first: an offset means nothing against a pane that has
    // never been sized, and would be clamped straight back to zero.
    let column = &mut browser.columns[0];
    column
        .scroll
        .state
        .set_viewport(Rect::from_xywh(0.0, 0.0, 600.0, 400.0));
    column.scroll.state.set_content_length(4000.0);
    column.scroll.state.set_offset(750.0);
    let before = column.scroll.offset();
    assert!(before > 0.0);

    std::fs::write(dir.0.join("aaa.txt"), b"x").expect("write");
    let landed = settle(&mut browser, |b| {
        b.visible(0).iter().any(|e| e.name == "aaa.txt")
    });
    assert!(landed, "the new file never appeared");
    assert_eq!(browser.columns[0].scroll.offset(), before);
}

/// A directory that goes away takes its pane with it: the window lands on
/// the nearest place that still exists rather than showing a listing of
/// something that is not there.
#[test]
fn losing_the_directory_falls_back_to_an_ancestor() {
    let dir = TempDir::holding(&[]);
    let child = dir.0.join("sub");
    std::fs::create_dir_all(&child).expect("child dir");

    let mut browser = Browser::new(child.clone());
    assert!(settle(&mut browser, |b| !b.loading()));

    std::fs::remove_dir_all(&child).expect("remove");
    let moved = settle(&mut browser, |b| b.current_path() != child);
    assert!(moved, "the window stayed on a directory that is gone");
    assert_eq!(browser.current_path(), dir.0);
}
