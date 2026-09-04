//! A fullscreen window must not reach past its own workspace (headless).
//!
//! Fullscreening moves the window onto a dedicated workspace and registers it
//! as the output's `FullscreenSurface`. That registration is cleared on
//! unfullscreen, not on a workspace switch, so it used to keep answering for
//! the whole output after the user scrolled away: every click on the visible
//! desktop hit-tested against the off-screen fullscreen window, was swallowed,
//! and dragged the keyboard focus with it.

#[cfg(feature = "headless")]
mod fullscreen_focus_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::time::Duration;

    const FULLSCREEN: &str = "fullscreen-window";
    const OTHER: &str = "other-window";

    // Both windows announce an app_id. Without one the compositor resolves the
    // app from the client's PID, which scans every desktop entry on the host —
    // once per configure, and the fullscreen animation sends one per frame.

    /// Both windows are 640x480 and parked at the output origin area, so a
    /// click at this point lands inside both of them — the whole question is
    /// which one the compositor decides owns it.
    const CLICK: (f64, f64) = (200.0, 200.0);

    struct Fixture {
        handle: HeadlessHandle,
        _clients: (TestClient, TestClient),
        /// The workspace the fullscreen window was sent to.
        fullscreen_workspace: usize,
        /// The workspace `OTHER` stayed on.
        home_workspace: usize,
    }

    fn setup() -> Fixture {
        let handle = HeadlessHandle::start(HeadlessConfig::default());

        let mut other_client =
            TestClient::connect(&handle.socket_name).expect("Failed to connect to compositor");
        let _other = other_client.create_toplevel_with_app_id(OTHER, "otto.test.Other", 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = other_client.roundtrip();
        handle.settle(300);

        let mut fs_client =
            TestClient::connect(&handle.socket_name).expect("Failed to connect to compositor");
        let _fs =
            fs_client.create_toplevel_with_app_id(FULLSCREEN, "otto.test.Fullscreen", 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = fs_client.roundtrip();
        handle.settle(300);

        // Park both over the click point so the hit test is genuinely
        // ambiguous once the fullscreen window is off-screen.
        handle.move_window(OTHER, 0, 0);
        handle.move_window(FULLSCREEN, 0, 0);
        handle.settle(300);

        let home_workspace = handle.current_workspace_index();

        // The fullscreen transition sends the client a configure on every
        // animation frame; pump its queue while the animation runs so the
        // compositor is never talking into a socket nobody drains.
        handle.fullscreen_window(FULLSCREEN);
        for _ in 0..12 {
            handle.settle(30);
            let _ = fs_client.roundtrip();
            let _ = other_client.roundtrip();
        }

        assert!(
            handle.window_is_fullscreen(FULLSCREEN),
            "the fixture needs the window to actually be fullscreen"
        );
        let fullscreen_workspace = handle.current_workspace_index();
        assert_ne!(
            fullscreen_workspace, home_workspace,
            "fullscreen should move the window onto its own workspace"
        );

        Fixture {
            handle,
            _clients: (other_client, fs_client),
            fullscreen_workspace,
            home_workspace,
        }
    }

    /// Switching away from the fullscreen workspace hands the keyboard to the
    /// window on the workspace you land on.
    #[test]
    #[serial]
    fn leaving_the_fullscreen_workspace_moves_focus() {
        let f = setup();

        f.handle.with_state({
            let index = f.home_workspace;
            move |state| state.set_current_workspace_index(index)
        });
        f.handle.settle(600);

        assert_eq!(
            f.handle.focused_window_title().as_deref(),
            Some(OTHER),
            "the visible workspace's window should hold the keyboard"
        );

        f.handle.stop();
    }

    /// Clicking on the visible desktop must focus the window that is actually
    /// there, not the fullscreen window parked on another workspace.
    #[test]
    #[serial]
    fn clicking_the_visible_desktop_does_not_focus_the_offscreen_fullscreen_window() {
        let f = setup();

        f.handle.with_state({
            let index = f.home_workspace;
            move |state| state.set_current_workspace_index(index)
        });
        f.handle.settle(600);

        // Focus something else first, so the assertion cannot pass by accident
        // on state left over from the workspace switch.
        f.handle.focus_window(OTHER);
        f.handle.settle(200);

        f.handle.pointer_move(CLICK.0, CLICK.1);
        f.handle.settle(200);
        f.handle.pointer_click();
        f.handle.settle(300);

        assert_eq!(
            f.handle.focused_window_title().as_deref(),
            Some(OTHER),
            "a click on the visible desktop must not be stolen by the \
             fullscreen window on workspace {}",
            f.fullscreen_workspace
        );

        f.handle.stop();
    }

    /// The flip side: while you are ON the fullscreen workspace, the
    /// fullscreen window still owns clicks over it. This is the behaviour the
    /// XWayland game-focus work depends on and must not regress.
    #[test]
    #[serial]
    fn clicking_the_fullscreen_workspace_focuses_the_fullscreen_window() {
        let f = setup();

        assert_eq!(
            f.handle.current_workspace_index(),
            f.fullscreen_workspace,
            "fullscreening should have left us on the fullscreen workspace"
        );

        f.handle.pointer_move(CLICK.0, CLICK.1);
        f.handle.settle(200);
        f.handle.pointer_click();
        f.handle.settle(300);

        assert_eq!(
            f.handle.focused_window_title().as_deref(),
            Some(FULLSCREEN),
            "the fullscreen window owns its own workspace"
        );

        f.handle.stop();
    }
}
