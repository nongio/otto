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

    /// The removal animation crops the preview against the collapsing item
    /// rather than fading or scaling it — and the spacing on both sides of the
    /// shrinking sliver stays a full gap, so the neighbour on the right never
    /// crowds in while the preview is still visible.
    #[test]
    #[serial]
    fn removal_crops_preview_and_keeps_spacing() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        handle.with_state(|state| {
            state.workspaces.add_workspace_to_output("headless");
            state.workspaces.add_workspace_to_output("headless");
        });
        handle.settle(200);
        handle.toggle_expose();
        handle.settle(300);

        let indices = workspace_indices(&handle);
        let current = handle.current_workspace_index();
        let pos = indices
            .iter()
            .enumerate()
            .position(|(pos, _)| pos != current && pos + 1 < indices.len())
            .expect("a removable workspace with a right neighbour");
        let (left, target, right) = (indices[0], indices[pos], indices[pos + 1]);

        hover_layer_centre(
            &handle,
            &format!("workspace_selector_desktop_content_{target}"),
        );
        hover_layer_centre(
            &handle,
            &format!("workspace_selector_desktop_remove_{target}"),
        );
        handle.pointer_click();

        let gap = 50.0_f32; // WORKSPACE_SELECTOR_GAP
        let mut cropped = false;
        for _ in 0..120 {
            handle.tick(1.0 / 60.0);
            // One snapshot per sample: the animation clock runs on wall time, so
            // reading each layer from its own snapshot would compare frames.
            let snap = handle.scene_snapshot();
            let g = |key: String| {
                find(&snap.nodes, &key).map(|n| {
                    (
                        n.global_bounds.x,
                        n.global_bounds.width,
                        n.global_bounds.x + n.global_bounds.width,
                    )
                })
            };
            let (Some(wrap), Some(preview), Some(l), Some(r)) = (
                g(format!("workspace_selector_desktop_wrap_{target}")),
                g(format!("workspace_selector_desktop_content_{target}")),
                g(format!("workspace_selector_desktop_content_{left}")),
                g(format!("workspace_selector_desktop_content_{right}")),
            ) else {
                break; // the workspace is gone: the collapse finished
            };
            if wrap.1 <= 0.0 {
                break; // fully cropped; the rest of the collapse closes the gap
            }
            // The preview keeps its full width and is cropped by the wrap.
            assert!(
                preview.1 > wrap.1 - 1.0,
                "preview should keep its size and be cropped, got preview {} wrap {}",
                preview.1,
                wrap.1
            );
            cropped |= preview.1 > wrap.1 + 1.0;
            // The crop is centred: it takes as much off the left as the right.
            let visible_start = preview.0.max(wrap.0);
            let visible_end = preview.2.min(wrap.2);
            assert!(
                ((visible_start - preview.0) - (preview.2 - visible_end)).abs() < 2.0,
                "crop should stay centred, took {} off the left and {} off the right",
                visible_start - preview.0,
                preview.2 - visible_end
            );
            assert!(
                (visible_start - l.2 - gap).abs() < 2.0,
                "spacing to the left neighbour should stay {gap}, got {}",
                visible_start - l.2
            );
            assert!(
                (r.0 - visible_end - gap).abs() < 2.0,
                "spacing to the right neighbour should stay {gap}, got {}",
                r.0 - visible_end
            );
        }
        assert!(cropped, "the preview was never cropped by the collapse");

        handle.stop();
    }

    // ── Reordering ───────────────────────────────────────────────────────

    /// The width one workspace occupies in the strip, in scene (physical)
    /// pixels — `WORKSPACE_SELECTOR_PREVIEW_WIDTH + WORKSPACE_SELECTOR_GAP`.
    const SLOT_PITCH: f32 = 350.0;

    /// The centre of a workspace's preview, in logical coordinates — where the
    /// pointer has to be to have grabbed it.
    fn preview_centre(handle: &HeadlessHandle, index: usize) -> (f64, f64) {
        let scale = output_scale(handle);
        let snapshot = handle.scene_snapshot();
        let key = format!("workspace_selector_desktop_wrap_{index}");
        let bounds = find(&snapshot.nodes, &key)
            .unwrap_or_else(|| panic!("layer {key} not in scene"))
            .global_bounds
            .clone();
        (
            ((bounds.x + bounds.width / 2.0) / scale) as f64,
            ((bounds.y + bounds.height / 2.0) / scale) as f64,
        )
    }

    /// Drag the workspace `index` `slots` places along the strip and hold it
    /// there, button still down. Returns where the pointer ended up.
    fn drag_workspace_holding(handle: &HeadlessHandle, index: usize, slots: f32) -> (f64, f64) {
        let scale = output_scale(handle);
        let (x, y) = preview_centre(handle, index);
        handle.pointer_move(x, y);
        handle.settle(60);
        handle.pointer_press();
        handle.settle(10);
        // A few steps rather than one jump, so the drag crosses each slot the
        // way a hand would — and so the first step clears the threshold.
        let total = (SLOT_PITCH * slots / scale) as f64;
        for step in 1..=8 {
            handle.pointer_move(x + total * step as f64 / 8.0, y);
            handle.settle(10);
        }
        (x + total, y)
    }

    /// Drag the workspace `index` `slots` places along the strip and drop it.
    fn drag_workspace(handle: &HeadlessHandle, index: usize, slots: f32) {
        drag_workspace_holding(handle, index, slots);
        handle.pointer_release();
        handle.settle(400);
    }

    /// Dragging a workspace past its neighbour puts it there, and the drop is
    /// not also read as a click on whatever the pointer landed over.
    #[test]
    #[serial]
    fn dragging_a_workspace_reorders_the_strip() {
        let handle = open_selector_with_workspaces();
        let before = workspace_indices(&handle);
        assert!(before.len() >= 2, "expected at least two workspaces");
        let current = handle.current_workspace_index();

        drag_workspace(&handle, before[0], 1.0);

        let after = workspace_indices(&handle);
        let mut expected = before.clone();
        expected.swap(0, 1);
        assert_eq!(
            after, expected,
            "the compositor should hold the order the strip was dragged into"
        );
        assert_eq!(
            handle.current_workspace_index(),
            if current == 0 { 1 } else { 0 },
            "the workspace the user is on should have followed its own move"
        );

        handle.stop();
    }

    /// A press that never travels is still a click: it switches workspace
    /// rather than being swallowed by a reorder that did not happen.
    #[test]
    #[serial]
    fn a_press_without_travel_still_switches_workspace() {
        let handle = open_selector_with_workspaces();
        let before = workspace_indices(&handle);
        let target_pos = before.len() - 1;
        assert_ne!(handle.current_workspace_index(), target_pos);

        let (x, y) = preview_centre(&handle, before[target_pos]);
        handle.pointer_move(x, y);
        handle.settle(60);
        handle.pointer_click();
        handle.settle(400);

        assert_eq!(
            workspace_indices(&handle),
            before,
            "a click must not reorder anything"
        );
        assert_eq!(handle.current_workspace_index(), target_pos);

        handle.stop();
    }

    /// The windows go with their workspace. A workspace's place along the
    /// scroll axis is a function of its position, so a reorder that did not
    /// carry the windows would strand them at another workspace's coordinates.
    #[test]
    #[serial]
    fn windows_travel_with_the_workspace_they_are_on() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client = TestClient::connect(&handle.socket_name).expect("client");
        map_window(&handle, &mut client, "Passenger");
        handle.with_state(|state| {
            state.workspaces.add_workspace_to_output("headless");
        });
        handle.settle(300);
        let geometry = handle
            .window_logical_geometry("Passenger")
            .expect("window mapped");

        handle.toggle_expose();
        handle.settle(400);
        let before = workspace_indices(&handle);
        let counts = handle.workspace_preview_window_counts();
        assert_eq!(counts[0].1, 1, "the window starts on the first workspace");

        // Drag the workspace holding the window one place to the right.
        drag_workspace(&handle, before[0], 1.0);

        let counts = handle.workspace_preview_window_counts();
        assert_eq!(
            counts[1].0, before[0],
            "the dragged workspace should now be second"
        );
        assert_eq!(
            (counts[0].1, counts[1].1),
            (0, 1),
            "the window should have moved with its workspace: {counts:?}"
        );

        // Closing expose leaves the window where it always was: it changed
        // workspace *position*, not workspace, and not its place on screen.
        handle.toggle_expose();
        handle.settle(500);
        assert_eq!(handle.window_logical_geometry("Passenger"), Some(geometry));
        assert_eq!(handle.window_stack_titles(), vec!["Passenger".to_string()]);

        handle.stop();
    }

    /// Every key in the subtree rooted at `node`.
    fn keys_under(node: &layers::engine::scene::SceneNodeSnapshot) -> Vec<String> {
        let mut out = vec![node.key.clone()];
        for child in &node.children {
            out.extend(keys_under(child));
        }
        out
    }

    /// The node whose children include `key`.
    fn parent_of<'a>(
        nodes: &'a [layers::engine::scene::SceneNodeSnapshot],
        key: &str,
    ) -> Option<&'a layers::engine::scene::SceneNodeSnapshot> {
        for n in nodes {
            if n.children.iter().any(|c| c.key == key) {
                return Some(n);
            }
            if let Some(found) = parent_of(&n.children, key) {
                return Some(found);
            }
        }
        None
    }

    /// The workspace being dragged has to be *visible* while it is dragged.
    ///
    /// The slot it came out of is blanked, so if the lifted copy does not draw
    /// the workspace simply disappears for the length of the gesture. The copy
    /// therefore has to live in the selector's own tree (which is what exposé
    /// shows) and mirror the workspace's own nodes — a mirror of the preview
    /// would be a mirror of a mirror, and draws nothing.
    #[test]
    #[serial]
    fn the_dragged_workspace_stays_visible_while_it_is_carried() {
        let handle = open_selector_with_workspaces();
        let order = workspace_indices(&handle);
        assert!(order.len() >= 2, "expected at least two workspaces");

        let (x, y) = drag_workspace_holding(&handle, order[0], 1.0);

        let snapshot = handle.scene_snapshot();
        let ghost = find(&snapshot.nodes, "workspace_selector_drag_ghost")
            .expect("nothing was lifted out of the strip");
        assert!(!ghost.hidden, "the lifted workspace is hidden");
        assert!(
            ghost.opacity > 0.9,
            "the lifted workspace is transparent: {}",
            ghost.opacity
        );
        let content = find(&snapshot.nodes, "workspace_selector_drag_ghost_content")
            .expect("the lifted copy has no preview box");
        assert!(
            content.global_bounds.width > 1.0 && content.global_bounds.height > 1.0,
            "the lifted preview has no area: {:?}",
            content.global_bounds
        );

        // It mirrors the workspace itself. A copy of the strip's preview would
        // sit under one of the preview wraps; this one must not.
        let under_ghost = keys_under(ghost);
        for mirror in [
            "workspace_selector_drag_ghost_bg_mirror",
            "workspace_selector_drag_ghost_content_mirror",
        ] {
            assert!(
                under_ghost.iter().any(|k| k == mirror),
                "the lifted copy is missing {mirror}: {under_ghost:?}"
            );
            let owner = parent_of(&snapshot.nodes, mirror).expect("mirror has no parent");
            assert!(
                !owner.key.starts_with("workspace_selector_desktop_wrap_"),
                "the lifted copy mirrors a preview, not the workspace: parent {}",
                owner.key
            );
        }

        // It is in the selector's own subtree — the thing exposé puts on
        // screen — and it is the last child there, so it paints over the
        // workspaces it is passing rather than under them.
        let parent = parent_of(&snapshot.nodes, "workspace_selector_drag_ghost")
            .expect("the lifted copy is not parented anywhere");
        assert_eq!(
            parent.key, "workspace_selector_view",
            "the lifted copy must ride in the selector, not on some other layer"
        );
        assert!(!parent.hidden, "the selector itself is hidden");
        assert_eq!(
            parent.children.last().map(|c| c.key.as_str()),
            Some("workspace_selector_drag_ghost"),
            "the lifted copy must be painted last, over the strip"
        );

        // And it is carried: move the pointer and it comes along.
        let before = content.global_bounds.x;
        handle.pointer_move(x - 40.0, y);
        handle.settle(30);
        let snapshot = handle.scene_snapshot();
        let after = find(&snapshot.nodes, "workspace_selector_drag_ghost_content")
            .expect("the lifted copy vanished mid-drag")
            .global_bounds
            .x;
        assert!(
            after < before - 1.0,
            "the lifted copy did not follow the pointer: {before} -> {after}"
        );

        // Dropping puts it away and gives the slot its preview back.
        handle.pointer_release();
        handle.settle(400);
        let snapshot = handle.scene_snapshot();
        assert!(
            find(&snapshot.nodes, "workspace_selector_drag_ghost").is_none(),
            "the lifted copy outlived the drop"
        );
        assert_eq!(
            handle.layer_opacity(&format!("workspace_selector_desktop_{}", order[0])),
            Some(1.0),
            "the slot the workspace was lifted out of stayed blank"
        );

        handle.stop();
    }

    /// Leaving a fullscreen workspace through exposé must bring the dock back.
    ///
    /// Going fullscreen hides the dock outright — the layer is marked hidden
    /// and the dock inactive, not merely slid off. The workspace switch that
    /// would undo that is deliberately skipped while exposé is up, so crossing
    /// to an ordinary workspace inside exposé and closing it used to leave the
    /// desktop with no dock at all.
    #[test]
    #[serial]
    fn the_dock_comes_back_when_expose_leaves_a_fullscreen_workspace() {
        let handle = open_selector_with_workspaces();
        handle.toggle_expose();
        handle.settle(400);
        let order = workspace_indices(&handle);
        assert!(order.len() >= 2, "expected at least two workspaces");
        let current = handle.current_workspace_index();
        let other = if current == 0 { 1 } else { 0 };

        // The state a fullscreen window leaves behind on the workspace the
        // user is on: the workspace is in fullscreen mode and the dock is not
        // just parked off screen, it is hidden and inactive.
        handle.with_state(move |state| {
            let workspace = state
                .workspaces
                .get_workspace_at(current)
                .expect("current workspace");
            workspace.set_fullscreen_mode(true);
            state.workspaces.dock.hide(None);
        });
        handle.settle(200);
        assert_eq!(
            handle.is_layer_hidden("dock_layout"),
            Some(true),
            "the dock should start out hidden, as fullscreen leaves it"
        );

        // Into exposé, across to a workspace that is not fullscreen, and out.
        handle.toggle_expose();
        handle.settle(400);
        handle.set_workspace(other);
        handle.settle(400);
        handle.toggle_expose();
        handle.settle(600);

        assert_eq!(
            handle.current_workspace_index(),
            other,
            "the switch inside exposé should have stuck"
        );
        assert_eq!(
            handle.is_layer_hidden("dock_layout"),
            Some(false),
            "the dock never came back after exposé left the fullscreen workspace"
        );
        assert!(
            handle.query(|state| !state.workspaces.dock.is_hidden()),
            "the dock is on screen but still marked inactive"
        );

        handle.stop();
    }

    /// A drag drives the pointer across every preview it passes and parks it
    /// on one at the end. None of that is hovering, so no close button may
    /// appear for the length of the gesture — and the release must not leave
    /// one lit under the pointer either.
    #[test]
    #[serial]
    fn no_close_buttons_appear_while_a_workspace_is_being_dragged() {
        let handle = open_selector_with_workspaces();
        let order = workspace_indices(&handle);
        assert!(order.len() >= 2, "expected at least two workspaces");

        let visible_buttons = |handle: &HeadlessHandle| -> Vec<(usize, f32)> {
            order
                .iter()
                .filter_map(|index| {
                    handle
                        .layer_opacity(&format!("workspace_selector_desktop_remove_{index}"))
                        .filter(|opacity| *opacity > 0.05)
                        .map(|opacity| (*index, opacity))
                })
                .collect()
        };

        // The current workspace has no close button, so drag one that does.
        let dragged = *order.last().expect("a workspace");
        assert_ne!(
            dragged,
            order[handle.current_workspace_index()],
            "the workspace to drag must not be the current one"
        );

        // Start from a preview whose button is showing, so the drag has one to
        // suppress rather than merely never raising one.
        hover_layer_centre(
            &handle,
            &format!("workspace_selector_desktop_content_{dragged}"),
        );
        assert!(
            !visible_buttons(&handle).is_empty(),
            "the hover should have revealed a close button to begin with"
        );

        let (x, y) = drag_workspace_holding(&handle, dragged, -1.0);
        assert_eq!(
            visible_buttons(&handle),
            vec![],
            "a close button is showing mid-drag"
        );

        // Keep sweeping: the pointer crosses the previews it is passing.
        for step in 1..=4 {
            handle.pointer_move(x + 20.0 * step as f64, y);
            handle.settle(20);
            assert_eq!(
                visible_buttons(&handle),
                vec![],
                "a close button lit up as the drag swept over it"
            );
        }

        // The release parks the pointer on whatever it landed over.
        handle.pointer_release();
        handle.settle(400);
        assert_eq!(
            visible_buttons(&handle),
            vec![],
            "the drop left a close button lit under the pointer"
        );

        // And hovering works again once the gesture is over.
        hover_layer_centre(
            &handle,
            &format!("workspace_selector_desktop_content_{dragged}"),
        );
        assert!(
            !visible_buttons(&handle).is_empty(),
            "hovering stopped revealing close buttons after a drag"
        );

        handle.stop();
    }

    /// A window's remembered workspace is a *position*, and a reorder moves
    /// positions around — so the reorder has to move the remembered one too.
    ///
    /// The case that matters is a window whose remembered workspace is not the
    /// one it is currently sitting in, which is exactly what fullscreen does:
    /// it parks the window on a temporary workspace of its own and keeps the
    /// workspace to restore it to. Re-deriving that from where the window
    /// currently is would throw the restore target away, and unfullscreening
    /// after a reorder would drop the window on a stranger's workspace.
    #[test]
    #[serial]
    fn a_window_remembers_the_right_workspace_across_a_reorder() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client = TestClient::connect(&handle.socket_name).expect("client");
        map_window(&handle, &mut client, "Restorer");
        handle.with_state(|state| {
            state.workspaces.add_workspace_to_output("headless");
            state.workspaces.add_workspace_to_output("headless");
        });
        handle.settle(300);

        // The window sits on workspace 0 but remembers workspace 2 — the shape
        // fullscreen leaves behind.
        handle.with_state(|state| {
            let window = state
                .workspaces
                .spaces_elements()
                .find(|w| w.xdg_title() == "Restorer")
                .expect("window mapped");
            window.set_workspace(2);
        });
        handle.settle(100);

        handle.toggle_expose();
        handle.settle(400);
        let order = workspace_indices(&handle);
        assert!(
            order.len() >= 3,
            "expected at least three workspaces: {order:?}"
        );

        // Drag workspace 2 to the front. Everything else shifts one along, and
        // the window's remembered workspace has to follow the workspace it
        // named, not stay on the number.
        drag_workspace(&handle, order[2], -2.0);
        let mut expected = order.clone();
        let moved = expected.remove(2);
        expected.insert(0, moved);
        assert_eq!(
            workspace_indices(&handle),
            expected,
            "the drag did not put the third workspace first"
        );

        let remembered = handle.query(|state| {
            state
                .workspaces
                .spaces_elements()
                .find(|w| w.xdg_title() == "Restorer")
                .map(|w| w.get_workspace())
        });
        assert_eq!(
            remembered,
            Some(0),
            "the window should still remember the workspace that is now first"
        );

        handle.stop();
    }

    /// Forget every custom workspace name.
    ///
    /// Names are persisted in the user's config, which is process-global and
    /// outlives a single test, so a test about names has to start from a known
    /// slate and leave one behind.
    fn clear_workspace_names(handle: &HeadlessHandle) {
        let indices = workspace_indices(handle);
        handle.with_state(move |state| {
            for index in &indices {
                state.workspaces.rename_workspace("headless", *index, None);
            }
        });
        handle.settle(100);
    }

    /// The name each workspace shows, in strip order.
    fn workspace_names(handle: &HeadlessHandle) -> Vec<String> {
        handle.query(|state| {
            (0..8)
                .filter_map(|i| {
                    state
                        .workspaces
                        .get_workspace_at(i)
                        .map(|w| w.display_name())
                })
                .collect()
        })
    }

    /// A name belongs to the workspace, not to the slot it happens to be in:
    /// drag a named workspace along the strip and its name goes with it.
    ///
    /// Both kinds of name are checked, because they fail differently: a name
    /// the user typed is stored on the workspace, while the default
    /// `Workspace N` is a number that must be the workspace's own and not its
    /// position, or the labels stay behind while the previews move.
    #[test]
    #[serial]
    fn a_workspace_keeps_its_name_when_it_is_dragged_elsewhere() {
        let handle = open_selector_with_workspaces();
        clear_workspace_names(&handle);
        let order = workspace_indices(&handle);
        assert!(order.len() >= 2, "expected at least two workspaces");

        // The default names first: whatever the first workspace is called, it
        // must still be called that after it has been dragged.
        let before = workspace_names(&handle);
        drag_workspace(&handle, order[0], 1.0);
        let after = workspace_names(&handle);
        assert_eq!(
            after[1], before[0],
            "the dragged workspace kept its slot's name instead of its own: \
             {before:?} -> {after:?}"
        );
        assert_eq!(
            after[0], before[1],
            "the workspace that was displaced should have brought its name \
             along too: {before:?} -> {after:?}"
        );

        // And a name the user typed travels the same way.
        let renamed = order[0];
        handle.with_state(move |state| {
            state
                .workspaces
                .rename_workspace("headless", renamed, Some("Correspondence".into()));
        });
        handle.settle(200);
        assert_eq!(
            workspace_names(&handle)[1],
            "Correspondence",
            "the rename should land on the workspace, at its current position"
        );

        drag_workspace(&handle, order[0], -1.0);
        let after = workspace_names(&handle);
        assert_eq!(
            after[0], "Correspondence",
            "a typed name must follow its workspace back: {after:?}"
        );
        assert_ne!(
            after[1], "Correspondence",
            "and must not be left behind in the slot it came from: {after:?}"
        );

        clear_workspace_names(&handle);
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
