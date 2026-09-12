//! End-to-end tests for the `[tiling]` settings applying to a running session.
//!
//! Every one of these keys is marked `live` in the schema, which is a promise
//! that changing it in Settings reaches the windows already on screen. The
//! promise is only worth what it is tested at: these drive
//! `HeadlessHandle::set_setting`, which is the compositor's own settings
//! entry point — schema, running configuration, `apply_live`, persist — and
//! then look at the cells the tree resolves to and at what the clients were
//! configured with.

#[cfg(feature = "headless")]
mod tiling_settings_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto::settings::value::SettingValue;
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::time::Duration;

    /// A window, kept alive by its client.
    struct Window {
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

    /// Two windows, tiled side by side and settled.
    fn setup(titles: &[&str]) -> (HeadlessHandle, Vec<Window>) {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let windows: Vec<Window> = titles.iter().map(|t| spawn(&handle, t)).collect();
        handle.settle(300);
        handle.toggle_tiling();
        handle.settle(400);
        assert!(
            handle.workspace_tiling_enabled(),
            "the fixture needs a tiling workspace"
        );
        (handle, windows)
    }

    fn set(handle: &HeadlessHandle, id: &str, value: SettingValue) {
        handle
            .set_setting(id, value)
            .unwrap_or_else(|err| panic!("{id} was refused: {err}"));
        handle.settle(400);
    }

    /// The gap between two tiles, along the axis they are split on.
    fn horizontal_gap(handle: &HeadlessHandle) -> i32 {
        let mut cells: Vec<(i32, i32, i32, i32)> = handle
            .tiling_cell_rects()
            .into_iter()
            .map(|(_, rect)| rect)
            .collect();
        assert_eq!(cells.len(), 2, "the fixture needs exactly two tiles");
        cells.sort_by_key(|rect| rect.0);
        let (left, right) = (cells[0], cells[1]);
        right.0 - (left.0 + left.2)
    }

    /// The distance from the usable area's left edge to the leftmost tile.
    fn left_margin(handle: &HeadlessHandle) -> i32 {
        handle
            .tiling_cell_rects()
            .into_iter()
            .map(|(_, rect)| rect.0)
            .min()
            .expect("no tiles")
    }

    /// Moving `tiling.inner_gap` re-resolves the layout under the windows
    /// that are already tiled, rather than waiting for the next thing that
    /// happens to relayout.
    #[test]
    #[serial]
    fn the_inner_gap_reaches_the_tiles_already_on_screen() {
        let (handle, _windows) = setup(&["gap-a", "gap-b"]);

        set(&handle, "tiling.inner_gap", SettingValue::Int(4));
        let narrow = horizontal_gap(&handle);

        set(&handle, "tiling.inner_gap", SettingValue::Int(40));
        let wide = horizontal_gap(&handle);

        assert!(
            wide > narrow + 20,
            "inner gap did not reach the tiles: {narrow} then {wide}"
        );
    }

    /// The same for the outer gap, which moves every tile rather than the
    /// space between two of them.
    #[test]
    #[serial]
    fn the_outer_gap_reaches_the_tiles_already_on_screen() {
        let (handle, _windows) = setup(&["outer-a", "outer-b"]);

        set(&handle, "tiling.outer_gap", SettingValue::Int(4));
        let narrow = left_margin(&handle);

        set(&handle, "tiling.outer_gap", SettingValue::Int(40));
        let wide = left_margin(&handle);

        assert!(
            wide > narrow + 20,
            "outer gap did not reach the tiles: {narrow} then {wide}"
        );
    }

    /// `smart_gaps` drops the gaps for a lone tile, and turning it off puts
    /// them back — with one window mapped, which is the case it is about.
    #[test]
    #[serial]
    fn smart_gaps_reaches_a_lone_tile() {
        let (handle, _windows) = setup(&["smart-a"]);
        set(&handle, "tiling.outer_gap", SettingValue::Int(32));

        set(&handle, "tiling.smart_gaps", SettingValue::Bool(true));
        let inset = left_margin(&handle);

        set(&handle, "tiling.smart_gaps", SettingValue::Bool(false));
        let gapped = left_margin(&handle);

        assert!(
            gapped > inset,
            "smart gaps did not reach the lone tile: {inset} then {gapped}"
        );
    }

    /// The resize step is read when a resize command runs, so a new value is
    /// used by the very next keystroke.
    #[test]
    #[serial]
    fn the_resize_step_is_used_by_the_next_keystroke() {
        use otto::headless::Axis;

        let (handle, _windows) = setup(&["step-a", "step-b"]);

        let width = |handle: &HeadlessHandle| -> i32 {
            let mut cells: Vec<(i32, i32, i32, i32)> = handle
                .tiling_cell_rects()
                .into_iter()
                .map(|(_, rect)| rect)
                .collect();
            cells.sort_by_key(|rect| rect.0);
            cells[0].2
        };

        set(&handle, "tiling.resize_step", SettingValue::Double(0.02));
        let before = width(&handle);
        handle.tiling_resize(Axis::Row, true);
        handle.settle(400);
        let small_step = width(&handle) - before;

        set(&handle, "tiling.resize_step", SettingValue::Double(0.4));
        let before = width(&handle);
        handle.tiling_resize(Axis::Row, true);
        handle.settle(400);
        let big_step = width(&handle) - before;

        assert!(
            big_step > small_step * 2,
            "the resize step did not follow the setting: {small_step}px then {big_step}px"
        );
    }

    /// Every decoration the setting offers reaches the tiles already up, and
    /// each leaves the client the height its bar says it does.
    #[test]
    #[serial]
    fn every_decoration_reaches_the_tiles_already_on_screen() {
        let (handle, _windows) = setup(&["deco-a", "deco-b"]);

        // The bar only exists on a window that negotiated a server-side
        // decoration; a bare test client has none to measure.
        handle.decorate_window("deco-a");
        handle.settle(300);

        let height = |handle: &HeadlessHandle| {
            handle
                .window_decoration_height("deco-a")
                .expect("window should be mapped")
        };

        set(
            &handle,
            "tiling.decoration",
            SettingValue::Str("normal".into()),
        );
        let normal = height(&handle);

        set(
            &handle,
            "tiling.decoration",
            SettingValue::Str("minimal".into()),
        );
        let minimal = height(&handle);

        set(
            &handle,
            "tiling.decoration",
            SettingValue::Str("none".into()),
        );
        let none = height(&handle);

        assert!(
            normal > minimal && minimal > none,
            "decorations do not step down: normal {normal}, minimal {minimal}, none {none}"
        );
        assert_eq!(none, 0, "`none` should leave no bar at all");
    }

    /// A workspace that was given its own gaps by `gaps <n>` follows the
    /// slider again once the session default moves: the pane's sliders are
    /// that default, and `gaps <n> all` clears the overrides the same way.
    #[test]
    #[serial]
    fn the_gap_sliders_reach_a_workspace_with_its_own_gaps() {
        let (handle, _windows) = setup(&["own-a", "own-b"]);
        set(&handle, "tiling.inner_gap", SettingValue::Int(8));

        // The workspace takes an override of its own, as sway's `gaps inner
        // 40` gives it one.
        for outcome in handle.run_command("gaps inner current 40") {
            outcome.expect("the gaps command should be accepted");
        }
        handle.settle(400);
        assert!(
            horizontal_gap(&handle) > 30,
            "the fixture needs the override to have taken"
        );

        set(&handle, "tiling.inner_gap", SettingValue::Int(4));
        assert!(
            horizontal_gap(&handle) < 12,
            "the slider did not reach a workspace holding its own gaps: {}",
            horizontal_gap(&handle)
        );
    }

    /// Every `[tiling]` setting the pane draws as a slider of fractions is
    /// stored as an `f32`, so asking the configuration for the double that
    /// was written gets a slightly different number back. That is the
    /// storage's precision, not a write that failed — and treating it as a
    /// failure refused every one of these settings.
    #[test]
    #[serial]
    fn the_fractional_settings_are_accepted_and_kept() {
        let (handle, _windows) = setup(&["frac-a"]);

        for (id, wanted) in [
            ("tiling.resize_step", 0.02_f64),
            ("tiling.layout_duration", 0.3),
            ("tiling.layout_bounce", 0.45),
            ("tiling.mode_duration", 0.4),
            ("tiling.mode_bounce", 0.1),
        ] {
            handle
                .set_setting(id, SettingValue::Double(wanted))
                .unwrap_or_else(|err| panic!("{id} was refused: {err}"));

            match handle.setting_value(id) {
                Some(SettingValue::Double(got)) => assert!(
                    (got - wanted).abs() < 1e-6,
                    "{id} came back as {got}, not {wanted}"
                ),
                other => panic!("{id} read back as {other:?}"),
            }
        }
    }

    /// A value outside the schema's range is refused, and the running
    /// configuration is left as it was — the sliders are clamped in the UI,
    /// but the bus is not.
    #[test]
    #[serial]
    fn an_out_of_range_gap_is_refused() {
        let (handle, _windows) = setup(&["range-a", "range-b"]);
        set(&handle, "tiling.inner_gap", SettingValue::Int(12));
        let before = horizontal_gap(&handle);

        assert!(
            handle
                .set_setting("tiling.inner_gap", SettingValue::Int(4096))
                .is_err(),
            "a gap far outside the schema's range should be refused"
        );
        handle.settle(200);
        assert_eq!(
            horizontal_gap(&handle),
            before,
            "a refused gap still moved the tiles"
        );
    }
}
