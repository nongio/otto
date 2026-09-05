//! End-to-end tests for design mode.
//!
//! Drives the same entry points the `TilingDesignToggle` shortcut, the bar
//! drag and the pane toolbar land on, and asserts on the cells the tree
//! resolves to — the rectangles the clients are configured with. The drag is
//! called through the harness's drag entry point rather than by synthesising
//! pointer input, so a test does not have to reason about pointer focus.

#[cfg(feature = "headless")]
mod tiling_design_tests {
    use otto::headless::{Axis, HeadlessConfig, HeadlessHandle};
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

    /// A compositor with `titles` mapped, tiled and settled, design mode off.
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

    fn cell(handle: &HeadlessHandle, title: &str) -> (i32, i32, i32, i32) {
        handle
            .tiling_cell_rects()
            .into_iter()
            .find(|(t, _)| t == title)
            .unwrap_or_else(|| panic!("{title} should be a tile"))
            .1
    }

    /// The cell at `index` in layout order — left to right, top to bottom.
    /// Which *window* lands where depends on the order the tree was built in,
    /// which is not what these tests are about.
    fn nth(handle: &HeadlessHandle, index: usize) -> (i32, i32, i32, i32) {
        handle
            .tiling_cell_rects()
            .get(index)
            .unwrap_or_else(|| panic!("a cell at {index}"))
            .1
    }

    // ── Entering and leaving ─────────────────────────────────────────────

    #[test]
    #[serial]
    fn design_mode_draws_a_pane_per_cell() {
        let (handle, windows) = tiled(&["design-a", "design-b"]);
        assert!(!handle.tiling_design_active(), "off until it is asked for");

        handle.toggle_tiling_design();
        handle.settle(400);

        assert!(handle.tiling_design_active());
        let panes = handle.tiling_design_panes();
        assert_eq!(panes.len(), 2, "one pane per cell: {panes:?}");
        assert!(panes.iter().all(|(empty, _, _)| !*empty), "{panes:?}");
        assert_eq!(
            panes.iter().filter(|(_, focused, _)| *focused).count(),
            1,
            "exactly one pane carries the accent border: {panes:?}"
        );
        // Two columns: one bar between them, no corner.
        assert_eq!(handle.tiling_design_bars().len(), 1);

        drop(windows);
        handle.stop();
    }

    #[test]
    #[serial]
    fn a_floating_workspace_has_nothing_to_design() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let window = spawn(&handle, "design-float");
        handle.settle(300);

        handle.toggle_tiling_design();
        handle.settle(300);

        assert!(
            !handle.tiling_design_active(),
            "design mode is a no-op off a tiling workspace"
        );
        assert!(handle.tiling_design_panes().is_empty());

