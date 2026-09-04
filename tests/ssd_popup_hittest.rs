//! A popup on a server-decorated window must be clickable where it is drawn.
//!
//! Otto draws the titlebar itself and starts the client's surface tree below
//! it, so a popup anchored to the client's window geometry belongs at
//! `frame_top + titlebar + anchor`. The hit test has always resolved it there
//! (`WindowElement::surface_under` lifts the point by the bar); the scene
//! import used to place the layer at `frame_top + anchor`, so the popup
//! responded exactly one titlebar below where it was painted.
//!
//! The bar and the anchor are each snapped onto the physical pixel grid, the
//! same way the client's own content layer is, so on a fractional scale the
//! popup's logical origin is deliberately not a whole number of points. The
//! expectation below is derived with that same rounding: the headless output
//! takes its scale from the effective `screen_scale` setting, so hard-coding
//! `titlebar + anchor` only holds at an integer scale.

#[cfg(feature = "headless")]
mod ssd_popup_hittest_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::time::Duration;

    const TITLE: &str = "ssd-popup-window";
    const POPUP_W: u32 = 200;
    const POPUP_H: u32 = 150;
    /// `WindowElement::DECORATION_HEIGHT`, in logical points.
    const BAR: f64 = 34.0;

    #[test]
    #[serial]
    fn a_popup_on_a_decorated_window_is_hit_where_it_is_drawn() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client =
            TestClient::connect(&handle.socket_name).expect("Failed to connect to compositor");

        let toplevel = client.create_toplevel(TITLE, 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(300);
        handle.decorate_window(TITLE);
        handle.settle(300);
        handle.focus_window(TITLE);
        handle.settle(200);
        // Park it: placement is randomised, and a window low on the output
        // would put the popup's bottom edge past it, where there is nothing to
        // hit-test against.
        handle.move_window(TITLE, 120, 80);
        handle.settle(200);

        // Anchored well inside the client area, so neither the titlebar strip
        // nor the resize border can claim the points tested below.
        let (anchor_x, anchor_y) = (80, 120);
        let _popup = client.create_popup(&toplevel, anchor_x, anchor_y, POPUP_W, POPUP_H);
        let _ = client.roundtrip();
        handle.settle(400);
        let _ = client.roundtrip();
        handle.settle(400);

        let (wx, wy, _ww, _wh) = handle
            .window_logical_geometry(TITLE)
            .expect("the window is mapped");
        let scale = handle.output_scale();
        let rects = handle.popup_logical_rects();
        eprintln!("window=({wx},{wy}) scale={scale} popup rects={rects:?}");
        assert_eq!(rects.len(), 1, "exactly one popup is on screen");
        let (px, py) = (rects[0].0, rects[0].1);

        // Where the popup is painted: the anchor is measured from the client's
        // window geometry, which starts one titlebar below the frame. Rebuild
        // the expectation through the same three roundings the compositor
        // applies, in the same order — see the module comment.
        //
        //   1. the popup's offset inside its parent, physical, rounded
        //      (`to_physical_precise_round`),
        //   2. the titlebar's height, physical, rounded (`snap_extent_px`),
        //   3. the whole resulting origin, rounded onto the pixel grid
        //      (`snap_position_px`), which is what actually reaches the layer.
        //
        // The window's own origin is NOT rounded before that last step, so at
        // a fractional scale the answer is not simply the sum of the parts.
        let to_px = |points: f64| (points * scale).round();
        // The anchor rect is 1 point tall and the gravity is down, so the
        // popup's own top edge is one point below the anchor's y.
        let expected_px = (wx as f64 * scale + to_px(anchor_x as f64)).round() / scale;
        let expected_py =
            (wy as f64 * scale + to_px(BAR) + to_px(anchor_y as f64 + 1.0)).round() / scale;
        assert!(
            (px as f64 - expected_px).abs() < 0.01,
            "the popup is drawn at the anchor's x: got {px}, expected {expected_px}"
        );
        assert!(
            (py as f64 - expected_py).abs() < 0.01,
            "the popup is drawn one titlebar below the frame, at the anchor's y: \
             got {py}, expected {expected_py} (scale {scale})"
        );

        // And the pointer agrees with the paint, along the whole popup.
        let cx = (px + POPUP_W as f32 / 2.0) as f64;
        let bottom = py + POPUP_H as f32;
        assert!(
            handle.point_hits_popup(cx, (py + 3.0) as f64),
            "the top edge of the drawn popup responds to the pointer"
        );
        assert!(
            handle.point_hits_popup(cx, (py + POPUP_H as f32 / 2.0) as f64),
            "the middle of the drawn popup responds to the pointer"
        );
        assert!(
            handle.point_hits_popup(cx, (bottom - 3.0) as f64),
            "the bottom edge of the drawn popup responds to the pointer"
        );
        assert!(
            !handle.point_hits_popup(cx, (bottom + 8.0) as f64),
            "the hit area does not run past the paint"
        );

        handle.stop();
    }
}
