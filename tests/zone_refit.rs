//! End-to-end tests for maximized and half-tiled windows following the usable
//! area: when the dock changes edge, size or autohide, or a panel reserves or
//! releases space, those windows are moved and resized to fill the new zone.
//! See `specs/usable-area-refit.md`.

#[cfg(feature = "headless")]
mod zone_refit_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle, TileZone};
    use otto::settings::value::SettingValue;
    use otto_kit::testing::{KeyboardInteractivity, Layer, TestClient, TestToplevel};
    use serial_test::serial;
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    const TITLE: &str = "zone-refit-window";

    /// The dock keys these tests change. The running configuration outlives a
    /// headless session, so each test starts from the values the process
    /// began with rather than whatever the previous test left.
    const DOCK_KEYS: [&str; 3] = ["dock.position", "dock.size", "dock.autohide"];

    struct Fixture {
        handle: HeadlessHandle,
        client: TestClient,
        toplevel: Arc<Mutex<TestToplevel>>,
    }

    fn setup() -> Fixture {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        restore_dock_settings(&handle);
        let mut client =
            TestClient::connect(&handle.socket_name).expect("Failed to connect to compositor");
        let toplevel = client.create_toplevel(TITLE, 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(300);
        handle.move_window(TITLE, 700, 300);
        handle.settle(300);
        handle.focus_window(TITLE);
        Fixture {
            handle,
            client,
            toplevel,
        }
    }

    fn restore_dock_settings(handle: &HeadlessHandle) {
        static ORIGINAL: OnceLock<Vec<(&'static str, SettingValue)>> = OnceLock::new();
        let original = ORIGINAL.get_or_init(|| {
            DOCK_KEYS
                .iter()
                .map(|id| (*id, handle.setting_value(id).expect("dock setting")))
                .collect()
        });
        for (id, value) in original {
            if handle.setting_value(id).as_ref() != Some(value) {
                handle
                    .set_setting(id, value.clone())
                    .unwrap_or_else(|err| panic!("restoring {id} was refused: {err}"));
            }
        }
        handle.settle(300);
    }

    impl Fixture {
        /// Where the window sits and how wide the client was last told to
        /// be. The test client never resizes its buffer, so the configured
        /// width is what shows the resize; the height also loses any
        /// titlebar Otto draws, so it is left out.
        fn placement(&mut self) -> (i32, i32, i32) {
            let _ = self.client.roundtrip();
            let (x, y, _, _) = self
                .handle
                .window_logical_geometry(TITLE)
                .expect("window should be mapped");
            (x, y, self.toplevel.lock().unwrap().width)
        }

        fn maximize(&mut self) {
            self.handle.toggle_maximize_focused();
            self.handle.settle(400);
            let _ = self.client.roundtrip();
            assert_eq!(self.placement(), filling(self.handle.usable_zone()));
        }

        /// Change a dock setting, wait for the windows to follow, and return
        /// the new usable zone, which the change must have moved.
        fn change_dock(&mut self, id: &str, value: SettingValue) -> (i32, i32, i32, i32) {
            let before = self.handle.usable_zone();
            self.handle
                .set_setting(id, value)
                .unwrap_or_else(|err| panic!("{id} was refused: {err}"));
            self.wait_for_dock_refit();
            let after = self.handle.usable_zone();
            assert_ne!(after, before, "{id} should have changed the usable area");
            after
        }

        /// The dock animates to its new shape and the windows follow once it
        /// rests, which Otto checks on a wall-clock timer. The client acks
        /// configures as they come: each ack costs the compositor a
        /// workspace-model update, and a whole animation's worth at once can
        /// outlast a query.
        fn wait_for_dock_refit(&mut self) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while self.handle.dock_refit_pending() {
                assert!(
                    Instant::now() < deadline,
                    "the dock never came to rest after the change"
                );
                self.handle.settle(10);
                let _ = self.client.roundtrip();
                self.handle.wait(Duration::from_millis(20));
            }
            // The refit animation itself.
            self.handle.settle(120);
            let _ = self.client.roundtrip();
        }
    }

    /// The placement a window filling `rect` should have.
    fn filling(rect: (i32, i32, i32, i32)) -> (i32, i32, i32) {
        (rect.0, rect.1, rect.2)
    }

    fn position(edge: &str) -> SettingValue {
        SettingValue::Str(edge.into())
    }

    /// Every edge once, starting from the one after the dock's and ending
    /// back where it began.
    fn edge_tour(handle: &HeadlessHandle) -> Vec<&'static str> {
        const EDGES: [&str; 3] = ["bottom", "left", "right"];
        let Some(SettingValue::Str(current)) = handle.setting_value("dock.position") else {
            panic!("dock.position should be a string");
        };
        let start = EDGES
            .iter()
            .position(|edge| *edge == current)
            .unwrap_or_else(|| panic!("unknown dock edge {current}"));
        (1..=EDGES.len())
            .map(|step| EDGES[(start + step) % EDGES.len()])
            .collect()
    }

    #[test]
    #[serial]
    fn a_maximized_window_follows_the_dock_to_every_edge() {
        let mut f = setup();
        f.maximize();
        let floating = f.handle.window_floating_rect(TITLE);

        for edge in edge_tour(&f.handle) {
            let zone = f.change_dock("dock.position", position(edge));
            assert_eq!(
                f.placement(),
                filling(zone),
                "the maximized window should fill the zone beside a {edge} dock"
            );
        }
        assert_eq!(
            f.handle.window_floating_rect(TITLE),
            floating,
            "refitting must not touch the rect unmaximize restores to"
        );

        f.handle.toggle_maximize_focused();
        f.handle.settle(400);
        let restored = floating.expect("a maximized window keeps a floating rect");
        let (x, y, _, _) = f
            .handle
            .window_logical_geometry(TITLE)
            .expect("window should be mapped");
        assert_eq!(
            (x, y),
            (restored.0, restored.1),
            "unmaximizing after a refit should go back to the floating rect"
        );

        f.handle.stop();
    }

    #[test]
    #[serial]
    fn a_maximized_window_follows_the_dock_size() {
        let mut f = setup();
        f.maximize();

        let Some(SettingValue::Double(size)) = f.handle.setting_value("dock.size") else {
            panic!("dock.size should be a number");
        };
        let bigger = (size * 1.5).min(2.0);
        let zone = f.change_dock("dock.size", SettingValue::Double(bigger));
        assert_eq!(f.placement(), filling(zone));

        f.handle.stop();
    }

    #[test]
    #[serial]
    fn a_maximized_window_follows_the_dock_autohide() {
        let mut f = setup();
        f.maximize();

        let Some(SettingValue::Bool(autohide)) = f.handle.setting_value("dock.autohide") else {
            panic!("dock.autohide should be a bool");
        };
        for value in [!autohide, autohide] {
            let zone = f.change_dock("dock.autohide", SettingValue::Bool(value));
            assert_eq!(
                f.placement(),
                filling(zone),
                "the maximized window should fill the zone with autohide {value}"
            );
        }

        f.handle.stop();
    }

    #[test]
    #[serial]
    fn a_half_tiled_window_follows_the_dock_to_every_edge() {
        let mut f = setup();
        f.handle.tile_focused(TileZone::RightHalf);
        f.handle.settle(400);

        for edge in edge_tour(&f.handle) {
            let (zx, zy, zw, _) = f.change_dock("dock.position", position(edge));
            assert_eq!(
                f.placement(),
                (zx + zw - zw / 2, zy, zw / 2),
                "the tile should fill the right half beside a {edge} dock"
            );
        }
        assert_eq!(f.handle.window_tiled_zone(TITLE), Some(TileZone::RightHalf));

        f.handle.stop();
    }

    #[test]
    #[serial]
    fn a_maximized_window_on_a_hidden_workspace_is_refitted_in_place() {
        let mut f = setup();
        f.maximize();
        f.handle.move_window_to_workspace(TITLE, 1);
        f.handle.settle(300);
        assert_eq!(f.handle.window_workspace_index(TITLE), Some(1));
        assert_eq!(f.handle.current_workspace_index(), 0);

        let edge = edge_tour(&f.handle)[0];
        let zone = f.change_dock("dock.position", position(edge));

        assert_eq!(
            f.handle.window_workspace_index(TITLE),
            Some(1),
            "the refit must not pull the window onto the visible workspace"
        );
        assert_eq!(f.handle.current_workspace_index(), 0);
        assert_eq!(f.placement(), filling(zone));

        f.handle.stop();
    }

    #[test]
    #[serial]
    fn a_refit_keeps_the_stacking_order() {
        let mut f = setup();
        f.maximize();
        let other = f.client.create_toplevel("zone-refit-floating", 320, 240);
        f.handle.wait(Duration::from_millis(100));
        let _ = f.client.roundtrip();
        f.handle.settle(300);
        assert!(other.lock().unwrap().configured);
        let stack = f.handle.space_stack_titles();
        assert_eq!(
            stack.last().map(String::as_str),
            Some("zone-refit-floating"),
            "the fixture needs the maximized window below the other one"
        );

        let edge = edge_tour(&f.handle)[0];
        f.change_dock("dock.position", position(edge));

        assert_eq!(
            f.handle.space_stack_titles(),
            stack,
            "refitting the maximized window must not raise it"
        );
        assert_eq!(f.handle.window_stack_titles(), stack);

        f.handle.stop();
    }

    #[test]
    #[serial]
    fn a_maximized_window_makes_room_for_a_new_panel() {
        let mut f = setup();
        f.maximize();
        let before = f.handle.usable_zone();

        let panel = f.client.create_layer_surface(
            "test-bar",
            Layer::Top,
            200,
            30,
            KeyboardInteractivity::None,
        );
        panel.lock().unwrap().reserve_top(30);
        let _ = f.client.roundtrip();
        f.handle.settle(400);

        let zone = f.handle.usable_zone();
        assert_eq!(zone.1, before.1 + 30, "the panel should reserve the top");
        assert_eq!(
            f.placement(),
            filling(zone),
            "the maximized window should move below the panel"
        );

        panel.lock().unwrap().reserve_top(0);
        let _ = f.client.roundtrip();
        f.handle.settle(400);
        assert_eq!(
            f.placement(),
            filling(before),
            "the window should grow back into the released space"
        );

        f.handle.stop();
    }
}
