//! `otto-canvas-v1` end to end (headless): a client places an item in the
//! side canvas, and showing, resizing and dismissing the canvas reach it as
//! the protocol says they should, the keyboard included.

#[cfg(feature = "headless")]
mod headless_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto::settings::value::SettingValue;
    use serial_test::serial;
    use std::time::Duration;
    use wayland_client::protocol::{
        wl_buffer, wl_compositor, wl_keyboard, wl_registry, wl_seat, wl_shm, wl_shm_pool,
        wl_surface,
    };
    use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle};

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

    /// What the item's keyboard was told.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Keyboard {
        Enter,
        Leave,
    }

    #[derive(Default)]
    struct Client {
        compositor: Option<wl_compositor::WlCompositor>,
        manager: Option<otto_canvas_manager_v1::OttoCanvasManagerV1>,
        seat: Option<wl_seat::WlSeat>,
        shm: Option<wl_shm::WlShm>,
        events: Vec<Seen>,
        keyboard: Vec<Keyboard>,
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
                        state.manager = Some(registry.bind(name, version.min(3), qh, ()));
                    }
                    "wl_seat" => {
                        state.seat = Some(registry.bind(name, version.min(7), qh, ()));
                    }
                    "wl_shm" => {
                        state.shm = Some(registry.bind(name, version.min(1), qh, ()));
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

    impl Dispatch<wl_keyboard::WlKeyboard, ()> for Client {
        fn event(
            state: &mut Self,
            _: &wl_keyboard::WlKeyboard,
            event: wl_keyboard::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                wl_keyboard::Event::Enter { .. } => state.keyboard.push(Keyboard::Enter),
                wl_keyboard::Event::Leave { .. } => state.keyboard.push(Keyboard::Leave),
                _ => {}
            }
        }
    }

    delegate_noop!(Client: ignore wl_compositor::WlCompositor);
    delegate_noop!(Client: ignore wl_seat::WlSeat);
    delegate_noop!(Client: ignore wl_surface::WlSurface);
    delegate_noop!(Client: ignore wl_shm::WlShm);
    delegate_noop!(Client: ignore wl_shm_pool::WlShmPool);
    delegate_noop!(Client: ignore wl_buffer::WlBuffer);
    delegate_noop!(Client: otto_canvas_manager_v1::OttoCanvasManagerV1);

    /// A connected client with one canvas item, and everything it has been
    /// told so far.
    struct Placed {
        _connection: Connection,
        queue: EventQueue<Client>,
        client: Client,
        item: otto_canvas_item_v1::OttoCanvasItemV1,
        surface: wl_surface::WlSurface,
    }

    impl Placed {
        fn roundtrip(&mut self) {
            self.queue.roundtrip(&mut self.client).expect("roundtrip");
        }

        /// The events since the last call.
        fn take(&mut self) -> Vec<Seen> {
            std::mem::take(&mut self.client.events)
        }

        /// What the keyboard was told since the last call.
        fn take_keyboard(&mut self) -> Vec<Keyboard> {
            std::mem::take(&mut self.client.keyboard)
        }

        /// Another item from the same client, below the first.
        fn add_item(&mut self) -> (otto_canvas_item_v1::OttoCanvasItemV1, wl_surface::WlSurface) {
            let qh = self.queue.handle();
            let compositor = self.client.compositor.clone().expect("wl_compositor");
            let manager = self.client.manager.clone().expect("manager");
            let surface = compositor.create_surface(&qh, ());
            let item = manager.get_canvas_item(&surface, &qh, ());
            self.roundtrip();
            (item, surface)
        }

        /// Give the first item a transparent buffer `width` x `height`.
        fn attach_buffer(&mut self, width: i32, height: i32) {
            use std::os::fd::AsFd;
            let qh = self.queue.handle();
            let shm = self.client.shm.clone().expect("wl_shm");
            let stride = width * 4;
            let file = tempfile::tempfile().expect("a file for the buffer");
            file.set_len(u64::try_from(stride * height).expect("a size"))
                .expect("size the buffer");
            let pool = shm.create_pool(file.as_fd(), stride * height, &qh, ());
            let buffer =
                pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, &qh, ());
            self.surface.attach(Some(&buffer), 0, 0);
            self.surface.damage_buffer(0, 0, width, height);
            self.surface.commit();
            self.roundtrip();
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
        // The keyboard only reports focus on this client's surfaces, so it
        // hears about the item and nothing else.
        if let Some(seat) = client.seat.as_ref() {
            seat.get_keyboard(&qh, ());
        }
        let surface = compositor.create_surface(&qh, ());
        let item = manager.get_canvas_item(&surface, &qh, ());
        queue.roundtrip(&mut client).expect("first configure");
        Placed {
            _connection: connection,
            queue,
            client,
            item,
            surface,
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

    /// An item that asks for the keyboard on show gets it every time the
    /// canvas opens, and the window that had it gets it back when the canvas
    /// closes. While the item has it, Escape is the item's.
    #[test]
    #[serial]
    fn an_item_that_asks_takes_the_keyboard_on_show_and_gives_it_back() {
        use otto_kit::testing::TestClient;

        const WINDOW: &str = "canvas-focus-window";

        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut windows = TestClient::connect(&handle.socket_name).expect("connect");
        let window = windows.create_toplevel(WINDOW, 400, 300);
        handle.wait(Duration::from_millis(100));
        let _ = windows.roundtrip();
        window.lock().unwrap().commit_frame();
        let _ = windows.roundtrip();
        handle.settle(300);
        handle.focus_window(WINDOW);
        assert_eq!(handle.focused_window_title().as_deref(), Some(WINDOW));

        let mut placed = place_item(&handle.socket_name);
        placed.take();

        // By default the canvas opens without taking the keyboard.
        handle.with_state(|state| state.canvas_toggle());
        handle.settle(120);
        placed.roundtrip();
        assert_eq!(placed.take_keyboard(), vec![]);
        assert_eq!(handle.focused_window_title().as_deref(), Some(WINDOW));
        assert!(
            !handle.query(|state| state.canvas_item_has_keyboard()),
            "with no item holding the keyboard, Escape hides the canvas"
        );
        handle.with_state(|state| state.canvas_toggle());
        handle.settle(120);
        handle.wait(Duration::from_millis(50));
        placed.roundtrip();
        placed.take();

        placed
            .item
            .set_keyboard_interactivity(otto_canvas_item_v1::KeyboardInteractivity::OnShow);
        placed.roundtrip();
        for round in 0..2 {
            handle.with_state(|state| state.canvas_toggle());
            handle.settle(120);
            placed.roundtrip();
            assert_eq!(
                placed.take_keyboard(),
                vec![Keyboard::Enter],
                "the item takes the keyboard as the canvas opens (round {round})"
            );
            assert!(
                handle.query(|state| state.canvas_item_has_keyboard()),
                "the item holds the keyboard, so Escape reaches it"
            );
            assert_eq!(handle.focused_window_title(), None);

            handle.with_state(|state| state.canvas_toggle());
            placed.roundtrip();
            assert_eq!(
                placed.take_keyboard(),
                vec![Keyboard::Leave],
                "the item gives the keyboard up as the canvas closes (round {round})"
            );
            assert_eq!(
                handle.focused_window_title().as_deref(),
                Some(WINDOW),
                "the window that had the keyboard gets it back (round {round})"
            );
            handle.settle(120);
            handle.wait(Duration::from_millis(50));
            placed.roundtrip();
        }

        handle.stop();
    }

    /// A window with the keyboard, for the item to leave it with.
    fn focused_window(
        handle: &HeadlessHandle,
        title: &str,
    ) -> (otto_kit::testing::TestClient, impl Sized) {
        let mut windows =
            otto_kit::testing::TestClient::connect(&handle.socket_name).expect("connect");
        let window = windows.create_toplevel(title, 400, 300);
        handle.wait(Duration::from_millis(100));
        let _ = windows.roundtrip();
        window.lock().unwrap().commit_frame();
        let _ = windows.roundtrip();
        handle.settle(300);
        handle.focus_window(title);
        assert_eq!(handle.focused_window_title().as_deref(), Some(title));
        (windows, window)
    }

    /// Somewhere on the canvas item, in global logical coordinates.
    fn point_on_item(handle: &HeadlessHandle) -> (f64, f64) {
        handle.query(|state| {
            (0..400)
                .flat_map(|x| (0..100).map(move |y| (f64::from(x) * 10.0, f64::from(y) * 10.0)))
                .find(|&(x, y)| state.canvas_surface_under((x, y).into()).is_some())
                .expect("the item is on screen somewhere")
        })
    }

    /// `show` opens the canvas like a toggle, but leaves the keyboard where
    /// it is, even with an item that takes it on show. A later open by the
    /// user hands it over as usual.
    #[test]
    #[serial]
    fn show_opens_the_canvas_without_moving_the_keyboard() {
        const WINDOW: &str = "canvas-show-window";

        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let _window = focused_window(&handle, WINDOW);
        let mut placed = place_item(&handle.socket_name);
        placed
            .item
            .set_keyboard_interactivity(otto_canvas_item_v1::KeyboardInteractivity::OnShow);
        placed.roundtrip();
        placed.take();

        placed.item.show();
        placed.roundtrip();
        handle.settle(120);
        placed.roundtrip();
        assert!(handle.query(|state| state.canvas_is_shown()));
        assert!(handle.query(|state| state.canvas_is_passive()));
        assert_eq!(placed.take(), vec![Seen::Shown]);
        assert_eq!(
            placed.take_keyboard(),
            vec![],
            "the item is not given the keyboard"
        );
        assert_eq!(handle.focused_window_title().as_deref(), Some(WINDOW));
        assert!(
            !handle.query(|state| state.canvas_owns_escape()),
            "Escape stays with the app"
        );

        // Asking again while shown changes nothing.
        placed.item.show();
        placed.roundtrip();
        assert_eq!(placed.take(), vec![]);

        handle.with_state(|state| state.canvas_toggle());
        handle.settle(120);
        handle.wait(Duration::from_millis(50));
        placed.roundtrip();
        placed.take();
        handle.with_state(|state| state.canvas_toggle());
        handle.settle(120);
        placed.roundtrip();
        assert_eq!(
            placed.take_keyboard(),
            vec![Keyboard::Enter],
            "the user opening it gives the item the keyboard"
        );

        handle.stop();
    }

    /// A press outside a canvas a client showed reaches the app, and the
    /// canvas goes once the button is released outside it.
    #[test]
    #[serial]
    fn a_click_outside_a_shown_by_client_canvas_goes_through_and_hides_it() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut placed = place_item(&handle.socket_name);
        placed.take();
        handle.pointer_move(100.0, 400.0);
        placed.item.show();
        placed.roundtrip();
        handle.settle(120);
        assert!(handle.query(|state| state.canvas_is_shown()));

        handle.pointer_press();
        assert!(
            handle.query(|state| state.canvas_is_shown()),
            "the press alone leaves it open, so a drag can end on an item"
        );
        handle.pointer_release();
        assert!(!handle.query(|state| state.canvas_is_shown()));

        handle.stop();
    }

    /// An item that never takes the keyboard is clicked without the app
    /// losing it.
    #[test]
    #[serial]
    fn a_click_on_a_never_item_leaves_the_keyboard_with_the_app() {
        const WINDOW: &str = "canvas-never-window";

        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let _window = focused_window(&handle, WINDOW);
        let mut placed = place_item(&handle.socket_name);
        placed
            .item
            .set_keyboard_interactivity(otto_canvas_item_v1::KeyboardInteractivity::Never);
        let width = i32::try_from(configured_width(&handle)).expect("a width");
        placed.attach_buffer(width, 120);
        placed.take();

        placed.item.show();
        placed.roundtrip();
        handle.settle(120);
        let (x, y) = point_on_item(&handle);
        handle.pointer_move(x, y);
        handle.pointer_click();
        placed.roundtrip();
        assert_eq!(placed.take_keyboard(), vec![]);
        assert_eq!(handle.focused_window_title().as_deref(), Some(WINDOW));
        assert!(handle.query(|state| state.canvas_is_shown()));

        handle.stop();
    }

    /// Items stack by their order, then by when they were created.
    #[test]
    #[serial]
    fn set_order_restacks_the_items() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut placed = place_item(&handle.socket_name);
        let (second, second_surface) = placed.add_item();
        let first = placed.surface.id().protocol_id();
        let later = second_surface.id().protocol_id();
        let order = |handle: &HeadlessHandle| {
            handle.query(|state| {
                state
                    .canvas_item_surfaces()
                    .iter()
                    .map(|surface| {
                        smithay::reexports::wayland_server::Resource::id(surface).protocol_id()
                    })
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(order(&handle), vec![first, later]);

        second.set_order(-1);
        placed.roundtrip();
        assert_eq!(
            order(&handle),
            vec![later, first],
            "a lower order sits higher"
        );

        second.set_order(0);
        placed.roundtrip();
        assert_eq!(
            order(&handle),
            vec![first, later],
            "with the same order, the older item sits higher"
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
