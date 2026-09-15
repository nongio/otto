use super::*;

#[test]
fn the_list_opens_with_the_newest_on_top() {
    let mut browser = Browser::new(std::env::temp_dir());
    browser.set_mode(ViewMode::List);
    assert_eq!(browser.sort, SortKey::Modified);
    assert!(!browser.ascending);
}

#[test]
fn the_other_views_open_sorted_by_name() {
    for mode in [ViewMode::Grid, ViewMode::Columns] {
        let mut browser = Browser::new(std::env::temp_dir());
        browser.set_mode(ViewMode::List);
        browser.set_mode(mode);
        assert_eq!(browser.sort, SortKey::Name);
        assert!(browser.ascending);
    }
}

#[test]
fn a_sort_the_user_picked_survives_a_view_change() {
    let mut browser = Browser::new(std::env::temp_dir());
    browser.sort = SortKey::Size;
    browser.ascending = false;
    browser.sort_pinned = true;
    browser.set_mode(ViewMode::List);
    assert_eq!(browser.sort, SortKey::Size);
    assert!(!browser.ascending);
}
