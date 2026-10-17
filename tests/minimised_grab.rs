//! A client cannot crash the compositor by asking to move or resize one of
//! its windows that is minimised (headless, #322).
//!
//! Minimising unmaps a window from every space, so it has no location. The
//! request guards only check that the press serial belongs to the same
//! client, so a client with one window on screen and another in the dock can
//! hand a serial from a click in the first to `xdg_toplevel.move` or
//! `xdg_toplevel.resize` on the second.

#[cfg(feature = "headless")]
mod minimised_grab_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::{TestClient, TestToplevel};
    use serial_test::serial;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use wayland_protocols::xdg::shell::client::xdg_toplevel::ResizeEdge;

    const VISIBLE: &str = "grab-visible";
    const MINIMISED: &str = "grab-minimised";

    struct Fixture {
        handle: HeadlessHandle,
        client: TestClient,
        minimised: Arc<Mutex<TestToplevel>>,
    }

    fn map(
        handle: &HeadlessHandle,
        client: &mut TestClient,
        title: &str,
    ) -> Arc<Mutex<TestToplevel>> {
        let toplevel = client.create_toplevel(title, 400, 300);
        handle.wait(Duration::from_millis(100));
        client.roundtrip().expect("roundtrip");
        toplevel.lock().unwrap().commit_frame();
        client.roundtrip().expect("roundtrip");
        handle.settle(300);
        toplevel
    }

    /// One client, two windows: `MINIMISED` in the dock, and the button held
    /// down on `VISIBLE`, so the client holds a live press serial.
    fn setup() -> Fixture {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client = TestClient::connect(&handle.socket_name).expect("connect");

        let _visible = map(&handle, &mut client, VISIBLE);
        let minimised = map(&handle, &mut client, MINIMISED);
        handle.move_window(VISIBLE, 0, 100);
        handle.move_window(MINIMISED, 450, 100);
        let _ = client.roundtrip();
        handle.settle(300);

        handle.minimize_window(MINIMISED);
        handle.settle(300);
        let _ = client.roundtrip();
        assert!(
            !handle.space_stack_titles().iter().any(|t| t == MINIMISED),
            "minimising unmaps the window from the space"
        );

        let (x, y, w, h) = handle
            .window_logical_geometry(VISIBLE)
            .expect("the visible window is mapped");
        handle.pointer_move(x as f64 + w as f64 / 2.0, y as f64 + h as f64 / 2.0);
        handle.wait(Duration::from_millis(50));
        client.roundtrip().expect("roundtrip");
        handle.pointer_press();
        handle.wait(Duration::from_millis(50));
        client.roundtrip().expect("roundtrip");
        assert!(
            client.state.last_button_serial.is_some(),
            "the press never reached the client, so it holds no serial"
        );

        Fixture {
            handle,
            client,
            minimised,
        }
    }

    /// After the request: the compositor is alive, the window stays in the
    /// dock, and the release ends the click grab as usual.
    fn assert_survives(handle: &HeadlessHandle, client: &mut TestClient) {
        handle.wait(Duration::from_millis(100));
        // A panicked compositor drops the query channel: this fails loudly.
        let windows = handle.window_count();
        assert!(windows >= 1, "the compositor still tracks the windows");
        assert!(handle.is_running(), "the compositor is still running");
        assert!(
            !handle.space_stack_titles().iter().any(|t| t == MINIMISED),
            "a refused grab does not map the minimised window back"
        );

        handle.pointer_release();
        handle.wait(Duration::from_millis(50));
        client.roundtrip().expect("the client is still connected");
        handle.settle(120);
        assert!(handle.is_running());
    }

    #[test]
    #[serial]
    fn move_request_on_a_minimised_window_is_refused() {
        let Fixture {
            handle,
            mut client,
            minimised,
        } = setup();

        let seat = client.state.wl_seat.clone().expect("wl_seat bound");
        let serial = client.state.last_button_serial.expect("press serial");
        minimised
            .lock()
            .unwrap()
            .toplevel
            .as_ref()
            .expect("xdg_toplevel")
            ._move(&seat, serial);
        client
            .roundtrip()
            .expect("roundtrip after the move request");

        assert_survives(&handle, &mut client);
        handle.stop();
    }

    #[test]
    #[serial]
    fn resize_request_on_a_minimised_window_is_refused() {
        let Fixture {
            handle,
            mut client,
            minimised,
        } = setup();

        let seat = client.state.wl_seat.clone().expect("wl_seat bound");
        let serial = client.state.last_button_serial.expect("press serial");
        minimised
            .lock()
            .unwrap()
            .toplevel
            .as_ref()
            .expect("xdg_toplevel")
            .resize(&seat, serial, ResizeEdge::TopLeft);
        client
            .roundtrip()
            .expect("roundtrip after the resize request");

        assert_survives(&handle, &mut client);
        handle.stop();
    }
}
