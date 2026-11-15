//! An exposé swipe that is started and then undone gives the keyboard back
//! (headless).
//!
//! The swipe takes the keyboard as soon as it starts, before exposé is open,
//! so the window in front loses its activated state. Swiping back before the
//! overview opens has to hand it back: otherwise the window stays unfocused,
//! drawn as inactive, until it is clicked.

#[cfg(feature = "headless")]
mod expose_gesture_focus_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::time::Duration;

    const TITLE: &str = "expose-undo";

    #[test]
    #[serial]
    fn an_undone_expose_swipe_gives_the_keyboard_back() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client = TestClient::connect(&handle.socket_name).expect("connect");
        let window = client.create_toplevel_with_app_id(TITLE, "otto.test.ExposeUndo", 400, 300);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        window.lock().unwrap().commit_frame();
        let _ = client.roundtrip();
        handle.settle(300);
        let _ = client.roundtrip();
        assert_eq!(handle.focused_window_title().as_deref(), Some(TITLE));
        assert!(window.lock().unwrap().activated);

        // Up a little, then back down past where it started, and lift.
        handle.swipe_begin();
        handle.swipe_update(0.0, -20.0);
        let _ = client.roundtrip();
        assert_eq!(
            handle.focused_window_title(),
            None,
            "the swipe takes the keyboard while it runs"
        );
        handle.swipe_update(0.0, 30.0);
        handle.swipe_update(0.0, 30.0);
        handle.swipe_end();
        handle.settle(300);
        let _ = client.roundtrip();

        assert!(!handle.is_expose_active(), "the swipe was undone");
        assert_eq!(handle.focused_window_title().as_deref(), Some(TITLE));
        assert!(
            window.lock().unwrap().activated,
            "the window is told it is active again"
        );

        handle.stop();
    }
}
