//! Press and hold a letter for its accented forms, end to end: a window with
//! a caret, a key held down through the compositor's real keyboard path, and
//! what the window is sent along the way.

#[cfg(feature = "headless")]
mod headless_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::sync::mpsc;
    use std::time::Duration;

    // evdev codes.
    const KEY_E: u32 = 18;
    const KEY_X: u32 = 45;
    const KEY_RIGHT: u32 = 106;
    const KEY_ESC: u32 = 1;

    /// Longer than the repeat delay the row waits for.
    const HOLD: Duration = Duration::from_millis(600);

    fn row_shown(handle: &HeadlessHandle) -> bool {
        let (tx, rx) = mpsc::channel();
        handle.with_state(move |state| {
            let _ = tx.send(state.workspaces.accent_picker.is_visible());
        });
        rx.recv().unwrap()
    }

    fn start(caret: bool) -> (HeadlessHandle, TestClient) {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        std::env::set_var("WAYLAND_DISPLAY", &handle.socket_name);
        let mut app = TestClient::connect(&handle.socket_name).expect("connect");
        let _window = app.create_toplevel("accent-target", 640, 480);
        handle.wait(Duration::from_millis(150));
        app.roundtrip().expect("roundtrip");
        if caret {
            assert!(app.set_text_cursor(40, 60, 2, 18));
            handle.wait(Duration::from_millis(150));
            app.roundtrip().expect("roundtrip");
        }
        app.state.keys.clear();
        (handle, app)
    }

    #[test]
    #[serial]
    fn holding_a_letter_types_it_once_and_shows_its_forms() {
        let (handle, mut app) = start(true);

        handle.key(KEY_E, true);
        handle.wait(Duration::from_millis(50));
        app.roundtrip().expect("roundtrip");
        assert_eq!(
            app.state.keys,
            vec![(KEY_E, true), (KEY_E, false)],
            "the letter is typed at once and released at once, so the window never repeats it"
        );
        assert!(!row_shown(&handle), "no row before the hold is long enough");

        handle.wait(HOLD);
        assert!(
            row_shown(&handle),
            "the row is up once the key has been held"
        );

        // The real release, the arrows and Escape all stay with the row.
        handle.key(KEY_E, false);
        handle.key(KEY_RIGHT, true);
        handle.key(KEY_RIGHT, false);
        assert!(
            row_shown(&handle),
            "releasing the key and moving leave the row up"
        );
        handle.key(KEY_ESC, true);
        handle.key(KEY_ESC, false);
        handle.wait(Duration::from_millis(50));
        app.roundtrip().expect("roundtrip");
        assert_eq!(
            app.state.keys,
            vec![(KEY_E, true), (KEY_E, false)],
            "nothing the row handled reaches the window"
        );

        handle.wait(Duration::from_millis(300));
        assert!(!row_shown(&handle), "Escape puts the row away");
        handle.stop();
    }

    #[test]
    #[serial]
    fn another_key_closes_the_row_and_goes_through() {
        let (handle, mut app) = start(true);

        handle.key(KEY_E, true);
        handle.wait(HOLD);
        handle.key(KEY_E, false);
        assert!(row_shown(&handle));

        handle.key(KEY_X, true);
        handle.key(KEY_X, false);
        handle.wait(Duration::from_millis(50));
        app.roundtrip().expect("roundtrip");
        assert_eq!(
            app.state.keys,
            vec![(KEY_E, true), (KEY_E, false), (KEY_X, true), (KEY_X, false)]
        );
        handle.wait(Duration::from_millis(300));
        assert!(!row_shown(&handle));
        handle.stop();
    }

    #[test]
    #[serial]
    fn without_a_caret_a_held_key_is_left_to_repeat() {
        let (handle, mut app) = start(false);

        handle.key(KEY_E, true);
        handle.wait(HOLD);
        app.roundtrip().expect("roundtrip");
        assert_eq!(
            app.state.keys,
            vec![(KEY_E, true)],
            "a window with no caret (a game) keeps the key down and repeats it itself"
        );
        assert!(!row_shown(&handle));
        handle.key(KEY_E, false);
        handle.stop();
    }

    #[test]
    #[serial]
    fn a_key_with_no_forms_is_untouched() {
        let (handle, mut app) = start(true);

        handle.key(KEY_X, true);
        handle.wait(HOLD);
        app.roundtrip().expect("roundtrip");
        assert_eq!(app.state.keys, vec![(KEY_X, true)]);
        assert!(!row_shown(&handle));
        handle.key(KEY_X, false);
        handle.stop();
    }
}