        drop(window);
        handle.stop();
    }

    #[test]
    #[serial]
    fn leaving_design_mode_changes_nothing() {
        let (handle, windows) = tiled(&["design-a", "design-b"]);
        let before = handle.tiling_cell_rects();

        handle.toggle_tiling_design();
        handle.settle(400);
        // The action again is the same as Escape.
        handle.toggle_tiling_design();
        handle.settle(400);

        assert!(!handle.tiling_design_active());
        assert!(handle.tiling_design_panes().is_empty(), "the grid is gone");
        assert_eq!(
            handle.tiling_cell_rects(),
            before,
            "the windows are where the layout put them"
        );

        drop(windows);
        handle.stop();
    }

    // ── Dragging a bar ───────────────────────────────────────────────────

    #[test]
    #[serial]
    fn dragging_a_bar_moves_the_two_shares_and_relays_out() {
        let (handle, windows) = tiled(&["design-a", "design-b"]);
        handle.toggle_tiling_design();
        handle.settle(400);

        let before_left = nth(&handle, 0);
        let before_right = nth(&handle, 1);
        let (zx, _, zw, _) = handle.usable_zone();

        // Pull the split to a quarter of the way across.
        let target = zx as f64 + zw as f64 * 0.25;
        handle.tiling_design_drag_bar(0, target, 400.0);
        handle.settle(800);

        let after_a = nth(&handle, 0);
        let after_b = nth(&handle, 1);
        assert!(
            after_a.2 < before_left.2,
            "the left cell shrank: {before_left:?} -> {after_a:?}"
        );
        assert!(
            after_b.2 > before_right.2,
            "the right cell grew: {before_right:?} -> {after_b:?}"
        );
        assert_eq!(
            after_a.2 + after_b.2,
            before_left.2 + before_right.2,
            "the pair still fills the same width"
        );
        // The snap targets are halves, thirds and quarters: a drag to a
        // quarter lands exactly on one.
        let usable = (after_a.2 + after_b.2) as f64;
        let ratio = after_a.2 as f64 / usable;
        assert!((ratio - 0.25).abs() < 0.02, "snapped to a quarter: {ratio}");

        drop(windows);
        handle.stop();
    }

    #[test]
    #[serial]
    fn double_clicking_a_bar_equalises_its_container() {
        let (handle, windows) = tiled(&["design-a", "design-b"]);
        handle.toggle_tiling_design();
        handle.settle(400);

        let (zx, _, zw, _) = handle.usable_zone();
        handle.tiling_design_drag_bar(0, zx as f64 + zw as f64 * 0.25, 400.0);
        handle.settle(600);
        assert!(
            nth(&handle, 0).2 < nth(&handle, 1).2,
            "the drag skewed them"
        );

        handle.tiling_design_equalize_bar(0);
        handle.settle(600);

        let a = nth(&handle, 0);
        let b = nth(&handle, 1);
        assert!(
            (a.2 - b.2).abs() <= 1,
            "the two cells are even again: {a:?} vs {b:?}"
        );

        drop(windows);
        handle.stop();
    }

    // ── Empty slots ──────────────────────────────────────────────────────

    #[test]
    #[serial]
    fn splitting_a_cell_leaves_a_slot_the_next_window_fills() {
        let (handle, mut windows) = tiled(&["design-a"]);
        handle.toggle_tiling_design();
        handle.settle(400);
        assert_eq!(handle.tiling_design_panes().len(), 1);

        handle.tiling_design_split_pane(0, Axis::Row);
        handle.settle(600);

        assert_eq!(handle.tiling_empty_slots(), 1, "the split left a slot");
        let panes = handle.tiling_design_panes();
        assert_eq!(panes.len(), 2);
        assert_eq!(
            panes.iter().filter(|(empty, _, _)| *empty).count(),
            1,
            "one dashed pane: {panes:?}"
        );

        // The next window to map fills it rather than splitting anything.
        windows.push(spawn(&handle, "design-b"));
        handle.settle(600);

        assert_eq!(handle.tiling_empty_slots(), 0, "the slot was filled");
        let cells = handle.tiling_cell_rects();
        assert_eq!(cells.len(), 2, "still two cells: {cells:?}");
        let a = cell(&handle, "design-a");
        let b = cell(&handle, "design-b");
        assert!(a.0 < b.0, "the new window took the slot beside the old one");

        drop(windows);
        handle.stop();
    }

    #[test]
    #[serial]
    fn closing_an_empty_slot_gives_its_space_back() {
        let (handle, windows) = tiled(&["design-a"]);
        handle.toggle_tiling_design();
        handle.settle(400);
        let whole = cell(&handle, "design-a");

        handle.tiling_design_split_pane(0, Axis::Row);
        handle.settle(600);
        assert!(cell(&handle, "design-a").2 < whole.2);

        // The slot is the second pane in layout order.
        let panes = handle.tiling_design_panes();
        let slot = panes
            .iter()
            .position(|(empty, _, _)| *empty)
            .expect("a dashed pane");
        handle.tiling_design_close_pane(slot);
        handle.settle(600);

        assert_eq!(handle.tiling_empty_slots(), 0);
        assert_eq!(
            cell(&handle, "design-a"),
            whole,
            "the window has the whole area back"
        );

        drop(windows);
        handle.stop();
    }

    // ── Presets ──────────────────────────────────────────────────────────

    #[test]
    #[serial]
    fn a_preset_lays_an_empty_workspace_out_in_slots() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        handle.settle(300);
        handle.toggle_tiling();
        handle.settle(400);
        handle.toggle_tiling_design();
        handle.settle(400);
        assert!(
            handle.tiling_design_active(),
            "an empty workspace still tiles"
        );

        handle.tiling_design_apply_preset("main-and-stack");
        handle.settle(600);

        assert_eq!(handle.tiling_empty_slots(), 3);
        let panes = handle.tiling_design_panes();
        assert_eq!(panes.len(), 3, "{panes:?}");
        assert!(panes.iter().all(|(empty, _, _)| *empty));
        // Main on the left is taller than either of the stacked pair.
        let (_, _, main) = panes[0];
        let (_, _, top) = panes[1];
        assert!(main.3 > top.3, "main is full height: {main:?} vs {top:?}");
        assert!(main.0 < top.0, "main is on the left");

        let grid = handle.tiling_design_bars().len();
        assert!(grid >= 2, "a main-and-stack has two splits: {grid}");

        handle.stop();
    }

    #[test]
    #[serial]
    fn a_two_column_preset_makes_two_slots() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        handle.settle(300);
        handle.toggle_tiling();
        handle.settle(400);
        handle.toggle_tiling_design();
        handle.settle(400);

        handle.tiling_design_apply_preset("two-columns");
        handle.settle(600);
        assert_eq!(handle.tiling_empty_slots(), 2);

        handle.tiling_design_apply_preset("grid");
        handle.settle(600);
        assert_eq!(handle.tiling_empty_slots(), 4);

        handle.stop();
    }

    // ── Undo ─────────────────────────────────────────────────────────────

    #[test]
    #[serial]
    fn undo_restores_the_geometry_before_the_last_edit() {
        let (handle, windows) = tiled(&["design-a", "design-b"]);
        handle.toggle_tiling_design();
        handle.settle(400);
        let before = handle.tiling_cell_rects();

        let (zx, _, zw, _) = handle.usable_zone();
        handle.tiling_design_drag_bar(0, zx as f64 + zw as f64 * 0.25, 400.0);
        handle.settle(600);
        assert_ne!(handle.tiling_cell_rects(), before, "the drag moved things");

        handle.tiling_undo();
        handle.settle(800);

        assert_eq!(
            handle.tiling_cell_rects(),
            before,
            "undo put the shares back"
        );

        drop(windows);
        handle.stop();
    }

    #[test]
    #[serial]
    fn undo_takes_a_split_back_out() {
        let (handle, windows) = tiled(&["design-a"]);
        handle.toggle_tiling_design();
        handle.settle(400);
        let whole = cell(&handle, "design-a");

        handle.tiling_design_split_pane(0, Axis::Row);
        handle.settle(600);
        assert_eq!(handle.tiling_empty_slots(), 1);

        handle.tiling_undo();
        handle.settle(600);

        assert_eq!(handle.tiling_empty_slots(), 0);
        assert_eq!(cell(&handle, "design-a"), whole);
        // Nothing left to undo: a second one is a no-op, not a panic.
        handle.tiling_undo();
        handle.settle(300);
        assert_eq!(cell(&handle, "design-a"), whole);

        drop(windows);
        handle.stop();
    }

    // ── Tiling off ───────────────────────────────────────────────────────

    #[test]
    #[serial]
    fn leaving_tiling_mode_ends_design_mode() {
        let (handle, windows) = tiled(&["design-a", "design-b"]);
        handle.toggle_tiling_design();
        handle.settle(400);
        assert!(handle.tiling_design_active());

        handle.toggle_tiling();
        handle.settle(600);

        assert!(!handle.workspace_tiling_enabled());
        assert!(!handle.tiling_design_active(), "the editor went with it");
        assert!(handle.tiling_design_panes().is_empty());

        drop(windows);
        handle.stop();
    }
}
