//! An overlay that takes the keyboard gives it back to whoever it took it
//! from (headless).
//!
//! Peek opens on an exclusive-keyboard overlay layer surface, from either a
//! window or the desk — a bottom layer-shell surface covering the output.
//! Closing it has to put the keyboard back where it was: on the window, or on
//! the desk, not on whichever window happened to be focused last.

#[cfg(feature = "headless")]
mod overlay_focus_return_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::{KeyboardInteractivity, Layer, TestClient};
    use serial_test::serial;
    use std::time::Duration;

    const WINDOW: &str = "overlay-focus-window";

    /// Map an exclusive overlay, check it took the keyboard, destroy it.
    fn open_and_close_overlay(handle: &HeadlessHandle, client: &mut TestClient) {
        let overlay = client.create_layer_surface(
            "otto-peek",
            Layer::Overlay,
            400,
            300,
            KeyboardInteractivity::Exclusive,
        );
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        assert_eq!(
            handle.focused_layer_namespace().as_deref(),
            Some("otto-peek"),
            "the overlay takes the keyboard as soon as it is mapped"
        );

        {
            let overlay = overlay.lock().unwrap();
            overlay.layer_surface.destroy();
            overlay.surface.destroy();
        }
        let _ = client.roundtrip();
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
    }

    #[test]
    #[serial]
    fn closing_an_overlay_opened_from_the_desk_gives_the_desk_the_keyboard() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client = TestClient::connect(&handle.socket_name).expect("connect");

        let _desk = client.create_layer_surface(
            "otto-desk",
            Layer::Bottom,
            1920,
            1080,
            KeyboardInteractivity::OnDemand,
        );
        // A window that has had the keyboard, so there is a "last focused
        // window" for the wrong answer to be.
        let window = client.create_toplevel(WINDOW, 400, 300);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        window.lock().unwrap().commit_frame();
        let _ = client.roundtrip();
        handle.settle(300);
        handle.move_window(WINDOW, 300, 200);
        let _ = client.roundtrip();
        handle.settle(200);
        assert_eq!(handle.focused_window_title().as_deref(), Some(WINDOW));

        // A press on bare desk gives it the keyboard.
        let (x, y, w, h) = handle.window_logical_geometry(WINDOW).expect("mapped");
        handle.pointer_move(x as f64 + w as f64 + 100.0, y as f64 + h as f64 / 2.0);
        handle.pointer_click();
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        assert_eq!(
            handle.focused_layer_namespace().as_deref(),
            Some("otto-desk")
        );

        open_and_close_overlay(&handle, &mut client);
        assert_eq!(
            handle.focused_layer_namespace().as_deref(),
            Some("otto-desk"),
            "the keyboard goes back to the desk the overlay was opened from"
        );

        handle.stop();
    }

    #[test]
    #[serial]
    fn closing_an_overlay_opened_from_a_window_gives_the_window_the_keyboard() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client = TestClient::connect(&handle.socket_name).expect("connect");

        let window = client.create_toplevel(WINDOW, 400, 300);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        window.lock().unwrap().commit_frame();
        let _ = client.roundtrip();
        handle.settle(300);
        assert_eq!(handle.focused_window_title().as_deref(), Some(WINDOW));

        open_and_close_overlay(&handle, &mut client);
        assert_eq!(handle.focused_window_title().as_deref(), Some(WINDOW));

        handle.stop();
    }
}
