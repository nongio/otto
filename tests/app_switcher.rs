//! Focus, stacking and the app switcher (headless).
//!
//! Covers what a user sees when they change which window is in front: clicking
//! raises it and repaints, alt-tab lands on the app they were on before, and
//! the switcher's order tracks the z-order — across workspaces too.

#[cfg(feature = "headless")]
mod app_switcher_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::time::Duration;

    fn start() -> HeadlessHandle {
        HeadlessHandle::start(HeadlessConfig::default())
    }

    /// Map a window belonging to app `org.otto.<title>`.
    fn map_window(handle: &HeadlessHandle, client: &mut TestClient, title: &str) {
        client.create_toplevel_with_app_id(title, &app_id(title), 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(300);
    }

    fn app_id(title: &str) -> String {
        format!("org.otto.{title}")
    }

    /// The switcher's list is rebuilt off the workspace model by a background
    /// task, so wait for it to settle on `want` rather than reading it once.
    fn wait_for_apps(handle: &HeadlessHandle, want: &[&str]) -> Vec<String> {
        let want: Vec<String> = want.iter().map(|t| app_id(t)).collect();
        let mut seen = Vec::new();
        for _ in 0..50 {
            seen = handle.app_switcher_apps();
            if seen == want {
                return seen;
            }
            handle.wait(Duration::from_millis(100));
        }
        assert_eq!(
            seen, want,
            "app switcher never settled on the expected order"
        );
        seen
    }

    /// Click the top-left corner of a window — the part a window stacked later
    /// (offset down and right) does not cover.
    fn click_corner_of(handle: &HeadlessHandle, title: &str) {
        let (x, y, _, _) = handle
            .window_logical_geometry(title)
            .unwrap_or_else(|| panic!("window {title:?} not mapped"));
        handle.pointer_move((x + 8) as f64, (y + 8) as f64);
        handle.pointer_click();
    }

    // ── Focus and stacking ───────────────────────────────────────────────

    /// Clicking a window behind another brings it to the front, and the change
    /// actually repaints — a raise that damages nothing is a raise the user
    /// never sees.
    #[test]
    #[serial]
    fn clicking_a_window_raises_it_and_repaints() {
        let handle = start();
        let mut first = TestClient::connect(&handle.socket_name).expect("client");
        let mut second = TestClient::connect(&handle.socket_name).expect("client");

        map_window(&handle, &mut first, "First");
        map_window(&handle, &mut second, "Second");
        handle.settle(400);

        assert_eq!(
            handle.window_stack_titles(),
            vec!["First".to_string(), "Second".to_string()],
            "the window mapped last starts on top"
        );
        assert_eq!(handle.top_window_title().as_deref(), Some("Second"));
        assert!(
            !handle.scene_has_damage(),
            "the scene should be idle before the click"
        );

        click_corner_of(&handle, "First");

        assert!(
            handle.scene_has_damage(),
            "raising a window must damage the scene"
        );
        let frames = handle.settle(600);
        assert!(
            frames > 0,
            "the raise should have driven at least one frame"
        );

        assert_eq!(
            handle.window_stack_titles(),
            vec!["Second".to_string(), "First".to_string()],
            "the clicked window should be on top of the stack"
        );
        assert_eq!(handle.top_window_title().as_deref(), Some("First"));

        handle.stop();
    }

    // ── App switcher ─────────────────────────────────────────────────────

    /// The switcher lists apps front to back, so the app you are looking at is
    /// first and the one you were on before is second.
    #[test]
    #[serial]
    fn the_switcher_lists_apps_front_to_back() {
        let handle = start();
        let mut alpha = TestClient::connect(&handle.socket_name).expect("client");
        let mut beta = TestClient::connect(&handle.socket_name).expect("client");
        let mut gamma = TestClient::connect(&handle.socket_name).expect("client");

        map_window(&handle, &mut alpha, "Alpha");
        map_window(&handle, &mut beta, "Beta");
        map_window(&handle, &mut gamma, "Gamma");

        assert_eq!(
            handle.window_stack_titles(),
            vec!["Alpha".to_string(), "Beta".to_string(), "Gamma".to_string()]
        );
        wait_for_apps(&handle, &["Gamma", "Beta", "Alpha"]);

        handle.stop();
    }

    /// One alt-tab step lands on the app used before the current one, and
    /// releasing brings that app's window to the front.
    #[test]
    #[serial]
    fn alt_tab_switches_to_the_previously_used_app() {
        let handle = start();
        let mut alpha = TestClient::connect(&handle.socket_name).expect("client");
        let mut beta = TestClient::connect(&handle.socket_name).expect("client");
        let mut gamma = TestClient::connect(&handle.socket_name).expect("client");

        map_window(&handle, &mut alpha, "Alpha");
        map_window(&handle, &mut beta, "Beta");
        map_window(&handle, &mut gamma, "Gamma");
        wait_for_apps(&handle, &["Gamma", "Beta", "Alpha"]);

        handle.app_switcher_next();
        handle.settle(300);
        assert_eq!(
            handle.app_switcher_selection().as_deref(),
            Some(app_id("Beta").as_str()),
            "the first step selects the app behind the current one"
        );

        handle.app_switcher_commit();
        handle.settle(600);

        assert_eq!(
            handle.top_window_title().as_deref(),
            Some("Beta"),
            "committing the switch raises that app's window"
        );
        assert_eq!(
            handle.window_stack_titles(),
            vec!["Alpha".to_string(), "Gamma".to_string(), "Beta".to_string()]
        );

        // ...and the list re-sorts behind it, so the next alt-tab goes back.
        wait_for_apps(&handle, &["Beta", "Gamma", "Alpha"]);

        handle.app_switcher_next();
        handle.settle(300);
        assert_eq!(
            handle.app_switcher_selection().as_deref(),
            Some(app_id("Gamma").as_str()),
            "stepping again offers the app that was in front before"
        );

        handle.stop();
    }

    /// Stepping backwards wraps to the end of the list.
    #[test]
    #[serial]
    fn stepping_back_from_the_first_app_wraps_to_the_last() {
        let handle = start();
        let mut alpha = TestClient::connect(&handle.socket_name).expect("client");
        let mut beta = TestClient::connect(&handle.socket_name).expect("client");

        map_window(&handle, &mut alpha, "Alpha");
        map_window(&handle, &mut beta, "Beta");
        wait_for_apps(&handle, &["Beta", "Alpha"]);

        handle.app_switcher_previous();
        handle.settle(300);
        assert_eq!(
            handle.app_switcher_selection().as_deref(),
            Some(app_id("Alpha").as_str())
        );

        handle.stop();
    }

    /// An app whose windows are all on another workspace sorts after the apps
    /// on the one in front of the user — it is further away, and the switcher
    /// order says so.
    #[test]
    #[serial]
    fn apps_on_other_workspaces_sort_behind_the_current_one() {
        let handle = start();
        let mut alpha = TestClient::connect(&handle.socket_name).expect("client");
        let mut beta = TestClient::connect(&handle.socket_name).expect("client");
        let mut gamma = TestClient::connect(&handle.socket_name).expect("client");

        map_window(&handle, &mut alpha, "Alpha");
        map_window(&handle, &mut beta, "Beta");
        map_window(&handle, &mut gamma, "Gamma");
        wait_for_apps(&handle, &["Gamma", "Beta", "Alpha"]);

        handle.with_state(|state| {
            state.workspaces.add_workspace_to_output("headless");
        });
        handle.settle(300);

        // Gamma was in front; sending it to the next workspace should drop it
        // to the back of the switcher, not leave it where it was.
        handle.move_window_to_workspace("Gamma", 1);
        handle.settle(500);

        assert_eq!(
            handle.window_stack_titles(),
            vec!["Alpha".to_string(), "Beta".to_string()],
            "the moved window is off this workspace"
        );
        wait_for_apps(&handle, &["Beta", "Alpha", "Gamma"]);

        // Following it there puts it back in front.
        handle.set_workspace(1);
        handle.settle(500);
        wait_for_apps(&handle, &["Gamma", "Beta", "Alpha"]);

        handle.stop();
    }

    /// Quitting the last app while the switcher is up takes the panel away.
    /// Its layout has no zero-app case — the width collapses to a sliver the
    /// full height of the panel — so leaving it on screen until the modifier
    /// is released puts a stray bar on an empty desktop.
    #[test]
    #[serial]
    fn the_switcher_closes_when_the_last_app_quits() {
        let handle = start();
        let mut only = TestClient::connect(&handle.socket_name).expect("client");
        map_window(&handle, &mut only, "Only");
        wait_for_apps(&handle, &["Only"]);

        handle.app_switcher_next();
        handle.settle(300);
        assert!(handle.app_switcher_is_open(), "alt-tab opens the panel");

        // The app quits: the client goes away and its window is unmapped.
        drop(only);
        wait_for_apps(&handle, &[]);

        let mut closed = false;
        for _ in 0..30 {
            if !handle.app_switcher_is_open() {
                closed = true;
                break;
            }
            handle.wait(Duration::from_millis(100));
        }
        assert!(
            closed,
            "the panel is still up with no apps left to switch to"
        );

        handle.stop();
    }
}
