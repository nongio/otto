use super::*;
use otto_kit::components::scroll::Direction;
use otto_kit::filetype::Kind;
use std::time::{Duration, SystemTime};

/// An entry modified `days` days and an hour before now.
fn entry(name: &str, kind: Kind, days: u64) -> Entry {
    Entry {
        name: name.to_string(),
        path: PathBuf::from("/tmp/otto-photos-tests").join(name),
        is_dir: kind == Kind::Folder,
        is_symlink: false,
        hidden: false,
        kind,
        size: Some(1),
        modified: Some(SystemTime::now() - Duration::from_secs(days * 86_400 + 3_600)),
        origin: None,
    }
}

fn photos_over(entries: Vec<Entry>) -> Browser {
    let mut browser = Browser::new(std::env::temp_dir());
    // Let the real read land first, so it cannot replace these entries and
    // the pane is not held at the top as one still loading.
    for _ in 0..1000 {
        if browser.columns[0].poll() && !browser.columns[0].loading() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    browser.columns[0].snapshot.entries = entries;
    browser.columns[0].epoch += 1;
    // Tall enough that every tile in these tests is on screen.
    browser.size = (1100.0, 3000.0);
    browser.set_mode(ViewMode::Photos);
    browser.rebuild_photos_layout();
    browser
}

fn names(browser: &Browser) -> Vec<String> {
    browser.visible(0).iter().map(|e| e.name.clone()).collect()
}

#[test]
fn photos_opens_with_the_newest_on_top() {
    let mut browser = Browser::new(std::env::temp_dir());
    browser.set_mode(ViewMode::Photos);
    assert_eq!(browser.sort, SortKey::Modified);
    assert!(!browser.ascending);
}

#[test]
fn folders_then_pictures_a_day_at_a_time_then_everything_else() {
    let mut browser = photos_over(vec![
        entry("notes.txt", Kind::Text, 0),
        entry("old.jpg", Kind::Image, 3),
        entry("Albums", Kind::Folder, 0),
        entry("new.jpg", Kind::Image, 0),
        entry("older.png", Kind::Image, 5),
    ]);
    assert_eq!(
        names(&browser),
        ["Albums", "new.jpg", "old.jpg", "older.png", "notes.txt"]
    );
    let shape: Vec<(usize, usize)> = browser
        .photos
        .sections
        .iter()
        .map(|s| (s.first, s.count))
        .collect();
    assert_eq!(shape, vec![(0, 1), (1, 1), (2, 1), (3, 1), (4, 1)]);
    // The folder is a card, not a square tile.
    assert_eq!(
        browser.entry_rect(0, 0).width(),
        view::FOLDER_CARD_W,
        "{:?}",
        browser.entry_rect(0, 0)
    );
    assert_eq!(browser.photos.len(), 5);
    assert_eq!(browser.photos_subtitle(0), "3 images, 1 folder");

    // Sorted by name within a day, the days themselves stay newest first.
    browser.sort = SortKey::Name;
    browser.ascending = true;
    assert_eq!(names(&browser)[1..4], ["new.jpg", "old.jpg", "older.png"]);
}

#[test]
fn grouping_by_month_or_not_at_all_changes_the_headings() {
    let mut browser = photos_over(vec![
        entry("a.jpg", Kind::Image, 0),
        entry("b.jpg", Kind::Image, 1),
        entry("c.jpg", Kind::Image, 400),
    ]);
    browser.set_photos_group(crate::photos::Grouping::None);
    browser.rebuild_photos_layout();
    assert_eq!(browser.photos.sections.len(), 1);
    assert!(browser.photos.sections[0].title.is_empty());

    browser.set_photos_group(crate::photos::Grouping::Month);
    browser.rebuild_photos_layout();
    // Over a year apart: two months at least, whatever today is.
    assert!(browser.photos.sections.len() >= 2);
    assert!(browser.photos.sections[0]
        .title
        .chars()
        .any(|c| c.is_ascii_digit()));
}

#[test]
fn the_slider_and_the_zoom_chords_size_the_rows() {
    let entries = (0..30)
        .map(|i| entry(&format!("p{i:02}.jpg"), Kind::Image, 0))
        .collect();
    let mut browser = photos_over(entries);
    // Wide enough that a row at the smallest size and one at the largest
    // hold different numbers of pictures beside the info panel.
    browser.size.0 = 1600.0;
    browser.rebuild_photos_layout();
    let before = browser.entry_rect(0, 0).height();

    browser.step_zoom(1.0);
    assert_eq!(
        browser.photos_row_h,
        view::PHOTOS_ROW_H + view::PHOTOS_ROW_STEP
    );
    // One step need not change a row — it changes when the next picture
    // stops fitting — but the largest size must.
    browser.set_photos_row_h(view::PHOTOS_ROW_MAX);
    browser.rebuild_photos_layout();
    assert!(browser.entry_rect(0, 0).height() > before);

    // Pressed at the slider's left end: the smallest size.
    let track = view::photos_slider_rect(browser.size.0).expect("room for the slider");
    let after = browser.photos_controls_press(track.left, track.center_y(), 1);
    assert!(matches!(after, Some(listing_pointer::After::Stop)));
    assert_eq!(browser.photos_row_h, view::PHOTOS_ROW_MIN);
    // Dragged to the far end, and let go.
    assert!(browser.photos_slider_drag(track.right + 50.0));
    assert_eq!(browser.photos_row_h, view::PHOTOS_ROW_MAX);
    browser.photos_slider_release();
    assert!(!browser.photos_slider.is_dragging());

    // The grouping button asks for its menu.
    let group = view::photos_group_rect(browser.size.0);
    let after = browser.photos_controls_press(group.center_x(), group.center_y(), 7);
    assert!(matches!(
        after,
        Some(listing_pointer::After::GroupMenu { serial: 7, .. })
    ));
    assert!(browser.photos_group_open);
}

#[test]
fn a_tile_is_clicked_where_it_is_drawn() {
    let entries = (0..12)
        .map(|i| entry(&format!("p{i:02}.jpg"), Kind::Image, i / 4))
        .collect();
    let browser = photos_over(entries);
    for index in 0..12 {
        let rect = browser.entry_rect(0, index);
        assert!(!rect.is_empty(), "{index}");
        assert_eq!(
            browser.entry_at(rect.center_x(), rect.center_y()),
            Some((0, index)),
            "{index} at {rect:?}"
        );
    }
    // The first day's heading is not a tile.
    let first = browser.entry_rect(0, 0);
    assert_eq!(browser.entry_at(first.center_x(), first.top - 10.0), None);
}

#[test]
fn the_arrows_follow_the_layout() {
    let entries = (0..12)
        .map(|i| entry(&format!("p{i:02}.jpg"), Kind::Image, 0))
        .collect();
    let mut browser = photos_over(entries);
    browser.move_photo_cursor(Direction::Right, false);
    assert_eq!(browser.columns[0].cursor, Some(0));
    browser.move_photo_cursor(Direction::Right, false);
    assert_eq!(browser.columns[0].cursor, Some(1));
    browser.move_photo_cursor(Direction::Down, false);
    let below = browser.photos.neighbor(1, Direction::Down);
    assert!(below.is_some());
    assert_eq!(browser.columns[0].cursor, below);
    let name = browser.visible(0)[below.unwrap()].name.clone();
    assert!(browser.selected_named(0, &name));
}

#[test]
fn the_layout_is_kept_until_something_it_depends_on_moves() {
    let mut browser = photos_over(vec![entry("a.jpg", Kind::Image, 0)]);
    let key = browser.photos_key.clone();
    browser.rebuild_photos_layout();
    assert_eq!(browser.photos_key, key);
    browser.size.0 = 900.0;
    browser.rebuild_photos_layout();
    assert_ne!(browser.photos_key, key);
}

#[test]
fn the_info_panel_sits_beside_the_wall_and_describes_the_selection() {
    let mut browser = photos_over(vec![
        entry("Trips", Kind::Folder, 0),
        entry("a.jpg", Kind::Image, 0),
        entry("b.jpg", Kind::Image, 0),
        entry("notes.txt", Kind::Text, 0),
    ]);
    // Wide enough for the wall's four pictures across and the panel.
    browser.size.0 = 2000.0;
    browser.sync_scroll_metrics();
    let (width, height) = (browser.size.0, browser.content_h());
    let full = view::content_viewport(width, height, ViewMode::Photos);
    let area = browser.photos.area(width, height);
    // Room for both: the wall packed into what the panel leaves, and
    // nothing to pan.
    assert!(browser.photos.has_panel(width, height));
    assert_eq!(full.width() - area.width(), view::PHOTOS_INFO_W);
    assert_eq!(browser.photos.panel_rect(width, height).right, full.right);
    assert_eq!(browser.pan.state.max_offset(), 0.0);
    assert!(matches!(
        browser.photos_info_data(),
        Some(view::PhotosInfoData::Here { .. })
    ));

    // One picture: its panel, and the wall does not move for it.
    let before = browser.entry_rect(0, 1);
    browser.select(0, 1);
    browser.sync_scroll_metrics();
    assert_eq!(browser.entry_rect(0, 1), before);
    assert!(browser.photos_info_wants_preview());
    assert!(matches!(
        browser.photos_info_data(),
        Some(view::PhotosInfoData::One { .. })
    ));
    assert_eq!(browser.path_bar_note().as_deref(), Some("1 of 4 selected"));
    // A press in the panel is the panel's, not a click on nothing.
    let panel = browser.photos.panel_rect(browser.size.0, browser.content_h());
    assert!(browser
        .photos_controls_press(panel.center_x(), panel.center_y(), 1)
        .is_some());

    // Two: how many, and how much.
    browser.extend_select(0, 2);
    match browser.photos_info_data() {
        Some(view::PhotosInfoData::Many { count, bytes }) => {
            assert_eq!(count, 2);
            assert_eq!(bytes, 2);
        }
        _ => panic!("expected the many-items panel"),
    }

    // A folder: its own facts, and no decode asked for.
    browser.select(0, 0);
    assert!(matches!(
        browser.photos_info_data(),
        Some(view::PhotosInfoData::Folder { .. })
    ));
    assert!(!browser.photos_info_wants_preview());
}

#[test]
fn the_info_panel_copies_a_swatch() {
    let mut browser = photos_over(vec![entry("a.jpg", Kind::Image, 0)]);
    browser.select(0, 0);
    browser.sync_scroll_metrics();
    reveal_info_panel(&mut browser);
    // The decode landing, with its palette worked out.
    let path = browser.visible(0)[0].path.clone();
    browser.sync_preview_target();
    let pane = browser.preview.as_mut().expect("a decode was asked for");
    assert_eq!(pane.path, path);
    pane.palette = vec![
        skia_safe::Color::from_rgb(0xF2, 0x84, 0x5C),
        skia_safe::Color::from_rgb(0x20, 0x20, 0x20),
    ];

    let panel = browser.photos.panel_rect(browser.size.0, browser.content_h());
    let layout = view::photos_info_layout(panel, 2);
    // Two colours sit together at the left, not at the two ends of the row.
    assert!(layout.swatches[1].left - layout.swatches[0].right < 20.0);
    let swatch = layout.swatches[0];
    let hit = browser.photos_info_data().and_then(|data| {
        view::photos_info_swatch_at(panel, &data, swatch.center_x(), swatch.center_y())
    });
    assert_eq!(hit, Some(0));
    // The pointer over it lights it up, and moving off puts it out.
    browser.track_photo_hover(swatch.center_x(), swatch.center_y());
    assert_eq!(browser.photo_swatch_hover, Some(0));
    assert_eq!(
        browser.photos_swatch_at(swatch.center_x(), swatch.center_y()),
        Some(0)
    );
    browser.track_photo_hover(panel.center_x(), panel.bottom - 4.0);
    assert_eq!(browser.photo_swatch_hover, None);
    // What a click on it copies (the clipboard itself needs a compositor).
    assert_eq!(browser.pick_swatch(0).as_deref(), Some("#F2845C"));
    assert_eq!(browser.photos_copied.map(|(i, _)| i), Some(0));
    assert!(browser.tick_photos_copied(), "still showing");
}

#[test]
fn a_narrower_window_keeps_the_selected_tile_in_view() {
    let entries = (0..300)
        .map(|i| entry(&format!("p{i:03}.jpg"), Kind::Image, i / 20))
        .collect();
    let mut browser = photos_over(entries);
    browser.size = (1300.0, 700.0);
    browser.sync_scroll_metrics();
    browser.columns[0].scroll.state.set_offset(3_000.0);
    assert!(browser.columns[0].scroll.offset() > 0.0, "scrolled");
    let area = browser.photos.area(browser.size.0, browser.content_h());
    let scroll = browser.columns[0].scroll.offset();
    let range = browser.photos.visible_range(area, scroll, area);
    let pick = range.start + (range.end - range.start) / 2;
    browser.select(0, pick);
    let before = browser.photos.tile_rect(area, pick, scroll).top;

    browser.size.0 = 1100.0;
    browser.sync_scroll_metrics();
    let area = browser.photos.area(browser.size.0, browser.content_h());
    let scroll = browser.columns[0].scroll.offset();
    let after = browser.photos.tile_rect(area, pick, scroll).top;
    assert!((after - before).abs() < 1.0, "{before} → {after}");
}

#[test]
fn a_new_size_keeps_the_picture_at_the_top_in_place() {
    let entries = (0..300)
        .map(|i| entry(&format!("p{i:03}.jpg"), Kind::Image, i / 20))
        .collect();
    let mut browser = photos_over(entries);
    browser.size = (1100.0, 700.0);
    browser.sync_scroll_metrics();
    let area = view::content_viewport(browser.size.0, browser.content_h(), ViewMode::Photos);
    browser.columns[0].scroll.state.set_offset(2_000.0);
    let scroll = browser.columns[0].scroll.offset();
    assert!(scroll > 0.0, "scrolled");
    let first = browser.photos.visible_range(area, scroll, area).start;
    let before = browser.photos.tile_rect(area, first, scroll).top;

    browser.set_photos_row_h(view::PHOTOS_ROW_MAX);
    browser.sync_scroll_metrics();
    let scroll = browser.columns[0].scroll.offset();
    let after = browser.photos.tile_rect(area, first, scroll).top;
    assert!((after - before).abs() < 1.0, "{before} → {after}");
}

#[test]
fn the_info_panel_text_can_be_selected_and_copied() {
    let mut browser = photos_over(vec![entry("holiday.jpg", Kind::Image, 0)]);
    browser.select(0, 0);
    browser.sync_scroll_metrics();
    reveal_info_panel(&mut browser);
    let data = browser.photos_info_data().unwrap();
    let panel = browser.photos.panel_rect(browser.size.0, browser.content_h());
    let runs = view::photos_info_runs(panel, &data, &otto_kit::theme::Theme::light());
    let name = runs[0].rect();
    let (x, y) = (name.center_x(), name.center_y());

    // A double click on the name takes the word under it.
    assert!(browser.panel_text_press(x, y));
    assert!(browser.panel_text_press(x, y));
    let word = browser.panel_selected_text().expect("a word is selected");
    assert!("holiday.jpg".contains(&word), "{word}");
    assert!(browser.over_panel_text(x, y));

    // A drag from the name down to the last value takes the lot, a line each.
    assert!(browser.panel_text_press(runs[0].rect().left + 0.5, y));
    let last = runs.last().unwrap().rect();
    assert!(browser.panel_text_drag(last.right, last.center_y()));
    browser.panel_text_release();
    let text = browser.panel_selected_text().unwrap();
    assert!(text.starts_with("holiday.jpg\n"), "{text}");
    assert!(text.contains("/tmp/otto-photos-tests"), "{text}");

    // A press off the text lets go, and is left for the rest of the window.
    assert!(!browser.panel_text_press(panel.left + 2.0, panel.bottom - 4.0));
    assert!(browser.panel_selected_text().is_none());

    // Another file's facts do not inherit the selection.
    assert!(browser.panel_text_press(x, y));
    assert!(browser.panel_text_press(x, y));
    browser.clear_pane_selection(0);
    assert!(browser.panel_selected_text().is_none());
}

#[test]
fn get_info_text_can_be_selected() {
    let path = std::env::temp_dir().join(format!("otto-info-select-{}.txt", std::process::id()));
    std::fs::write(&path, b"hello").unwrap();
    let mut browser = Browser::new(std::env::temp_dir());
    browser.info = Some(model::read_info(&path));
    let runs = browser.info_runs();
    let name = runs[0].rect();
    let now = std::time::Instant::now();
    for step in 0..3 {
        browser.info_selection.press(
            &runs,
            name.center_x(),
            name.center_y(),
            now + std::time::Duration::from_millis(step * 50),
        );
    }
    let copied = browser.info_selection.selected_text(&runs).unwrap();
    assert_eq!(copied, path.file_name().unwrap().to_string_lossy());
    // The Where row copies the whole folder, however it is shortened.
    let parent = path.parent().unwrap().to_string_lossy().into_owned();
    assert!(runs.iter().any(|run| run.full == parent));
    browser.close_info();
    assert!(!browser.info_selection.has_selection());
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn the_icon_view_has_a_size_of_its_own() {
    let entries = (0..200)
        .map(|i| entry(&format!("f{i:03}.txt"), Kind::Text, 0))
        .collect();
    let mut browser = photos_over(entries);
    browser.set_mode(ViewMode::Grid);
    browser.size = (1100.0, 700.0);
    browser.sync_scroll_metrics();
    // The default is the grid as it always was.
    let (cell_w, cell_h) = (view::cell_w(), view::cell_h());
    assert_eq!(view::grid_icon(), view::DEFAULT_GRID_ICON);
    assert_eq!(browser.entry_rect(0, 0).width(), cell_w);
    assert_eq!(browser.entry_rect(0, 0).height(), cell_h);

    // Ctrl+= steps it, and the cells grow with it.
    browser.step_zoom(1.0);
    assert_eq!(
        browser.grid_icon,
        view::DEFAULT_GRID_ICON + view::GRID_ICON_STEP
    );
    browser.sync_scroll_metrics();
    assert_eq!(
        browser.entry_rect(0, 0).width(),
        cell_w + view::GRID_ICON_STEP
    );
    // Photos keeps its own size.
    assert_eq!(browser.photos_row_h, view::PHOTOS_ROW_H);

    // The slider sits beside the switcher, with no grouping button.
    let track = view::zoom_slider_rect(browser.size.0, false).expect("room for the slider");
    assert!(browser
        .photos_controls_press(track.left, track.center_y(), 1)
        .is_some());
    assert_eq!(browser.grid_icon, view::GRID_ICON_MIN);
    browser.photos_slider_release();

    // The icon at the top of the view stays there as the size changes.
    browser.sync_scroll_metrics();
    browser.columns[0].scroll.state.set_offset(1_200.0);
    let scroll = browser.columns[0].scroll.offset();
    assert!(scroll > 0.0, "scrolled");
    let area = view::content_viewport(browser.size.0, browser.content_h(), ViewMode::Grid);
    let first =
        view::grid_visible_range_in(area, &browser.recent_sections, 200, scroll, area).start;
    let before = browser.entry_rect(0, first).top;
    browser.set_zoom(view::GRID_ICON_MAX);
    browser.sync_scroll_metrics();
    let after = browser.entry_rect(0, first).top;
    assert!((after - before).abs() < 1.0, "{before} → {after}");
    assert_eq!(
        browser.entry_rect(0, first).width(),
        cell_w + view::GRID_ICON_MAX - view::DEFAULT_GRID_ICON
    );
}

/// Pan the Photos view all the way to its info panel, as a sideways scroll
/// would.
fn reveal_info_panel(browser: &mut Browser) {
    browser.pan.state.set_offset(view::PHOTOS_INFO_W);
    browser.sync_scroll_metrics();
}

#[test]
fn a_narrow_window_pans_to_the_info_panel() {
    let mut browser = photos_over(vec![entry("a.jpg", Kind::Image, 0)]);
    browser.size.0 = 700.0;
    browser.sync_scroll_metrics();
    let (width, height) = (browser.size.0, browser.content_h());
    let full = view::content_viewport(width, height, ViewMode::Photos);
    let least = view::photos_wall_min(browser.photos_row_h);
    assert!(full.width() < least + view::PHOTOS_INFO_W);
    // The wall keeps its least width and the panel is past the edge …
    assert_eq!(
        browser.photos.area(width, height).width(),
        least.min(full.width())
    );
    assert!(!browser.photos.has_panel(width, height));
    // … as far away as the pan reaches, and no further.
    let reach = view::photos_content_width(full.width(), browser.photos_row_h) - full.width();
    assert_eq!(browser.pan.state.max_offset(), reach);
    reveal_info_panel(&mut browser);
    assert!(browser.photos.has_panel(width, height));
    assert_eq!(browser.photos.panel_rect(width, height).right, full.right);
    // What slid behind the sidebar is not there to be clicked.
    let tile = browser.entry_rect(0, 0);
    assert!(tile.left < full.left, "the first tile is under the sidebar");
    assert_eq!(browser.entry_at(tile.left + 2.0, tile.center_y()), None);
}
