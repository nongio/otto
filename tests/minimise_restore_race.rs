//! A window restored while it is still being minimised comes back whole.
//!
//! Minimising plays the genie on a task of its own and, once it is done,
//! moves the window into its dock drawer at mini scale. A restore that lands
//! before the genie has finished must win: the window is back on its
//! workspace, full size and shown, and the dock holds no mini window for it.

#[cfg(feature = "headless")]
mod minimise_restore_race_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::time::Duration;

    const TITLE: &str = "restore-race";
    /// Restore only once the minimise has run its course.
    const SETTLED: usize = usize::MAX;

    /// The window's drawn bounds in physical px, whether it is hidden, and
    /// whether the workspace model still lists it as minimised.
    fn window_state(handle: &HeadlessHandle) -> (f32, f32, bool, bool) {
        handle.query(|state| {
            let (id, _) = state
                .workspaces
                .windows_map
                .iter()
                .find(|(_, w)| w.xdg_title() == TITLE)
                .map(|(id, w)| (id.clone(), w.clone()))
                .expect("the window is known");
            let view = state.workspaces.get_window_view(&id).expect("a view");
            let bounds = view.window_layer.render_bounds_with_children_transformed();
            let listed = state
                .workspaces
                .with_model(|m| m.minimized_windows.iter().any(|(w, _)| *w == id));
            (
                bounds.width(),
                bounds.height(),
                view.window_layer.hidden(),
                listed,
            )
        })
    }

    /// Lets the genie's task and the engine's animations run their course.
    fn run_out(handle: &HeadlessHandle) {
        for _ in 0..8 {
            handle.settle(120);
            handle.wait(Duration::from_millis(100));
        }
    }

    fn check(delay_frames: usize) {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client = TestClient::connect(&handle.socket_name).expect("connect");
        let toplevel = client.create_toplevel(TITLE, 600, 400);
        handle.wait(Duration::from_millis(100));
        client.roundtrip().expect("roundtrip");
        toplevel.lock().unwrap().commit_frame();
        client.roundtrip().expect("roundtrip");
        handle.settle(300);
        handle.move_window(TITLE, 200, 150);
        let _ = client.roundtrip();
        handle.settle(300);
        let (w0, h0, hidden0, _) = window_state(&handle);
        assert!(
            !hidden0 && w0 > 100.0,
            "the window starts on screen ({w0}x{h0})"
        );

        handle.minimize_window(TITLE);
        if delay_frames == SETTLED {
            run_out(&handle);
        } else {
            for _ in 0..delay_frames {
                handle.tick(1.0 / 60.0);
            }
        }
        handle.unminimize_window(TITLE);
        run_out(&handle);

        let (w, h, hidden, listed) = window_state(&handle);
        assert!(
            !listed,
            "restored after {delay_frames} frames: still listed as minimised"
        );
        assert!(
            !hidden,
            "restored after {delay_frames} frames: the window is hidden"
        );
        assert!(
            (w - w0).abs() < 2.0 && (h - h0).abs() < 2.0,
            "restored after {delay_frames} frames: drawn {w}x{h}, not its own {w0}x{h0} \
             — left at mini scale"
        );
        handle.stop();
    }

    #[test]
    #[serial]
    fn a_restore_during_the_genie_brings_the_window_back_whole() {
        // Right away, and a few frames into the genie.
        for delay in [0, 6, 20] {
            check(delay);
        }
    }

    #[test]
    #[serial]
    fn a_restore_after_the_genie_brings_the_window_back_whole() {
        check(SETTLED);
    }

    /// Minimising a fullscreen window leaves fullscreen first: the user is
    /// back on the workspace the window came from, the fullscreen one is gone,
    /// and the window, in the dock, comes back windowed when restored.
    #[test]
    #[serial]
    fn minimising_a_fullscreen_window_leaves_fullscreen() {
        // Settled in fullscreen first, and halfway into it.
        for frames_in in [usize::MAX, 6] {
            let handle = HeadlessHandle::start(HeadlessConfig::default());
            let mut client = TestClient::connect(&handle.socket_name).expect("connect");
            let toplevel = client.create_toplevel(TITLE, 600, 400);
            handle.wait(Duration::from_millis(100));
            client.roundtrip().expect("roundtrip");
            toplevel.lock().unwrap().commit_frame();
            client.roundtrip().expect("roundtrip");
            handle.settle(300);
            handle.move_window(TITLE, 200, 150);
            let _ = client.roundtrip();
            run_out(&handle);
            let workspaces = || {
                handle.query(|state| {
                    let ows = state.workspaces.output_workspaces.values().next().unwrap();
                    (ows.current_workspace, ows.workspace_views.len())
                })
            };
            let (_, count0) = workspaces();
            let (w0, h0, _, _) = window_state(&handle);

            handle.fullscreen_window(TITLE);
            let _ = client.roundtrip();
            if frames_in == usize::MAX {
                run_out(&handle);
                assert!(
                    handle.window_is_fullscreen(TITLE),
                    "the window went fullscreen"
                );
            } else {
                for _ in 0..frames_in {
                    handle.tick(1.0 / 60.0);
                }
            }
            handle.with_state(|state| {
                let window = state
                    .workspaces
                    .windows_map
                    .values()
                    .find(|w| w.xdg_title() == TITLE)
                    .cloned()
                    .unwrap();
                state.minimize_window_element(&window);
            });
            let _ = client.roundtrip();
            run_out(&handle);

            let what = if frames_in == usize::MAX {
                "settled"
            } else {
                "mid-transition"
            };
            let (current, count) = workspaces();
            assert_eq!(current, 0, "{what}: back on the window's own workspace");
            assert_eq!(count, count0, "{what}: the fullscreen workspace is gone");
            let (_, h, hidden, listed) = window_state(&handle);
            assert!(
                listed && !hidden && h < h0 / 4.0,
                "{what}: the window is in the dock"
            );

            handle.unminimize_window(TITLE);
            let _ = client.roundtrip();
            run_out(&handle);
            let (w, h, hidden, listed) = window_state(&handle);
            assert!(!listed && !hidden, "{what}: restored");
            assert!(
                !handle.window_is_fullscreen(TITLE),
                "{what}: restored windowed"
            );
            assert!(
                (w - w0).abs() < 2.0 && (h - h0).abs() < 2.0,
                "{what}: restored at {w}x{h}, not its own {w0}x{h0}"
            );
            handle.stop();
        }
    }

    /// Going fullscreen parks the window above everything for the transition
    /// and moves it into its fullscreen workspace when the transition ends. A
    /// window minimised before then is in its dock drawer by that time, and
    /// stays there.
    #[test]
    #[serial]
    fn a_window_minimised_while_going_fullscreen_stays_in_the_dock() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client = TestClient::connect(&handle.socket_name).expect("connect");
        let toplevel = client.create_toplevel(TITLE, 600, 400);
        handle.wait(Duration::from_millis(100));
        client.roundtrip().expect("roundtrip");
        toplevel.lock().unwrap().commit_frame();
        client.roundtrip().expect("roundtrip");
        handle.settle(300);
        let (_, h0, _, _) = window_state(&handle);

        handle.fullscreen_window(TITLE);
        let _ = client.roundtrip();
        for _ in 0..6 {
            handle.tick(1.0 / 60.0);
        }
        handle.minimize_window(TITLE);
        run_out(&handle);

        let (w, h, hidden, listed) = window_state(&handle);
        let y = handle.query(|state| {
            let id = state
                .workspaces
                .windows_map
                .iter()
                .find(|(_, w)| w.xdg_title() == TITLE)
                .map(|(id, _)| id.clone())
                .unwrap();
            let view = state.workspaces.get_window_view(&id).unwrap();
            view.window_layer
                .render_bounds_with_children_transformed()
                .y()
        });
        // The headless output's mode, in physical px, as the bounds are.
        let screen_h = 1080.0_f32;
        assert!(
            listed && !hidden,
            "the window is minimised and shown in the dock"
        );
        assert!(
            h < h0 / 4.0 && y > screen_h / 2.0,
            "the mini window ({w}x{h} at y {y}) is not in the dock at the bottom \
             of the {screen_h}px screen"
        );
        handle.stop();
    }
}
