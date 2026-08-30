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

        assert!(
            client.state.wl_compositor.is_some(),
            "wl_compositor not bound"
        );
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

        assert!(
            toplevel.lock().unwrap().configured,
            "Toplevel should be configured"
        );

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
        assert!(
            frames > 0,
            "Expected animation frames during expose transition"
        );

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
        handle.swipe(&[(0.0, -10.0), (0.0, -50.0), (0.0, -80.0), (0.0, -80.0)]);

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
        let mut client = connect_client(&handle);
        let _toplevel = client.create_toplevel("show-desktop-window", 400, 300);
        handle.wait(Duration::from_millis(200));
        let _ = client.roundtrip();
        handle.settle(200);

        assert!(!handle.is_show_desktop_active());

        // 4-finger pinch out (spread) to show desktop
        handle.pinch_begin();
        for scale in [1.1f64, 1.3, 1.6, 2.0, 2.5] {
            handle.pinch_update(scale); // scale > 1.0 = spread
            handle.settle(5);
        }
        handle.pinch_end();
        handle.settle(600);

        assert!(
            handle.is_show_desktop_active(),
            "pinch out should activate show desktop"
        );

        // The windows are shown through their expose mirrors while the desktop
        // is revealed, so the real workspace content must be hidden — otherwise
        // the untouched windows keep drawing on top of the mirrors sliding away
        // and the gesture looks like it does nothing.
        let snapshot = handle.scene_snapshot();
        fn find<'a>(
            node: &'a layers::engine::scene::SceneNodeSnapshot,
            key: &str,
        ) -> Option<&'a layers::engine::scene::SceneNodeSnapshot> {
            if node.key == key {
                return Some(node);
            }
            node.children.iter().find_map(|child| find(child, key))
        }
        let workspaces = snapshot
            .nodes
            .iter()
            .find_map(|node| find(node, "workspaces_headless"))
            .expect("no workspaces layer in the scene");
        assert!(
            workspaces.hidden,
            "real workspace content still visible during show desktop"
        );

        // Clicking a window dismisses show desktop, and the mirrors animate
        // back to their places first. The real windows may only take over once
        // that animation ends — swapping them in on the click frame makes the
        // windows snap back with no animation at all.
        handle.pointer_move(760.0, 400.0);
        handle.settle(5);
        let hidden_on_click = handle.click_and_sample_workspaces_hidden();
        assert!(
            hidden_on_click,
            "real windows swapped back in on the click frame — the exit animation is skipped"
        );

        handle.settle(600);
        assert!(
            !handle.is_show_desktop_active(),
            "clicking a window should dismiss show desktop"
        );

        handle.stop();
    }

    // ── Layer visibility ─────────────────────────────────────────────────

    #[test]
    #[serial]
    fn scene_snapshot_has_root() {
        let handle = start_compositor();

        let snapshot = handle.scene_snapshot();
        assert!(
            !snapshot.nodes.is_empty(),
            "Scene should have at least the root node"
        );

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
        assert!(
            handle.is_expose_active(),
            "Expose should be active after swipe up"
        );

        // Swipe DOWN to close expose (without selecting a window)
        handle.swipe_begin();
        handle.swipe_update(0.0, 10.0);
        handle.swipe_update(0.0, 50.0);
        handle.swipe_update(0.0, 80.0);
        handle.swipe_update(0.0, 80.0);
        handle.swipe_end();
        handle.settle(300);
        assert!(
            !handle.is_expose_active(),
            "Expose should be closed after swipe down"
        );

        // Stacking order must be identical
        let order_after = window_order(&handle);
        eprintln!("Order after expose: {:?}", order_after);

        assert_eq!(
            order_before, order_after,
            "Window stacking order should be preserved after expose roundtrip"
        );
    }

    #[test]
    #[serial]
    fn expose_click_raises_window() {
        let handle = start_compositor();
        let mut client = connect_client(&handle);

        // Create 3 windows — last opened ends up on top
        let _w1 = client.create_toplevel("window-1", 640, 480);
        let _w2 = client.create_toplevel("window-2", 800, 600);
        let _w3 = client.create_toplevel("window-3", 400, 300);
        handle.wait(Duration::from_millis(200));
        let _ = client.roundtrip();
        assert_eq!(handle.window_count(), 3);

        // Record which window is on top before expose
        let top_before = window_order(&handle).last().cloned().unwrap();
        eprintln!("Top window before expose: {}", top_before);

        // Enter expose via swipe
        handle.swipe_begin();
        handle.swipe_update(0.0, -10.0);
        handle.swipe_update(0.0, -50.0);
        handle.swipe_update(0.0, -80.0);
        handle.swipe_update(0.0, -80.0);
        handle.swipe_end();
        handle.settle(300);
        assert!(handle.is_expose_active(), "Expose should be active");

        // Find the expose rect for a window that is NOT currently on top
        let rects = handle.expose_window_rects();
        eprintln!("Expose rects: {:?}", rects);
        assert!(!rects.is_empty(), "Expose should have window rects");

        let target = rects
            .iter()
            .find(|(title, _, _, _, _)| *title != top_before)
            .expect("Should find a non-top window to click");
        let target_title = target.0.clone();
        eprintln!(
            "Clicking on '{}' at physical ({}, {}, {}, {})",
            target_title, target.1, target.2, target.3, target.4
        );

        // Click the center of the target window rect.
        // Rects are in physical pixels; pointer_move takes logical pixels.
        let scale: f64 = handle.query(|state| {
            state
                .workspaces
                .outputs()
                .next()
                .map(|o| o.current_scale().fractional_scale())
                .unwrap_or(1.0)
        });
        let center_x = (target.1 + target.3 / 2.0) as f64 / scale;
        let center_y = (target.2 + target.4 / 2.0) as f64 / scale;
        eprintln!("Pointer move to logical ({}, {})", center_x, center_y);

        // Establish pointer focus on window selector (first move triggers smithay
        // enter, not motion — the selection is only updated on motion events).
        handle.pointer_move(5.0, 300.0);
        handle.settle(2);
        handle.pointer_move(center_x, center_y);
        handle.settle(10);
        handle.pointer_click();
        handle.settle(300);

        assert!(
            !handle.is_expose_active(),
            "Expose should close after clicking a window"
        );

        // The clicked window should now be on top
        let order_after = window_order(&handle);
        let top_after = order_after.last().cloned().unwrap();
        eprintln!("Order after: {:?}", order_after);

        assert_eq!(
            top_after, target_title,
            "Clicked window '{}' should be raised to top, but top is '{}'",
            target_title, top_after
        );
    }

    #[test]
    #[serial]
    fn expose_pointer_selects_hovered_window() {
        let handle = start_compositor();
        let mut client = connect_client(&handle);

        let _w1 = client.create_toplevel("window-1", 640, 480);
        let _w2 = client.create_toplevel("window-2", 800, 600);
        let _w3 = client.create_toplevel("window-3", 400, 300);
        handle.wait(Duration::from_millis(200));
        let _ = client.roundtrip();

        // Enter expose
        handle.swipe_begin();
        handle.swipe_update(0.0, -10.0);
        handle.swipe_update(0.0, -50.0);
        handle.swipe_update(0.0, -80.0);
        handle.swipe_update(0.0, -80.0);
        handle.swipe_end();
        handle.settle(300);
        assert!(handle.is_expose_active(), "Expose should be active");

        // No selection yet
        assert_eq!(
            handle.expose_selected_title(),
            None,
            "No window should be selected before pointer enters any rect"
        );

        let rects = handle.expose_window_rects();
        let scale: f64 = handle.query(|state| {
            state
                .workspaces
                .outputs()
                .next()
                .map(|o| o.current_scale().fractional_scale())
                .unwrap_or(1.0)
        });

        // Establish pointer focus on the window selector area (smithay sends
        // enter on first focus, then motion on subsequent moves within the
        // same target).  Move to a point that lies inside the window selector
        // but outside any window rect.
        handle.pointer_move(5.0, 300.0);
        handle.settle(2);

        // Move pointer over each window rect and verify it becomes selected
        for (title, x, y, w, h) in &rects {
            let cx = (*x + *w / 2.0) as f64 / scale;
            let cy = (*y + *h / 2.0) as f64 / scale;
            handle.pointer_move(cx, cy);
            handle.settle(10);

            let selected = handle.expose_selected_title();
            assert_eq!(
                selected.as_deref(),
                Some(title.as_str()),
                "Moving pointer over '{}' should select it, but selected is {:?}",
                title,
                selected
            );
        }

        // Move pointer away from all rects — selection should clear
        handle.pointer_move(0.0, 0.0);
        handle.settle(10);
        assert_eq!(
            handle.expose_selected_title(),
            None,
            "Moving pointer away should clear selection"
        );
    }

    #[test]
    #[serial]
    fn expose_selection_survives_client_commit() {
        let handle = start_compositor();
        let mut client = connect_client(&handle);

        let w1 = client.create_toplevel("window-1", 640, 480);
        let _w2 = client.create_toplevel("window-2", 800, 600);
        handle.wait(Duration::from_millis(200));
        let _ = client.roundtrip();

        // Enter expose
        handle.swipe(&[(0.0, -10.0), (0.0, -50.0), (0.0, -80.0), (0.0, -80.0)]);
        handle.settle(300);
        assert!(handle.is_expose_active(), "Expose should be active");

        let rects = handle.expose_window_rects();
        let scale: f64 = handle.query(|state| {
            state
                .workspaces
                .outputs()
                .next()
                .map(|o| o.current_scale().fractional_scale())
                .unwrap_or(1.0)
        });
        let target = rects
            .iter()
            .find(|(title, _, _, _, _)| title == "window-2")
            .expect("window-2 should have an expose rect");

        // Establish pointer focus, then hover window-2
        handle.pointer_move(5.0, 300.0);
        handle.settle(2);
        let cx = (target.1 + target.3 / 2.0) as f64 / scale;
        let cy = (target.2 + target.4 / 2.0) as f64 / scale;
        handle.pointer_move(cx, cy);
        handle.settle(10);
        assert_eq!(
            handle.expose_selected_title().as_deref(),
            Some("window-2"),
            "window-2 should be selected while hovered"
        );

        // Another window redraws while the pointer stays put.
        w1.lock().unwrap().commit_frame();
        let _ = client.roundtrip();
        handle.wait(Duration::from_millis(100));
        handle.settle(30);

        assert_eq!(
            handle.expose_selected_title().as_deref(),
            Some("window-2"),
            "A client commit must not clear the expose hover selection"
        );

        // A relayout (window geometry changed under the grid) must also keep
        // the hovered window selected.
        handle.with_state(|state| {
            let ws_index = state.workspaces.get_current_workspace_index();
            if let Some(workspace) = state.workspaces.get_workspace_at(ws_index) {
                workspace.window_selector_view.invalidate_layout();
            }
            state.workspaces.expose_update_if_needed();
        });

        assert_eq!(
            handle.expose_selected_title().as_deref(),
            Some("window-2"),
            "An expose relayout must not clear the hover selection"
        );

        // ...and the overlay that draws the highlight must stay visible
        // instead of being blanked for the length of the re-layout animation.
        let overlay_opacity: f32 = handle.query(|state| {
            let ws_index = state.workspaces.get_current_workspace_index();
            state
                .workspaces
                .get_workspace_at(ws_index)
                .map(|w| w.window_selector_view.window_selector_view.opacity())
                .unwrap_or(-1.0)
        });
        assert_eq!(
            overlay_opacity, 1.0,
            "Selection overlay should stay visible across a re-layout"
        );
    }

    #[test]
    #[serial]
    fn expose_preview_repaints_on_client_commit() {
        let handle = start_compositor();
        let mut client = connect_client(&handle);

        let w1 = client.create_toplevel("window-1", 640, 480);
        let _w2 = client.create_toplevel("window-2", 800, 600);
        handle.wait(Duration::from_millis(200));
        let _ = client.roundtrip();

        // Enter expose and let everything come to rest.
        handle.swipe(&[(0.0, -10.0), (0.0, -50.0), (0.0, -80.0), (0.0, -80.0)]);
        handle.settle(300);
        assert!(handle.is_expose_active(), "Expose should be active");
        assert_eq!(
            handle.settle(300),
            0,
            "Scene should be quiescent before the commit"
        );

        // A client redraws while expose is open: its preview mirror has to
        // repaint, or the previews freeze on their last frame (a playing
        // video looks stuck). The compositor loop ticks the engine on its
        // own, so watch the accumulated damage rect rather than a tick's
        // return value.
        handle.with_state(|state| state.layers_engine.clear_damage());
        w1.lock().unwrap().commit_frame();
        let _ = client.roundtrip();
        handle.wait(Duration::from_millis(200));
        handle.settle(10);

        let preview_damage = handle.query(|state| {
            let mirror = state
                .workspaces
                .spaces_elements()
                .find(|w| w.xdg_title() == "window-1")
                .map(|w| w.mirror_layer().id)
                .expect("window-1 should have a preview mirror");
            state
                .layers_engine
                .subtree_damage(mirror)
                .map(|r| (r.width(), r.height()))
        });
        assert!(
            preview_damage.is_some_and(|(w, h)| w > 0.0 && h > 0.0),
            "A client commit while expose is open must repaint that window's \
             preview mirror (subtree damage was {preview_damage:?})"
        );
    }

    #[test]
    #[serial]
    fn expose_gesture_close_raises_hovered_window() {
        let handle = start_compositor();
        let mut client = connect_client(&handle);

        // Open 4 windows — w4 ends up on top
        let _w1 = client.create_toplevel("window-1", 640, 480);
        let _w2 = client.create_toplevel("window-2", 800, 600);
        let _w3 = client.create_toplevel("window-3", 400, 300);
        let _w4 = client.create_toplevel("window-4", 500, 400);
        handle.wait(Duration::from_millis(200));
        let _ = client.roundtrip();
        assert_eq!(handle.window_count(), 4);

        let order_before = window_order(&handle);
        let top_before = order_before.last().cloned().unwrap();
        assert_eq!(
            top_before, "window-4",
            "window-4 should be on top initially"
        );

        // Enter expose via swipe
        handle.swipe_begin();
        handle.swipe_update(0.0, -10.0);
        handle.swipe_update(0.0, -50.0);
        handle.swipe_update(0.0, -80.0);
        handle.swipe_update(0.0, -80.0);
        handle.swipe_end();
        handle.settle(300);
        assert!(handle.is_expose_active(), "Expose should be active");

        // Find the expose rect for window-1 (the bottom-most window)
        let rects = handle.expose_window_rects();
        let scale: f64 = handle.query(|state| {
            state
                .workspaces
                .outputs()
                .next()
                .map(|o| o.current_scale().fractional_scale())
                .unwrap_or(1.0)
        });
        let target = rects
            .iter()
            .find(|(title, _, _, _, _)| title == "window-1")
            .expect("window-1 should have an expose rect");

        // Establish pointer focus, then hover window-1
        handle.pointer_move(5.0, 300.0);
        handle.settle(2);
        let cx = (target.1 + target.3 / 2.0) as f64 / scale;
        let cy = (target.2 + target.4 / 2.0) as f64 / scale;
        handle.pointer_move(cx, cy);
        handle.settle(10);
        assert_eq!(
            handle.expose_selected_title().as_deref(),
            Some("window-1"),
            "window-1 should be selected"
        );

        // Close expose via downward swipe gesture (no click)
        handle.swipe_begin();
        handle.swipe_update(0.0, 10.0);
        handle.swipe_update(0.0, 50.0);
        handle.swipe_update(0.0, 80.0);
        handle.swipe_update(0.0, 80.0);
        handle.swipe_end();
        handle.settle(300);
        assert!(!handle.is_expose_active(), "Expose should be closed");

        // window-1 must now be on top
        let order_after = window_order(&handle);
        let top_after = order_after.last().cloned().unwrap();
        assert_eq!(
            top_after, "window-1",
            "Hovered window-1 should be raised to top after gesture close, but top is '{}'",
            top_after
        );
    }

    // ── Scene JSON for debugging ─────────────────────────────────────────

    #[test]
    #[serial]
    fn scene_json_is_valid() {
        let handle = start_compositor();

        let json = handle.scene_json();
        assert!(!json.is_empty(), "Scene JSON should not be empty");
        assert!(
            json.contains("otto_root"),
            "Scene JSON should contain root node"
        );

        // Should be valid JSON
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json);
        assert!(parsed.is_ok(), "Scene JSON should be valid: {}", json);

        handle.stop();
    }

    // ── Direct scanout candidate selection ───────────────────────────────
    //
    // These tests exercise `Workspaces::get_scanout_candidates()` — the
    // selector the udev render path uses to decide which client windows are
    // promoted to KMS plane scanout. A window is a candidate when no overlay
    // UI (expose, app switcher, OSD, layer-shell chrome) overlaps it, it is
    // not animating/minimizing, owns no popups, and is not covered by a
    // higher window; the set is capped at the promotion limit.

    /// Titles of the current scanout candidates, top-most first.
    fn scanout_candidate_titles(handle: &HeadlessHandle) -> Vec<String> {
        handle.query(|state| {
            let Some(output) = state.workspaces.outputs().next().cloned() else {
                return Vec::new();
            };
            state
                .workspaces
                .get_scanout_candidates(&output)
                .iter()
                .filter_map(|id| {
                    state
                        .workspaces
                        .get_window_for_surface(id)
                        .map(|w| w.xdg_title())
                })
                .collect()
        })
    }

    /// Wait until animations from compositor startup have settled, so
    /// `get_scanout_candidates()` reflects the steady state.
    ///
    /// Note: in headless mode the workspace `is_animating` flag isn't
    /// reset by the udev render loop (which doesn't run), so we also
    /// clear it explicitly to put the compositor into a stable state
    /// the eligibility predicate can return true for.
    fn settle_animations(handle: &HeadlessHandle) {
        handle.settle(300);
        handle.with_state(|state| {
            state
                .workspaces
                .is_animating
                .store(false, std::sync::atomic::Ordering::Relaxed);
        });
    }

    #[test]
    #[serial]
    fn scanout_candidate_with_no_clients_is_none() {
        let handle = start_compositor();
        settle_animations(&handle);

        // Diagnostic: query each global gate individually so a failure
        // pinpoints which one is active.
        let (show_all, app_switcher_alive, osd, animating): (bool, bool, bool, bool) = handle
            .query(|state| {
                (
                    state.workspaces.get_show_all(),
                    {
                        use otto::focus::IsAlive;
                        state.workspaces.app_switcher.alive()
                    },
                    state.workspaces.osd.is_visible(),
                    state
                        .workspaces
                        .is_animating
                        .load(std::sync::atomic::Ordering::Relaxed),
                )
            });

        eprintln!(
            "show_all={show_all} app_switcher={app_switcher_alive} \
             osd={osd} animating={animating}"
        );

        let candidates = scanout_candidate_titles(&handle);
        assert!(
            candidates.is_empty(),
            "No candidates when no clients are connected, got {candidates:?}"
        );

        handle.stop();
    }

    #[test]
    #[serial]
    fn scanout_candidate_with_one_toplevel_returns_it() {
        let handle = start_compositor();
        let mut client = connect_client(&handle);

        // Sized to fit above the dock: the dock bar is a scanout occluder, so
        // a window tall enough to reach the bottom-centre strip of the
        // 960x540-logical headless desktop is legitimately not promotable.
        let _w = client.create_toplevel("scanout-only-window", 400, 300);
        handle.wait(Duration::from_millis(200));
        let _ = client.roundtrip();
        settle_animations(&handle);

        let candidates = scanout_candidate_titles(&handle);
        assert_eq!(
            candidates,
            vec!["scanout-only-window".to_string()],
            "One windowed toplevel + no overlays → it is the only candidate"
        );

        handle.stop();
    }

    #[test]
    #[serial]
    fn scanout_candidate_returns_topmost_after_raise() {
        let handle = start_compositor();
        let mut client = connect_client(&handle);

        // All three must clear the dock strip — whichever is on top has to be
        // promotable for the assertions below to mean anything.
        let _w1 = client.create_toplevel("bottom-window", 400, 300);
        let _w2 = client.create_toplevel("middle-window", 360, 240);
        let _w3 = client.create_toplevel("top-window", 320, 200);
        handle.wait(Duration::from_millis(200));
        let _ = client.roundtrip();
        settle_animations(&handle);

        // Last-created window is on top by default.
        let top = scanout_candidate_titles(&handle);
        assert_eq!(top.first().map(String::as_str), Some("top-window"));

        // Raise bottom-window to the top.
        handle.with_state(|state| {
            let id = state
                .workspaces
                .spaces_elements()
                .find(|w| w.xdg_title() == "bottom-window")
                .map(|w| w.id())
                .expect("bottom-window not found");
            state.workspaces.raise_element(&id, true, true);
        });
        handle.settle(60);

        let top_after = scanout_candidate_titles(&handle);
        assert_eq!(
            top_after.first().map(String::as_str),
            Some("bottom-window"),
            "After raising bottom-window, it should be the scanout candidate"
        );

        handle.stop();
    }

    #[test]
    #[serial]
    fn expose_blocks_scanout_eligibility() {
        let handle = start_compositor();
        let mut client = connect_client(&handle);

        let _w = client.create_toplevel("hidden-by-expose", 400, 300);
        handle.wait(Duration::from_millis(200));
        let _ = client.roundtrip();
        settle_animations(&handle);

        // Sanity check: window is a candidate before expose.
        let before = scanout_candidate_titles(&handle);
        assert!(!before.is_empty(), "Candidate exists before expose opens");

        // Open expose.
        handle.toggle_expose();
        handle.settle(300);
        assert!(handle.is_expose_active(), "Expose should be active");

        let during = scanout_candidate_titles(&handle);
        assert!(
            during.is_empty(),
            "Expose mode must block scanout candidates (overlay UI visible), got {during:?}"
        );

        // Close expose — candidates return.
        handle.toggle_expose();
        settle_animations(&handle);
        assert!(!handle.is_expose_active(), "Expose should be closed");

        let after = scanout_candidate_titles(&handle);
        assert!(
            !after.is_empty(),
            "Candidates should return after expose closes"
        );

        handle.stop();
    }

    #[test]
    #[serial]
    fn scanned_out_flag_default_is_false() {
        let handle = start_compositor();
        let mut client = connect_client(&handle);

        let _w = client.create_toplevel("flag-test", 800, 600);
        handle.wait(Duration::from_millis(200));
        let _ = client.roundtrip();

        // The is_scanned_out flag is set/cleared by the udev render path.
        // In the headless backend the udev render does not run, so the flag
        // stays at its default of `false`. This test pins down that default
        // behavior so future changes can't accidentally change it (e.g., by
        // initializing the flag to `true` or having some other code path
        // toggle it from headless).
        let scanned_out: bool = handle.query(|state| {
            state
                .workspaces
                .spaces_elements()
                .find(|w| w.xdg_title() == "flag-test")
                .map(|w| w.is_scanned_out())
                .unwrap_or(false)
        });

        assert!(
            !scanned_out,
            "is_scanned_out should default to false; only the udev render path \
             toggles it. Headless runs without the udev render so it must stay false."
        );

        handle.stop();
    }

    // ── ext-background-effect-v1 ─────────────────────────────────────────

    /// A client that commits a blur region through the standard protocol
    /// gets the same frost an otto-kit popup gets: its surface layer flips to
    /// `BackgroundBlur` and the window counts as carrying a compositor-drawn
    /// material (so it stays off a raw scanout plane). Destroying the effect
    /// object takes both back on the next commit.
    #[test]
    #[serial]
    fn background_effect_blur_follows_commits() {
        use layers::types::BlendMode;

        let handle = start_compositor();
        let mut client = connect_client(&handle);

        let toplevel = client.create_toplevel("blur-me", 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();

        assert_eq!(
            client.state.background_effect_capabilities,
            Some(1),
            "the global must advertise the blur capability on bind"
        );

        fn material_and_blend(handle: &HeadlessHandle, title: &'static str) -> (bool, BlendMode) {
            handle.query(move |state| {
                let window = state
                    .workspaces
                    .spaces_elements()
                    .find(|w| w.xdg_title() == title)
                    .expect("window mapped");
                let blend = state
                    .surface_layers
                    .get(&window.id())
                    .expect("surface layer")
                    .render_layer()
                    .blend_mode;
                (window.has_material(), blend)
            })
        }

        assert_eq!(
            material_and_blend(&handle, "blur-me"),
            (false, BlendMode::Normal),
            "a plain window starts without a material"
        );

        // Pending until the surface commits: the region alone changes nothing.
        let surface = toplevel.lock().unwrap().surface.clone();
        let effect = client
            .request_background_blur(&surface, 640, 480)
            .expect("ext_background_effect_manager_v1 advertised");
        let _ = client.roundtrip();
        handle.wait(Duration::from_millis(50));
        assert_eq!(
            material_and_blend(&handle, "blur-me"),
            (false, BlendMode::Normal),
            "set_blur_region is double-buffered"
        );

        toplevel.lock().unwrap().commit_frame();
        let _ = client.roundtrip();
        handle.wait(Duration::from_millis(100));
        assert_eq!(
            material_and_blend(&handle, "blur-me"),
            (true, BlendMode::BackgroundBlur),
            "the commit applies the blur to the surface layer"
        );

        // Destroying the effect removes the blur on the next commit.
        effect.destroy();
        let _ = client.roundtrip();
        toplevel.lock().unwrap().commit_frame();
        let _ = client.roundtrip();
        handle.wait(Duration::from_millis(100));
        assert_eq!(
            material_and_blend(&handle, "blur-me"),
            (false, BlendMode::Normal),
            "destroying the effect object takes the blur back"
        );

        handle.stop();
    }

    // ── Expose: moving a window to another workspace ─────────────────────

    /// Pick a preview up in expose and drop it on another workspace's
    /// thumbnail: the source grid must re-layout for the windows that stay,
    /// and the target grid must pick the moved one up.
    #[test]
    #[serial]
    fn expose_drag_window_to_other_workspace() {
        let handle = start_compositor();
        let mut client = connect_client(&handle);

        let _w1 = client.create_toplevel("window-1", 640, 480);
        let _w2 = client.create_toplevel("window-2", 800, 600);
        let _w3 = client.create_toplevel("window-3", 400, 300);
        handle.wait(Duration::from_millis(200));
        let _ = client.roundtrip();

        handle.toggle_expose();
        handle.settle(300);
        let before = handle.expose_window_rects();
        assert_eq!(before.len(), 3);

        // Grid rects are physical; the pointer takes logical coordinates.
        let scale = handle.query(|state| {
            state
                .workspaces
                .outputs()
                .next()
                .map(|o| o.current_scale().fractional_scale())
                .unwrap_or(1.0)
        }) as f32;

        // The second workspace, addressed the way the drop targets are: by
        // workspace *view* index, not by position.
        let second = handle
            .query(|state| {
                state
                    .workspaces
                    .with_model(|m| m.workspaces.get(1).map(|w| w.index))
            })
            .expect("a second workspace");
        let target = handle
            .query(move |state| {
                state
                    .workspaces
                    .focused_output_selector()
                    .and_then(|selector| {
                        selector
                            .get_drop_targets()
                            .iter()
                            .find(|t| t.workspace_index == second)
                            .map(|t| {
                                let b = t.drop_layer.render_bounds_transformed();
                                (b.left(), b.top(), b.width(), b.height())
                            })
                    })
            })
            .expect("a drop target for the second workspace");

        // Pick window-3 up. The first motion after entering the selector only
        // delivers `enter`, so hover (and with it the pressed selection) needs
        // a second one.
        let (moved_title, x, y, w, h) = before[2].clone();
        let (sx, sy) = ((x + w / 2.0) / scale, (y + h / 2.0) / scale);
        handle.pointer_move(sx as f64, sy as f64);
        handle.settle(5);
        handle.pointer_move(sx as f64 + 1.0, sy as f64);
        handle.settle(5);
        handle.pointer_press();
        handle.pointer_move(sx as f64 + 4.0, sy as f64 - 4.0);
        handle.settle(20);
        assert!(
            handle.query(|state| state.workspaces.is_window_selector_dragging()),
            "drag should have started past the threshold"
        );

        // Drop it on the second workspace's thumbnail.
        let (tx, ty) = (
            (target.0 + target.2 / 2.0) / scale,
            (target.1 + target.3 / 2.0) / scale,
        );
        for i in 1..=10 {
            let f = i as f32 / 10.0;
            handle.pointer_move((sx + (tx - sx) * f) as f64, (sy + (ty - sy) * f) as f64);
            handle.settle(3);
        }
        assert_eq!(
            handle.query(|state| state
                .workspaces
                .focused_output_selector()
                .and_then(|s| s.get_drop_hover())),
            Some(second),
            "the thumbnail under the dragged preview should be the drop target"
        );
        // The drop itself must re-lay the grid out. The drag already laid the
        // source grid out without the dragged window when it was picked up, so
        // a drop that leans on the cached layout applies nothing here and the
        // grid stays as dropped until an unrelated commit moves it.
        handle.pointer_release();
        let frames = handle.settle(300);
        assert!(
            frames > 0,
            "the drop should animate the grids into their new layout"
        );

        // Nothing is under the pointer in the grid the window just left — it
        // is over the strip — so no preview may be left highlighted.
        assert_eq!(
            handle.expose_selected_title(),
            None,
            "the source grid should hold no selection after the drop"
        );
        // The pointer lingers on the strip afterwards, as it does for real.
        for d in [1.0f32, 2.0, 3.0] {
            handle.pointer_move((tx + d) as f64, (ty + d) as f64);
            handle.settle(5);
        }
        assert_eq!(handle.expose_selected_title(), None);
        // Source grid: the moved window is gone and the rest have re-laid out.
        let after = handle.expose_window_rects();
        assert_eq!(after.len(), 2, "source grid should hold 2 previews");
        assert!(
            !after.iter().any(|(title, ..)| *title == moved_title),
            "moved window should be gone from the source grid"
        );
        assert!(
            after[0] != before[0],
            "the previews that stay should re-layout, got {after:#?}"
        );

        // Target grid: the moved window arrived.
        let target_rects = handle.query(|state| {
            let Some(workspace) = state.workspaces.get_workspace_at(1) else {
                return Vec::new();
            };
            let selector = workspace.window_selector_view.clone();
            let rects = selector.view.get_state().rects;
            rects
                .iter()
                .filter_map(|r| r.window_id.as_ref().map(|_| r.window_title.clone()))
                .collect::<Vec<_>>()
        });
        assert_eq!(target_rects, vec![moved_title]);

        handle.stop();
    }
}
