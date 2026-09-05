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

    /// A GTK-style menu pads its buffer with a drop shadow and reports the
    /// menu itself as its window geometry. The anchor is measured against
    /// that geometry, and the scene already offsets every surface layer by
    /// its own `geometry.loc` (`window_view_for_surface`) — so subtracting the
    /// popup's geometry origin a second time when placing the popup's layer
    /// slid the whole menu up and to the left of the region that responds to
    /// the pointer, by exactly the shadow.
    #[test]
    #[serial]
    fn a_popup_with_a_shadow_is_hit_where_it_is_drawn() {
        const SHADOW: i32 = 20;

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
        handle.move_window(TITLE, 120, 80);
        handle.settle(200);

        let (anchor_x, anchor_y) = (80, 120);
        let _popup = client
            .create_popup_with_shadow(&toplevel, anchor_x, anchor_y, POPUP_W, POPUP_H, SHADOW);
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

        // The popup layer's origin is where the client's *window geometry*
        // starts — the menu, not the shadow around it — so the expectation is
        // the same as for a popup with no shadow at all. Same rounding chain
        // as the test above.
        let to_px = |points: f64| (points * scale).round();
        let expected_px = (wx as f64 * scale + to_px(anchor_x as f64)).round() / scale;
        let expected_py =
            (wy as f64 * scale + to_px(BAR) + to_px(anchor_y as f64 + 1.0)).round() / scale;
        assert!(
            (px as f64 - expected_px).abs() < 0.01,
            "the menu is drawn at the anchor's x, shadow excluded: got {px}, expected {expected_px}"
        );
        assert!(
            (py as f64 - expected_py).abs() < 0.01,
            "the menu is drawn at the anchor's y, shadow excluded: \
             got {py}, expected {expected_py} (scale {scale})"
        );

        // And the pointer agrees with the paint. The buffer — shadow included
        // — starts one shadow above and left of the menu; that whole rect is
        // the popup's surface, so it is what the hit test has to cover.
        let (bx, by) = (px - SHADOW as f32, py - SHADOW as f32);
        assert!(
            handle.point_hits_popup((bx + 3.0) as f64, (by + 3.0) as f64),
            "the top-left corner of the drawn popup responds to the pointer"
        );
        assert!(
            handle.point_hits_popup(
                (px + POPUP_W as f32 / 2.0 - SHADOW as f32) as f64,
                (py + POPUP_H as f32 / 2.0 - SHADOW as f32) as f64,
            ),
            "the middle of the drawn popup responds to the pointer"
        );
        assert!(
            !handle.point_hits_popup(
                (bx + POPUP_W as f32 + 8.0) as f64,
                (by + POPUP_H as f32 / 2.0) as f64,
            ),
            "the hit area does not run past the paint"
        );

        handle.stop();
    }

    /// A menu is free to hang off its parent window — that is most of what a
    /// context menu near an edge does. The part outside the window's own rect
    /// is still the popup, and it has to answer the pointer there.
    #[test]
    #[serial]
    fn a_popup_hanging_off_the_window_is_clickable_outside_it() {
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
        handle.move_window(TITLE, 120, 80);
        handle.settle(200);

        // Anchored near the right edge, so most of the popup hangs past it —
        // and far enough from the output's own edge that the positioner has no
        // reason to slide it back.
        let (anchor_x, anchor_y) = (600, 200);
        let _popup = client.create_popup(&toplevel, anchor_x, anchor_y, POPUP_W, POPUP_H);
        let _ = client.roundtrip();
        handle.settle(400);
        let _ = client.roundtrip();
        handle.settle(400);

        let (wx, wy, ww, wh) = handle
            .window_logical_geometry(TITLE)
            .expect("the window is mapped");
        let rects = handle.popup_logical_rects();
        eprintln!("window=({wx},{wy},{ww},{wh}) popup rects={rects:?}");
        assert_eq!(rects.len(), 1, "exactly one popup is on screen");
        let (px, py) = (rects[0].0, rects[0].1);
        let window_right = (wx + ww) as f32;
        assert!(
            px + POPUP_W as f32 > window_right,
            "the popup hangs off the window's right edge: popup x {px}, window right {window_right}"
        );

        let inside_y = (py + POPUP_H as f32 / 2.0) as f64;
        assert!(
            handle.point_hits_popup((px + 5.0) as f64, inside_y),
            "the part of the popup still over the window responds to the pointer"
        );
        assert!(
            handle.point_hits_popup((window_right + 10.0) as f64, inside_y),
            "the part of the popup hanging off the window responds to the pointer too"
        );
        assert!(
            handle.point_hits_popup((px + POPUP_W as f32 - 5.0) as f64, inside_y),
            "the popup's far edge, well outside the window, responds to the pointer"
        );

        handle.stop();
    }
}
