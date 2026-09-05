//! What a client gets when it names a cursor instead of drawing one.
//!
//! A client that binds `wp_cursor_shape_manager_v1` never uploads a pointer
//! bitmap: it names a shape and the compositor draws it from the configured
//! theme. That makes the theme and the size of every such cursor Otto's
//! responsibility, and both have gone wrong.
//!
//! Cursor themes name the same icon two ways — a modern name (`default`,
//! `pointer`) and a legacy X11 one (`left_ptr`, `hand2`) — and a theme with no
//! `Inherits=` still falls back to `default`, usually Adwaita. Resolving the
//! modern name to exhaustion first found the icon in the *fallback* theme and
//! never tried the legacy name in the configured one, so the desktop drew the
//! fallback theme's art almost everywhere and the user's own theme only where
//! the two names coincide (`move`). Separately, a theme that ships a single
//! small bitmap drew a pointer a fraction of `cursor_size` on a HiDPI screen,
//! because the nearest available art was emitted unscaled.
//!
//! The two failures need separate assertions. Size alone cannot catch the
//! lookup bug — rescaled art from the wrong theme is the right size and still
//! the wrong cursor — so these tests check the pixels too, against themes they
//! plant themselves rather than whatever the machine has installed.

#[cfg(feature = "headless")]
mod cursor_shape_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::path::{Path, PathBuf};
    use std::time::Duration;
    use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::Shape;

    /// Art in the theme the user configured. Every cursor should be this
    /// colour; any other means Otto went to a different theme for it.
    const CONFIGURED: [u8; 4] = [10, 200, 30, 255];
    /// Art in the theme the configured one implicitly falls back to.
    const FALLBACK: [u8; 4] = [200, 10, 30, 255];

    /// The configured theme ships one small bitmap, as several legacy X11
    /// themes do, so the resampling has real work to do.
    const CONFIGURED_ART_PX: u32 = 32;
    /// The fallback theme's art is a different size again, so art taken from
    /// the wrong theme is wrong in both colour and (unresampled) size.
    const FALLBACK_ART_PX: u32 = 48;

    /// The cursor size to configure, in logical pixels. Deliberately unequal
    /// to either theme's art, so the right answer is never one Otto could
    /// reach by emitting a bitmap unscaled.
    const CURSOR_SIZE: u32 = 40;

    /// Encode a single-frame xcursor file: one `size`×`size` image filled with
    /// `fill`, in the RGBA byte order the parser hands back.
    fn write_cursor(path: &Path, size: u32, fill: [u8; 4]) {
        let mut out = Vec::new();
        out.extend_from_slice(b"Xcur");
        out.extend_from_slice(&16u32.to_le_bytes()); // header length
        out.extend_from_slice(&0x0001_0000u32.to_le_bytes()); // file version
        out.extend_from_slice(&1u32.to_le_bytes()); // one table entry

        out.extend_from_slice(&0xfffd_0002u32.to_le_bytes()); // image chunk
        out.extend_from_slice(&size.to_le_bytes()); // nominal size
        out.extend_from_slice(&28u32.to_le_bytes()); // its position

        for word in [0x24u32, 0xfffd_0002, size, 1, size, size, 0, 0, 0] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        for _ in 0..(size * size) {
            out.extend_from_slice(&fill);
        }

        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, out).unwrap();
    }

    /// Plant two themes and point `XCURSOR_PATH` at them.
    ///
    /// `legacy` has no `index.theme`, so it implicitly inherits `default` —
    /// and it carries its icons only under the old X11 names, exactly the
    /// shape of theme that used to lose every modern name to the fallback.
    fn plant_themes() -> PathBuf {
        let root = std::env::temp_dir().join("otto-cursor-shape-themes");
        let _ = std::fs::remove_dir_all(&root);

        for name in ["left_ptr", "hand2", "xterm", "watch", "move"] {
            write_cursor(
                &root.join("legacy/cursors").join(name),
                CONFIGURED_ART_PX,
                CONFIGURED,
            );
        }
        for name in ["default", "pointer", "text", "wait", "move"] {
            write_cursor(
                &root.join("default/cursors").join(name),
                FALLBACK_ART_PX,
                FALLBACK,
            );
        }

        std::env::set_var("XCURSOR_PATH", &root);
        root
    }

    /// A mapped window with the pointer resting inside it. The pointer has to
    /// be over one of the client's surfaces: the `wl_pointer.enter` serial is
    /// what authorises naming a shape.
    fn window_with_pointer(handle: &HeadlessHandle, client: &mut TestClient) {
        let toplevel = client.create_toplevel("cursor-shape", 400, 300);
        handle.wait(Duration::from_millis(120));
        client.roundtrip().expect("roundtrip");
        toplevel.lock().unwrap().commit_frame();
        client.roundtrip().expect("roundtrip");
        handle.wait(Duration::from_millis(120));

        let (x, y, w, h) = handle
            .window_logical_geometry("cursor-shape")
            .expect("the window is mapped");
        handle.pointer_move(x as f64 + w as f64 / 2.0, y as f64 + h as f64 / 2.0);
        handle.wait(Duration::from_millis(80));
        client.roundtrip().expect("roundtrip");
    }

    /// Name `shape`, let the compositor settle, and report what it drew.
    fn shape_render_state(
        handle: &HeadlessHandle,
        client: &mut TestClient,
        shape: Shape,
    ) -> (String, u32, u32, [u8; 4]) {
        assert!(
            client.set_cursor_shape(shape),
            "the compositor should advertise wp_cursor_shape_manager_v1 and the \
             pointer should have entered the window"
        );
        client.roundtrip().expect("roundtrip");
        handle.wait(Duration::from_millis(80));
        handle.cursor_render_state()
    }

    /// Start a compositor already using the planted legacy theme, with a
    /// client whose pointer is inside its window. Returns the size in physical
    /// pixels every cursor should come out at.
    fn setup() -> (HeadlessHandle, TestClient, u32) {
        plant_themes();
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client = TestClient::connect(&handle.socket_name).expect("connect");
        window_with_pointer(&handle, &mut client);
        let expected = handle.use_cursor_theme("legacy", CURSOR_SIZE);
        (handle, client, expected)
    }

    /// A named shape must be drawn by the compositor rather than handed back
    /// as a client bitmap, and it must come out at the configured size — not
    /// at whatever size the theme happens to ship art for.
    #[test]
    #[serial]
    fn a_named_shape_is_drawn_at_the_configured_size() {
        let (handle, mut client, expected) = setup();

        let (kind, w, h, _) = shape_render_state(&handle, &mut client, Shape::Pointer);

        assert!(
            kind.starts_with("named:"),
            "a cursor-shape request should resolve to a compositor-drawn cursor, got {kind}"
        );
        assert_ne!(
            expected, CONFIGURED_ART_PX,
            "the test is pointless unless the requested size differs from the art"
        );
        assert_eq!(
            (w, h),
            (expected, expected),
            "{kind} came out {w}x{h}; a theme shipping only {CONFIGURED_ART_PX}px art \
             should still be drawn at cursor_size * scale = {expected}px"
        );

        handle.stop();
    }

    /// The bug this guards: `pointer` resolved in the fallback theme while
    /// `move` resolved in the configured one, so the pointer changed theme
    /// mid-gesture. Every shape must come out of the theme the user chose.
    #[test]
    #[serial]
    fn every_named_shape_comes_from_the_configured_theme() {
        let (handle, mut client, expected) = setup();

        // `default`, `pointer`, `text` and `wait` exist in both planted themes
        // — under their modern names in the fallback and their legacy names in
        // the configured one, which is what used to split the cursor across
        // two themes. `move` is spelled the same in both and never split.
        for shape in [
            Shape::Default,
            Shape::Pointer,
            Shape::Text,
            Shape::Wait,
            Shape::Move,
        ] {
            let (kind, w, h, pixel) = shape_render_state(&handle, &mut client, shape);

            assert_eq!(
                pixel, CONFIGURED,
                "{kind} was drawn from the fallback theme, not the configured one \
                 (pixel {pixel:?}, expected {CONFIGURED:?})"
            );
            assert_eq!(
                (w, h),
                (expected, expected),
                "{kind} came out {w}x{h}, but every named cursor should be {expected}px"
            );
        }

        handle.stop();
    }
}

