//! End-to-end tests for the tiling tree.
//!
//! Drives the same entry points the `TilingToggle`, `Focus*`, `MoveContainer*`
//! shortcuts land on, and asserts on the cells the tree resolves to — the
//! rectangles the clients are configured with. The test client never resizes
//! its own buffer, so a window's *position* is what shows it moved.

#[cfg(feature = "headless")]
mod tiling_tree_tests {
    use otto::headless::{Axis, Direction, HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::time::Duration;

    /// One window, with the client that owns it kept alive so the window can
    /// be closed by dropping it.
    struct Window {
        /// Kept alive so dropping it closes the window.
        #[allow(dead_code)]
        client: TestClient,
        title: String,
    }

    fn spawn(handle: &HeadlessHandle, title: &str) -> Window {
        let mut client =
            TestClient::connect(&handle.socket_name).expect("Failed to connect to compositor");
        let _toplevel = client.create_toplevel(title, 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(300);
        Window {
            client,
            title: title.to_string(),
        }
    }

    /// A compositor with `titles` mapped and settled, floating.
    fn setup(titles: &[&str]) -> (HeadlessHandle, Vec<Window>) {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let windows: Vec<Window> = titles.iter().map(|t| spawn(&handle, t)).collect();
        handle.settle(300);
        (handle, windows)
    }

    /// Cells keyed by title, in layout order.
    fn cells(handle: &HeadlessHandle) -> Vec<(String, (i32, i32, i32, i32))> {
        handle.tiling_cell_rects()
    }

    /// Rectangles that agree to within one inner gap — enough slack for the
    /// gap an extra sibling costs its container, and for integer rounding.
    fn assert_close(got: (i32, i32, i32, i32), want: (i32, i32, i32, i32), what: &str) {
        const SLACK: i32 = 8;
        let deltas = [
            (got.0 - want.0).abs(),
            (got.1 - want.1).abs(),
            (got.2 - want.2).abs(),
            (got.3 - want.3).abs(),
        ];
        assert!(
            deltas.iter().all(|d| *d <= SLACK),
            "{what}: {got:?} is not within {SLACK}px of {want:?}"
        );
    }

    fn cell(handle: &HeadlessHandle, title: &str) -> (i32, i32, i32, i32) {
        cells(handle)
            .into_iter()
            .find(|(t, _)| t == title)
            .unwrap_or_else(|| panic!("{title} should be a tile"))
            .1
    }

    // ── Entering tiling mode ─────────────────────────────────────────────

    #[test]
    #[serial]
    fn tiling_two_windows_fills_the_usable_zone_side_by_side() {
        let (handle, windows) = setup(&["tile-a", "tile-b"]);
        handle.focus_window("tile-a");
        handle.toggle_tiling();
        handle.settle(600);

        assert!(handle.workspace_tiling_enabled(), "the workspace tiles");
        let cells = cells(&handle);
        assert_eq!(cells.len(), 2, "both windows joined the tree: {cells:?}");

        let (zx, zy, zw, zh) = handle.usable_zone();
        // The defaults: an 8px gap around the tree and between the tiles.
        let (_, left) = &cells[0];
        let (_, right) = &cells[1];
        assert_eq!(left.0, zx + 8, "the first cell starts inside the outer gap");
        assert_eq!(left.1, zy + 8);
        assert_eq!(left.3, zh - 16, "cells fill the usable height");
        assert_eq!(
            right.0 - (left.0 + left.2),
            8,
            "one inner gap between the two cells"
        );
        assert_eq!(
            right.0 + right.2,
            zx + zw - 8,
            "the tree ends inside the outer gap"
        );

        drop(windows);
        handle.stop();
    }

    #[test]
    #[serial]
    fn windows_are_moved_into_their_cells() {
        let (handle, windows) = setup(&["tile-a", "tile-b"]);
        handle.focus_window("tile-a");
        handle.toggle_tiling();
        handle.settle(600);

        for (title, rect) in cells(&handle) {
            let geometry = handle
                .window_logical_geometry(&title)
                .expect("tiles stay mapped");
            assert_eq!(
                (geometry.0, geometry.1),
                (rect.0, rect.1),
                "{title} should sit at its cell origin"
            );
        }

        drop(windows);
        handle.stop();
    }

    // ── Insertion ────────────────────────────────────────────────────────

    #[test]
    #[serial]
    fn a_third_window_splits_the_focused_cell() {
        let (handle, mut windows) = setup(&["tile-a", "tile-b"]);
        handle.focus_window("tile-b");
        handle.toggle_tiling();
        handle.settle(600);

        let before = cell(&handle, "tile-b");
        let other_before = cell(&handle, "tile-a");

        let third = spawn(&handle, "tile-c");
        handle.settle(600);

        let leaves = handle.tiling_tree_leaves();
        assert_eq!(
            leaves.len(),
            3,
            "the new window joined the tree: {leaves:?}"
        );

        // The insertion takes half of the focused window's share and leaves
        // the other tile alone: together the two cells still cover the
        // rectangle tile-b used to have, minus the gap between them.
        let b = cell(&handle, "tile-b");
        let c = cell(&handle, "tile-c");
        let union = (
            b.0.min(c.0),
            b.1.min(c.1),
            (b.0 + b.2).max(c.0 + c.2) - b.0.min(c.0),
            (b.1 + b.3).max(c.1 + c.3) - b.1.min(c.1),
        );
        // Within the inner gap the extra child costs: the two cells still
        // stand where tile-b's one did.
        assert_close(union, before, "the split stays inside the focused cell");
        assert!(
            b.2 * b.3 < before.2 * before.3,
            "the focused cell gave up half its space: {b:?} was {before:?}"
        );
        assert_close(
            cell(&handle, "tile-a"),
            other_before,
            "the other siblings keep their place",
        );

        windows.push(third);
        drop(windows);
        handle.stop();
    }

    #[test]
    #[serial]
    fn a_preselect_forces_the_axis_of_the_next_split() {
        let (handle, mut windows) = setup(&["tile-a", "tile-b"]);
        handle.focus_window("tile-b");
        handle.toggle_tiling();
        handle.settle(600);

        let before = cell(&handle, "tile-b");
        // Split top/bottom, whatever shape the focused cell has.
        handle.tiling_split(Axis::Column);

        let third = spawn(&handle, "tile-c");
        handle.settle(600);

        let b = cell(&handle, "tile-b");
        let c = cell(&handle, "tile-c");
        assert_eq!(
            (b.0, b.2),
            (before.0, before.2),
            "the column keeps its width"
        );
        assert_eq!((c.0, c.2), (before.0, before.2));
        assert!(c.1 > b.1, "the new cell took the bottom half: {b:?} {c:?}");

        windows.push(third);
        drop(windows);
        handle.stop();
    }

    // ── Removal ──────────────────────────────────────────────────────────

    #[test]
    #[serial]
    fn closing_a_tile_gives_its_space_to_the_survivor() {
        let (handle, mut windows) = setup(&["tile-a", "tile-b"]);
        handle.focus_window("tile-a");
        handle.toggle_tiling();
        handle.settle(600);
        assert_eq!(handle.tiling_tree_leaves().len(), 2);

        // Dropping the client destroys its toplevel, which unmaps the window.
        let closed = windows.pop().expect("two windows");
        assert_eq!(closed.title, "tile-b");
        drop(closed);
        handle.wait(Duration::from_millis(100));
        handle.settle(600);

        let leaves = handle.tiling_tree_leaves();
        assert_eq!(leaves, vec!["tile-a".to_string()], "one tile is left");

        // A lone tile with smart gaps on fills the usable area outright.
        let (zx, zy, zw, zh) = handle.usable_zone();
        assert_eq!(cell(&handle, "tile-a"), (zx, zy, zw, zh));

        drop(windows);
        handle.stop();
    }

    // ── Navigation and movement ──────────────────────────────────────────

    #[test]
    #[serial]
    fn focus_right_moves_the_keyboard_focus_to_the_next_cell() {
        let (handle, windows) = setup(&["tile-a", "tile-b"]);
        handle.focus_window("tile-a");
        handle.toggle_tiling();
        handle.settle(600);

        let cells = cells(&handle);
        let leftmost = cells[0].0.clone();
        let rightmost = cells[1].0.clone();
        handle.focus_window(&leftmost);
        handle.settle(100);

        handle.tiling_focus(Direction::Right);
        handle.settle(300);
        assert_eq!(handle.focused_window_title(), Some(rightmost.clone()));

        // Focus never wraps: there is nothing further right.
        handle.tiling_focus(Direction::Right);
        handle.settle(300);
        assert_eq!(handle.focused_window_title(), Some(rightmost.clone()));

        handle.tiling_focus(Direction::Left);
        handle.settle(300);
        assert_eq!(handle.focused_window_title(), Some(leftmost));

        drop(windows);
        handle.stop();
    }

    #[test]
    #[serial]
    fn moving_a_tile_swaps_it_with_its_neighbour() {
        let (handle, windows) = setup(&["tile-a", "tile-b"]);
        handle.focus_window("tile-a");
        handle.toggle_tiling();
        handle.settle(600);

        let before = handle.tiling_tree_leaves();
        assert_eq!(before.len(), 2);
        handle.focus_window(&before[1]);
        handle.settle(100);

        handle.tiling_move(Direction::Left);
        handle.settle(600);

        let after = handle.tiling_tree_leaves();
        assert_eq!(
            after,
            vec![before[1].clone(), before[0].clone()],
            "the two tiles traded places"
        );

        drop(windows);
        handle.stop();
    }

    // ── Leaving tiling mode ──────────────────────────────────────────────

    #[test]
    #[serial]
    fn leaving_tiling_mode_restores_the_floating_rects() {
        let (handle, windows) = setup(&["tile-a", "tile-b"]);
        handle.move_window("tile-a", 120, 90);
        handle.move_window("tile-b", 700, 300);
        handle.settle(300);
        let before: Vec<(String, (i32, i32, i32, i32))> = ["tile-a", "tile-b"]
            .iter()
            .map(|t| {
                (
                    t.to_string(),
                    handle.window_logical_geometry(t).expect("mapped"),
                )
            })
            .collect();

        handle.focus_window("tile-a");
        handle.toggle_tiling();
        handle.settle(600);
        assert!(handle.workspace_tiling_enabled());

        handle.toggle_tiling();
        handle.settle(600);
        assert!(!handle.workspace_tiling_enabled());
        assert!(
            handle.tiling_tree_leaves().is_empty(),
            "the tree is emptied on the way out"
        );

        for (title, rect) in before {
            let now = handle
                .window_logical_geometry(&title)
                .expect("still mapped");
            assert_eq!(
                (now.0, now.1),
                (rect.0, rect.1),
                "{title} went back to where it floated"
            );
        }

        drop(windows);
        handle.stop();
    }
}
