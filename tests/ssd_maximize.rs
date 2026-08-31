//! Server-side decoration sizing across tiling and maximize.
//!
//! Unlike `tests/tiling.rs`, the client here RESIZES: it attaches a buffer of
//! the size it was last configured with, the way a real application does. The
//! titlebar Otto draws is sized from the window's geometry, so a client that
//! never resizes can never show the bar getting out of sync with it.

#[cfg(feature = "headless")]
mod ssd_maximize_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle, TileZone};
    use otto_kit::testing::{ShmBuffer, TestClient, TestToplevel};
    use serial_test::serial;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    const TITLE: &str = "ssd-window";

    /// Attach a buffer of the size the compositor last configured, so the
    /// window's geometry follows the configure the way a real client's does.
    fn resize_to_configured(
        client: &mut TestClient,
        toplevel: &Arc<Mutex<TestToplevel>>,
        keep: &mut Vec<ShmBuffer>,
    ) {
        let _ = client.roundtrip();
        let (w, h, surface) = {
            let tl = toplevel.lock().unwrap();
            (tl.width, tl.height, tl.surface.clone())
        };
        if w <= 0 || h <= 0 {
            return;
        }
        let buffer = ShmBuffer::new(
            client.state.wl_shm.as_ref().expect("wl_shm"),
            &client.qh,
            w as u32,
            h as u32,
        );
        surface.attach(Some(buffer.buffer()), 0, 0);
        surface.damage(0, 0, w, h);
        surface.commit();
        keep.push(buffer);
        let _ = client.roundtrip();
    }

    #[test]
    #[serial]
    fn maximizing_a_tiled_window_resizes_the_titlebar() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client =
            TestClient::connect(&handle.socket_name).expect("Failed to connect to compositor");
        let mut buffers = Vec::new();

        let toplevel = client.create_toplevel(TITLE, 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(300);
        handle.decorate_window(TITLE);
        handle.settle(200);
        handle.focus_window(TITLE);

        let (zx, _zy, zw, _zh) = handle.usable_zone();
        eprintln!("usable zone x={zx} w={zw}");

        // ── Tile left ────────────────────────────────────────────────────
        handle.tile_focused(TileZone::LeftHalf);
        handle.settle(600);
        resize_to_configured(&mut client, &toplevel, &mut buffers);
        handle.settle(300);

        let tiled_geo = handle.window_logical_geometry(TITLE).expect("mapped");
        let tiled_bar = handle.window_decoration_width(TITLE).expect("decorated");
        eprintln!("tiled geometry={tiled_geo:?} bar={tiled_bar}");
        assert_eq!(
            tiled_geo.2,
            zw / 2,
            "a left tile fills half the usable width"
        );
        assert_eq!(
            tiled_bar, tiled_geo.2 as f32,
            "the titlebar spans the tiled window"
        );

        // ── Maximize on top of the tile ──────────────────────────────────
        handle.toggle_maximize_focused();
        handle.settle(600);
        resize_to_configured(&mut client, &toplevel, &mut buffers);
        handle.settle(300);

        let max_geo = handle.window_logical_geometry(TITLE).expect("mapped");
        let max_bar = handle.window_decoration_width(TITLE).expect("decorated");
        eprintln!("maximized geometry={max_geo:?} bar={max_bar}");
        assert!(
            handle.window_is_maximized(TITLE),
            "the window should be maximized"
        );
        assert_eq!(max_geo.2, zw, "a maximized window fills the usable width");
        assert_eq!(
            max_bar, max_geo.2 as f32,
            "the titlebar follows the window when it is maximized out of a tile"
        );

        handle.stop();
    }

    /// Same sequence, with the window promoted to a KMS plane — the state a
    /// maximized window is normally in on the DRM backend. The commit path
    /// skips the scene import for a promoted window, so the titlebar has to be
    /// resized by the chrome refresh that runs instead.
    #[test]
    #[serial]
    fn maximizing_a_scanned_out_tiled_window_resizes_the_titlebar() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client =
            TestClient::connect(&handle.socket_name).expect("Failed to connect to compositor");
        let mut buffers = Vec::new();

        let toplevel = client.create_toplevel(TITLE, 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(300);
        handle.decorate_window(TITLE);
        handle.settle(200);
        handle.focus_window(TITLE);

        let (_zx, _zy, zw, _zh) = handle.usable_zone();

        handle.tile_focused(TileZone::LeftHalf);
        handle.settle(600);
        resize_to_configured(&mut client, &toplevel, &mut buffers);
        handle.settle(300);

        // Promoted only now: a window is promoted once it is stable at a size,
        // which is where the user finds it before reaching for maximize.
        handle.set_window_scanned_out(TITLE, true);

        handle.toggle_maximize_focused();
        handle.settle(600);
        resize_to_configured(&mut client, &toplevel, &mut buffers);
        handle.settle(300);

        let geo = handle.window_logical_geometry(TITLE).expect("mapped");
        let bar = handle.window_decoration_width(TITLE).expect("decorated");
        assert_eq!(geo.2, zw, "a maximized window fills the usable width");
        assert_eq!(
            bar, geo.2 as f32,
            "the titlebar of a promoted window follows it out of the tile"
        );

        handle.stop();
    }
}
