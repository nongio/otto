//! The server-side titlebar follows the keyboard focus (headless).
//!
//! The bar Otto draws has a focused and an unfocused look. A focus change has
//! to reach both bars involved — the one gaining focus and the one losing it —
//! whether or not either client paints in response. xdg-shell lets a client
//! ack the `activated` configure without committing, and a promoted window's
//! commits skip the scene import, so a bar that only learns about focus on the
//! window's next import is left showing the wrong state.
//!
//! `TestClient` is the idle client here: it acks every configure and never
//! commits on its own.

#[cfg(feature = "headless")]
mod ssd_focus_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::{TestClient, TestToplevel};
    use serial_test::serial;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    const FIRST: &str = "ssd-first";
    const SECOND: &str = "ssd-second";

    struct Fixture {
        handle: HeadlessHandle,
        client: TestClient,
        first: Arc<Mutex<TestToplevel>>,
        second: Arc<Mutex<TestToplevel>>,
    }

    /// Two decorated windows side by side, the second one mapped last.
    fn setup() -> Fixture {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client =
            TestClient::connect(&handle.socket_name).expect("Failed to connect to compositor");

        let first = client.create_toplevel_with_app_id(FIRST, "otto.test.First", 400, 300);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(300);
        handle.decorate_window(FIRST);
        handle.settle(200);

        let second = client.create_toplevel_with_app_id(SECOND, "otto.test.Second", 400, 300);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(300);
        handle.decorate_window(SECOND);
        handle.settle(200);

        handle.move_window(FIRST, 0, 100);
        handle.move_window(SECOND, 500, 100);
        let _ = client.roundtrip();
        handle.settle(300);

        Fixture {
            handle,
            client,
            first,
            second,
        }
    }

    fn assert_bars(handle: &HeadlessHandle, focused: &str, unfocused: &str, when: &str) {
        assert_eq!(
            handle.focused_window_title().as_deref(),
            Some(focused),
            "{when}: the fixture expects {focused} to hold the keyboard"
        );
        assert_eq!(
            handle.window_decoration_active(focused),
            Some(true),
            "{when}: the focused window's titlebar should wear the active look"
        );
        assert_eq!(
            handle.window_decoration_active(unfocused),
            Some(false),
            "{when}: the window that lost focus should wear the inactive look"
        );
    }

    /// Mapping a window moves the keyboard to it. The window it takes the
    /// focus from never commits again, so its bar must be dimmed by the focus
    /// change itself.
    #[test]
    #[serial]
    fn mapping_a_window_dims_the_previous_titlebar() {
        let Fixture {
            handle,
            client: _client,
            ..
        } = setup();

        assert_bars(&handle, SECOND, FIRST, "after mapping the second window");

        handle.stop();
    }

    /// Focusing a window by clicking it, with neither client repainting.
    #[test]
    #[serial]
    fn clicking_a_window_moves_the_active_titlebar() {
        let Fixture {
            handle, mut client, ..
        } = setup();

        let (x, y, w, h) = handle.window_logical_geometry(FIRST).expect("mapped");
        handle.pointer_move((x + w / 2) as f64, (y + h / 2) as f64);
        handle.pointer_click();
        let _ = client.roundtrip();
        handle.settle(100);

        assert_bars(&handle, FIRST, SECOND, "after clicking the first window");

        handle.stop();
    }

    /// The same focus change, where both clients DO commit afterwards — but
    /// both windows are promoted to planes, the way two tiled windows side by
    /// side are on the DRM backend. Their commits take the scanout branch,
    /// which refreshes chrome geometry only.
    #[test]
    #[serial]
    fn focusing_a_scanned_out_window_moves_the_active_titlebar() {
        let Fixture {
            handle,
            mut client,
            first,
            second,
        } = setup();

        // A commit on each while still composited, so both bars start from
        // the focus the seat really has.
        first.lock().unwrap().commit_frame();
        second.lock().unwrap().commit_frame();
        let _ = client.roundtrip();
        handle.settle(100);

        handle.set_window_scanned_out(FIRST, true);
        handle.set_window_scanned_out(SECOND, true);
        // And one after promotion: the first promoted commit may still carry
        // a chrome geometry catch-up, which refreshes the bar for reasons that
        // have nothing to do with focus.
        first.lock().unwrap().commit_frame();
        second.lock().unwrap().commit_frame();
        let _ = client.roundtrip();
        handle.settle(100);
        assert_bars(&handle, SECOND, FIRST, "before the focus change");

        handle.focus_window(FIRST);
        let _ = client.roundtrip();
        first.lock().unwrap().commit_frame();
        second.lock().unwrap().commit_frame();
        let _ = client.roundtrip();
        handle.settle(100);

        assert_bars(
            &handle,
            FIRST,
            SECOND,
            "after focusing a promoted window that then commits",
        );

        handle.stop();
    }
}
