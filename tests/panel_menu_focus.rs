//! A panel's menu keeps the window it is about focused (headless).
//!
//! otto-bar opens its menus as popups of its top layer surface and takes the
//! keyboard for them, so the arrow keys move through the items. The window
//! the menu is about is still the one being used, so it stays `activated`.
//! A layer surface that takes the
//! keyboard without a menu, such as the launcher, does deactivate it.

#[cfg(feature = "headless")]
mod panel_menu_focus_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::{KeyboardInteractivity, Layer, TestClient};
    use serial_test::serial;
    use std::time::Duration;

    const WINDOW: &str = "panel-menu-window";
    const PANEL: &str = "panel-menu-bar";

    fn settle(handle: &HeadlessHandle, client: &mut TestClient) {
        for _ in 0..4 {
            handle.wait(Duration::from_millis(50));
            let _ = client.roundtrip();
        }
    }

    #[test]
    #[serial]
    fn a_panel_menu_keeps_the_window_activated() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client = TestClient::connect(&handle.socket_name).expect("connect");

        let window = client.create_toplevel(WINDOW, 400, 300);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        window.lock().unwrap().commit_frame();
        settle(&handle, &mut client);
        assert_eq!(handle.focused_window_title().as_deref(), Some(WINDOW));
        assert!(
            window.lock().unwrap().activated,
            "the window starts focused"
        );

        let panel =
            client.create_layer_surface(PANEL, Layer::Top, 400, 30, KeyboardInteractivity::None);
        settle(&handle, &mut client);

        // Open a menu, then take the keyboard for it, in otto-bar's order.
        let _menu = client.create_layer_popup(&panel, 120, 80);
        panel
            .lock()
            .unwrap()
            .set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        settle(&handle, &mut client);
        assert_eq!(
            handle.focused_layer_namespace().as_deref(),
            Some(PANEL),
            "the panel holds the keyboard while its menu is open"
        );
        assert!(
            window.lock().unwrap().activated,
            "a panel menu must not deactivate the window"
        );

        handle.stop();
    }

    #[test]
    #[serial]
    fn a_panel_without_a_menu_deactivates_the_window() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client = TestClient::connect(&handle.socket_name).expect("connect");

        let window = client.create_toplevel(WINDOW, 400, 300);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        window.lock().unwrap().commit_frame();
        settle(&handle, &mut client);
        assert!(
            window.lock().unwrap().activated,
            "the window starts focused"
        );

        let _launcher = client.create_layer_surface(
            PANEL,
            Layer::Top,
            400,
            300,
            KeyboardInteractivity::Exclusive,
        );
        settle(&handle, &mut client);
        assert_eq!(handle.focused_layer_namespace().as_deref(), Some(PANEL));
        assert!(
            !window.lock().unwrap().activated,
            "a layer surface typed into takes the window's focus"
        );

        handle.stop();
    }
}
