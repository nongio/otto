//! The compositor-side contract the RDP bridge runs on (headless).
//!
//! `otto-rdp` is a separate process: it captures an output through
//! `org.otto.ScreenCast` and injects the remote client's input back through
//! `zwlr_virtual_pointer_v1` and `zwp_virtual_keyboard_v1`, sizing its picture
//! from `xdg-output`'s logical geometry. The RDP wire protocol itself needs a
//! real client (and a VA-API encoder) and is out of reach here, but everything
//! the bridge asks of Otto is not: these tests bind the same globals it binds,
//! in the same versions, and drive the same requests.
//!
//! See `specs/rdp-bridge.md` and `components/otto-rdp/src/wl_input.rs`.

#[cfg(feature = "headless")]
mod rdp_bridge_tests {
    use std::io::Write;
    use std::os::fd::AsFd;
    use std::time::Duration;

    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use wayland_client::{
        delegate_noop,
        protocol::{wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat},
        Connection, Dispatch, EventQueue, QueueHandle, WEnum,
    };
    use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};
    use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
        zwp_virtual_keyboard_manager_v1, zwp_virtual_keyboard_v1,
    };
    use wayland_protocols_wlr::virtual_pointer::v1::client::{
        zwlr_virtual_pointer_manager_v1, zwlr_virtual_pointer_v1,
    };

    /// BTN_LEFT, as `otto-rdp` sends it.
    const BTN_LEFT: u32 = 0x110;
    /// evdev KEY_A.
    const KEY_A: u32 = 30;

    // ── A stand-in for otto-rdp's injection client ───────────────────────

    #[derive(Default)]
    struct InjectorState {
        seat: Option<wl_seat::WlSeat>,
        pointer_manager: Option<zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1>,
        pointer_manager_version: u32,
        keyboard_manager: Option<zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1>,
        output_manager: Option<zxdg_output_manager_v1::ZxdgOutputManagerV1>,
        output_manager_version: u32,
        output: Option<wl_output::WlOutput>,
        /// Mode reported by `wl_output`, in physical pixels.
        mode: Option<(i32, i32)>,
        /// Logical geometry reported by `xdg-output` — the space the bridge
        /// maps remote pointer coordinates into.
        logical_size: Option<(i32, i32)>,
        logical_position: Option<(i32, i32)>,
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for InjectorState {
        fn event(
            state: &mut Self,
            registry: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            _: &(),
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            let wl_registry::Event::Global {
                name,
                interface,
                version,
            } = event
            else {
                return;
            };
            // Exactly what `otto-rdp`'s wl_input thread binds.
            match interface.as_str() {
                "wl_seat" => state.seat = Some(registry.bind(name, version.min(5), qh, ())),
                "zwlr_virtual_pointer_manager_v1" => {
                    state.pointer_manager_version = version;
                    state.pointer_manager = Some(registry.bind(name, version.min(2), qh, ()));
                }
                "zwp_virtual_keyboard_manager_v1" => {
                    state.keyboard_manager = Some(registry.bind(name, 1, qh, ()));
                }
                "zxdg_output_manager_v1" => {
                    state.output_manager_version = version;
                    state.output_manager = Some(registry.bind(name, version.min(3), qh, ()));
                }
                "wl_output" => {
                    state.output = Some(registry.bind(name, version.min(4), qh, ()));
                }
                _ => {}
            }
        }
    }

    impl Dispatch<wl_output::WlOutput, ()> for InjectorState {
        fn event(
            state: &mut Self,
            _: &wl_output::WlOutput,
            event: wl_output::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_output::Event::Mode {
                flags,
                width,
                height,
                ..
            } = event
            {
                if matches!(flags, WEnum::Value(f) if f.contains(wl_output::Mode::Current)) {
                    state.mode = Some((width, height));
                }
            }
        }
    }

    impl Dispatch<zxdg_output_v1::ZxdgOutputV1, ()> for InjectorState {
        fn event(
            state: &mut Self,
            _: &zxdg_output_v1::ZxdgOutputV1,
            event: zxdg_output_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                zxdg_output_v1::Event::LogicalSize { width, height } => {
                    state.logical_size = Some((width, height));
                }
                zxdg_output_v1::Event::LogicalPosition { x, y } => {
                    state.logical_position = Some((x, y));
                }
                _ => {}
            }
        }
    }

    delegate_noop!(InjectorState: ignore wl_seat::WlSeat);
    delegate_noop!(InjectorState: zxdg_output_manager_v1::ZxdgOutputManagerV1);
    delegate_noop!(InjectorState: zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1);
    delegate_noop!(InjectorState: zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1);
    delegate_noop!(InjectorState: zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1);
    delegate_noop!(InjectorState: zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1);

    struct Injector {
        conn: Connection,
        queue: EventQueue<InjectorState>,
        qh: QueueHandle<InjectorState>,
        state: InjectorState,
    }

    impl Injector {
        /// Connect and bind the bridge's globals.
        fn connect(handle: &HeadlessHandle) -> Self {
            let path = format!(
                "{}/{}",
                std::env::var("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR"),
                handle.socket_name
            );
            let stream = std::os::unix::net::UnixStream::connect(path).expect("connect");
            let conn = Connection::from_socket(stream).expect("wayland connection");
            let mut queue = conn.new_event_queue();
            let qh = queue.handle();
            conn.display().get_registry(&qh, ());

            let mut state = InjectorState::default();
            queue.roundtrip(&mut state).expect("bind globals");

            Self {
                conn,
                queue,
                qh,
                state,
            }
        }

        fn roundtrip(&mut self) {
            self.queue.roundtrip(&mut self.state).expect("roundtrip");
        }

        /// `xdg-output` for the bound output — the bridge reads its logical
        /// size to size the picture it serves.
        fn fetch_xdg_output(&mut self) {
            let manager = self
                .state
                .output_manager
                .clone()
                .expect("zxdg_output_manager_v1 missing");
            let output = self.state.output.clone().expect("wl_output missing");
            manager.get_xdg_output(&output, &self.qh, ());
            self.roundtrip();
            self.roundtrip();
        }

        /// A virtual pointer bound to the output, as the bridge creates it.
        fn pointer(&mut self) -> zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1 {
            let manager = self
                .state
                .pointer_manager
                .clone()
                .expect("zwlr_virtual_pointer_manager_v1 missing");
            let seat = self.state.seat.clone();
            let output = self.state.output.clone();
            manager.create_virtual_pointer_with_output(seat.as_ref(), output.as_ref(), &self.qh, ())
        }

        /// A virtual keyboard with the US keymap the bridge uploads.
        fn keyboard(&mut self) -> zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1 {
            let manager = self
                .state
                .keyboard_manager
                .clone()
                .expect("zwp_virtual_keyboard_manager_v1 missing");
            let seat = self.state.seat.clone().expect("wl_seat missing");
            let keyboard = manager.create_virtual_keyboard(&seat, &self.qh, ());

            use xkbcommon::xkb;
            let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
            let keymap = xkb::Keymap::new_from_names(
                &context,
                "",
                "",
                "us",
                "",
                None,
                xkb::COMPILE_NO_FLAGS,
            )
            .expect("compile us keymap");
            let text = keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);

            let mut file = tempfile::tempfile().expect("keymap file");
            file.write_all(text.as_bytes()).expect("write keymap");
            file.write_all(&[0]).expect("write NUL"); // keymaps are NUL-terminated
            file.flush().expect("flush keymap");

            keyboard.keymap(
                wl_keyboard::KeymapFormat::XkbV1 as u32,
                file.as_fd(),
                text.len() as u32 + 1,
            );
            self.roundtrip();
            keyboard
        }

        /// Flush the injected requests and let the compositor act on them.
        fn settle(&mut self, handle: &HeadlessHandle) {
            self.conn.flush().expect("flush");
            handle.wait(Duration::from_millis(120));
            self.roundtrip();
            handle.settle(200);
        }
    }

    fn start() -> HeadlessHandle {
        HeadlessHandle::start(HeadlessConfig::default())
    }

    fn map_window(handle: &HeadlessHandle, client: &mut TestClient, title: &str) {
        client.create_toplevel_with_app_id(title, &format!("org.otto.{title}"), 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(200);
    }

    // ── Globals and geometry ─────────────────────────────────────────────

    /// The bridge cannot start at all unless the compositor advertises these
    /// four globals, at versions high enough for the requests it makes:
    /// `create_virtual_pointer_with_output` needs virtual-pointer v2.
    #[test]
    #[serial]
    fn the_globals_the_bridge_binds_are_advertised() {
        let handle = start();
        let injector = Injector::connect(&handle);

        assert!(injector.state.seat.is_some(), "wl_seat");
        assert!(injector.state.output.is_some(), "wl_output");
        assert!(
            injector.state.pointer_manager.is_some(),
            "zwlr_virtual_pointer_manager_v1"
        );
        assert!(
            injector.state.pointer_manager_version >= 2,
            "virtual pointer v2 is required for create_virtual_pointer_with_output, got v{}",
            injector.state.pointer_manager_version
        );
        assert!(
            injector.state.keyboard_manager.is_some(),
            "zwp_virtual_keyboard_manager_v1"
        );
        assert!(
            injector.state.output_manager.is_some(),
            "zxdg_output_manager_v1"
        );
        assert!(
            injector.state.output_manager_version >= 3,
            "xdg-output v3, got v{}",
            injector.state.output_manager_version
        );

        drop(injector);
        handle.stop();
    }

    /// `xdg-output` reports the output in *logical* pixels — physical mode
    /// divided by the output scale. The bridge maps remote pointer coordinates
    /// into this space, so a mix-up here puts the remote cursor at the wrong
    /// place on a scaled output.
    #[test]
    #[serial]
    fn xdg_output_reports_logical_geometry() {
        let handle = start();
        let mut injector = Injector::connect(&handle);
        injector.fetch_xdg_output();

        let scale = handle.query(|state| {
            state
                .workspaces
                .outputs()
                .next()
                .map(|o| o.current_scale().fractional_scale())
                .unwrap_or(1.0)
        });

        let mode = injector.state.mode.expect("wl_output mode");
        let logical = injector
            .state
            .logical_size
            .expect("xdg-output logical size");
        assert_eq!(mode, (1920, 1080), "physical mode");
        assert_eq!(
            logical,
            (
                (mode.0 as f64 / scale).round() as i32,
                (mode.1 as f64 / scale).round() as i32
            ),
            "logical size must be the mode divided by the output scale ({scale})"
        );
        assert_eq!(injector.state.logical_position, Some((0, 0)));

        drop(injector);
        handle.stop();
    }

    // ── Remote input ─────────────────────────────────────────────────────

    /// An absolute motion from the remote client lands where its normalized
    /// coordinates say, in the output's logical space.
    #[test]
    #[serial]
    fn remote_pointer_motion_moves_the_compositor_pointer() {
        let handle = start();
        let mut injector = Injector::connect(&handle);
        injector.fetch_xdg_output();
        let pointer = injector.pointer();

        let (logical_w, logical_h) = injector.state.logical_size.expect("logical size");

        // A quarter across, three quarters down — extents are the remote
        // client's own resolution, not the output's.
        pointer.motion_absolute(0, 500, 1500, 2000, 2000);
        pointer.frame();
        injector.settle(&handle);

        let (x, y) = handle.query(|state| state.last_pointer_location);
        assert_eq!(x, logical_w as f64 * 0.25);
        assert_eq!(y, logical_h as f64 * 0.75);

        drop(injector);
        handle.stop();
    }

    /// A remote click focuses the window under the pointer — the same
    /// click-to-focus a physical mouse gets — and the application it belongs to
    /// is told it has keyboard focus.
    #[test]
    #[serial]
    fn remote_click_focuses_the_window_under_the_pointer() {
        let handle = start();
        let mut background = TestClient::connect(&handle.socket_name).expect("client");
        let mut foreground = TestClient::connect(&handle.socket_name).expect("client");

        map_window(&handle, &mut background, "Background");
        map_window(&handle, &mut foreground, "Foreground");
        let _ = background.roundtrip();
        let _ = foreground.roundtrip();
        assert!(
            foreground.state.keyboard_focused,
            "the window mapped last starts focused"
        );

        // Click a corner of the background window that the foreground one does
        // not cover.
        let (bx, by, bw, bh) = handle
            .window_logical_geometry("Background")
            .expect("background window mapped");
        let (fx, fy, _, _) = handle
            .window_logical_geometry("Foreground")
            .expect("foreground window mapped");
        let (target_x, target_y) = (bx + 8, by + 8);
        assert!(
            target_x < fx || target_y < fy,
            "expected the background window ({bx},{by} {bw}x{bh}) to peek out from under \
             the foreground one at ({fx},{fy})"
        );

        let mut injector = Injector::connect(&handle);
        injector.fetch_xdg_output();
        let pointer = injector.pointer();
        let (logical_w, logical_h) = injector.state.logical_size.expect("logical size");

        pointer.motion_absolute(
            0,
            target_x as u32,
            target_y as u32,
            logical_w as u32,
            logical_h as u32,
        );
        pointer.frame();
        injector.settle(&handle);

        pointer.button(1, BTN_LEFT, wl_pointer::ButtonState::Pressed);
        pointer.frame();
        pointer.button(2, BTN_LEFT, wl_pointer::ButtonState::Released);
        pointer.frame();
        injector.settle(&handle);

        let _ = background.roundtrip();
        let _ = foreground.roundtrip();
        assert!(
            background.state.keyboard_focused,
            "the clicked window should have taken keyboard focus"
        );
        assert!(
            !foreground.state.keyboard_focused,
            "the previously focused window should have lost it"
        );

        drop(injector);
        handle.stop();
    }

    /// A remote keystroke reaches the focused application, carrying the evdev
    /// code the bridge translated the RDP scancode into.
    #[test]
    #[serial]
    fn remote_keystrokes_reach_the_focused_application() {
        let handle = start();
        let mut client = TestClient::connect(&handle.socket_name).expect("client");
        map_window(&handle, &mut client, "Remote");
        let _ = client.roundtrip();
        assert!(client.state.keyboard_focused, "window should be focused");

        let mut injector = Injector::connect(&handle);
        let keyboard = injector.keyboard();

        keyboard.key(0, KEY_A, 1);
        keyboard.key(10, KEY_A, 0);
        injector.settle(&handle);

        let _ = client.roundtrip();
        assert_eq!(
            client.state.keys,
            vec![(KEY_A, true), (KEY_A, false)],
            "the application should have seen the press and the release"
        );

        drop(injector);
        handle.stop();
    }
}
