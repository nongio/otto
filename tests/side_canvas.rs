//! `otto-canvas-v1` end to end (headless): a client places an item in the
//! side canvas, and showing, resizing and dismissing the canvas reach it as
//! the protocol says they should.

#[cfg(feature = "headless")]
mod headless_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto::settings::value::SettingValue;
    use serial_test::serial;
    use std::time::Duration;
    use wayland_client::protocol::{wl_compositor, wl_registry, wl_surface};
    use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, QueueHandle};

    // The client's side of the protocol, generated from the XML rather than
    // taken from otto-kit, so the test speaks the wire format itself.
    #[allow(non_snake_case)]
    mod protocol {
        // The generated code names `wayland_client` by path.
        #[allow(clippy::single_component_path_imports)]
        use wayland_client;
        pub use wayland_client::protocol::{__interfaces::*, wl_surface};
        wayland_scanner::generate_interfaces!("./protocols/otto-canvas-v1.xml");
        wayland_scanner::generate_client_code!("./protocols/otto-canvas-v1.xml");
    }
    use protocol::{otto_canvas_item_v1, otto_canvas_manager_v1};

    /// Something the compositor sent the item.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Seen {
        Configure(u32),
        Shown,
        Hidden,
    }

    #[derive(Default)]
    struct Client {
        compositor: Option<wl_compositor::WlCompositor>,
        manager: Option<otto_canvas_manager_v1::OttoCanvasManagerV1>,
        events: Vec<Seen>,
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for Client {
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
                    "wl_compositor" => {
                        state.compositor = Some(registry.bind(name, version.min(4), qh, ()));
                    }
                    "otto_canvas_manager_v1" => {
                        state.manager = Some(registry.bind(name, 1, qh, ()));
                    }
                    _ => {}
                }
            }
        }
    }

    impl Dispatch<otto_canvas_item_v1::OttoCanvasItemV1, ()> for Client {
        fn event(
            state: &mut Self,
            item: &otto_canvas_item_v1::OttoCanvasItemV1,
            event: otto_canvas_item_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            state.events.push(match event {
                otto_canvas_item_v1::Event::Configure { serial, width } => {
                    item.ack_configure(serial);
                    Seen::Configure(width)
                }
                otto_canvas_item_v1::Event::Shown => Seen::Shown,
                otto_canvas_item_v1::Event::Hidden => Seen::Hidden,
            });
        }
    }

    delegate_noop!(Client: ignore wl_compositor::WlCompositor);
    delegate_noop!(Client: ignore wl_surface::WlSurface);
    delegate_noop!(Client: otto_canvas_manager_v1::OttoCanvasManagerV1);

    /// A connected client with one canvas item, and everything it has been
    /// told so far.
    struct Placed {
        _connection: Connection,
        queue: EventQueue<Client>,
        client: Client,
        item: otto_canvas_item_v1::OttoCanvasItemV1,
        _surface: wl_surface::WlSurface,
    }

    impl Placed {
        fn roundtrip(&mut self) {
            self.queue.roundtrip(&mut self.client).expect("roundtrip");
        }

        /// The events since the last call.
        fn take(&mut self) -> Vec<Seen> {
            std::mem::take(&mut self.client.events)
        }
    }

    fn place_item(socket: &str) -> Placed {
        std::env::set_var("WAYLAND_DISPLAY", socket);
        let connection = Connection::connect_to_env().expect("connect");
        let mut queue = connection.new_event_queue();
        let qh = queue.handle();
        connection.display().get_registry(&qh, ());
        let mut client = Client::default();
        queue.roundtrip(&mut client).expect("bind globals");

        let compositor = client.compositor.clone().expect("wl_compositor");
        let manager = client
            .manager
            .clone()
            .expect("the compositor offers otto_canvas_manager_v1");
        let surface = compositor.create_surface(&qh, ());
        let item = manager.get_canvas_item(&surface, &qh, ());
        queue.roundtrip(&mut client).expect("first configure");
        Placed {
            _connection: connection,
            queue,
            client,
            item,
            _surface: surface,
        }
    }

    /// The column width the running configuration holds.
    fn configured_width(handle: &HeadlessHandle) -> u32 {
        match handle.setting_value("canvas.width") {
            Some(SettingValue::Int(width)) => u32::try_from(width).expect("a positive width"),
            other => panic!("canvas.width should be an integer setting, got {other:?}"),
        }
    }

    #[test]
    #[serial]
    fn an_item_is_configured_and_told_when_the_canvas_shows_and_hides() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut placed = place_item(&handle.socket_name);

        // The configuration is process-wide, so another test in this binary
        // may have changed the width already.
        let width = configured_width(&handle);
        assert_eq!(
            placed.take(),
            vec![Seen::Configure(width), Seen::Hidden],
            "a new item is configured at the column width and told the canvas is hidden"
        );
        assert!(handle.query(|state| state.canvas_available()));
        assert!(!handle.query(|state| state.canvas_is_shown()));

        handle.with_state(|state| state.canvas_toggle());
        placed.roundtrip();
        assert_eq!(placed.take(), vec![Seen::Shown]);
        assert!(handle.query(|state| state.canvas_is_shown()));

        // The item asks for the canvas to go; it is told once the slide out
        // has finished.
        placed.item.dismiss();
        placed.roundtrip();
        assert!(!handle.query(|state| state.canvas_is_shown()));
        handle.settle(120);
        handle.wait(Duration::from_millis(50));
        placed.roundtrip();
        assert_eq!(placed.take(), vec![Seen::Hidden]);
        assert!(!handle.query(|state| state.canvas_on_screen()));

        handle.stop();
    }

    #[test]
    #[serial]
    fn changing_the_width_reconfigures_the_item() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut placed = place_item(&handle.socket_name);
        placed.take();

        let width = if configured_width(&handle) == 480 {
            520
        } else {
            480
        };
        handle
            .set_setting("canvas.width", SettingValue::Int(i64::from(width)))
            .expect("canvas.width is a live setting");
        placed.roundtrip();
        assert_eq!(placed.take(), vec![Seen::Configure(width)]);

        handle.stop();
    }

    #[test]
    #[serial]
    fn a_click_outside_the_canvas_hides_it() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut placed = place_item(&handle.socket_name);
        placed.take();

        handle.pointer_move(100.0, 400.0);
        handle.with_state(|state| state.canvas_toggle());
        handle.settle(120);
        placed.roundtrip();
        assert_eq!(placed.take(), vec![Seen::Shown]);

        handle.pointer_click();
        assert!(
            !handle.query(|state| state.canvas_is_shown()),
            "a press outside the column hides the canvas"
        );

        handle.stop();
    }

    #[test]
    #[serial]
    fn toggling_with_nothing_in_the_canvas_does_nothing() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        handle.with_state(|state| state.canvas_toggle());
        assert!(!handle.query(|state| state.canvas_available()));
        assert!(!handle.query(|state| state.canvas_on_screen()));
        handle.stop();
    }
}
