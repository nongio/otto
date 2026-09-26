//! A popup that draws into a subsurface of its own redraws (headless).
//!
//! Firefox puts its menus up as an xdg_popup with a desync subsurface
//! carrying the content, and every later frame (the fade-in, hover
//! highlights) is a commit on that subsurface alone. The commit has to be
//! traced back through the popup to its window, or nothing is damaged, no
//! frame is drawn, and the menu stays on whatever frame it first showed.

#[cfg(feature = "headless")]
mod popup_subsurface_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::{ShmBuffer, TestClient};
    use serial_test::serial;
    use std::time::Duration;

    const TITLE: &str = "popup-subsurface-window";

    #[test]
    #[serial]
    fn a_commit_on_a_popup_subsurface_reaches_the_scene() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client = TestClient::connect(&handle.socket_name).expect("connect");

        let toplevel = client.create_toplevel(TITLE, 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(300);
        handle.focus_window(TITLE);
        handle.settle(200);

        let popup = client.create_popup(&toplevel, 80, 120, 200, 150);
        let _ = client.roundtrip();
        handle.settle(400);
        let popup_surface = popup.lock().unwrap().surface.clone();
        let (content, subsurface, buffer) =
            client.create_subsurface(&popup_surface, 0, 0, 200, 150);
        subsurface.set_desync();
        let _ = client.roundtrip();
        handle.settle(400);
        assert_eq!(handle.popup_logical_rects().len(), 1, "the popup is up");

        // A frame of the menu's own, at a size nothing else in the scene
        // has: the subsurface commits, the popup does not.
        let scale = handle.output_scale();
        let (w, h) = (123u32, 77u32);
        let frame = ShmBuffer::new(
            client.state.wl_shm.as_ref().expect("wl_shm"),
            &client.qh,
            w,
            h,
        );
        content.attach(Some(frame.buffer()), 0, 0);
        content.damage(0, 0, w as i32, h as i32);
        content.commit();
        let _ = client.roundtrip();
        handle.settle(100);
        drop(buffer);

        let width_px = (w as f64 * scale).round();
        let json = handle.scene_json();
        let needle = format!("{width_px}");
        assert!(
            json.lines()
                .any(|l| l.contains("\"width\"") && l.contains(&needle)),
            "the popup's layer shows the subsurface's new {w}x{h} frame"
        );

        handle.stop();
    }
}
