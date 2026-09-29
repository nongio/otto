//! `otto-canvas-v1` end to end (headless): a client places an item in the
//! side canvas, and showing, resizing and dismissing the canvas reach it as
//! the protocol says they should, the keyboard included, and items share
//! the column's height.

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
        MaxHeight(u32),
    }

    /// What the manager was told about drags.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Drag {
        MimeType(String),
        Started,
        Ended,
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
        drags: Vec<Drag>,
        /// The version to bind the canvas manager at; 4 when unset.
        canvas_version: Option<u32>,
        /// The last `max_height` each item was sent.
        max_heights: std::collections::HashMap<wayland_client::backend::ObjectId, u32>,
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
                        let wanted = state.canvas_version.unwrap_or(4);
                        state.manager = Some(registry.bind(name, version.min(wanted), qh, ()));
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
                otto_canvas_item_v1::Event::MaxHeight { height } => {
                    state.max_heights.insert(item.id(), height);
                    Seen::MaxHeight(height)
                }
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

    impl Dispatch<otto_canvas_manager_v1::OttoCanvasManagerV1, ()> for Client {
        fn event(
            state: &mut Self,
            _: &otto_canvas_manager_v1::OttoCanvasManagerV1,
            event: otto_canvas_manager_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            state.drags.push(match event {
                otto_canvas_manager_v1::Event::DragMimeType { mime_type } => {
                    Drag::MimeType(mime_type)
                }
                otto_canvas_manager_v1::Event::DragStarted => Drag::Started,
                otto_canvas_manager_v1::Event::DragEnded => Drag::Ended,
            });
        }
    }

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

        /// What the manager was told about drags since the last call.
        fn take_drags(&mut self) -> Vec<Drag> {
            std::mem::take(&mut self.client.drags)
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

        /// The last `max_height` `item` was sent.
        fn max_height(&self, item: &otto_canvas_item_v1::OttoCanvasItemV1) -> Option<u32> {
            self.client.max_heights.get(&item.id()).copied()
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
        place_item_at(socket, 4)
    }

    /// A client bound at `version` of the protocol, with one item.
    fn place_item_at(socket: &str, version: u32) -> Placed {
        std::env::set_var("WAYLAND_DISPLAY", socket);
        let connection = Connection::connect_to_env().expect("connect");
        let mut queue = connection.new_event_queue();
        let qh = queue.handle();
        connection.display().get_registry(&qh, ());
        let mut client = Client {
            canvas_version: Some(version),
            ..Client::default()
        };
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

        handle.pointer_press();
        assert!(
            handle.query(|state| state.canvas_is_shown()),
            "the press goes through and leaves the canvas open"
        );
        handle.pointer_release();
        assert!(
            !handle.query(|state| state.canvas_is_shown()),
            "a click outside the column hides the canvas on release"
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

    /// A window to drag from, with the pointer resting in its middle.
    fn drag_source(
        handle: &HeadlessHandle,
        title: &str,
    ) -> (
        otto_kit::testing::TestClient,
        impl Sized,
        wayland_client::protocol::wl_surface::WlSurface,
    ) {
        let mut windows =
            otto_kit::testing::TestClient::connect(&handle.socket_name).expect("connect");
        let window = windows.create_toplevel(title, 400, 300);
        handle.wait(Duration::from_millis(120));
        windows.roundtrip().expect("roundtrip");
        window.lock().unwrap().commit_frame();
        windows.roundtrip().expect("roundtrip");
        handle.settle(300);
        let (x, y, w, h) = handle
            .window_logical_geometry(title)
            .expect("the window is mapped");
        handle.pointer_move(x as f64 + w as f64 / 2.0, y as f64 + h as f64 / 2.0);
        handle.wait(Duration::from_millis(50));
        windows.roundtrip().expect("roundtrip");
        let surface = window.lock().unwrap().surface.clone();
        (windows, window, surface)
    }

    /// Press on the window and start a drag of files from it.
    fn start_dragging(
        handle: &HeadlessHandle,
        windows: &mut otto_kit::testing::TestClient,
        origin: &wayland_client::protocol::wl_surface::WlSurface,
    ) {
        handle.pointer_press();
        handle.wait(Duration::from_millis(50));
        windows.roundtrip().expect("roundtrip");
        assert!(
            windows.state.last_button_serial.is_some(),
            "the press reached the window, so it can start a drag"
        );
        windows
            .start_drag(origin, None, &["text/uri-list"])
            .expect("the drag started");
        windows.roundtrip().expect("roundtrip");
        handle.wait(Duration::from_millis(50));
        assert!(handle.query(|state| state.canvas_drag_active()));
    }

    /// The point a pointer pushed against the right edge of the output rests
    /// at, halfway down: exactly on the output's right bound, where pointer
    /// clamping leaves it.
    fn right_edge(handle: &HeadlessHandle) -> (f64, f64) {
        handle.query(|state| {
            let output = state
                .workspaces
                .outputs()
                .next()
                .cloned()
                .expect("an output");
            let geo = state
                .workspaces
                .output_geometry(&output)
                .expect("its geometry");
            (
                f64::from(geo.loc.x + geo.size.w),
                f64::from(geo.loc.y) + f64::from(geo.size.h) / 2.0,
            )
        })
    }

    /// A press on a window beside the open canvas reaches the window, and a
    /// drag started from it keeps the canvas open to the end, dropped
    /// anywhere: the drag did not open it.
    #[test]
    #[serial]
    fn a_drag_from_a_window_keeps_the_open_canvas_open() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let (mut windows, _window, origin) = drag_source(&handle, "canvas-drag-keep");
        let mut placed = place_item(&handle.socket_name);
        placed.take();
        handle.with_state(|state| state.canvas_toggle());
        handle.settle(120);
        assert!(handle.query(|state| state.canvas_is_shown()));

        // The window's left end, clear of the column.
        let (x, y, _, h) = handle
            .window_logical_geometry("canvas-drag-keep")
            .expect("the window is mapped");
        let at = (x as f64 + 20.0, y as f64 + h as f64 / 2.0);
        assert!(
            !handle.query(move |state| state.canvas_column_contains(at.into())),
            "the press is outside the column"
        );
        handle.pointer_move(at.0, at.1);
        start_dragging(&handle, &mut windows, &origin);
        assert!(handle.query(|state| state.canvas_is_shown()));
        handle.pointer_move(120.0, 300.0);
        handle.pointer_release();
        handle.wait(Duration::from_millis(50));
        let _ = windows.roundtrip();
        assert!(!handle.query(|state| state.canvas_drag_active()));
        assert!(
            handle.query(|state| state.canvas_is_shown()),
            "a press that started a drag does not hide the canvas"
        );

        handle.stop();
    }

    /// A drag resting at the right edge opens the canvas without moving the
    /// keyboard, every manager hears of the drag, and a drop elsewhere hides
    /// the canvas again.
    #[test]
    #[serial]
    fn a_drag_resting_at_the_edge_opens_the_canvas_until_it_ends() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let (mut windows, _window, origin) = drag_source(&handle, "canvas-drag-edge");
        let mut placed = place_item(&handle.socket_name);
        placed.take();

        start_dragging(&handle, &mut windows, &origin);
        placed.roundtrip();
        assert_eq!(
            placed.take_drags(),
            vec![Drag::MimeType("text/uri-list".into()), Drag::Started]
        );
        assert!(!handle.query(|state| state.canvas_on_screen()));

        let (x, y) = right_edge(&handle);
        handle.pointer_move(x, y);
        handle.wait(Duration::from_millis(700));
        assert!(
            handle.query(|state| state.canvas_is_shown()),
            "resting at the edge opens the canvas"
        );
        assert!(handle.query(|state| state.canvas_is_passive()));
        assert_eq!(
            handle.focused_window_title().as_deref(),
            Some("canvas-drag-edge"),
            "the keyboard stays with the window"
        );

        handle.pointer_move(120.0, 300.0);
        handle.pointer_release();
        handle.wait(Duration::from_millis(50));
        let _ = windows.roundtrip();
        placed.roundtrip();
        assert_eq!(placed.take_drags(), vec![Drag::Ended]);
        assert!(
            !handle.query(|state| state.canvas_is_shown()),
            "dropped away from the items, the canvas the drag opened goes"
        );

        handle.stop();
    }

    /// A drop on an item leaves the canvas the drag opened on screen.
    #[test]
    #[serial]
    fn a_drop_on_an_item_keeps_the_canvas_the_drag_opened() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let (mut windows, _window, origin) = drag_source(&handle, "canvas-drag-drop");
        let mut placed = place_item(&handle.socket_name);
        let width = i32::try_from(configured_width(&handle)).expect("a width");
        placed.attach_buffer(width, 120);
        placed.take();

        start_dragging(&handle, &mut windows, &origin);
        let (x, y) = right_edge(&handle);
        handle.pointer_move(x, y);
        handle.wait(Duration::from_millis(700));
        handle.settle(120);
        assert!(handle.query(|state| state.canvas_is_shown()));

        let (x, y) = point_on_item(&handle);
        handle.pointer_move(x, y);
        handle.pointer_release();
        handle.wait(Duration::from_millis(50));
        let _ = windows.roundtrip();
        assert!(!handle.query(|state| state.canvas_drag_active()));
        assert!(
            handle.query(|state| state.canvas_is_shown()),
            "the drop landed on an item, so the canvas stays"
        );

        handle.stop();
    }

    /// Items at version 5 share the column: a short one keeps its content's
    /// height, a tall one gets the rest, and an item bound below version 5
    /// counts at its buffer's height and hears nothing of it.
    #[test]
    #[serial]
    fn items_share_the_column_height() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let available = handle
            .query(|state| state.canvas_available_height())
            .expect("the column has a height on the headless output");
        let mut placed = place_item_at(&handle.socket_name, 5);
        let width = configured_width(&handle);
        assert_eq!(
            placed.take(),
            vec![
                Seen::Configure(width),
                Seen::MaxHeight(available),
                Seen::Hidden
            ],
            "a new item hears its share after its width and before it is hidden"
        );

        // Together far taller than the column.
        let tall = placed.item.clone();
        tall.set_content_height(available * 3);
        let (short, _short_surface) = placed.add_item();
        short.set_content_height(200);
        placed.roundtrip();
        let tall_max = placed.max_height(&tall).expect("the tall item has a share");
        let short_max = placed
            .max_height(&short)
            .expect("the short item has a share");
        assert_eq!(short_max, 200, "the short item keeps its content's height");
        assert!(
            tall_max + short_max < available,
            "the shares fit in the column with a gap: {tall_max} + {short_max} of {available}"
        );
        let gap = available - tall_max - short_max;
        assert!(gap < 100, "all that is left over is the gap between them");

        // An item bound at version 4, 60 points tall, keeps that; the tall
        // item gives up the room it takes and the gap above it. (The
        // headless column is short: much more and the short item would
        // have to give up room too.)
        let mut older = place_item_at(&handle.socket_name, 4);
        older.attach_buffer(i32::try_from(width).expect("a width"), 60);
        placed.roundtrip();
        assert!(
            !older
                .take()
                .iter()
                .any(|seen| matches!(seen, Seen::MaxHeight(_))),
            "an item below version 5 is never told a share"
        );
        assert_eq!(placed.max_height(&short), Some(200));
        assert_eq!(placed.max_height(&tall), Some(tall_max - 60 - gap));

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
