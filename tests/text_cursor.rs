//! `otto-text-cursor-v1` end to end: an application reports where its caret
//! is, and a second, unrelated client is told where that is on screen.
//!
//! This exercises a chain that spans two repositories — smithay retaining the
//! rectangle instead of handing it only to an input method, Otto turning a
//! surface-local rectangle into a position on the output, and the protocol
//! delivering it — so it is worth testing as one thing rather than three.

#[cfg(feature = "headless")]
mod headless_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::time::Duration;
    use wayland_client::protocol::wl_registry;
    use wayland_client::{delegate_noop, Connection, Dispatch, QueueHandle};

    // The watcher's side of the protocol, generated here rather than pulled
    // from otto-kit: a test should speak the wire format the XML describes,
    // not the client library's opinion of it.
    #[allow(non_snake_case)]
    mod protocol {
        // The generated code refers to `wayland_client` by path, so it has to
        // be nameable here — not redundant, whatever clippy makes of it.
        #[allow(clippy::single_component_path_imports)]
        use wayland_client;
        pub use wayland_client::protocol::{__interfaces::*, wl_seat};
        wayland_scanner::generate_interfaces!("./protocols/otto-text-cursor-v1.xml");
        wayland_scanner::generate_client_code!("./protocols/otto-text-cursor-v1.xml");
    }
    use protocol::{otto_text_cursor_manager_v1, otto_text_cursor_v1};

    /// What the compositor last told the watcher.
    #[derive(Default)]
    struct Watcher {
        seat: Option<wayland_client::protocol::wl_seat::WlSeat>,
        manager: Option<otto_text_cursor_manager_v1::OttoTextCursorManagerV1>,
        /// Every answer received, in order.
        answers: Vec<Option<(i32, i32, i32, i32)>>,
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for Watcher {
        fn event(
            state: &mut Self,
            registry: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            _: &(),
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global {
                name,
                interface,
                version,
            } = event
            {
                match interface.as_str() {
                    "wl_seat" if state.seat.is_none() => {
                        state.seat = Some(registry.bind(name, version.min(7), qh, ()));
                    }
                    "otto_text_cursor_manager_v1" => {
                        state.manager = Some(registry.bind(name, 1, qh, ()));
                    }
                    _ => {}
                }
            }
        }
    }

    impl Dispatch<otto_text_cursor_v1::OttoTextCursorV1, ()> for Watcher {
        fn event(
            state: &mut Self,
            _proxy: &otto_text_cursor_v1::OttoTextCursorV1,
            event: otto_text_cursor_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            state.answers.push(match event {
                otto_text_cursor_v1::Event::Position {
                    x,
                    y,
                    width,
                    height,
                } => Some((x, y, width, height)),
                otto_text_cursor_v1::Event::Unavailable => None,
            });
        }
    }

    delegate_noop!(Watcher: ignore wayland_client::protocol::wl_seat::WlSeat);
    delegate_noop!(Watcher: otto_text_cursor_manager_v1::OttoTextCursorManagerV1);

    /// Connect a watcher and take its first answer.
    fn watch(
        socket: &str,
    ) -> (
        Connection,
        wayland_client::EventQueue<Watcher>,
        Watcher,
        otto_text_cursor_v1::OttoTextCursorV1,
    ) {
        let connection = Connection::connect_to_env().expect("watcher connection");
        let _ = socket;
        let mut queue = connection.new_event_queue();
        let qh = queue.handle();
        connection.display().get_registry(&qh, ());
        let mut watcher = Watcher::default();
        queue.roundtrip(&mut watcher).expect("bind globals");

        let manager = watcher
            .manager
            .clone()
            .expect("the compositor offers otto_text_cursor_manager_v1");
        let seat = watcher.seat.clone().expect("a seat");
        let cursor = manager.get_text_cursor(&seat, &qh, ());
        queue.roundtrip(&mut watcher).expect("first answer");
        (connection, queue, watcher, cursor)
    }

    #[test]
    #[serial]
    fn a_reported_caret_reaches_an_unrelated_client() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        std::env::set_var("WAYLAND_DISPLAY", &handle.socket_name);

        // The application: a window, and a text field in it that says where
        // its caret is.
        let mut app = TestClient::connect(&handle.socket_name).expect("connect");
        let _window = app.create_toplevel("caret-owner", 640, 480);
        handle.wait(Duration::from_millis(150));
        app.roundtrip().expect("roundtrip");

        // A watcher, before any caret exists: the honest answer is that
        // nobody has said.
        let (_conn, mut queue, mut watcher, _cursor) = watch(&handle.socket_name);
        assert_eq!(
            watcher.answers.first(),
            Some(&None),
            "with no caret reported the compositor should say so, not stay silent"
        );

        // Now the application reports one.
        let caret = (40, 60, 2, 18);
        let bound = app.set_text_cursor(caret.0, caret.1, caret.2, caret.3);
        assert!(
            bound,
            "the compositor should offer zwp_text_input_manager_v3"
        );
        handle.wait(Duration::from_millis(150));
        app.roundtrip().expect("roundtrip");
        queue.roundtrip(&mut watcher).expect("watcher update");

        let position = watcher
            .answers
            .iter()
            .rev()
            .find_map(|answer| *answer)
            .expect("the caret should have reached the watcher");
        assert_eq!(
            (position.2, position.3),
            (caret.2, caret.3),
            "the caret keeps its size; only its origin is moved into layout space"
        );
        // The rectangle is reported relative to the surface, so what comes out
        // is offset by wherever the compositor put that window — never the raw
        // surface-local value unless the window happens to sit at the origin.
        assert!(
            position.0 >= caret.0 && position.1 >= caret.1,
            "expected the caret offset onto the output, got {position:?}"
        );

        handle.stop();
    }

    /// Chromium, GTK and every other client that follows the protocol to the
    /// letter say nothing about their caret until the compositor has sent
    /// `enter`, and hold every later request until the `done` for their last
    /// commit arrives. Both used to depend on an input method being present;
    /// with none, Chromium reported one placeholder rectangle and then went
    /// quiet for good.
    #[test]
    #[serial]
    fn a_strict_client_is_told_enter_and_done_without_an_ime() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        std::env::set_var("WAYLAND_DISPLAY", &handle.socket_name);

        let mut app = TestClient::connect(&handle.socket_name).expect("connect");
        let _window = app.create_toplevel("strict-caret-owner", 640, 480);
        handle.wait(Duration::from_millis(150));
        app.roundtrip().expect("roundtrip");

        // Enables (one commit) and reports a caret (a second commit).
        assert!(app.set_text_cursor(40, 60, 2, 18));
        handle.wait(Duration::from_millis(150));
        app.roundtrip().expect("roundtrip");

        assert!(
            app.state.text_input_entered,
            "the focused text input should be sent enter even with no input method running"
        );
        assert_eq!(
            app.state.text_input_done_serials,
            vec![1, 2],
            "every commit should be answered with done carrying the commit count"
        );

        handle.stop();
    }
}
