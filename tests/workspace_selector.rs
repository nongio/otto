//! Workspace selector interaction tests (headless).

#[cfg(feature = "headless")]
mod workspace_selector_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use serial_test::serial;

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
}
