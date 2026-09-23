//! A left press on a window behind others raises it on release (headless).
//!
//! Raising at the press would bring the window forward before its client even
//! sees the button, so nothing could be dragged out of a window that is not
//! already in front. The press goes to the client straight away; the raise
//! and the keyboard follow when the button comes up, unless the press became
//! a drag, in which case the source window stays where it is.

#[cfg(feature = "headless")]
mod raise_on_release_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::{TestClient, TestToplevel};
    use serial_test::serial;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    const BACK: &str = "raise-back";
    const FRONT: &str = "raise-front";

    struct Fixture {
        handle: HeadlessHandle,
        client: TestClient,
        back: Arc<Mutex<TestToplevel>>,
        // `FRONT` lives in its own client: a drag passing over a surface of
        // the dragging client would hand it a data offer `TestClient` does
        // not take.
        _front_client: TestClient,
    }

    /// Two overlapping windows, `FRONT` mapped last so it is on top and
    /// focused, with the pointer on the part of `BACK` it does not cover.
    fn setup() -> Fixture {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client = TestClient::connect(&handle.socket_name).expect("connect");

        let back = client.create_toplevel_with_app_id(BACK, "otto.test.Back", 400, 300);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        back.lock().unwrap().commit_frame();
        let _ = client.roundtrip();
        handle.settle(300);

        let mut front_client = TestClient::connect(&handle.socket_name).expect("connect");
        let front = front_client.create_toplevel_with_app_id(FRONT, "otto.test.Front", 400, 300);
        handle.wait(Duration::from_millis(100));
        let _ = front_client.roundtrip();
        front.lock().unwrap().commit_frame();
        let _ = front_client.roundtrip();
        handle.settle(300);

        handle.move_window(BACK, 0, 100);
        handle.move_window(FRONT, 250, 100);
        let _ = client.roundtrip();
        let _ = front_client.roundtrip();
        handle.settle(300);

        assert_eq!(handle.top_window_title().as_deref(), Some(FRONT));
        assert_eq!(handle.focused_window_title().as_deref(), Some(FRONT));

        let (x, y, _, h) = handle.window_logical_geometry(BACK).expect("mapped");
        handle.pointer_move(x as f64 + 100.0, y as f64 + h as f64 / 2.0);
        handle.wait(Duration::from_millis(50));
        let _ = client.roundtrip();

        Fixture {
            handle,
            client,
            back,
            _front_client: front_client,
        }
    }

    /// A plain click still brings the window forward, once the button is up.
    #[test]
    #[serial]
    fn clicking_a_window_behind_raises_it_on_release() {
        let Fixture {
            handle,
            mut client,
            _front_client: _front,
            ..
        } = setup();

        handle.pointer_press();
        handle.wait(Duration::from_millis(50));
        let _ = client.roundtrip();
        handle.settle(50);
        assert!(
            client.state.last_button_serial.is_some(),
            "the press should reach the window behind"
        );
        assert_eq!(
            handle.top_window_title().as_deref(),
            Some(FRONT),
            "the press alone must not raise the window behind"
        );

        handle.pointer_release();
        handle.wait(Duration::from_millis(50));
        let _ = client.roundtrip();
        handle.settle(100);
        assert_eq!(handle.top_window_title().as_deref(), Some(BACK));
        assert_eq!(handle.focused_window_title().as_deref(), Some(BACK));

        handle.stop();
    }

    /// A drag out of a window behind leaves the stacking and focus alone.
    #[test]
    #[serial]
    fn dragging_from_a_window_behind_leaves_it_behind() {
        let Fixture {
            handle,
            mut client,
            back,
            _front_client: _front,
        } = setup();

        let (x, y, w, h) = handle.window_logical_geometry(FRONT).expect("mapped");
        let drop_point = (x as f64 + w as f64 - 50.0, y as f64 + h as f64 / 2.0);

        handle.pointer_press();
        handle.wait(Duration::from_millis(50));
        let _ = client.roundtrip();
        assert!(
            client.state.last_button_serial.is_some(),
            "the press never reached the client, so no drag can be authorised"
        );

        let origin = back.lock().unwrap().surface.clone();
        client
            .start_drag(&origin, None, &["text/uri-list"])
            .expect("the drag started");
        let _ = client.roundtrip();
        handle.settle(100);

        handle.pointer_move(drop_point.0, drop_point.1);
        handle.wait(Duration::from_millis(50));
        let _ = client.roundtrip();

        handle.pointer_release();
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(200);

        assert_eq!(
            handle.top_window_title().as_deref(),
            Some(FRONT),
            "the drag source should stay behind"
        );
        assert_eq!(handle.focused_window_title().as_deref(), Some(FRONT));

        handle.stop();
    }
}
