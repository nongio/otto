//! End-to-end tests for pointer interaction with tiles.
//!
//! Drives the same entry points the titlebar drag and the resize border land
//! on, with logical pointer positions, and asserts on the cells the tree
//! resolves to — the rectangles the clients are configured with. The grabs are
//! called directly rather than by synthesising input, so a test does not have
//! to reason about pointer focus or the drag threshold.

#[cfg(feature = "headless")]
mod tiling_drag_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::time::Duration;

    struct Window {
        /// Kept alive so dropping it closes the window.
        #[allow(dead_code)]
        client: TestClient,
    }

    fn spawn(handle: &HeadlessHandle, title: &str) -> Window {
        let mut client =
            TestClient::connect(&handle.socket_name).expect("Failed to connect to compositor");
        let _toplevel = client.create_toplevel(title, 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(300);
        Window { client }
    }

    /// A compositor with `titles` mapped, tiled and settled.
    fn tiled(titles: &[&str]) -> (HeadlessHandle, Vec<Window>) {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let windows: Vec<Window> = titles.iter().map(|t| spawn(&handle, t)).collect();
        handle.settle(300);
        if let Some(first) = titles.first() {
            handle.focus_window(first);
        }
        handle.toggle_tiling();
        handle.settle(600);
        (handle, windows)
    }

    /// The titles in layout order — left to right, top to bottom.
    fn order(handle: &HeadlessHandle) -> Vec<String> {
        handle
            .tiling_cell_rects()
            .into_iter()
            .map(|(title, _)| title)
            .collect()
    }

    fn cell(handle: &HeadlessHandle, title: &str) -> (i32, i32, i32, i32) {
        handle
            .tiling_cell_rects()
            .into_iter()
            .find(|(t, _)| t == title)
            .unwrap_or_else(|| panic!("{title} should be a tile"))
            .1
    }

    /// Detach `dragged`, aim at `fx`/`fy` of the way across `target`'s cell
    /// **as it stands with the dragged window out of the tree**, and drop.
    ///
    /// The tree closes up on the detach, so the cell the pointer is over is
    /// not the one it was before the drag started — which is exactly what the
    /// user sees.
    fn drag_onto(handle: &HeadlessHandle, dragged: &str, target: &str, fx: f64, fy: f64) {
        handle.tiling_drag_begin(dragged);
        handle.settle(200);
        let (x, y) = at(cell(handle, target), fx, fy);
        handle.tiling_drag_motion(x, y);
        handle.tiling_drag_drop(x, y);
        handle.settle(400);
    }

    /// A point inside `rect`, `fx`/`fy` of the way across it.
    fn at(rect: (i32, i32, i32, i32), fx: f64, fy: f64) -> (f64, f64) {
        (
            rect.0 as f64 + rect.2 as f64 * fx,
            rect.1 as f64 + rect.3 as f64 * fy,
        )
    }

    // ── Dragging a window into a slot ────────────────────────────────────

    #[test]
    #[serial]
    fn a_drop_on_the_right_half_inserts_after_that_tile() {
        let (handle, windows) = tiled(&["drag-a", "drag-b", "drag-c"]);
        let start = order(&handle);
        assert_eq!(start.len(), 3, "three tiles: {start:?}");
        let (dragged, target) = (start[0].clone(), start[2].clone());

        // 90% of the way across the target tile: its right half, well clear
        // of the central band that means "swap".
        drag_onto(&handle, &dragged, &target, 0.9, 0.5);

        assert_eq!(
            order(&handle),
            vec![start[1].clone(), target.clone(), dragged.clone()],
            "the dragged window lands right of the tile it was dropped on"
        );
        drop(windows);
        handle.stop();
    }

    #[test]
    #[serial]
    fn a_drop_on_the_left_half_inserts_before_that_tile() {
        let (handle, windows) = tiled(&["left-a", "left-b", "left-c"]);
        let start = order(&handle);
        let (dragged, target) = (start[0].clone(), start[2].clone());

        drag_onto(&handle, &dragged, &target, 0.1, 0.5);

        assert_eq!(
            order(&handle),
            vec![start[1].clone(), dragged.clone(), target.clone()],
        );
        drop(windows);
        handle.stop();
    }

    #[test]
    #[serial]
    fn a_drop_on_the_middle_swaps_the_two_windows() {
        let (handle, windows) = tiled(&["swap-a", "swap-b", "swap-c"]);
        let start = order(&handle);
        let before: Vec<(i32, i32, i32, i32)> = start.iter().map(|t| cell(&handle, t)).collect();
        let (dragged, target) = (start[0].clone(), start[2].clone());

        drag_onto(&handle, &dragged, &target, 0.5, 0.5);

        assert_eq!(
            order(&handle),
            vec![target.clone(), start[1].clone(), dragged.clone()],
            "the two windows trade places"
        );
        let after: Vec<(i32, i32, i32, i32)> =
            order(&handle).iter().map(|t| cell(&handle, t)).collect();
        assert_eq!(after, before, "and the cells themselves do not move");
        drop(windows);
        handle.stop();
    }

    #[test]
    #[serial]
    fn the_slot_overlay_shows_the_half_that_would_be_taken() {
        let (handle, windows) = tiled(&["overlay-a", "overlay-b"]);
        let start = order(&handle);
        let target = start[1].clone();
        let rect = cell(&handle, &target);

        handle.tiling_drag_begin(&start[0]);
        handle.settle(200);
        let (x, y) = at(rect, 0.9, 0.5);
        handle.tiling_drag_motion(x, y);
        handle.settle(400);

        let preview = handle.tiling_drag_preview().expect("a slot pane");
        // The tree closed up behind the detached window, so the target now
        // fills the area: the preview is its right half.
        let target_now = cell(&handle, &target);
        assert!(
            (preview.2 - target_now.2 / 2).abs() <= 2,
            "half the tile's width: {preview:?} of {target_now:?}"
        );
        assert!(
            preview.0 > target_now.0 + target_now.2 / 4,
            "and the right half of it: {preview:?} of {target_now:?}"
        );

        handle.tiling_drag_cancel();
        handle.settle(400);
        drop(windows);
        handle.stop();
    }

    #[test]
    #[serial]
    fn escape_puts_the_window_back_where_it_was() {
        let (handle, windows) = tiled(&["cancel-a", "cancel-b", "cancel-c"]);
        let before = handle.tiling_cell_rects();
        let dragged = order(&handle)[0].clone();

        handle.tiling_drag_begin(&dragged);
        handle.settle(200);
        assert!(handle.tiling_drag_active(), "a drag is in flight");
        assert_eq!(
            order(&handle).len(),
            2,
            "and the tree closed up behind the detached window"
        );

        let (x, y) = at(cell(&handle, &order(&handle)[1]), 0.9, 0.5);
        handle.tiling_drag_motion(x, y);
        handle.tiling_drag_cancel();
        handle.settle(600);

        assert!(!handle.tiling_drag_active());
        assert_eq!(
            handle.tiling_cell_rects(),
            before,
            "the layout is exactly as it was"
        );
        drop(windows);
        handle.stop();
    }

    // ── Dragging an edge ─────────────────────────────────────────────────

    #[test]
    #[serial]
    fn an_edge_between_two_tiles_moves_the_split() {
        let (handle, windows) = tiled(&["edge-a", "edge-b", "edge-c"]);
        let before = handle.tiling_cell_rects();
        assert_eq!(before.len(), 3, "three tiles: {before:?}");

        // Find a tile stacked directly on another — the same column, one
        // above the other — and the tile that is in neither of their cells.
        let (upper, lower) = before
            .iter()
            .flat_map(|a| before.iter().map(move |b| (a, b)))
            .find(|(a, b)| a.0 != b.0 && a.1 .0 == b.1 .0 && a.1 .2 == b.1 .2 && a.1 .1 < b.1 .1)
            .map(|(a, b)| (a.0.clone(), b.0.clone()))
            .expect("two tiles sharing a column");
        let other = before
            .iter()
            .find(|(t, _)| *t != upper && *t != lower)
            .expect("a third tile")
            .0
            .clone();

        let upper_before = cell(&handle, &upper);
        let lower_before = cell(&handle, &lower);
        let other_before = cell(&handle, &other);

        // The bar between them, pushed 60 logical pixels down.
        let boundary = (upper_before.1 + upper_before.3) as f64;
        let x = upper_before.0 as f64 + upper_before.2 as f64 / 2.0;
        assert!(
            handle.tiling_resize_drag(&upper, "bottom", x, boundary + 60.0),
            "the inner edge has a split under it"
        );

        let upper_after = cell(&handle, &upper);
        let lower_after = cell(&handle, &lower);
        assert!(
            upper_after.3 > upper_before.3,
            "the upper tile grew: {upper_before:?} -> {upper_after:?}"
        );
        assert!(
            lower_after.3 < lower_before.3,
            "its neighbour gave the height up: {lower_before:?} -> {lower_after:?}"
        );
        assert_eq!(
            cell(&handle, &other),
            other_before,
            "and the tile outside the split did not move"
        );
        drop(windows);
        handle.stop();
    }

    #[test]
    #[serial]
    fn an_outer_edge_has_nothing_to_drag() {
        let (handle, windows) = tiled(&["outer-a", "outer-b"]);
        let first = order(&handle)[0].clone();
        let before = handle.tiling_cell_rects();

        assert!(
            !handle.tiling_resize_drag(&first, "left", 400.0, 300.0),
            "the leftmost tile's left edge is the edge of the usable area"
        );
        handle.settle(400);

        assert_eq!(handle.tiling_cell_rects(), before, "nothing moved");
        drop(windows);
        handle.stop();
    }

    #[test]
    #[serial]
    fn a_lone_tile_has_no_edge_to_drag_and_no_slot_to_leave() {
        let (handle, windows) = tiled(&["lone-a"]);
        let before = handle.tiling_cell_rects();
        assert_eq!(before.len(), 1);

        for edge in ["left", "right", "top", "bottom", "bottom-right"] {
            assert!(
                !handle.tiling_resize_drag("lone-a", edge, 500.0, 400.0),
                "{edge} of a lone tile drags nothing"
            );
        }
        assert_eq!(handle.tiling_cell_rects(), before);
        drop(windows);
        handle.stop();
    }
}