/// Whether a client's own cursor bitmap survives the pointer moving inside
/// the window it set the cursor for.
///
/// A client that does not use the cursor-shape protocol uploads a bitmap and
/// expects it to stay up until the pointer leaves. The compositor resets to
/// its default cursor on every pointer-focus change, which is correct — but
/// only if focus is actually stable while the pointer stays over one surface.
/// Otto was seen resetting to `default` eight times for every cursor the
/// client set, over a window the pointer never left, which leaves the client's
/// cursor mostly not drawn at all.
#[cfg(feature = "headless")]
mod client_cursor_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::time::Duration;

    /// The client's cursor bitmap, sized so it cannot be confused with
    /// anything Otto would draw from a theme.
    const CURSOR_PX: u32 = 48;

    #[test]
    #[serial]
    fn a_client_cursor_survives_the_pointer_moving_inside_its_window() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client = TestClient::connect(&handle.socket_name).expect("connect");

        let toplevel = client.create_toplevel("client-cursor", 400, 300);
        handle.wait(Duration::from_millis(120));
        client.roundtrip().expect("roundtrip");
        toplevel.lock().unwrap().commit_frame();
        client.roundtrip().expect("roundtrip");
        handle.wait(Duration::from_millis(120));

        let (x, y, w, h) = handle
            .window_logical_geometry("client-cursor")
            .expect("the window is mapped");
        let (cx, cy) = (x as f64 + w as f64 / 2.0, y as f64 + h as f64 / 2.0);

        handle.pointer_move(cx, cy);
        handle.wait(Duration::from_millis(80));
        client.roundtrip().expect("roundtrip");

        let cursor = client
            .set_cursor_surface(CURSOR_PX, CURSOR_PX)
            .expect("the pointer should have entered the window");
        client.roundtrip().expect("roundtrip");
        handle.wait(Duration::from_millis(80));

        let (kind, ..) = handle.cursor_render_state();
        assert_eq!(
            kind, "surface",
            "the compositor should be drawing the client's own bitmap right after it set one"
        );

        // Walk the pointer around well inside the window. Every one of these
        // stays over the same surface, so none of them is a focus change and
        // none should take the client's cursor away.
        let mut stomped = Vec::new();
        for (i, (dx, dy)) in [(6.0, 0.0), (0.0, 6.0), (-6.0, 0.0), (0.0, -6.0)]
            .into_iter()
            .cycle()
            .take(12)
            .enumerate()
        {
            handle.pointer_move(cx + dx, cy + dy);
            handle.wait(Duration::from_millis(30));
            client.roundtrip().expect("roundtrip");

            let (kind, ..) = handle.cursor_render_state();
            if kind != "surface" {
                stomped.push(format!("move {i}: {kind}"));
            }
        }

        // Drop the client before stopping: the compositor holds the cursor
        // surface as its current cursor image, and tearing the compositor down
        // underneath a live client wedges the shutdown.
        drop(cursor);
        drop(client);
        handle.wait(Duration::from_millis(80));

        assert!(
            stomped.is_empty(),
            "the client's cursor was replaced by one of Otto's while the pointer \
             stayed inside its window: {stomped:?}"
        );

        handle.stop();
    }
}
