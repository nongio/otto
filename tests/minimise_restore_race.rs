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
}
