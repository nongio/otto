use super::*;

/// A real directory holding files and subdirectories, swept up on drop.
///
/// Drop targets turn on `is_dir`, which comes off the filesystem, so these
/// tests need something actually on disk the way the type-ahead ones do.
struct TempDir(PathBuf);

impl TempDir {
    fn holding(files: &[&str], dirs: &[&str]) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);

        let path = std::env::temp_dir().join(format!(
            "otto-files-dnd-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        for name in files {
            std::fs::write(path.join(name), b"contents").expect("temp file");
        }
        for name in dirs {
            std::fs::create_dir_all(path.join(name)).expect("temp subdir");
        }
        Self(path)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A list-view browser over a loaded directory. List view because its row
/// geometry is a single strip under the header, which a test can point at
/// without reproducing the Miller stack's pan.
fn browser_over(files: &[&str], dirs: &[&str]) -> (Browser, TempDir) {
    let dir = TempDir::holding(files, dirs);
    let mut browser = Browser::new(dir.0.clone());
    browser.mode = ViewMode::List;
    for _ in 0..500 {
        if browser.columns[0].poll() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(!browser.columns[0].loading(), "listing never arrived");
    (browser, dir)
}

/// The middle of row `index` in list view.
fn row_point(index: usize) -> (f32, f32) {
    (
        view::sidebar_w() + 100.0,
        view::header_h() + view::COLUMNS_H + view::ROW_H * index as f32 + view::ROW_H / 2.0,
    )
}

/// Where `name` sits in the sorted listing.
fn row_of(browser: &Browser, name: &str) -> usize {
    browser
        .visible(browser.active)
        .iter()
        .position(|entry| entry.name == name)
        .expect("entry is in the listing")
}

#[test]
fn a_directory_row_takes_the_drop_itself() {
    let (browser, dir) = browser_over(&["a.txt"], &["target"]);
    let (x, y) = row_point(row_of(&browser, "target"));

    let hit = browser.drop_target_at(x, y).expect("a target");
    assert_eq!(hit.path(), &dir.join("target"));
    assert!(matches!(hit, DropTarget::Entry { .. }));
}

/// A press on an entry that is not selected acts at once: the selection
/// follows the pointer down, the way it always has.
#[test]
fn a_press_on_an_unselected_entry_selects_it_immediately() {
    let (mut browser, _dir) = browser_over(&["a.txt", "b.txt", "c.txt"], &[]);
    let b = row_of(&browser, "b.txt");

    browser.press_entry(0, b);

    assert_eq!(browser.columns[0].selection.len(), 1);
    assert!(browser.selected_named(0, "b.txt"));
}

/// A press on one of several selected entries leaves the group alone, so
/// the drag that usually follows can carry all of it.
#[test]
fn a_press_inside_a_group_keeps_the_whole_selection() {
    let (mut browser, _dir) = browser_over(&["a.txt", "b.txt", "c.txt"], &[]);
    let (a, c) = (row_of(&browser, "a.txt"), row_of(&browser, "c.txt"));
    browser.select(0, a);
    browser.extend_select(0, c);
    assert_eq!(browser.columns[0].selection.len(), 3, "three are selected");

    browser.press_entry(0, row_of(&browser, "b.txt"));

    assert_eq!(
        browser.columns[0].selection.len(),
        3,
        "the group survives the press"
    );
}

/// …and if the press comes back up without dragging, it was a click after
/// all: the selection narrows to the one under the pointer.
#[test]
fn a_click_inside_a_group_narrows_to_it_on_release() {
    let (mut browser, _dir) = browser_over(&["a.txt", "b.txt", "c.txt"], &[]);
    let (a, c) = (row_of(&browser, "a.txt"), row_of(&browser, "c.txt"));
    browser.select(0, a);
    browser.extend_select(0, c);
    let b = row_of(&browser, "b.txt");

    browser.press_entry(0, b);
    browser.release_entry();

    assert_eq!(browser.columns[0].selection.len(), 1);
    assert!(browser.selected_named(0, "b.txt"));
}

/// A drag cancels the deferred narrowing outright: the group is what
/// travels, and the button coming up at the end must not collapse it.
#[test]
fn a_drag_out_of_a_group_leaves_the_group_whole() {
    let (mut browser, _dir) = browser_over(&["a.txt", "b.txt", "c.txt"], &[]);
    let (a, c) = (row_of(&browser, "a.txt"), row_of(&browser, "c.txt"));
    browser.select(0, a);
    browser.extend_select(0, c);

    browser.press_entry(0, row_of(&browser, "b.txt"));
    browser.drag_started();
    browser.release_entry();

    assert_eq!(
        browser.columns[0].selection.len(),
        3,
        "all three are still selected after the drag"
    );
}

/// A Miller browser with one file selected, so the preview column is up
/// and the stack is panned to show it — the state the preview drag needs.
fn browser_with_preview() -> (Browser, TempDir) {
    let (mut browser, dir) = browser_over(&["a.txt", "b.txt"], &[]);
    browser.mode = ViewMode::Columns;
    browser.select(0, row_of(&browser, "a.txt"));
    browser.reveal_preview();
    // The reveal is a spring; the test wants where it lands, not where it
    // is one frame in.
    for _ in 0..600 {
        if !browser.pan.advance(1.0 / 60.0) {
            break;
        }
    }
    assert!(browser.preview_visible(), "the preview column is not up");
    (browser, dir)
}

/// The window's own border outranks a column divider, because that is the
/// order a press resolves them in.
///
/// A Miller pane's right edge lands on the window's right border whenever
/// the stack is panned fully over — the ordinary state once a preview
/// column is up — and the divider's grab band is narrower than the
/// window's, so it sits entirely inside it. Answering `ColResize` there
/// showed a cursor promising a column resize while the click underneath it
/// resized the window.
#[test]
fn the_window_border_outranks_a_column_divider() {
    let (mut browser, _dir) = browser_with_preview();

    // A window narrow enough that the stack overflows it, and panned so
    // the last pane's right edge is flush with the window's own — the
    // ordinary resting state once a preview column has been revealed.
    let panes = browser.columns.len();
    browser.size.0 = view::sidebar_w() + browser.miller_w - 50.0;
    browser.sync_scroll_metrics();
    browser
        .pan
        .state
        .set_offset(view::sidebar_w() + panes as f32 * browser.miller_w - browser.size.0);
    let (w, h) = browser.size;
    let y = view::header_h() + 80.0;
    let edge_x = w - 1.0;

    assert!(
        view::miller_boundary_at(
            edge_x,
            y,
            w,
            browser.content_h(),
            browser.pan.offset(),
            panes,
            browser.miller_w,
        )
        .is_some(),
        "the pane divider is not on the window border, so this proves nothing"
    );
    assert!(
        resize::edge_at(Rect::from_wh(w, h), edge_x, y).is_some(),
        "the window border is not where the test thinks it is"
    );
    assert_eq!(
        browser.hover_shape(edge_x, y),
        resize::edge_at(Rect::from_wh(w, h), edge_x, y)
            .expect("an edge")
            .cursor(),
        "the cursor on the window border must be the one the press acts on"
    );

    // Panned back a little, the same divider sits clear of the border and
    // gets its own cursor again: the border wins where they overlap, and
    // nowhere else.
    let flush = browser.pan.offset();
    browser.pan.state.set_offset(flush + 40.0);
    let inside = view::sidebar_w() + panes as f32 * browser.miller_w - browser.pan.offset();
    assert_eq!(
        browser.hover_shape(inside, y),
        CursorShape::ColResize,
        "a divider away from the border should still say column-resize"
    );
}

/// The preview column's picture is a handle on the file it is a picture
/// of: pressing it arms a drag, the same way pressing the row does.
#[test]
fn the_preview_picture_can_be_picked_up() {
    let (browser, _dir) = browser_with_preview();
    let panel = view::preview_pane_rect(
        browser.columns.len(),
        browser.content_h(),
        browser.pan.offset(),
        browser.miller_w,
    );
    let stage = view::preview_stage_rect(panel, 3);

    let on_picture = (stage.center_x(), stage.center_y());
    assert!(
        browser
            .preview_grab_at(on_picture.0, on_picture.1)
            .is_some(),
        "the middle of the preview picture is not a grab"
    );
    assert!(
        browser
            .drag_items(on_picture.0, on_picture.1)
            .is_some_and(|(picture, ..)| matches!(picture, DragPicture::Preview(_))),
        "a drag off the preview does not carry the preview's picture"
    );
    assert_eq!(
        browser.drag_paths().len(),
        1,
        "the preview drag carries the one file it is a picture of"
    );

    // The caption below it is a label, not a handle.
    assert!(
        browser
            .preview_grab_at(stage.center_x(), stage.bottom + 20.0)
            .is_none(),
        "the caption under the picture should not pick the file up"
    );
}

/// A deferred narrowing must never outlive its own gesture. If a release
/// goes missing — consumed by something else on its way through — the next
/// press drops the stale decision instead of letting it land on this click.
#[test]
fn a_pending_narrow_never_lands_on_a_later_click() {
    let (mut browser, _dir) = browser_over(&["a.txt", "b.txt", "c.txt"], &[]);
    let (a, c) = (row_of(&browser, "a.txt"), row_of(&browser, "c.txt"));
    browser.select(0, a);
    browser.extend_select(0, c);

    // A press inside the group whose release never arrives.
    browser.press_entry(0, row_of(&browser, "b.txt"));

    // A later, unrelated click elsewhere: it selects its own entry, and the
    // release resolves nothing left over from before.
    let a_row = row_of(&browser, "a.txt");
    browser.press_entry(0, a_row);
    browser.release_entry();

    assert_eq!(browser.columns[0].selection.len(), 1);
    assert!(
        browser.selected_named(0, "a.txt"),
        "the click that happened is the one that counts"
    );
}

#[test]
fn a_file_row_drops_into_the_directory_it_is_in() {
    // Dropping "onto" a file means beside it, not inside it.
    let (browser, dir) = browser_over(&["a.txt"], &["target"]);
    let (x, y) = row_point(row_of(&browser, "a.txt"));

    let hit = browser.drop_target_at(x, y).expect("a target");
    assert_eq!(hit.path(), &dir.0);
    assert!(matches!(hit, DropTarget::Pane { .. }));
}

#[test]
fn empty_space_below_the_rows_drops_into_the_pane() {
    let (browser, dir) = browser_over(&["a.txt"], &[]);
    // Past the one entry, but still inside the window: a point below the
    // viewport is off the pane altogether, which is a different miss.
    let (x, y) = row_point(10);

    let hit = browser.drop_target_at(x, y).expect("a target");
    assert_eq!(hit.path(), &dir.0);
}

#[test]
fn the_sidebar_takes_a_drop_only_on_a_place() {
    let (browser, _dir) = browser_over(&["a.txt"], &[]);
    // The first place that is a real folder. Recent leads the sidebar and
    // is a listing rather than a directory, so a drop on it has nowhere
    // to go and it is skipped here — see below, where that is asserted.
    let folder = browser
        .places
        .iter()
        .position(|p| !p.recent)
        .expect("the sidebar has a folder");

    let place = view::place_rect(folder);
    let hit = browser
        .drop_target_at(place.center_x(), place.center_y())
        .expect("a place takes a drop");
    assert_eq!(hit.path(), &browser.places[folder].path);

    // The header band above the first place is chrome, and takes nothing.
    assert_eq!(browser.drop_target_at(20.0, 4.0), None);
}

#[test]
fn recent_takes_no_drop() {
    let (browser, _dir) = browser_over(&["a.txt"], &[]);
    let index = browser
        .places
        .iter()
        .position(|p| p.recent)
        .expect("Recent is in the sidebar");
    let row = view::place_rect(index);
    assert_eq!(
        browser.drop_target_at(row.center_x(), row.center_y()),
        None,
        "Recent is a listing, so there is nowhere in it to put a file"
    );
}

#[test]
fn the_picker_refuses_every_drop() {
    let (mut browser, _dir) = browser_over(&["a.txt"], &["target"]);
    let (x, y) = row_point(row_of(&browser, "target"));
    assert!(
        browser.drop_target_at(x, y).is_some(),
        "the browser takes it"
    );

    let (responder, _receiver) = tokio::sync::oneshot::channel();
    browser.picker = Some(picker::Session::new(
        picker::Request {
            mode: picker::Mode::Open,
            handle: String::new(),
            app_id: String::new(),
            parent_window: String::new(),
            title: String::new(),
            accept_label: String::new(),
            multiple: false,
            directory: false,
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

    assert!(!browser.dnd_enabled());
    assert_eq!(browser.drop_target_at(x, y), None);
}

/// A drop is a reload, and a reload must leave the pane looking at what
/// it was looking at — in every view. Run over all three because the two
/// that measure an empty pane as taller than nothing (a grid counts its
/// padding, a Miller pane its row inset) are exactly the ones that used to
/// clamp to the top while the fresh listing was still being read.
#[test]
fn a_drop_keeps_the_pane_where_it_was_scrolled() {
    for mode in [ViewMode::List, ViewMode::Grid, ViewMode::Columns] {
        let names: Vec<String> = (0..60).map(|i| format!("f{i:02}.txt")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let (mut browser, dir) = browser_over(&refs, &["sub"]);
        browser.mode = mode;
        browser.size = (900.0, 400.0);
        browser.sync_scroll_metrics();

        browser.columns[0].scroll.state.set_offset(300.0);
        let before = browser.columns[0].scroll.offset();
        assert!(before > 0.0, "{mode:?}: the pane has to be scrolled");

        browser.drop_target = Some(DropTarget::Entry {
            depth: 0,
            index: 0,
            path: dir.join("sub"),
        });
        browser.apply_drop(vec![dir.join("f00.txt")], true);

        // The frame loop re-measures every frame, including the ones that
        // land while the re-read is still in flight. Those are the frames
        // that used to lose the position, so the test has to run them.
        for _ in 0..500 {
            browser.sync_scroll_metrics();
            if browser.columns[0].poll() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        browser.sync_scroll_metrics();

        assert_eq!(
            browser.columns[0].scroll.offset(),
            before,
            "{mode:?}: the drop scrolled the pane away from what the user was looking at"
        );
    }
}

#[test]
fn undoing_a_drop_puts_the_file_back() {
    let (mut browser, dir) = browser_over(&["a.txt"], &["sub"]);
    browser.drop_target = Some(DropTarget::Entry {
        depth: 0,
        index: 0,
        path: dir.join("sub"),
    });
    browser.apply_drop(vec![dir.join("a.txt")], true);
    assert!(dir.join("sub/a.txt").exists(), "the move happened");

    browser.undo_last();

    assert!(dir.join("a.txt").exists(), "back where it came from");
    assert!(!dir.join("sub/a.txt").exists(), "and not still there");
    // Against the catalogue, not against English: the label is what this
    // is checking — that the move, and not something else, is what got
    // recorded — and the wording around it is the catalogue's business.
    let expected = otto_kit::t_owned!("files-undid", label = otto_kit::t!("files-undo-move"));
    assert_eq!(browser.status.as_deref(), Some(expected.as_str()));
}

/// Undoing a copy takes the copy away — via the Trash, so an undo is never
/// itself the thing that loses a file.
#[test]
fn undoing_a_copy_removes_the_copy_and_leaves_the_original() {
    // Keeps the delete out of the real Trash; see `model::test_data_home`.
    let _trash = model::test_data_home();
    let (mut browser, dir) = browser_over(&["a.txt"], &["sub"]);
    browser.drop_target = Some(DropTarget::Entry {
        depth: 0,
        index: 0,
        path: dir.join("sub"),
    });
    browser.apply_drop(vec![dir.join("a.txt")], false);
    assert!(dir.join("sub/a.txt").exists(), "the copy happened");

    browser.undo_last();

    assert!(dir.join("a.txt").exists(), "the original is untouched");
    assert!(!dir.join("sub/a.txt").exists(), "the copy is gone");
}

#[test]
fn undoing_a_delete_restores_the_file() {
    // Keeps the delete out of the real Trash; see `model::test_data_home`.
    let _trash = model::test_data_home();
    let (mut browser, dir) = browser_over(&["a.txt"], &[]);
    browser.select(0, 0);
    browser.move_selected_to_trash();
    assert!(!dir.join("a.txt").exists(), "the delete happened");

    browser.undo_last();

    assert!(dir.join("a.txt").exists(), "restored out of the Trash");
    let expected = otto_kit::t_owned!("files-undid", label = otto_kit::t!("files-undo-delete"));
    assert_eq!(browser.status.as_deref(), Some(expected.as_str()));
}

/// The stack is per operation, and Ctrl+Z walks back through it.
#[test]
fn undo_walks_back_one_operation_at_a_time() {
    let (mut browser, dir) = browser_over(&["a.txt", "b.txt"], &["sub"]);
    for name in ["a.txt", "b.txt"] {
        browser.drop_target = Some(DropTarget::Entry {
            depth: 0,
            index: 0,
            path: dir.join("sub"),
        });
        browser.apply_drop(vec![dir.join(name)], true);
    }

    browser.undo_last();
    assert!(dir.join("b.txt").exists(), "the last one came back first");
    assert!(dir.join("sub/a.txt").exists(), "and only the last one");

    browser.undo_last();
    assert!(dir.join("a.txt").exists(), "then the one before it");

    browser.undo_last();
    let expected = otto_kit::t_owned!("files-nothing-to-undo");
    assert_eq!(browser.status.as_deref(), Some(expected.as_str()));
}

/// Selecting and navigating are not operations. Nothing about them may end
/// up on the stack, or a Ctrl+Z meant for a delete would spend itself on a
/// click instead.
#[test]
fn selecting_and_navigating_are_not_undoable() {
    let (mut browser, dir) = browser_over(&["a.txt"], &["sub"]);
    browser.select(0, 0);
    browser.select(0, 1);
    browser.clear_pane_selection(0);
    browser.navigate_to(&dir.join("sub"));

    assert!(browser.undo.is_empty(), "{:?}", browser.undo);
}

/// A click on nothing means nothing is selected — the way it does in every
/// file manager. Run over all three views because each resolves the click
/// through its own hit test.
#[test]
fn a_click_on_empty_space_clears_the_selection() {
    for mode in [ViewMode::List, ViewMode::Grid, ViewMode::Columns] {
        let (mut browser, _dir) = browser_over(&["a.txt"], &[]);
        browser.mode = mode;
        browser.size = (900.0, 600.0);
        browser.sync_scroll_metrics();
        browser.select(0, 0);
        assert!(
            !browser.columns[0].selection.is_empty(),
            "{mode:?}: selected"
        );

        browser.clear_pane_selection(0);

        assert!(
            browser.columns[0].selection.is_empty(),
            "{mode:?}: the click on nothing left a selection behind"
        );
        assert_eq!(browser.columns[0].cursor, None, "{mode:?}: and no cursor");
    }
}

/// A browser in icon view, sized, with its scroll metrics current — what
/// the marquee's geometry needs before it can be asked anything.
fn grid_over(files: &[&str]) -> (Browser, TempDir) {
    let (mut browser, dir) = browser_over(files, &[]);
    browser.mode = ViewMode::Grid;
    browser.size = (900.0, 600.0);
    browser.sync_scroll_metrics();
    (browser, dir)
}

/// The middle of cell `index`, in window coordinates, in an unscrolled
/// grid the size `grid_over` builds.
fn cell_center(index: usize) -> (f32, f32) {
    let area = view::content_viewport(900.0, 600.0, ViewMode::Grid);
    let cell = view::grid_cell_rect(area, index, 0.0);
    (cell.center_x(), cell.center_y())
}

/// Dragging from empty space rubber-bands: everything the band touches is
/// selected, and nothing else is.
#[test]
fn a_band_dragged_over_the_grid_selects_what_it_covers() {
    let (mut browser, _dir) = grid_over(&["a.txt", "b.txt", "c.txt"]);
    let names: Vec<String> = browser.visible(0).iter().map(|e| e.name.clone()).collect();

    let area = view::content_viewport(900.0, 600.0, ViewMode::Grid);
    browser.begin_marquee(0, area.right - 4.0, area.bottom - 4.0, false);
    let (x, y) = cell_center(1);
    browser.update_marquee(x, y);

    assert!(
        browser.selected_named(0, &names[1]),
        "the band caught the cell"
    );
    assert!(
        !browser.selected_named(0, &names[0]),
        "and left the one it never reached"
    );
}

/// The selection is recomputed from the band, not accumulated along the
/// way, so pulling the band back off an entry deselects it again.
#[test]
fn a_band_pulled_back_gives_up_what_it_leaves() {
    let (mut browser, _dir) = grid_over(&["a.txt", "b.txt", "c.txt"]);
    let names: Vec<String> = browser.visible(0).iter().map(|e| e.name.clone()).collect();

    let (x0, y0) = cell_center(0);
    browser.begin_marquee(0, x0, y0, false);
    let (x2, y2) = cell_center(2);
    browser.update_marquee(x2, y2);
    assert_eq!(browser.columns[0].selection.len(), 3, "all three, sweeping");

    browser.update_marquee(x0 + 1.0, y0);

    assert_eq!(browser.columns[0].selection.len(), 1);
    assert!(
        browser.selected_named(0, &names[0]),
        "only the one still under the band"
    );
}

/// Ctrl keeps what was selected and adds to it, the way Ctrl+click does.
#[test]
fn a_band_held_with_ctrl_adds_to_the_selection() {
    let (mut browser, _dir) = grid_over(&["a.txt", "b.txt", "c.txt"]);
    let names: Vec<String> = browser.visible(0).iter().map(|e| e.name.clone()).collect();
    browser.select(0, 0);

    let (x, y) = cell_center(2);
    browser.begin_marquee(0, x - view::CELL_W / 2.0 + 1.0, y, true);
    browser.update_marquee(x, y);

    assert!(
        browser.selected_named(0, &names[0]),
        "the earlier click survived"
    );
    assert!(
        browser.selected_named(0, &names[2]),
        "and the band added to it"
    );
}

/// A band the size of a point catches nothing — which is exactly what a
/// plain click on empty space has to mean.
#[test]
fn a_band_that_never_travels_selects_nothing() {
    let (mut browser, _dir) = grid_over(&["a.txt"]);
    browser.select(0, 0);

    let area = view::content_viewport(900.0, 600.0, ViewMode::Grid);
    browser.clear_pane_selection(0);
    browser.begin_marquee(0, area.right - 4.0, area.bottom - 4.0, false);
    browser.update_marquee(area.right - 4.0, area.bottom - 4.0);

    assert!(browser.columns[0].selection.is_empty());
}

/// The band is anchored to the content, not to the screen: scrolling under
/// it keeps it around the same files.
#[test]
fn a_band_stays_over_the_files_when_the_pane_scrolls() {
    // Enough cells that the pane has somewhere to scroll to: `set_offset`
    // clamps, and a short listing would silently stay at the top.
    let names: Vec<String> = (0..60).map(|i| format!("f{i:02}.txt")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let (mut browser, _dir) = grid_over(&refs);
    let (x, y) = cell_center(0);
    browser.begin_marquee(0, x, y, false);

    let band = browser.marquee_band().expect("a band is out");
    browser.columns[0].scroll.state.set_offset(40.0);

    let scrolled = browser.marquee_band().expect("still out");
    assert_eq!(scrolled.top, band.top - 40.0, "the band scrolled with them");
}

#[test]
fn moving_a_file_into_the_directory_it_is_already_in_does_nothing() {
    let (mut browser, dir) = browser_over(&["a.txt"], &[]);
    browser.drop_target = Some(DropTarget::Pane {
        depth: 0,
        path: dir.0.clone(),
    });

    browser.apply_drop(vec![dir.join("a.txt")], true);

    assert!(dir.join("a.txt").exists(), "the file is still there");
    assert!(
        !dir.join("a 2.txt").exists(),
        "and was not renamed out of the way of itself"
    );
    assert_eq!(browser.status, None, "a no-op reports nothing");
}

#[test]
fn copying_a_file_into_the_directory_it_is_already_in_duplicates_it() {
    // The same gesture with copy asked for is how a duplicate is made, so
    // this one is not skipped.
    let (mut browser, dir) = browser_over(&["a.txt"], &[]);
    browser.drop_target = Some(DropTarget::Pane {
        depth: 0,
        path: dir.0.clone(),
    });

    browser.apply_drop(vec![dir.join("a.txt")], false);

    assert!(dir.join("a.txt").exists(), "the original is untouched");
    let copies: Vec<_> = std::fs::read_dir(&dir.0)
        .expect("listing")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "a.txt")
        .collect();
    assert_eq!(copies.len(), 1, "exactly one copy, got {copies:?}");
}

#[test]
fn a_drop_moves_a_file_into_a_directory() {
    let (mut browser, dir) = browser_over(&["a.txt"], &["target"]);
    browser.drop_target = Some(DropTarget::Entry {
        depth: 0,
        index: row_of(&browser, "target"),
        path: dir.join("target"),
    });

    browser.apply_drop(vec![dir.join("a.txt")], true);

    assert!(dir.join("target/a.txt").exists(), "the file moved in");
    assert!(!dir.join("a.txt").exists(), "and left where it was");
}

#[test]
fn a_drop_copy_leaves_the_original_alone() {
    let (mut browser, dir) = browser_over(&["a.txt"], &["target"]);
    browser.drop_target = Some(DropTarget::Entry {
        depth: 0,
        index: row_of(&browser, "target"),
        path: dir.join("target"),
    });

    browser.apply_drop(vec![dir.join("a.txt")], false);

    assert!(dir.join("target/a.txt").exists(), "the copy arrived");
    assert!(dir.join("a.txt").exists(), "the original stayed");
}

#[test]
fn a_directory_cannot_be_dropped_into_itself() {
    let (mut browser, dir) = browser_over(&[], &["outer"]);
    std::fs::create_dir_all(dir.join("outer/inner")).expect("nested dir");
    browser.drop_target = Some(DropTarget::Pane {
        depth: 0,
        path: dir.join("outer/inner"),
    });

    browser.apply_drop(vec![dir.join("outer")], true);

    assert!(dir.join("outer").exists(), "nothing was moved");
    assert!(
        browser
            .status
            .as_deref()
            .is_some_and(|status| status.contains("itself")),
        "and it said so: {:?}",
        browser.status
    );
}

#[test]
fn the_drop_target_clears_once_it_has_been_applied() {
    let (mut browser, dir) = browser_over(&["a.txt"], &["target"]);
    browser.drop_target = Some(DropTarget::Entry {
        depth: 0,
        index: row_of(&browser, "target"),
        path: dir.join("target"),
    });

    browser.apply_drop(vec![dir.join("a.txt")], true);

    assert_eq!(browser.drop_target, None, "the outline goes with the drop");
    assert!(browser.dirty, "and the window repaints");
}
