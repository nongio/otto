//! A fullscreen window wears no server-side decoration.
//!
//! A window that covers its whole output has no frame: the titlebar Otto
//! draws, the drop shadow it sits in and the resize border along its edges
//! all have to go, and all of them have to come back when it leaves
//! fullscreen. The bar is the visible half of it; `decoration_height` is the
//! half the geometry cares about, because a titlebar the client is never told
//! about still eats 34pt off the surface it was promised.

#[cfg(feature = "headless")]
mod ssd_fullscreen_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::time::Duration;

    const TITLE: &str = "fullscreen-ssd-window";

    /// Run the scene forward while letting the client answer the configures
    /// the transition sends it. Small batches: the compositor sends one
    /// configure per animated frame, and a client left unpumped through a
    /// long batch stalls the loop the test is querying.
    fn settle_with_client(handle: &HeadlessHandle, client: &mut TestClient) {
        for _ in 0..400 {
            if handle.settle(3) == 0 {
                break;
            }
            let _ = client.roundtrip();
        }
        let _ = client.roundtrip();
        handle.settle(30);
    }

    #[test]
    #[serial]
    fn fullscreen_drops_the_titlebar_and_shadow() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client =
            TestClient::connect(&handle.socket_name).expect("Failed to connect to compositor");

        let _toplevel = client.create_toplevel(TITLE, 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(300);
        handle.decorate_window(TITLE);
        handle.settle(200);
        handle.focus_window(TITLE);
        handle.settle(200);

        // ── Windowed: the bar is there ───────────────────────────────────
        assert!(
            handle.window_is_decorated(TITLE),
            "the window negotiated a server-side titlebar"
        );
        let (bar, shadow) = handle
            .window_chrome_visible(TITLE)
            .expect("the window has a view");
        assert!(bar, "a windowed SSD window shows its titlebar");
        assert!(shadow, "a windowed SSD window casts a shadow");

        // ── Fullscreen: it is not ────────────────────────────────────────
        handle.fullscreen_window(TITLE);
        // The fullscreen transition animates for over a second and sends the
        // client a configure on every frame: pump the client between batches
        // of frames, or the compositor is left writing to a socket nobody is
        // reading from.
        settle_with_client(&handle, &mut client);

        assert!(
            !handle.window_is_decorated(TITLE),
            "a fullscreen window is not decorated"
        );
        let (bar, shadow) = handle
            .window_chrome_visible(TITLE)
            .expect("the window has a view");
        assert!(!bar, "a fullscreen window must not draw a titlebar");
        assert!(!shadow, "a fullscreen window must not draw a shadow");

        // ── Back out: the bar returns ────────────────────────────────────
        handle.unfullscreen_window(TITLE);
        settle_with_client(&handle, &mut client);

        assert!(
            handle.window_is_decorated(TITLE),
            "leaving fullscreen restores the negotiated titlebar"
        );
        let (bar, shadow) = handle
            .window_chrome_visible(TITLE)
            .expect("the window has a view");
        assert!(bar, "the titlebar comes back after fullscreen");
        assert!(shadow, "the shadow comes back after fullscreen");

        handle.stop();
    }
}
