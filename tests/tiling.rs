//! End-to-end tests for window tiling.
//!
//! Covers the state machine behind `TileWindowLeft` / `TileWindowRight`:
//! a window is Floating, Tiled(zone) or Maximized, and exactly one floating
//! rect is saved — captured when the window first leaves Floating, and never
//! overwritten while it stays out of it.

#[cfg(feature = "headless")]
mod tiling_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle, TileZone};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::time::Duration;

    /// A window under test, already focused and settled.
    struct Fixture {
        handle: HeadlessHandle,
        _client: TestClient,
        /// The rect the window had before anything tiled it.
        floating: (i32, i32, i32, i32),
    }

    const TITLE: &str = "tiling-window";

    /// Where the window is parked before any tiling. Deliberately not a spot
    /// any tiling zone would place it at: the client never resizes its buffer
    /// in these tests, so POSITION is what distinguishes tiled from floating.
    const PARK: (i32, i32) = (700, 300);

    fn setup() -> Fixture {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client =
            TestClient::connect(&handle.socket_name).expect("Failed to connect to compositor");

        let _toplevel = client.create_toplevel(TITLE, 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(300);

        handle.move_window(TITLE, PARK.0, PARK.1);
        handle.settle(300);
        handle.focus_window(TITLE);

        let floating = handle
            .window_logical_geometry(TITLE)
            .expect("window should be mapped");
        assert_eq!(
            (floating.0, floating.1),
            PARK,
            "the fixture needs a known floating position to compare against"
        );

        Fixture {
            handle,
            _client: client,
            floating,
        }
    }

    impl Fixture {
        fn tile(&self, zone: TileZone) {
            self.handle.tile_focused(zone);
            self.handle.settle(300);
        }

        fn toggle_maximize(&self) {
            self.handle.toggle_maximize_focused();
            self.handle.settle(300);
        }

        fn geometry(&self) -> (i32, i32, i32, i32) {
            self.handle
                .window_logical_geometry(TITLE)
                .expect("window should still be mapped")
        }

        fn zone(&self) -> Option<TileZone> {
            self.handle.window_tiled_zone(TITLE)
        }

        /// The rect untiling/unmaximizing aims at.
        fn saved_floating(&self) -> (i32, i32, i32, i32) {
            self.handle
                .window_floating_rect(TITLE)
                .expect("window should have a view")
        }
    }

    // ── Tiling and the untile toggle ─────────────────────────────────────

    #[test]
    #[serial]
    fn tile_left_snaps_to_the_left_half() {
        let f = setup();
        let (zx, zy, _, _) = f.handle.usable_zone();

        f.tile(TileZone::LeftHalf);

        assert_eq!(f.zone(), Some(TileZone::LeftHalf), "window should be tiled");
        let (x, y, _, _) = f.geometry();
        assert_eq!(
            (x, y),
            (zx, zy),
            "a left tile sits in the top-left corner of the usable area"
        );

        f.handle.stop();
    }

    #[test]
    #[serial]
    fn tile_right_snaps_to_the_right_half() {
        let f = setup();
        let (zx, zy, zw, _) = f.handle.usable_zone();

        f.tile(TileZone::RightHalf);

        assert_eq!(f.zone(), Some(TileZone::RightHalf));
        let (x, y, _, _) = f.geometry();
        assert_eq!(
            (x, y),
            (zx + zw - zw / 2, zy),
            "a right tile starts halfway across the usable area"
        );

        f.handle.stop();
    }

    #[test]
    #[serial]
    fn tiling_to_the_same_zone_again_untiles() {
        let f = setup();

        f.tile(TileZone::LeftHalf);
        assert_eq!(f.zone(), Some(TileZone::LeftHalf));

        // The same shortcut a second time is the way back out.
        f.tile(TileZone::LeftHalf);

        assert_eq!(f.zone(), None, "second tile-left should have untiled");
        let (x, y, _, _) = f.geometry();
        assert_eq!(
            (x, y),
            PARK,
            "untiled window should be back at its floating position"
        );

        f.handle.stop();
    }

    #[test]
    #[serial]
    fn tiling_to_the_other_zone_re_tiles_and_keeps_the_floating_rect() {
        let f = setup();

        f.tile(TileZone::LeftHalf);
        f.tile(TileZone::RightHalf);

        assert_eq!(
            f.zone(),
            Some(TileZone::RightHalf),
            "opposite zone should re-tile, not untile"
        );
        assert_eq!(
            f.saved_floating(),
            f.floating,
            "re-tiling must not overwrite the rect saved on the first snap"
        );

        // And the toggle still works from the new zone.
        f.tile(TileZone::RightHalf);
        assert_eq!(f.zone(), None);
        let (x, y, _, _) = f.geometry();
        assert_eq!((x, y), PARK);

        f.handle.stop();
    }

    #[test]
    #[serial]
    fn tiling_a_floating_window_records_its_rect() {
        let f = setup();

        f.tile(TileZone::LeftHalf);

        assert_eq!(
            f.saved_floating(),
            f.floating,
            "the first snap should record the pre-tile geometry"
        );

        f.handle.stop();
    }

    // ── Tiling and maximize ──────────────────────────────────────────────

    #[test]
    #[serial]
    fn unmaximizing_a_tiled_window_returns_to_the_tile() {
        let f = setup();

        f.tile(TileZone::LeftHalf);
        let tiled = f.geometry();

        f.toggle_maximize();
        assert!(f.handle.window_is_maximized(TITLE), "should be maximized");

        f.toggle_maximize();

        assert!(!f.handle.window_is_maximized(TITLE));
        assert_eq!(
            f.zone(),
            Some(TileZone::LeftHalf),
            "unmaximize is the inverse of maximize — land back on the tile"
        );
        assert_eq!(
            f.geometry(),
            tiled,
            "unmaximized window should be back on its tile, not floating"
        );

        f.handle.stop();
    }

    /// Regression: maximizing a tiled window used to overwrite the saved
    /// floating rect with the tile, losing the pre-tile geometry for good.
    #[test]
    #[serial]
    fn maximizing_a_tiled_window_preserves_the_floating_rect() {
        let f = setup();

        f.tile(TileZone::LeftHalf);
        f.toggle_maximize();

        assert_eq!(
            f.saved_floating(),
            f.floating,
            "maximize must not overwrite the floating rect with the tile"
        );

        f.handle.stop();
    }

    #[test]
    #[serial]
    fn tile_maximize_unmaximize_untile_ends_up_floating() {
        let f = setup();

        f.tile(TileZone::LeftHalf);
        f.toggle_maximize();
        f.toggle_maximize();
        // Back on the tile — one more press of the same shortcut goes floating.
        f.tile(TileZone::LeftHalf);

        assert_eq!(f.zone(), None);
        assert!(!f.handle.window_is_maximized(TITLE));
        let (x, y, _, _) = f.geometry();
        assert_eq!(
            (x, y),
            PARK,
            "the full unwind should reach the original position"
        );

        f.handle.stop();
    }

    #[test]
    #[serial]
    fn tiling_a_maximized_window_keeps_its_floating_rect() {
        let f = setup();

        f.toggle_maximize();
        f.tile(TileZone::LeftHalf);

        assert_eq!(f.zone(), Some(TileZone::LeftHalf));
        assert_eq!(
            f.saved_floating(),
            f.floating,
            "tiling out of maximize must keep the rect maximize saved"
        );

        f.handle.stop();
    }

    #[test]
    #[serial]
    fn double_clicking_the_titlebar_of_a_tiled_window_maximizes_it() {
        let f = setup();
        let (zx, zy, _, _) = f.handle.usable_zone();

        f.handle.decorate_window(TITLE);
        f.handle.settle(300);
        // The titlebar just changed the window's height — this is the floating
        // rect the zoom has to preserve.
        let floating = f.geometry();
        f.tile(TileZone::LeftHalf);

        f.handle.double_click_titlebar(TITLE);
        // The zoom is deferred to an idle callback (the pointer's lock is held
        // during dispatch) — give the event loop a turn before settling.
        f.handle.wait(Duration::from_millis(200));
        f.handle.settle(300);

        assert!(
            f.handle.window_is_maximized(TITLE),
            "a double click on a tiled window's titlebar should zoom it"
        );
        let (x, y, _, _) = f.geometry();
        assert_eq!(
            (x, y),
            (zx, zy),
            "the zoomed window should sit at the usable area's origin"
        );
        assert_eq!(
            f.saved_floating(),
            floating,
            "zooming out of a tile must not eat the floating rect"
        );

        // And back down onto the tile it came from.
        f.handle.double_click_titlebar(TITLE);
        // The zoom is deferred to an idle callback (the pointer's lock is held
        // during dispatch) — give the event loop a turn before settling.
        f.handle.wait(Duration::from_millis(200));
        f.handle.settle(300);

        assert!(!f.handle.window_is_maximized(TITLE));
        assert_eq!(f.zone(), Some(TileZone::LeftHalf));

        f.handle.stop();
    }

    // ── Hand-resizing drops the tile ─────────────────────────────────────

    #[test]
    #[serial]
    fn resizing_a_tiled_window_makes_it_floating_again() {
        let f = setup();

        f.tile(TileZone::LeftHalf);
        f.handle.begin_window_resize(TITLE);

        assert_eq!(f.zone(), None, "a hand-resized window is floating again");

        // So the shortcut snaps it afresh rather than untiling it to a rect
        // that no longer means anything.
        f.tile(TileZone::LeftHalf);
        assert_eq!(f.zone(), Some(TileZone::LeftHalf));

        f.handle.stop();
    }
}
