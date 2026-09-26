//! A press on a window keeps the keyboard on the window, even with a
//! bottom-layer surface behind it (headless).
//!
//! A desktop icon view is a bottom layer-shell surface covering the whole
//! output, with on-demand keyboard interactivity. A press on a window is also
//! over it, and must not hand it the keyboard: the window would be deactivated
//! at the press, and a browser opening a menu there opens it without a grab,
//! so a click on a menu item does nothing.

#[cfg(feature = "headless")]
mod desk_focus_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::{KeyboardInteractivity, Layer, TestClient};
    use serial_test::serial;
    use std::time::Duration;

    const WINDOW: &str = "desk-focus-window";

    #[test]
    #[serial]
    fn a_press_on_a_window_does_not_focus_the_desk_behind_it() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client = TestClient::connect(&handle.socket_name).expect("connect");

        let _desk = client.create_layer_surface(
            "otto-desk",
            Layer::Bottom,
            1920,
            1080,
            KeyboardInteractivity::OnDemand,
        );
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

        let (x, y, w, h) = handle.window_logical_geometry(WINDOW).expect("mapped");
        handle.pointer_move(x as f64 + w as f64 / 2.0, y as f64 + h as f64 / 2.0);
        handle.wait(Duration::from_millis(50));

        handle.pointer_press();
        handle.wait(Duration::from_millis(50));
        let _ = client.roundtrip();
        assert_eq!(
            handle.focused_layer_namespace(),
            None,
            "the desk behind the window must not take the keyboard at the press"
        );
        assert_eq!(handle.focused_window_title().as_deref(), Some(WINDOW));

        handle.pointer_release();
        handle.wait(Duration::from_millis(50));
        assert_eq!(handle.focused_window_title().as_deref(), Some(WINDOW));

        // A press on bare desk still gives it the keyboard.
        handle.pointer_move(x as f64 + w as f64 + 100.0, y as f64 + h as f64 / 2.0);
        handle.pointer_click();
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        assert_eq!(
            handle.focused_layer_namespace().as_deref(),
            Some("otto-desk")
        );

        handle.stop();
    }
}
