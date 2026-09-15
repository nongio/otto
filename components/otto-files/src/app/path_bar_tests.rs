
use super::*;

fn icons_of(crumbs: &[view::PathCrumb]) -> Vec<&str> {
    crumbs
        .iter()
        .map(|c| c.icon.first().map(String::as_str).unwrap_or(""))
        .collect()
}

fn labels_of(crumbs: &[view::PathCrumb]) -> Vec<&str> {
    crumbs.iter().map(|c| c.label.as_str()).collect()
}

/// Every component becomes a crumb, and each one carries the path that
/// far — that is what makes a click on it a step back up the trail.
#[test]
fn each_crumb_carries_the_path_down_to_itself() {
    let crumbs = crumbs_for(
        Path::new("/home/ada/Pictures/holiday.png"),
        vec!["image-png".to_string()],
        false,
        None,
    );

    assert_eq!(
        labels_of(&crumbs),
        ["/", "home", "ada", "Pictures", "holiday.png"]
    );
    assert_eq!(
        crumbs.iter().map(|c| c.path.clone()).collect::<Vec<_>>(),
        [
            PathBuf::from("/"),
            PathBuf::from("/home"),
            PathBuf::from("/home/ada"),
            PathBuf::from("/home/ada/Pictures"),
            PathBuf::from("/home/ada/Pictures/holiday.png"),
        ]
    );
    // The leaf is a file, so it leads nowhere; everything above it does.
    assert_eq!(
        crumbs.iter().map(|c| c.is_dir).collect::<Vec<_>>(),
        [true, true, true, true, false]
    );
    // The leaf wears the entry's own icon; the trail above it is folders,
    // and the root is the volume they hang off.
    assert_eq!(
        icons_of(&crumbs),
        ["drive-harddisk", "folder", "folder", "folder", "image-png"]
    );
}

/// A selected folder is a crumb like any other, and it leads somewhere.
#[test]
fn a_selected_folder_ends_the_trail_as_a_step() {
    let crumbs = crumbs_for(
        Path::new("/home/ada/Pictures"),
        vec!["folder".to_string()],
        true,
        None,
    );
    assert!(crumbs.last().unwrap().is_dir);
    assert_eq!(
        crumbs.last().unwrap().path,
        PathBuf::from("/home/ada/Pictures")
    );
}

/// The home directory keeps its name and takes the sidebar's icon, and
/// only that one component does — a folder deeper down that happens to
/// share the name is an ordinary folder.
#[test]
fn home_wears_the_sidebar_icon_and_keeps_its_name() {
    let crumbs = crumbs_for(
        Path::new("/home/ada/ada"),
        vec!["folder".to_string()],
        true,
        Some(Path::new("/home/ada")),
    );

    assert_eq!(labels_of(&crumbs), ["/", "home", "ada", "ada"]);
    assert_eq!(
        icons_of(&crumbs),
        ["drive-harddisk", "folder", "user-home", "folder"]
    );
    assert_eq!(crumbs[2].path, PathBuf::from("/home/ada"));
}

fn entry(dir: &str, name: &str, is_dir: bool) -> Entry {
    Entry {
        name: name.to_string(),
        path: Path::new(dir).join(name),
        is_dir,
        is_symlink: false,
        hidden: false,
        kind: if is_dir {
            otto_kit::filetype::Kind::Folder
        } else {
            otto_kit::filetype::Kind::Image
        },
        size: Some(1),
        modified: None,
        origin: None,
    }
}

/// What the bar spells out is the *selection* — the whole point of it. A
/// window whose active column has one thing selected reads out that
/// thing's path, not the folder it is sitting in.
#[test]
fn one_selected_file_is_what_the_bar_spells_out() {
    let mut browser = Browser::new(PathBuf::from("/home/ada"));
    browser.columns[0].snapshot.entries = vec![
        entry("/home/ada", "holiday.png", false),
        entry("/home/ada", "notes.txt", false),
    ];

    // Nothing selected: the column's own directory.
    let (path, _, is_dir) = browser.path_bar_target().expect("a real folder");
    assert_eq!(path, PathBuf::from("/home/ada"));
    assert!(is_dir);

    // Keyed by path — see `Entry::selection_key` — because a listing that
    // merges folders can hold two files of the same name.
    browser.columns[0]
        .selection
        .insert("/home/ada/holiday.png".to_string());
    let (path, icon, is_dir) = browser.path_bar_target().expect("the selected file");
    assert_eq!(path, PathBuf::from("/home/ada/holiday.png"));
    assert!(!is_dir);
    assert!(!icon.is_empty(), "the leaf carries the entry's own icon");

    // More than one, and there is no single path to give: the bar falls
    // back to where they all are.
    browser.columns[0]
        .selection
        .insert("/home/ada/notes.txt".to_string());
    assert_eq!(
        browser.path_bar_target().map(|(path, ..)| path),
        Some(PathBuf::from("/home/ada"))
    );
}

/// The strip is chrome the file area stops short of, the same way it stops
/// short of the picker's action row.
#[test]
fn the_file_area_stops_above_the_strip() {
    let mut browser = Browser::new(std::env::temp_dir());
    browser.size = (1100.0, 700.0);

    assert_eq!(browser.path_bar_h(), view::PATH_BAR_H);
    assert_eq!(browser.content_h(), 700.0 - view::PATH_BAR_H);

    // The Trash window has no strip: every path in it would spell out the
    // same stretch of `.local/share/Trash`.
    browser.trash = true;
    assert_eq!(browser.path_bar_h(), 0.0);
    assert_eq!(browser.content_h(), 700.0);
    assert!(browser.path_crumbs().is_empty());
}
