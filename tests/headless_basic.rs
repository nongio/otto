//! Integration tests for the Otto headless compositor.
//!
//! These tests start a headless compositor instance, connect Wayland clients,
//! and verify compositor behavior: gestures, expose mode, workspace switching,
//! layer visibility, and animations.

#[cfg(feature = "headless")]
mod headless_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::time::Duration;

    fn start_compositor() -> HeadlessHandle {
        HeadlessHandle::start(HeadlessConfig::default())
    }

    fn connect_client(handle: &HeadlessHandle) -> TestClient {
        TestClient::connect(&handle.socket_name).expect("Failed to connect to compositor")
    }

    // ── Basic lifecycle ──────────────────────────────────────────────────

    #[test]
    #[serial]
    fn compositor_starts_and_stops() {
        let handle = start_compositor();
        assert!(!handle.socket_name.is_empty());
        // compositor is running (it would have panicked on start otherwise)
        handle.stop();
    }

    #[test]
    #[serial]
    fn client_connects_and_binds_globals() {
        let handle = start_compositor();
        let client = connect_client(&handle);

        assert!(client.state.wl_compositor.is_some(), "wl_compositor not bound");
        assert!(client.state.wl_shm.is_some(), "wl_shm not bound");
        assert!(client.state.xdg_wm_base.is_some(), "xdg_wm_base not bound");

        handle.stop();
    }

    #[test]
    #[serial]
    fn client_creates_toplevel() {
        let handle = start_compositor();
        let mut client = connect_client(&handle);

        let toplevel = client.create_toplevel("test-window", 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();

        assert!(toplevel.lock().unwrap().configured, "Toplevel should be configured");

        handle.stop();
    }

    // ── Gesture: workspace switching ─────────────────────────────────────

    #[test]
    #[serial]
    fn swipe_gesture_state_machine() {
        let handle = start_compositor();

        // Initially idle
        assert_eq!(handle.swipe_gesture_state(), "idle");

        // Begin 3-finger swipe
        handle.swipe_begin();
        assert_eq!(handle.swipe_gesture_state(), "detecting");

        // Horizontal swipe → workspace switching
        handle.swipe_update(20.0, 0.0);
        assert_eq!(handle.swipe_gesture_state(), "workspace_switching");

        // End gesture
        handle.swipe_end();
        assert_eq!(handle.swipe_gesture_state(), "idle");

        handle.stop();
    }

    #[test]
    #[serial]
    fn vertical_swipe_triggers_expose() {
        let handle = start_compositor();

        // Begin 3-finger swipe
        handle.swipe_begin();
        assert_eq!(handle.swipe_gesture_state(), "detecting");

        // Vertical swipe → expose mode
        handle.swipe_update(0.0, -20.0);
        assert_eq!(handle.swipe_gesture_state(), "expose");

        // End gesture
        handle.swipe_end();
        assert_eq!(handle.swipe_gesture_state(), "idle");

        handle.stop();
    }

    // ── Expose mode ──────────────────────────────────────────────────────

    #[test]
    #[serial]
    fn expose_toggle_and_settle() {
        let handle = start_compositor();

        assert!(!handle.is_expose_active());

        // Toggle expose on
        handle.toggle_expose();

        // Should be active (or transitioning)
        assert!(handle.is_expose_active() || handle.is_expose_transitioning());

        // Let animations settle
        let frames = handle.settle(300);
        assert!(frames > 0, "Expected animation frames during expose transition");

        // Should be fully active after settling
        assert!(handle.is_expose_active());

        // Toggle expose off
        handle.toggle_expose();
        handle.settle(300);
        assert!(!handle.is_expose_active());

        handle.stop();
    }

    // ── Expose with windows ──────────────────────────────────────────────

    #[test]
    #[serial]
    fn expose_with_three_windows() {
        let handle = start_compositor();
        let mut client = connect_client(&handle);

        // Create 3 windows
        let _w1 = client.create_toplevel("window-1", 640, 480);
        let _w2 = client.create_toplevel("window-2", 800, 600);
        let _w3 = client.create_toplevel("window-3", 400, 300);
        handle.wait(Duration::from_millis(200));
        let _ = client.roundtrip();

        assert_eq!(handle.window_count(), 3);

        // Simulate a strong vertical swipe to enter expose
        handle.swipe(&[
            (0.0, -10.0),
            (0.0, -50.0),
            (0.0, -80.0),
            (0.0, -80.0),
        ]);

        // Let the spring animation finish
        handle.settle(300);

        // Verify expose is active
        assert!(
            handle.is_expose_active(),
            "Expose should be active after strong upward swipe"
        );

        handle.stop();
    }

    // ── Pinch: show desktop ──────────────────────────────────────────────

    #[test]
    #[serial]
    fn pinch_show_desktop() {
        let handle = start_compositor();

        assert!(!handle.is_show_desktop_active());

        // 4-finger pinch out (spread) to show desktop
        handle.pinch_begin();
        handle.pinch_update(1.5); // scale > 1.0 = spread
        handle.pinch_end();

        // May be transitioning
        handle.settle(300);

        handle.stop();
    }

    // ── Layer visibility ─────────────────────────────────────────────────

    #[test]
    #[serial]
    fn scene_snapshot_has_root() {
        let handle = start_compositor();

        let snapshot = handle.scene_snapshot();
        assert!(!snapshot.nodes.is_empty(), "Scene should have at least the root node");

        // The root should have key "otto_root"
        let root = &snapshot.nodes[0];
        assert_eq!(root.key, "otto_root");

        handle.stop();
    }

    #[test]
    #[serial]
    fn check_layer_hidden_by_key() {
        let handle = start_compositor();

        // The root layer should exist and not be hidden
        let hidden = handle.is_layer_hidden("otto_root");
        assert_eq!(hidden, Some(false), "otto_root should not be hidden");

        // A non-existent layer should return None
        let missing = handle.is_layer_hidden("nonexistent_layer_xyz");
        assert_eq!(missing, None, "Non-existent layer should return None");

        handle.stop();
    }

    // ── Workspace switching ──────────────────────────────────────────────

    #[test]
    #[serial]
    fn workspace_switch_programmatic() {
        let handle = start_compositor();

        let initial = handle.current_workspace_index();
        assert_eq!(initial, 0);

        // Switch to workspace 1
        handle.set_workspace(1);
        handle.settle(300);

        assert_eq!(handle.current_workspace_index(), 1);

        // Switch back
        handle.set_workspace(0);
        handle.settle(300);

        assert_eq!(handle.current_workspace_index(), 0);

        handle.stop();
    }

    // ── Compositor state query via closures ───────────────────────────────

    #[test]
    #[serial]
    fn state_query_window_count() {
        let handle = start_compositor();
        let mut client = connect_client(&handle);

        let _w1 = client.create_toplevel("query-test-1", 800, 600);
        let _w2 = client.create_toplevel("query-test-2", 400, 300);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();

        let count = handle.window_count();
        assert!(count >= 2, "Expected at least 2 windows, got {}", count);

        handle.stop();
    }

    // ── Bug: expose should preserve focused window ─────────────────────

    /// Helper: get the window stacking order as a list of titles (bottom to top).
    fn window_order(handle: &HeadlessHandle) -> Vec<String> {
        handle.query(|state| {
            state
                .workspaces
                .spaces_elements()
                .map(|w| w.xdg_title())
                .collect()
        })
    }

    #[test]
    #[serial]
    fn expose_roundtrip_preserves_window_order() {
        let handle = start_compositor();
        let mut client = connect_client(&handle);

        // Create 3 windows — last opened is on top
        let _w1 = client.create_toplevel("window-1", 640, 480);
        let _w2 = client.create_toplevel("window-2", 800, 600);
        let _w3 = client.create_toplevel("window-3", 400, 300);
        handle.wait(Duration::from_millis(200));
        let _ = client.roundtrip();

        assert_eq!(handle.window_count(), 3);

        // Focus window-1 (simulates clicking on it — raises + focuses)
        handle.with_state(|state| {
            let w1_id = state
                .workspaces
                .spaces_elements()
                .find(|w| w.xdg_title() == "window-1")
                .map(|w| w.id())
                .expect("window-1 not found");
            state.workspaces.raise_element(&w1_id, true, true);
            state.set_keyboard_focus_on_surface(&w1_id);
        });
        handle.settle(60);

        // Record stacking order before expose
        let order_before = window_order(&handle);
        eprintln!("Order before expose: {:?}", order_before);

        // Swipe UP to enter expose
        handle.swipe_begin();
        handle.swipe_update(0.0, -10.0);
        handle.swipe_update(0.0, -50.0);
        handle.swipe_update(0.0, -80.0);
        handle.swipe_update(0.0, -80.0);
        handle.swipe_end();
        handle.settle(300);
        assert!(handle.is_expose_active(), "Expose should be active after swipe up");

        // Swipe DOWN to close expose (without selecting a window)
        handle.swipe_begin();
        handle.swipe_update(0.0, 10.0);
        handle.swipe_update(0.0, 50.0);
        handle.swipe_update(0.0, 80.0);
        handle.swipe_update(0.0, 80.0);
        handle.swipe_end();
        handle.settle(300);
        assert!(!handle.is_expose_active(), "Expose should be closed after swipe down");

        // Stacking order must be identical
        let order_after = window_order(&handle);
        eprintln!("Order after expose: {:?}", order_after);

        assert_eq!(
            order_before, order_after,
            "Window stacking order should be preserved after expose roundtrip"
        );
    }

    // ── Scene JSON for debugging ─────────────────────────────────────────

    #[test]
    #[serial]
    fn scene_json_is_valid() {
        let handle = start_compositor();

        let json = handle.scene_json();
        assert!(!json.is_empty(), "Scene JSON should not be empty");
        assert!(json.contains("otto_root"), "Scene JSON should contain root node");

        // Should be valid JSON
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json);
        assert!(parsed.is_ok(), "Scene JSON should be valid: {}", json);

        handle.stop();
    }
}
