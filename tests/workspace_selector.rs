//! Workspace selector interaction tests (headless).

#[cfg(feature = "headless")]
mod workspace_selector_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::time::Duration;

    fn find<'a>(
        nodes: &'a [layers::engine::scene::SceneNodeSnapshot],
        key: &str,
    ) -> Option<&'a layers::engine::scene::SceneNodeSnapshot> {
        for n in nodes {
            if n.key == key {
                return Some(n);
            }
            if let Some(found) = find(&n.children, key) {
                return Some(found);
            }
        }
        None
    }

    fn workspace_indices(handle: &HeadlessHandle) -> Vec<usize> {
        handle.query(|state| {
            (0..8)
                .filter_map(|i| state.workspaces.get_workspace_at(i).map(|w| w.index))
                .collect()
        })
    }

    fn output_scale(handle: &HeadlessHandle) -> f32 {
        handle.query(|state| {
            state
                .workspaces
                .outputs()
                .next()
                .map(|o| o.current_scale().fractional_scale() as f32)
                .unwrap_or(1.0)
        })
    }

    /// Move the pointer to the centre of a selector layer. Scene bounds are
    /// physical pixels, synthetic pointer input is logical.
    fn hover_layer_centre(handle: &HeadlessHandle, key: &str) {
        let scale = output_scale(handle);
        let snapshot = handle.scene_snapshot();
        let bounds = find(&snapshot.nodes, key)
            .unwrap_or_else(|| panic!("layer {key} not in scene"))
            .global_bounds
            .clone();
        handle.pointer_move(
            ((bounds.x + bounds.width / 2.0) / scale) as f64,
            ((bounds.y + bounds.height / 2.0) / scale) as f64,
        );
        handle.settle(200);
    }

    fn open_selector_with_workspaces() -> HeadlessHandle {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        handle.with_state(|state| {
            state.workspaces.add_workspace_to_output("headless");
        });
        handle.settle(200);
        handle.toggle_expose();
        handle.settle(300);
        handle
    }

    /// Hovering a workspace preview reveals its remove button; the current
    /// workspace has none (it cannot be removed).
    #[test]
    #[serial]
    fn hover_reveals_remove_button() {
        let handle = open_selector_with_workspaces();
        let indices = workspace_indices(&handle);
        assert!(indices.len() >= 2, "expected at least two workspaces");
        let current = handle.current_workspace_index();

        for (pos, index) in indices.iter().enumerate() {
            hover_layer_centre(
                &handle,
                &format!("workspace_selector_desktop_content_{index}"),
            );
            let opacity =
                handle.layer_opacity(&format!("workspace_selector_desktop_remove_{index}"));
            if pos == current {
                assert_eq!(
                    opacity, None,
                    "current workspace must have no remove button"
                );
            } else {
                let opacity = opacity.expect("remove button missing");
                assert!(
                    opacity > 0.9,
                    "hovering workspace {index} should reveal its remove button, got {opacity}"
                );
            }
        }

        handle.stop();
    }

    /// Moving onto the remove button itself must not hide it: reaching for it
    /// leaves the preview layer, which emits a pointer-out on the container.
    #[test]
    #[serial]
    fn remove_button_stays_visible_while_hovered() {
        let handle = open_selector_with_workspaces();
        let indices = workspace_indices(&handle);
        let target = indices[indices.len() - 1];
        let remove_key = format!("workspace_selector_desktop_remove_{target}");

        hover_layer_centre(
            &handle,
            &format!("workspace_selector_desktop_content_{target}"),
        );
        hover_layer_centre(&handle, &remove_key);
        let opacity = handle.layer_opacity(&remove_key).expect("button missing");
        assert!(
            opacity > 0.9,
            "remove button should stay visible under the cursor, got {opacity}"
        );

        // Leaving the workspace entirely hides it again.
        handle.pointer_move(10.0, 900.0);
        handle.settle(200);
        let opacity = handle.layer_opacity(&remove_key).expect("button missing");
        assert!(
            opacity < 0.1,
            "remove button should hide once the pointer leaves, got {opacity}"
        );

        handle.stop();
    }

    // ── Preview layout ───────────────────────────────────────────────────

    fn map_window(handle: &HeadlessHandle, client: &mut TestClient, title: &str) {
        client.create_toplevel_with_app_id(title, &format!("org.otto.{title}"), 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(300);
    }

    /// The expose previews, as `(title, x, y, w, h)`, sorted by title so the
    /// assertions do not depend on layout order.
    fn previews(handle: &HeadlessHandle) -> Vec<(String, f32, f32, f32, f32)> {
        let mut rects = handle.expose_window_rects();
        rects.sort_by(|a, b| a.0.cmp(&b.0));
        rects
    }

    fn preview_titles(handle: &HeadlessHandle) -> Vec<String> {
        previews(handle).into_iter().map(|r| r.0).collect()
    }

    fn overlap(a: &(String, f32, f32, f32, f32), b: &(String, f32, f32, f32, f32)) -> bool {
        a.1 < b.1 + b.3 && b.1 < a.1 + a.3 && a.2 < b.2 + b.4 && b.2 < a.2 + a.4
    }

    /// Opening a window while the selector is up re-lays out the previews
    /// there and then — the new window gets a preview, the existing one makes
    /// room — and closing it puts the layout back.
    #[test]
    #[serial]
    fn the_preview_layout_follows_windows_opening_and_closing() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut first = TestClient::connect(&handle.socket_name).expect("client");
        let mut second = TestClient::connect(&handle.socket_name).expect("client");

        map_window(&handle, &mut first, "First");
        let before_expose = handle
            .window_logical_geometry("First")
            .expect("window mapped");

        handle.toggle_expose();
        handle.settle(400);
        let alone = previews(&handle);
        assert_eq!(preview_titles(&handle), vec!["First".to_string()]);
        assert_eq!(
            handle.workspace_preview_window_counts().first(),
            Some(&(1, 1))
        );

        // A window opening while expose is up must join the grid, not wait for
        // the next time expose is opened.
        map_window(&handle, &mut second, "Second");
        handle.settle(400);
        let together = previews(&handle);
        assert_eq!(
            preview_titles(&handle),
            vec!["First".to_string(), "Second".to_string()]
        );
        assert_ne!(
            together[0], alone[0],
            "the existing preview should have been re-laid out to make room"
        );
        assert!(
            !overlap(&together[0], &together[1]),
            "previews must not overlap: {together:?}"
        );
        assert_eq!(
            handle.workspace_preview_window_counts().first(),
            Some(&(1, 2))
        );

        // And closing one gives the space back.
        drop(second);
        handle.wait(Duration::from_millis(200));
        handle.settle(400);
        assert_eq!(preview_titles(&handle), vec!["First".to_string()]);
        assert_eq!(
            previews(&handle),
            alone,
            "the last preview should be back to the layout it had on its own"
        );
        assert_eq!(
            handle.workspace_preview_window_counts().first(),
            Some(&(1, 1))
        );

        // Closing the selector leaves the real window untouched.
        handle.toggle_expose();
        handle.settle(400);
        assert!(!handle.is_expose_active());
        assert_eq!(handle.window_logical_geometry("First"), Some(before_expose));

        handle.stop();
    }

    /// Moving a window to another workspace updates both previews — the one it
    /// left and the one it landed on — while the selector is open.
    #[test]
    #[serial]
    fn moving_a_window_between_workspaces_updates_both_previews() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client = TestClient::connect(&handle.socket_name).expect("client");
        map_window(&handle, &mut client, "Wanderer");
        handle.with_state(|state| {
            state.workspaces.add_workspace_to_output("headless");
        });
        handle.settle(300);

        handle.toggle_expose();
        handle.settle(400);
        // Expose keeps a spare empty workspace at the end, so there may be
        // more than the two that exist here.
        let counts = handle.workspace_preview_window_counts();
        assert!(counts.len() >= 2, "expected two workspaces: {counts:?}");
        assert_eq!(counts[0].1, 1, "the window starts on the first workspace");
        assert_eq!(counts[1].1, 0);

        handle.move_window_to_workspace("Wanderer", 1);
        handle.settle(500);

        let counts = handle.workspace_preview_window_counts();
        assert_eq!(
            (counts[0].1, counts[1].1),
            (0, 1),
            "both previews should have followed the move: {counts:?}"
        );
        assert!(
            preview_titles(&handle).is_empty(),
            "the workspace it left has nothing to preview"
        );

        // Following it to the other workspace, expose lays it out there.
        handle.set_workspace(1);
        handle.settle(500);
        assert_eq!(preview_titles(&handle), vec!["Wanderer".to_string()]);
        assert_eq!(handle.window_stack_titles(), vec!["Wanderer".to_string()]);

        handle.stop();
    }
}
