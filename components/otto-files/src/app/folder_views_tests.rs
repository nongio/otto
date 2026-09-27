use super::typeahead_tests::TempDir;
use super::*;

/// A browser remembering views, over a folder holding `names` as folders.
fn browser_over_folders(names: &[&str]) -> (Browser, TempDir) {
    let dir = TempDir::holding(&[]);
    for name in names {
        std::fs::create_dir_all(dir.0.join(name)).unwrap();
    }
    let mut browser = Browser::new(dir.0.clone());
    settle(&mut browser);
    (browser, dir)
}

/// Let every column's read land.
fn settle(browser: &mut Browser) {
    for _ in 0..1000 {
        browser.poll();
        if !browser.loading() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("the listing never arrived");
}

fn index_of(browser: &Browser, depth: usize, name: &str) -> usize {
    browser
        .visible(depth)
        .iter()
        .position(|e| e.name == name)
        .unwrap_or_else(|| panic!("{name} is not listed"))
}

#[test]
fn a_chosen_view_is_remembered_and_put_back_on_arrival() {
    let (mut browser, dir) = browser_over_folders(&["pics", "code"]);
    let pics = dir.0.join("pics");
    browser.set_mode(ViewMode::List);
    browser.navigate_to(&pics);
    browser.choose_mode(ViewMode::Photos);
    assert_eq!(browser.folder_view(&pics), Some(ViewMode::Photos));

    // Elsewhere, in another view; coming back puts Photos back.
    browser.navigate_to(&dir.0.join("code"));
    browser.choose_mode(ViewMode::List);
    browser.navigate_to(&pics);
    assert_eq!(browser.mode, ViewMode::Photos);

    // A folder nothing was chosen for keeps the view the window is in.
    browser.navigate_to(&dir.0);
    assert_eq!(browser.mode, ViewMode::Photos);
    assert_eq!(browser.folder_view(&dir.0), None);

    // Back to code, which was List.
    browser.navigate_to(&dir.0.join("code"));
    assert_eq!(browser.mode, ViewMode::List);
}

#[test]
fn a_view_the_window_takes_on_its_own_is_not_a_choice() {
    let (mut browser, dir) = browser_over_folders(&["a"]);
    browser.set_mode(ViewMode::List);
    // Recent forces the grid; it is not somewhere, and nothing is kept.
    browser.enter_recent();
    assert_eq!(browser.mode, ViewMode::Grid);
    browser.choose_mode(ViewMode::Grid);
    browser.leave_synthetic_to(&dir.0);
    assert!(browser.folder_views.is_empty());
    // A mode set by the window itself, not the user, is not recorded either.
    browser.set_mode(ViewMode::Photos);
    assert!(browser.folder_views.is_empty());
}

#[test]
fn opening_a_folder_from_a_one_folder_view_applies_its_view() {
    let (mut browser, dir) = browser_over_folders(&["pics"]);
    let pics = dir.0.join("pics");
    browser.folder_views.push((pics.clone(), ViewMode::Photos));
    browser.set_mode(ViewMode::List);
    let index = index_of(&browser, 0, "pics");
    browser.select(0, index);
    browser.open_selection();
    assert_eq!(browser.current_path(), pics);
    assert_eq!(browser.mode, ViewMode::Photos);
    // Up to the parent, which has no view of its own: Photos stays.
    browser.go_up();
    assert_eq!(browser.mode, ViewMode::Photos);
}

#[test]
fn moving_through_columns_never_changes_the_view() {
    let (mut browser, dir) = browser_over_folders(&["pics"]);
    let pics = dir.0.join("pics");
    std::fs::create_dir(pics.join("deeper")).unwrap();
    browser.folder_views.push((pics.clone(), ViewMode::Photos));
    browser
        .folder_views
        .push((pics.join("deeper"), ViewMode::Grid));
    browser.set_mode(ViewMode::Columns);

    // A click selects the folder and shows it as the next column.
    let index = index_of(&browser, 0, "pics");
    browser.select(0, index);
    settle(&mut browser);
    assert_eq!(browser.columns.len(), 2);
    assert_eq!(browser.mode, ViewMode::Columns);

    // Opening it — Return, a double-click, the right arrow — moves into it.
    browser.select(0, index);
    browser.open_selection();
    settle(&mut browser);
    assert_eq!(browser.mode, ViewMode::Columns);
    browser.move_lateral(1);
    settle(&mut browser);
    let deeper = index_of(&browser, 1, "deeper");
    browser.select(1, deeper);
    browser.open_selection();
    settle(&mut browser);
    assert_eq!(browser.mode, ViewMode::Columns);

    // Up out of a column, and back and forward below the same root: still
    // the columns.
    browser.go_up();
    assert_eq!(browser.mode, ViewMode::Columns);
    browser.go_back();
    assert_eq!(browser.mode, ViewMode::Columns);
    browser.go_forward();
    assert_eq!(browser.mode, ViewMode::Columns);
}

#[test]
fn back_and_forward_to_another_folder_put_its_view_back() {
    let (mut browser, dir) = browser_over_folders(&["pics", "code"]);
    let pics = dir.0.join("pics");
    browser.set_mode(ViewMode::List);
    browser.navigate_to(&pics);
    browser.choose_mode(ViewMode::Photos);
    browser.navigate_to(&dir.0.join("code"));
    browser.choose_mode(ViewMode::Columns);
    // A real navigation away from the columns, to a root chosen as Photos.
    browser.go_back();
    assert_eq!(browser.current_path(), pics);
    assert_eq!(browser.mode, ViewMode::Photos);
    browser.go_forward();
    assert_eq!(browser.mode, ViewMode::Columns);
}

#[test]
fn only_the_most_recent_choices_are_kept() {
    let (mut browser, dir) = browser_over_folders(&[]);
    for i in 0..folder_views::FOLDER_VIEWS_KEPT + 5 {
        browser.columns = vec![Column::new(dir.0.join(format!("f{i}")))];
        browser.choose_mode(if i % 2 == 0 {
            ViewMode::Grid
        } else {
            ViewMode::List
        });
    }
    assert_eq!(browser.folder_views.len(), folder_views::FOLDER_VIEWS_KEPT);
    assert_eq!(browser.folder_view(&dir.0.join("f0")), None);
    assert_eq!(
        browser.folder_view(
            &dir.0
                .join(format!("f{}", folder_views::FOLDER_VIEWS_KEPT + 4))
        ),
        Some(ViewMode::Grid)
    );
    // Choosing again moves a folder to the most recent end.
    browser.columns = vec![Column::new(dir.0.join("f10"))];
    browser.choose_mode(ViewMode::Photos);
    assert_eq!(
        browser.folder_views.last().map(|(p, _)| p.clone()),
        Some(dir.0.join("f10"))
    );
}

#[test]
fn a_state_file_without_views_still_reads() {
    let back: remembered::Remembered = toml::from_str("[palette]\noffset = [1.0, 2.0]\n").unwrap();
    assert!(back.views.is_empty());
    let mut browser = Browser::new(std::env::temp_dir());
    let with = remembered::Remembered {
        views: vec![
            remembered::FolderView {
                path: "/tmp/a".into(),
                view: "photos".into(),
            },
            remembered::FolderView {
                path: "/tmp/b".into(),
                view: "nonsense".into(),
            },
        ],
        ..Default::default()
    };
    browser.remember_folder_views_from(&with);
    assert_eq!(
        browser.folder_view(Path::new("/tmp/a")),
        Some(ViewMode::Photos)
    );
    assert_eq!(browser.folder_views.len(), 1);
}
