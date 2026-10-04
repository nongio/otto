//! End-to-end tests for logging out: the `logout` command asks every window
//! to close and ends the session once they have, unless an application opens
//! a window to ask something first.

#[cfg(feature = "headless")]
mod logout_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::time::Duration;

    /// Longer than the logout's poll, so a decision has been taken.
    const DECIDED: Duration = Duration::from_millis(800);

    fn logout(handle: &HeadlessHandle) {
        for result in handle.run_command("logout") {
            result.expect("logout is a known command");
        }
    }

    fn connect(handle: &HeadlessHandle) -> TestClient {
        TestClient::connect(&handle.socket_name).expect("Failed to connect to compositor")
    }

    #[test]
    #[serial]
    fn an_empty_session_ends_at_once() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        logout(&handle);
        handle.wait(DECIDED);
        assert!(
            !handle.is_running(),
            "nothing to close, so the session ends"
        );
        handle.stop();
    }

    #[test]
    #[serial]
    fn the_session_ends_once_every_window_has_closed() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut first = connect(&handle);
        let mut second = connect(&handle);
        let a = first.create_toplevel("first", 400, 300);
        let b = second.create_toplevel("second", 400, 300);
        let _ = first.roundtrip();
        let _ = second.roundtrip();
        handle.settle(300);

        logout(&handle);
        let _ = first.roundtrip();
        let _ = second.roundtrip();
        assert!(
            a.lock().unwrap().closed,
            "the first window was asked to close"
        );
        assert!(
            b.lock().unwrap().closed,
            "the second window was asked to close"
        );
        assert!(handle.is_running(), "the session waits for the windows");

        // Each application does as it is asked.
        drop(first);
        handle.wait(DECIDED);
        assert!(handle.is_running(), "one window is still open");
        drop(second);
        handle.wait(DECIDED);
        assert!(
            !handle.is_running(),
            "every window closed, so the session ends"
        );
        handle.stop();
    }

    #[test]
    #[serial]
    fn a_save_prompt_stops_the_logout() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut editor = connect(&handle);
        let document = editor.create_toplevel("document", 400, 300);
        let _ = editor.roundtrip();
        handle.settle(300);

        logout(&handle);
        let _ = editor.roundtrip();
        assert!(document.lock().unwrap().closed);

        // The editor answers the close with a "save changes?" dialog.
        let _prompt = editor.create_child_toplevel("Save changes?", &document, 300, 120);
        let _ = editor.roundtrip();
        handle.wait(DECIDED);
        assert!(handle.is_running(), "the logout stands down for the prompt");

        // The prompt is answered and the editor quits. Nothing ends the
        // session until the person logs out again, and then it does.
        drop(editor);
        handle.wait(DECIDED);
        assert!(handle.is_running(), "a stood-down logout does not resume");
        logout(&handle);
        handle.wait(DECIDED);
        assert!(!handle.is_running(), "logging out again ends the session");
        handle.stop();
    }
}
