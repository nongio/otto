//! The picker's toolbar: the location menu and New Folder.

use super::typeahead_tests::browser_over;
use super::*;

fn as_picker(browser: &mut Browser, mode: picker::Mode, directory: bool) {
    let (responder, receiver) = tokio::sync::oneshot::channel();
    // Kept alive for the test: a dropped receiver is a request nobody waits on.
    std::mem::forget(receiver);
    browser.picker = Some(picker::Session::new(
        picker::Request {
            mode,
            handle: String::new(),
            app_id: String::new(),
            parent_window: String::new(),
            title: String::new(),
            accept_label: String::new(),
            multiple: false,
            directory,
            modal: false,
            current_name: String::new(),
            current_folder: None,
            current_file: None,
            files: Vec::new(),
            filters: Vec::new(),
            current_filter: 0,
            choices: Vec::new(),
        },
        responder,
    ));
}

/// Poll until the new folder's rename field is up.
fn await_rename(browser: &mut Browser) {
    for _ in 0..500 {
        if browser.poll() && browser.rename.is_some() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("rename field never opened");
}

#[test]
fn the_location_menu_lists_where_you_are_then_every_folder_above() {
    let (browser, dir) = browser_over(&[]);
    let ancestors = browser.location_ancestors();
    assert_eq!(ancestors.first(), Some(&dir.0));
    assert_eq!(ancestors.get(1).map(PathBuf::as_path), dir.0.parent());
    assert_eq!(ancestors.last().map(PathBuf::as_path), Some(Path::new("/")));
}

#[test]
fn choosing_a_folder_above_goes_there() {
    let (mut browser, dir) = browser_over(&[]);
    as_picker(&mut browser, picker::Mode::Open, false);
    browser.location_open = true;
    browser.location_choose(1);
    assert!(!browser.location_open);
    assert_eq!(
        Some(browser.columns[browser.active].path.as_path()),
        dir.0.parent()
    );
}

#[test]
fn choosing_where_you_already_are_only_closes_the_menu() {
    let (mut browser, dir) = browser_over(&[]);
    as_picker(&mut browser, picker::Mode::Open, false);
    browser.location_open = true;
    browser.location_choose(0);
    assert!(!browser.location_open);
    assert_eq!(browser.columns[browser.active].path, dir.0);
    assert!(browser.back.is_empty(), "no step was taken");
}

#[test]
fn the_root_is_named_by_its_path() {
    assert_eq!(picking::location_label(Path::new("/")), "/");
    assert_eq!(picking::location_label(Path::new("/home/me")), "me");
}

/// A sidebar place keeps its own icon in the menu; home and the root get the
/// path bar's; any other directory is a folder.
#[test]
fn each_location_wears_the_icon_it_has_elsewhere() {
    let places = vec![model::Place {
        label: "Music".to_string(),
        path: PathBuf::from("/home/me/Music"),
        icon: "folder-music".to_string(),
        recent: false,
    }];
    let home = Some(Path::new("/home/me"));
    let first = |path: &str| picking::location_icon(Path::new(path), &places, home)[0].clone();
    assert_eq!(first("/home/me/Music"), "folder-music");
    assert_eq!(first("/home/me"), "user-home");
    assert_eq!(first("/"), "drive-harddisk");
    assert_eq!(first("/home"), "folder");
    // A themed icon a theme lacks still lands on a folder.
    assert!(
        picking::location_icon(Path::new("/home/me/Music"), &places, home)
            .contains(&"folder".to_string())
    );
}

/// Save: the new folder is named in place, and naming it goes into it —
/// the save lands in the directory being viewed, so a folder left merely
/// selected would not be where the file goes.
#[test]
fn a_folder_made_while_saving_is_named_then_entered() {
    let (mut browser, dir) = browser_over(&["a.txt"]);
    as_picker(&mut browser, picker::Mode::Save, false);
    browser.picker_new_folder();
    await_rename(&mut browser);

    let session = browser.rename.as_mut().unwrap();
    session.input.set_value("Invoices".to_string());
    browser.commit_rename();

    assert_eq!(
        browser.columns[browser.active].path,
        dir.0.join("Invoices"),
        "the save field now points into the new folder"
    );
    assert!(dir.0.join("Invoices").is_dir());
}

#[test]
fn escaping_the_name_keeps_the_folder_and_stays_put() {
    let (mut browser, dir) = browser_over(&[]);
    as_picker(&mut browser, picker::Mode::Save, false);
    browser.picker_new_folder();
    await_rename(&mut browser);
    browser.cancel_rename();
    assert_eq!(browser.columns[browser.active].path, dir.0);
    assert!(!browser.enter_after_rename);
}

/// Opening files: an empty new folder has nothing to pick, so the picker
/// stays beside it.
#[test]
fn a_folder_made_while_opening_files_is_named_but_not_entered() {
    let (mut browser, dir) = browser_over(&[]);
    as_picker(&mut browser, picker::Mode::Open, false);
    browser.picker_new_folder();
    await_rename(&mut browser);
    browser.commit_rename();
    assert_eq!(browser.columns[browser.active].path, dir.0);
}

#[test]
fn the_toolbar_finds_its_two_controls() {
    let width = 900.0;
    let location = view::location_rect(width);
    let button = view::new_folder_rect(width);
    assert_eq!(
        view::picker_toolbar_at(location.center_x(), location.center_y(), width),
        Some(view::ToolbarButton::Location)
    );
    assert_eq!(
        view::picker_toolbar_at(button.center_x(), button.center_y(), width),
        Some(view::ToolbarButton::NewFolder)
    );
    assert!(
        button.left > location.right,
        "side by side, not overlapping"
    );
}
