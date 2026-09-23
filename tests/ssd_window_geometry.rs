//! A server-decorated window is hit where it is drawn, whatever window
//! geometry its client last set.
//!
//! winit, without `sctk-adwaita`, builds its fallback frame before it learns
//! that the compositor decorates: subsurfaces around the content and a window
//! geometry of `(-4, -28, w + 8, h + 28)` that takes them in. When the
//! server-side mode arrives it drops the frame and never sets the geometry
//! again. The pointer then has to land on the client's surface where Otto
//! paints it.

#[cfg(feature = "headless")]
mod ssd_window_geometry_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::time::Duration;

    const TITLE: &str = "ssd-geometry-window";
    const W: i32 = 640;
    const H: i32 = 480;
    /// winit's fallback frame: a 4 pt border and a 24 pt bar above it.
    const BORDER: i32 = 4;
    const FRAME_TOP: i32 = 28;

    /// Map a decorated window, give it winit's frame geometry, and drop the
    /// frame again. With `commit_frame_first`, the frame is committed before
    /// it is dropped; otherwise it never reaches a commit.
    fn run(commit_frame_first: bool) {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client =
            TestClient::connect(&handle.socket_name).expect("Failed to connect to compositor");

        let toplevel = client.create_toplevel(TITLE, W as u32, H as u32);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(300);
        handle.decorate_window(TITLE);
        handle.settle(200);
        handle.move_window(TITLE, 120, 80);
        handle.settle(200);

        let (surface, xdg_surface) = {
            let t = toplevel.lock().unwrap();
            (
                t.surface.clone(),
                t.xdg_surface.clone().expect("xdg_surface"),
            )
        };

        // The frame: a bar above the content and a border down each side.
        let frame = [
            (
                -BORDER,
                -FRAME_TOP,
                (W + 2 * BORDER) as u32,
                (FRAME_TOP) as u32,
            ),
            (-BORDER, 0, BORDER as u32, H as u32),
            (W, 0, BORDER as u32, H as u32),
            (-BORDER, H, (W + 2 * BORDER) as u32, BORDER as u32),
        ];
        xdg_surface.set_window_geometry(-BORDER, -FRAME_TOP, W + 2 * BORDER, H + FRAME_TOP);
        let mut parts = Vec::new();
        if commit_frame_first {
            for (x, y, w, h) in frame {
                parts.push(client.create_subsurface(&surface, x, y, w, h));
            }
            toplevel.lock().unwrap().commit_frame();
            let _ = client.roundtrip();
            handle.settle(200);
        }
        for (s, sub, _) in parts.drain(..) {
            sub.destroy();
            s.destroy();
        }
        toplevel.lock().unwrap().commit_frame();
        let _ = client.roundtrip();
        handle.settle(300);
        let _ = client.roundtrip();
        handle.settle(300);

        let (drawn_x, drawn_y) = handle
            .window_surface_logical_origin(TITLE)
            .expect("the surface has a layer");
        // Well inside the drawn content, clear of the bar and the resize edges.
        let (px, py) = (drawn_x + W as f64 / 2.0, drawn_y + H as f64 / 2.0);
        let (hit_x, hit_y) = handle
            .surface_origin_under(px, py)
            .expect("the middle of the drawn content hits a surface");
        eprintln!(
            "frame committed: {commit_frame_first}; window={:?} drawn=({drawn_x},{drawn_y}) hit=({hit_x},{hit_y})",
            handle.window_logical_geometry(TITLE)
        );
        assert!(
            (drawn_x - hit_x).abs() < 0.5 && (drawn_y - hit_y).abs() < 0.5,
            "the surface is drawn at ({drawn_x}, {drawn_y}) but the pointer maps it to \
             ({hit_x}, {hit_y})"
        );

        handle.stop();
    }

    #[test]
    #[serial]
    fn stale_frame_geometry_never_committed_with_a_frame() {
        run(false);
    }

    #[test]
    #[serial]
    fn stale_frame_geometry_after_the_frame_is_dropped() {
        run(true);
    }
}
