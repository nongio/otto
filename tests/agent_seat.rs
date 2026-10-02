//! The agent seat (`[agent_cursor]`, `src/agent_cursor.rs`): a virtual pointer
//! created on it moves its own pointer and keyboard focus, never the user's.

#[cfg(feature = "headless")]
mod agent_seat_tests {
    use std::time::Duration;

    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use wayland_client::{
        delegate_noop,
        protocol::{wl_pointer, wl_registry, wl_seat},
        Connection, Dispatch, EventQueue, QueueHandle,
    };
    use wayland_protocols::ext::session_lock::v1::client::{
        ext_session_lock_manager_v1, ext_session_lock_v1,
    };
    use wayland_protocols::wp::security_context::v1::client::{
        wp_security_context_manager_v1, wp_security_context_v1,
    };
    use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
        zwp_virtual_keyboard_manager_v1, zwp_virtual_keyboard_v1,
    };
    use wayland_protocols_wlr::foreign_toplevel::v1::client::{
        zwlr_foreign_toplevel_handle_v1, zwlr_foreign_toplevel_manager_v1,
    };
    use wayland_protocols_wlr::virtual_pointer::v1::client::{
        zwlr_virtual_pointer_manager_v1, zwlr_virtual_pointer_v1,
    };

    const BTN_LEFT: u32 = 0x110;
    /// The seat of the first agent to ask.
    const AGENT: &str = "agent-1";

    /// Every seat, in the order the compositor advertised them.
    #[derive(Default)]
    struct DriverState {
        seats: Vec<wl_seat::WlSeat>,
        /// `wl_seat.name` of each seat, by the same index.
        seat_names: Vec<Option<String>>,
        pointer_manager: Option<zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1>,
        keyboard_manager: Option<zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1>,
        security_contexts: Option<wp_security_context_manager_v1::WpSecurityContextManagerV1>,
        toplevels: Option<zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1>,
        /// The windows the compositor told this client of, with their titles.
        windows: Vec<(
            zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
            String,
        )>,
        lock_manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for DriverState {
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
            match interface.as_str() {
                "wl_seat" => {
                    let index = state.seats.len();
                    state
                        .seats
                        .push(registry.bind(name, version.min(5), qh, index));
                    state.seat_names.push(None);
                }
                "zwlr_virtual_pointer_manager_v1" => {
                    state.pointer_manager = Some(registry.bind(name, version.min(2), qh, ()))
                }
                "zwp_virtual_keyboard_manager_v1" => {
                    state.keyboard_manager = Some(registry.bind(name, 1, qh, ()))
                }
                "wp_security_context_manager_v1" => {
                    state.security_contexts = Some(registry.bind(name, 1, qh, ()))
                }
                "zwlr_foreign_toplevel_manager_v1" => {
                    state.toplevels = Some(registry.bind(name, version.min(3), qh, ()))
                }
                "ext_session_lock_manager_v1" => {
                    state.lock_manager = Some(registry.bind(name, 1, qh, ()))
                }
                _ => {}
            }
        }
    }

    impl Dispatch<wl_seat::WlSeat, usize> for DriverState {
        fn event(
            state: &mut Self,
            _: &wl_seat::WlSeat,
            event: wl_seat::Event,
            index: &usize,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_seat::Event::Name { name } = event {
                state.seat_names[*index] = Some(name);
            }
        }
    }
    delegate_noop!(DriverState: zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1);
    delegate_noop!(DriverState: zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1);
    delegate_noop!(DriverState: zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1);
    delegate_noop!(DriverState: zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1);
    delegate_noop!(DriverState: ext_session_lock_manager_v1::ExtSessionLockManagerV1);
    delegate_noop!(DriverState: ignore wp_security_context_manager_v1::WpSecurityContextManagerV1);
    delegate_noop!(DriverState: ignore wp_security_context_v1::WpSecurityContextV1);

    impl Dispatch<zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, ()> for DriverState {
        fn event(
            state: &mut Self,
            _: &zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1,
            event: zwlr_foreign_toplevel_manager_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } = event {
                state.windows.push((toplevel, String::new()));
            }
        }

        wayland_client::event_created_child!(DriverState, zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, [
            zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1, ()),
        ]);
    }

    impl Dispatch<zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1, ()> for DriverState {
        fn event(
            state: &mut Self,
            handle: &zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
            event: zwlr_foreign_toplevel_handle_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                zwlr_foreign_toplevel_handle_v1::Event::Title { title } => {
                    if let Some(entry) = state.windows.iter_mut().find(|(h, _)| h == handle) {
                        entry.1 = title;
                    }
                }
                zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                    state.windows.retain(|(h, _)| h != handle);
                }
                _ => {}
            }
        }
    }
    delegate_noop!(DriverState: ignore ext_session_lock_v1::ExtSessionLockV1);

    /// An automation client, like `wlrctl` or an agent's MCP driver.
    struct Driver {
        conn: Connection,
        queue: EventQueue<DriverState>,
        qh: QueueHandle<DriverState>,
        state: DriverState,
    }

    impl Driver {
        fn connect(handle: &HeadlessHandle) -> Self {
            let path = format!(
                "{}/{}",
                std::env::var("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR"),
                handle.socket_name
            );
            Self::from_stream(std::os::unix::net::UnixStream::connect(path).expect("connect"))
        }

        /// The Wayland connection Otto hands the agent on `owner` over
        /// D-Bus (`ConnectAgent`): the only one that may drive its seat.
        fn connect_as_agent(handle: &HeadlessHandle, owner: &str) -> Self {
            let owner = owner.to_string();
            Self::from_stream(handle.query(move |state| {
                state
                    .connect_agent_client(&owner)
                    .expect("connect the agent")
            }))
        }

        /// A client connected the way Otto connects the locker it starts —
        /// the only kind offered `ext_session_lock_manager_v1`.
        fn connect_as_locker(handle: &HeadlessHandle) -> Self {
            Self::from_stream(
                handle.query(|state| state.connect_locker_client().expect("connect the locker").1),
            )
        }

        /// `from_stream`, or `None` when the compositor closes the connection
        /// before the registry arrives.
        fn from_stream_checked(stream: std::os::unix::net::UnixStream) -> Option<Self> {
            let conn = Connection::from_socket(stream).ok()?;
            let mut queue = conn.new_event_queue();
            let qh = queue.handle();
            conn.display().get_registry(&qh, ());
            let mut state = DriverState::default();
            queue.roundtrip(&mut state).ok()?;
            queue.roundtrip(&mut state).ok()?;
            Some(Self {
                conn,
                queue,
                qh,
                state,
            })
        }

        fn from_stream(stream: std::os::unix::net::UnixStream) -> Self {
            let conn = Connection::from_socket(stream).expect("wayland connection");
            let mut queue = conn.new_event_queue();
            let qh = queue.handle();
            conn.display().get_registry(&qh, ());
            let mut state = DriverState::default();
            queue.roundtrip(&mut state).expect("bind globals");
            queue.roundtrip(&mut state).expect("seat names");
            Self {
                conn,
                queue,
                qh,
                state,
            }
        }

        /// The advertised seat named `name`.
        fn seat_index(&self, name: &str) -> Option<usize> {
            self.state
                .seat_names
                .iter()
                .position(|seat| seat.as_deref() == Some(name))
        }

        /// A virtual pointer on the `index`th advertised seat.
        fn pointer_on(&self, index: usize) -> zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1 {
            let manager = self
                .state
                .pointer_manager
                .as_ref()
                .expect("zwlr_virtual_pointer_manager_v1 missing");
            manager.create_virtual_pointer(Some(&self.state.seats[index]), &self.qh, ())
        }

        /// A virtual pointer on no seat in particular: the compositor's
        /// default for this connection.
        fn default_pointer(&self) -> zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1 {
            let manager = self
                .state
                .pointer_manager
                .as_ref()
                .expect("zwlr_virtual_pointer_manager_v1 missing");
            manager.create_virtual_pointer(None, &self.qh, ())
        }

        /// A virtual keyboard on the `index`th advertised seat.
        fn keyboard_on(&self, index: usize) -> zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1 {
            let manager = self
                .state
                .keyboard_manager
                .as_ref()
                .expect("zwp_virtual_keyboard_manager_v1 missing");
            manager.create_virtual_keyboard(&self.state.seats[index], &self.qh, ())
        }

        /// The titles of the windows this client was told of.
        fn window_titles(&self) -> Vec<String> {
            let mut titles: Vec<String> =
                self.state.windows.iter().map(|(_, t)| t.clone()).collect();
            titles.sort();
            titles
        }

        /// The handle of the window titled `title`.
        fn window(
            &self,
            title: &str,
        ) -> &zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1 {
            &self
                .state
                .windows
                .iter()
                .find(|(_, t)| t == title)
                .expect("window listed")
                .0
        }

        fn settle(&mut self, handle: &HeadlessHandle) {
            self.conn.flush().expect("flush");
            handle.wait(Duration::from_millis(120));
            self.queue.roundtrip(&mut self.state).expect("roundtrip");
            handle.settle(200);
        }
    }

    /// A compositor with one agent seat, asked for by the connection `:1.10`.
    fn start() -> HeadlessHandle {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        request_seat(&handle, "Agent", ":1.10").expect("seat");
        handle
    }

    fn map_window(handle: &HeadlessHandle, client: &mut TestClient, title: &str) {
        client.create_toplevel_with_app_id(title, &format!("org.otto.{title}"), 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(200);
    }

    /// Move `pointer` to a logical position on the output.
    fn move_to(
        handle: &HeadlessHandle,
        pointer: &zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
        x: i32,
        y: i32,
    ) {
        let (w, h) = handle.query(|state| {
            let output = state.workspaces.outputs().next().cloned().expect("output");
            let geo = state.workspaces.output_geometry(&output).expect("geometry");
            (geo.size.w, geo.size.h)
        });
        pointer.motion_absolute(0, x as u32, y as u32, w as u32, h as u32);
        pointer.frame();
    }

    fn click(pointer: &zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1) {
        pointer.button(1, BTN_LEFT, wl_pointer::ButtonState::Pressed);
        pointer.frame();
        pointer.button(2, BTN_LEFT, wl_pointer::ButtonState::Released);
        pointer.frame();
    }

    fn agent_cursor_opacity(handle: &HeadlessHandle) -> f32 {
        handle.query(|state| {
            state
                .agent_seat(AGENT)
                .map_or(0.0, |agent| agent.cursor.opacity(std::time::Instant::now()))
        })
    }

    /// The window holding the agent seat's keyboard, by title.
    fn agent_keyboard_focus(handle: &HeadlessHandle) -> Option<String> {
        handle.query(|state| {
            use smithay::wayland::seat::WaylandFocus;
            let keyboard = state.agent_seat(AGENT)?.seat.get_keyboard()?;
            let focus = keyboard.current_focus()?;
            let surface = focus.wl_surface()?.into_owned();
            state
                .workspaces
                .spaces_elements()
                .find(|w| w.wl_surface().is_some_and(|s| *s == surface))
                .map(|w| w.xdg_title())
        })
    }

    fn user_pointer(handle: &HeadlessHandle) -> (f64, f64) {
        handle.query(|state| {
            let p = state.pointer.current_location();
            (p.x, p.y)
        })
    }

    fn agent_pointer(handle: &HeadlessHandle) -> Option<(f64, f64)> {
        handle.query(|state| {
            state.agent_seat(AGENT).map(|agent| {
                let p = agent.pointer.current_location();
                (p.x, p.y)
            })
        })
    }

    /// An agent's seat is advertised on the agent's own connections alone:
    /// every other client sees the user's seat only, and an agent's
    /// connection sees its own only, so tools that take the first seat land
    /// on the right one either way.
    #[test]
    #[serial]
    fn an_agents_seat_is_seen_only_on_its_connection() {
        let handle = start();
        let other = Driver::connect(&handle);
        assert_eq!(
            other.state.seats.len(),
            1,
            "another client saw the agent's seat"
        );
        assert!(other.seat_index(AGENT).is_none());

        let agent = Driver::connect_as_agent(&handle, ":1.10");
        assert_eq!(
            agent.state.seats.len(),
            1,
            "the agent saw a seat not its own"
        );
        assert_eq!(agent.seat_index(AGENT), Some(0));

        drop(other);
        drop(agent);
        handle.stop();
    }

    /// Motion on the agent seat moves the agent pointer and shows its cursor;
    /// the user's cursor stays where it was.
    #[test]
    #[serial]
    fn agent_motion_leaves_the_user_pointer_alone() {
        let handle = start();
        let mut driver = Driver::connect_as_agent(&handle, ":1.10");
        let before = user_pointer(&handle);
        assert_eq!(
            agent_cursor_opacity(&handle),
            0.0,
            "the agent cursor is hidden until the agent first moves"
        );

        let pointer = driver.pointer_on(driver.seat_index(AGENT).unwrap());
        move_to(&handle, &pointer, 480, 270);
        driver.settle(&handle);

        assert_eq!(agent_pointer(&handle), Some((480.0, 270.0)));
        assert_eq!(user_pointer(&handle), before, "the user's pointer moved");
        assert_eq!(agent_cursor_opacity(&handle), 1.0);

        drop(driver);
        handle.stop();
    }

    /// A pointer on the user's seat still drives the user's pointer while the
    /// agent seat exists — that is how `otto-rdp` works.
    #[test]
    #[serial]
    fn a_pointer_on_the_user_seat_still_moves_the_user_pointer() {
        let handle = start();
        let mut driver = Driver::connect(&handle);

        let pointer = driver.pointer_on(0);
        move_to(&handle, &pointer, 960, 540);
        driver.settle(&handle);

        assert_eq!(user_pointer(&handle), (960.0, 540.0));
        assert_eq!(agent_pointer(&handle), Some((0.0, 0.0)));

        drop(driver);
        handle.stop();
    }

    /// Once the agent stops, its cursor fades out; its next move brings it
    /// back at once.
    #[test]
    #[serial]
    fn the_agent_cursor_hides_when_the_agent_is_idle() {
        let handle = start();
        handle.query(|state| {
            state.agent_seat_mut(AGENT).unwrap().cursor =
                otto::agent_cursor::AgentCursor::new("#FF9500", Duration::from_millis(1500));
        });
        let mut driver = Driver::connect_as_agent(&handle, ":1.10");
        let pointer = driver.pointer_on(driver.seat_index(AGENT).unwrap());
        move_to(&handle, &pointer, 100, 100);
        driver.settle(&handle);
        assert_eq!(agent_cursor_opacity(&handle), 1.0);

        handle.wait(Duration::from_millis(2000));
        assert_eq!(agent_cursor_opacity(&handle), 0.0, "still shown when idle");
        assert_eq!(
            agent_pointer(&handle),
            Some((100.0, 100.0)),
            "hiding moved the pointer"
        );

        move_to(&handle, &pointer, 120, 100);
        driver.settle(&handle);
        assert_eq!(agent_cursor_opacity(&handle), 1.0);

        drop(driver);
        handle.stop();
    }

    /// Locking takes the agent's keyboard away, and while locked the agent
    /// can neither move nor click.
    #[test]
    #[serial]
    fn a_locked_session_stops_agent_input() {
        let handle = start();
        request_workspace(&handle, ":1.10").expect("workspace");
        let mut window = TestClient::connect(&handle.socket_name).expect("client");
        map_window(&handle, &mut window, "Target");
        let position = workspace_names(&handle).len() - 1;
        handle.move_window_to_workspace("Target", position);
        handle.settle(200);
        let _ = window.roundtrip();
        let (x, y, _, _) = handle
            .window_logical_geometry("Target")
            .expect("window mapped");

        let mut driver = Driver::connect_as_agent(&handle, ":1.10");
        let pointer = driver.pointer_on(driver.seat_index(AGENT).unwrap());
        move_to(&handle, &pointer, x + 20, y + 20);
        driver.settle(&handle);
        click(&pointer);
        driver.settle(&handle);
        assert_eq!(agent_keyboard_focus(&handle).as_deref(), Some("Target"));

        let mut locker = Driver::connect_as_locker(&handle);
        let lock = locker
            .state
            .lock_manager
            .as_ref()
            .expect("ext_session_lock_manager_v1 missing")
            .lock(&locker.qh, ());
        locker.settle(&handle);
        assert!(handle.query(|state| state.is_session_locked()));
        assert_eq!(
            agent_keyboard_focus(&handle),
            None,
            "the agent kept its keyboard through the lock"
        );

        move_to(&handle, &pointer, x + 60, y + 60);
        click(&pointer);
        driver.settle(&handle);
        assert_eq!(
            agent_pointer(&handle),
            Some(((x + 20) as f64, (y + 20) as f64)),
            "the agent pointer moved while locked"
        );
        assert_eq!(agent_keyboard_focus(&handle), None);

        drop(lock);
        drop(driver);
        handle.stop();
    }

    /// Ask for a seat the way `RequestAgentSeat` does over D-Bus.
    fn request_seat(
        handle: &HeadlessHandle,
        agent_name: &str,
        owner: &str,
    ) -> Result<(String, String), String> {
        let (agent_name, owner) = (agent_name.to_string(), owner.to_string());
        handle.query(move |state| {
            let (tx, mut rx) = tokio::sync::oneshot::channel();
            otto::screenshare::handle_screenshare_command(
                state,
                otto::screenshare::CompositorCommand::RequestAgentSeat {
                    agent_name,
                    owner,
                    response_tx: tx,
                },
            );
            rx.try_recv()
                .expect("answered at once")
                .map(|granted| (granted.seat, granted.color))
        })
    }

    /// Release the way `ReleaseAgentSeat`, or the owner leaving the bus, does.
    fn release_seats(handle: &HeadlessHandle, owner: &str) {
        let owner = owner.to_string();
        handle.query(move |state| {
            otto::screenshare::handle_screenshare_command(
                state,
                otto::screenshare::CompositorCommand::ReleaseAgentSeats {
                    owner,
                    response_tx: None,
                },
            )
        });
    }

    fn pointer_of(handle: &HeadlessHandle, seat: &str) -> Option<(f64, f64)> {
        let seat = seat.to_string();
        handle.query(move |state| {
            state.agent_seat(&seat).map(|agent| {
                let p = agent.pointer.current_location();
                (p.x, p.y)
            })
        })
    }

    /// Each agent that asks gets a seat of its own, named and coloured apart
    /// from the others, and advertised to clients.
    #[test]
    #[serial]
    fn each_agent_gets_its_own_seat() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let first = request_seat(&handle, "Claude", ":1.10").expect("seat");
        let second = request_seat(&handle, "Helper", ":1.11").expect("seat");
        assert_eq!(first.0, "agent-1");
        assert_eq!(second.0, "agent-2");
        assert_ne!(first.1, second.1, "two agents share a colour");

        let claude = Driver::connect_as_agent(&handle, ":1.10");
        let helper = Driver::connect_as_agent(&handle, ":1.11");
        assert_eq!(claude.state.seat_names, vec![Some("agent-1".to_string())]);
        assert_eq!(helper.state.seat_names, vec![Some("agent-2".to_string())]);

        drop(claude);
        drop(helper);
        handle.stop();
    }

    /// A pointer on one agent's seat moves that agent's pointer only.
    #[test]
    #[serial]
    fn agents_move_their_own_pointers() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        request_seat(&handle, "Claude", ":1.10").expect("seat");
        request_seat(&handle, "Helper", ":1.11").expect("seat");
        let mut driver = Driver::connect_as_agent(&handle, ":1.11");
        let before = user_pointer(&handle);

        let pointer = driver.pointer_on(driver.seat_index("agent-2").unwrap());
        move_to(&handle, &pointer, 200, 150);
        driver.settle(&handle);

        assert_eq!(pointer_of(&handle, "agent-2"), Some((200.0, 150.0)));
        assert_eq!(pointer_of(&handle, "agent-1"), Some((0.0, 0.0)));
        assert_eq!(user_pointer(&handle), before);

        drop(driver);
        handle.stop();
    }

    /// A name held by one connection cannot be taken by another; asking
    /// again from the holder returns the seat it has.
    #[test]
    #[serial]
    fn a_held_name_is_not_given_twice() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let seat = request_seat(&handle, "Claude", ":1.10").expect("seat");
        assert_eq!(request_seat(&handle, "Claude", ":1.10"), Ok(seat));
        assert!(request_seat(&handle, "Claude", ":1.99").is_err());
        assert!(request_seat(&handle, "  ", ":1.99").is_err());
        handle.stop();
    }

    /// A client connected before an agent asks for its seat is not told of
    /// the new seat, and goes on working: binding every seat it is told of,
    /// as toolkits do, never binds one it may not see.
    #[test]
    #[serial]
    fn a_seat_appearing_later_is_not_announced_to_other_clients() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut before = Driver::connect(&handle);
        assert_eq!(before.state.seats.len(), 1);

        request_seat(&handle, "Claude", ":1.10").expect("seat");
        before.settle(&handle);
        assert_eq!(before.state.seats.len(), 1, "the new seat was announced");
        assert!(
            before.queue.roundtrip(&mut before.state).is_ok(),
            "the client was disconnected when the seat appeared"
        );

        drop(before);
        handle.stop();
    }

    /// A seat an agent asked for is driven through the agent's own
    /// connections only: no other client is even offered it.
    #[test]
    #[serial]
    fn only_the_agents_connection_drives_its_seat() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        request_seat(&handle, "Claude", ":1.10").expect("seat");
        let stranger = Driver::connect(&handle);
        assert!(stranger.seat_index("agent-1").is_none());
        assert_eq!(stranger.state.seats.len(), 1);
        drop(stranger);
        handle.stop();
    }

    /// An agent's connection drives its own seat and no other: it is not
    /// offered the user's seat or another agent's, and a pointer it creates
    /// without naming a seat drives its own.
    #[test]
    #[serial]
    fn an_agents_connection_drives_no_other_seat() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        request_seat(&handle, "Claude", ":1.10").expect("seat");
        request_seat(&handle, "Helper", ":1.11").expect("seat");
        let mut driver = Driver::connect_as_agent(&handle, ":1.10");
        let before = user_pointer(&handle);
        assert_eq!(driver.state.seat_names, vec![Some("agent-1".to_string())]);

        let pointer = driver.default_pointer();
        move_to(&handle, &pointer, 250, 250);
        driver.settle(&handle);

        assert_eq!(user_pointer(&handle), before);
        assert_eq!(pointer_of(&handle, "agent-2"), Some((0.0, 0.0)));
        let (x, y) = pointer_of(&handle, "agent-1").expect("agent-1");
        assert_eq!(
            (x.round(), y.round()),
            (250.0, 250.0),
            "its own pointer did not move"
        );

        drop(driver);
        handle.stop();
    }

    /// An agent's connection types on its own seat, the only one it sees.
    #[test]
    #[serial]
    fn an_agent_types_on_its_own_seat() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        request_seat(&handle, "Claude", ":1.10").expect("seat");
        let mut agent = Driver::connect_as_agent(&handle, ":1.10");
        let _keyboard = agent.keyboard_on(agent.seat_index("agent-1").unwrap());
        agent.settle(&handle);
        assert!(agent.queue.roundtrip(&mut agent.state).is_ok());
        drop(agent);
        handle.stop();
    }

    /// A security context made on an agent's connection connects more of
    /// the agent's clients: a client through it sees the agent's seat alone
    /// and drives it. That is how a sandbox around an agent gets every
    /// client inside onto the seat, with the protocol sandboxes speak.
    #[test]
    #[serial]
    fn a_client_through_an_agents_security_context_is_the_agents() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        request_seat(&handle, "Claude", ":1.10").expect("seat");
        let mut agent = Driver::connect_as_agent(&handle, ":1.10");
        let manager = agent
            .state
            .security_contexts
            .clone()
            .expect("an agent's connection makes security contexts");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let (close_read, _close_write) = std::io::pipe().unwrap();
        let context = manager.create_listener(
            std::os::fd::AsFd::as_fd(&listener),
            std::os::fd::AsFd::as_fd(&close_read),
            &agent.qh,
            (),
        );
        context.set_sandbox_engine("org.otto.test".into());
        context.commit();
        agent.settle(&handle);
        // Otto holds the only listening end from here on.
        drop(listener);

        let mut inside =
            Driver::from_stream(std::os::unix::net::UnixStream::connect(&path).expect("connect"));
        assert_eq!(inside.state.seat_names, vec![Some("agent-1".to_string())]);
        let pointer = inside.pointer_on(0);
        move_to(&handle, &pointer, 200, 150);
        inside.settle(&handle);
        assert_eq!(pointer_of(&handle, "agent-1"), Some((200.0, 150.0)));

        release_seats(&handle, ":1.10");
        handle.settle(200);
        assert!(inside.queue.roundtrip(&mut inside.state).is_err());
        assert!(
            std::os::unix::net::UnixStream::connect(&path).is_err()
                || Driver::from_stream_checked(
                    std::os::unix::net::UnixStream::connect(&path).unwrap()
                )
                .is_none(),
            "the listener outlived the seat"
        );

        drop(inside);
        drop(agent);
        handle.stop();
    }

    /// A window from an agent's connection opens on the agent's workspace,
    /// with the agent's keyboard; the user's focus and workspace stay.
    #[test]
    #[serial]
    fn an_agents_window_opens_on_its_workspace_with_its_focus() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        request_seat(&handle, "Claude", ":1.10").expect("seat");
        request_workspace(&handle, ":1.10").expect("workspace");
        let mut user = TestClient::connect(&handle.socket_name).expect("client");
        map_window(&handle, &mut user, "User");
        let _ = user.roundtrip();
        assert!(user.state.keyboard_focused);

        let stream = handle.query(|state| {
            state
                .connect_agent_client(":1.10")
                .expect("connect the agent")
        });
        let mut mine = TestClient::from_stream(stream).expect("agent client");
        map_window(&handle, &mut mine, "Mine");
        let _ = user.roundtrip();

        let on_agent_workspace = handle.query(|state| {
            let id = state
                .workspaces
                .spaces_elements()
                .find(|w| w.xdg_title() == "Mine")
                .map(|w| w.id())
                .expect("Mine mapped");
            state.agent_scope_window_ids("agent-1").contains(&id)
        });
        assert!(
            on_agent_workspace,
            "the agent's window opened on the user's workspace"
        );
        assert_eq!(
            keyboard_focus_of(&handle, "agent-1").as_deref(),
            Some("Mine")
        );
        assert!(
            user.state.keyboard_focused,
            "the user's window lost the user's keyboard"
        );
        assert_eq!(handle.current_workspace_index(), 0);

        drop(mine);
        drop(user);
        handle.stop();
    }

    /// A window from an agent's connection presenting itself with an
    /// activation token, as GTK does when it opens, right after the user
    /// pressed something: it gets the agent's keyboard, and the user's
    /// workspace and focus stay.
    #[test]
    #[serial]
    fn an_agents_window_presenting_itself_does_not_come_to_the_user() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        request_seat(&handle, "Claude", ":1.10").expect("seat");
        request_workspace(&handle, ":1.10").expect("workspace");
        let mut user = TestClient::connect(&handle.socket_name).expect("client");
        map_window(&handle, &mut user, "User");
        let _ = user.roundtrip();
        assert!(user.state.keyboard_focused);

        let stream = handle.query(|state| {
            state
                .connect_agent_client(":1.10")
                .expect("connect the agent")
        });
        let mut mine = TestClient::from_stream(stream).expect("agent client");
        let window = mine.create_toplevel_with_app_id("Mine", "org.otto.Mine", 640, 480);
        handle.settle(200);
        let surface = window.lock().unwrap().surface.clone();

        // The user just pressed a key: tokens are handed out for a moment.
        handle.query(|state| state.last_press = Some(std::time::Instant::now()));
        mine.activate_self(&surface);
        handle.settle(200);
        let _ = user.roundtrip();

        assert_eq!(handle.current_workspace_index(), 0, "the user was moved");
        assert!(
            user.state.keyboard_focused,
            "the user's window lost the user's keyboard"
        );
        assert_eq!(
            keyboard_focus_of(&handle, "agent-1").as_deref(),
            Some("Mine")
        );

        drop(mine);
        drop(user);
        handle.stop();
    }

    /// The user can lend an agent the workspace they are on: the agent's
    /// scope is that workspace, with the user's windows on it, the user
    /// stays where they are, and the loan is not kept for the agent's
    /// return.
    #[test]
    #[serial]
    fn an_agent_works_on_a_workspace_the_user_lends_it() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut user = TestClient::connect(&handle.socket_name).expect("client");
        map_window(&handle, &mut user, "User");
        let _ = user.roundtrip();
        assert!(user.state.keyboard_focused);

        // Without a seat there is nothing to lend to.
        assert!(lend_workspace(&handle, ":1.10", "").is_err());
        request_seat(&handle, "Claude", ":1.10").expect("seat");
        assert!(lend_workspace(&handle, ":1.10", "no such workspace").is_err());
        lend_workspace(&handle, ":1.10", "").expect("the user's workspace");
        handle.settle(200);

        let user_window_in_scope = handle.query(|state| {
            let id = state
                .workspaces
                .spaces_elements()
                .find(|w| w.xdg_title() == "User")
                .map(|w| w.id())
                .expect("User mapped");
            state.agent_scope_window_ids("agent-1").contains(&id)
        });
        assert!(user_window_in_scope, "the lent workspace is not in scope");
        assert_eq!(handle.current_workspace_index(), 0, "the user was moved");
        let _ = user.roundtrip();
        assert!(user.state.keyboard_focused, "the user lost their keyboard");
        assert!(
            agent_frame(&handle, ":1.10").is_some(),
            "the lent workspace is not framed"
        );

        // Gone and back: a lent workspace is asked for again.
        release_seats(&handle, ":1.10");
        handle.settle(200);
        request_seat(&handle, "Claude", ":1.20").expect("seat back");
        let regained = handle.query(|state| {
            state
                .agent_seat("agent-1")
                .and_then(|agent| agent.grant.clone())
        });
        assert_eq!(regained, None, "the loan outlived the agent");

        drop(user);
        handle.stop();
    }

    /// An agent's connection is told of the windows on its workspace alone,
    /// and activating one gives it the agent's keyboard, not the user's.
    #[test]
    #[serial]
    fn an_agent_lists_and_focuses_only_its_own_windows() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        request_seat(&handle, "Claude", ":1.10").expect("seat");
        request_workspace(&handle, ":1.10").expect("workspace");
        let mut user = TestClient::connect(&handle.socket_name).expect("client");
        map_window(&handle, &mut user, "User");
        let connect = || {
            handle.query(|state| {
                state
                    .connect_agent_client(":1.10")
                    .expect("connect the agent")
            })
        };
        let mut first = TestClient::from_stream(connect()).expect("agent client");
        map_window(&handle, &mut first, "Mine");
        let mut second = TestClient::from_stream(connect()).expect("agent client");
        map_window(&handle, &mut second, "Mine too");
        assert_eq!(
            keyboard_focus_of(&handle, "agent-1").as_deref(),
            Some("Mine too")
        );

        let mut agent = Driver::from_stream(connect());
        agent.settle(&handle);
        assert_eq!(
            agent.window_titles(),
            vec!["Mine".to_string(), "Mine too".to_string()]
        );
        let everyone = Driver::connect(&handle);
        assert_eq!(
            everyone.window_titles(),
            vec![
                "Mine".to_string(),
                "Mine too".to_string(),
                "User".to_string()
            ]
        );

        agent.window("Mine").activate(&agent.state.seats[0]);
        agent.settle(&handle);
        let _ = user.roundtrip();
        assert_eq!(
            keyboard_focus_of(&handle, "agent-1").as_deref(),
            Some("Mine")
        );
        assert!(
            user.state.keyboard_focused,
            "the agent took the user's keyboard"
        );

        drop(agent);
        drop(everyone);
        drop(first);
        drop(second);
        drop(user);
        handle.stop();
    }

    /// Releasing removes the seat; the same agent coming back in the same
    /// session gets the same seat name and colour.
    #[test]
    #[serial]
    fn a_returning_agent_gets_its_seat_back() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let seat = request_seat(&handle, "Claude", ":1.10").expect("seat");
        request_seat(&handle, "Helper", ":1.11").expect("seat");

        let mut driver = Driver::connect_as_agent(&handle, ":1.10");
        let _pointer = driver.pointer_on(driver.seat_index("agent-1").unwrap());
        driver.settle(&handle);
        release_seats(&handle, ":1.10");
        handle.settle(200);
        assert!(handle.query(|state| state.agent_seat("agent-1").is_none()));

        // The agent's connection goes with its seat.
        assert!(
            driver.queue.roundtrip(&mut driver.state).is_err(),
            "the agent's connection outlived its seat"
        );
        drop(driver);

        let fresh = Driver::connect(&handle);
        assert_eq!(
            fresh.seat_index("agent-1"),
            None,
            "the seat is still advertised"
        );
        drop(fresh);

        assert_eq!(request_seat(&handle, "Claude", ":1.20"), Ok(seat));
        handle.stop();
    }

    fn request_workspace(handle: &HeadlessHandle, owner: &str) -> Result<String, String> {
        let owner = owner.to_string();
        handle.query(move |state| {
            let (tx, mut rx) = tokio::sync::oneshot::channel();
            otto::screenshare::handle_screenshare_command(
                state,
                otto::screenshare::CompositorCommand::RequestOwnWorkspace {
                    owner,
                    response_tx: tx,
                },
            );
            rx.try_recv()
                .expect("answered at once")
                .map(|workspace| workspace.output)
        })
    }

    /// Find the workspace `name` for `owner`'s agent and grant it, as if the
    /// user said yes in the dialog.
    fn lend_workspace(handle: &HeadlessHandle, owner: &str, name: &str) -> Result<(), String> {
        let (owner, name) = (owner.to_string(), name.to_string());
        handle.query(move |state| {
            let (tx, mut rx) = tokio::sync::oneshot::channel();
            otto::screenshare::handle_screenshare_command(
                state,
                otto::screenshare::CompositorCommand::FindAgentWorkspace {
                    owner: owner.clone(),
                    name,
                    response_tx: tx,
                },
            );
            let found = rx.try_recv().expect("answered at once")?;
            let (tx, mut rx) = tokio::sync::oneshot::channel();
            otto::screenshare::handle_screenshare_command(
                state,
                otto::screenshare::CompositorCommand::GrantAgentWorkspace {
                    owner,
                    output: found.output,
                    workspace: found.workspace,
                    response_tx: tx,
                },
            );
            rx.try_recv().expect("answered at once").map(|_| ())
        })
    }

    /// Names of the primary output's workspaces, in order.
    fn workspace_names(handle: &HeadlessHandle) -> Vec<String> {
        handle.query(|state| {
            let ows = state
                .workspaces
                .primary_output_workspaces()
                .expect("output");
            ows.workspace_views
                .iter()
                .map(|view| view.display_name())
                .collect()
        })
    }

    /// The window on `seat`'s keyboard, by title.
    fn keyboard_focus_of(handle: &HeadlessHandle, seat: &str) -> Option<String> {
        let seat = seat.to_string();
        handle.query(move |state| {
            use smithay::wayland::seat::WaylandFocus;
            let keyboard = state.agent_seat(&seat)?.seat.get_keyboard()?;
            let focus = keyboard.current_focus()?;
            let surface = focus.wl_surface()?.into_owned();
            state
                .workspaces
                .spaces_elements()
                .find(|w| w.wl_surface().is_some_and(|s| *s == surface))
                .map(|w| w.xdg_title())
        })
    }

    /// The user's window "User" on the current workspace, and the agent's
    /// "Mine" on the agent's own workspace, overlapping it. Returns a point
    /// inside both.
    fn user_and_agent_windows(
        handle: &HeadlessHandle,
        user: &mut TestClient,
        mine: &mut TestClient,
    ) -> (i32, i32) {
        map_window(handle, user, "User");
        map_window(handle, mine, "Mine");
        let position = workspace_names(handle).len() - 1;
        handle.move_window_to_workspace("Mine", position);
        handle.settle(200);
        let _ = user.roundtrip();
        let _ = mine.roundtrip();
        let (ux, uy, uw, uh) = handle.window_logical_geometry("User").expect("User");
        let (mx, my, mw, mh) = handle.window_logical_geometry("Mine").expect("Mine");
        let (left, top) = (ux.max(mx), uy.max(my));
        let (right, bottom) = ((ux + uw).min(mx + mw), (uy + uh).min(my + mh));
        assert!(
            left + 20 < right && top + 20 < bottom,
            "the windows must overlap"
        );
        (left + 10, top + 10)
    }

    /// An agent asking for a workspace gets a new one, named after it; the
    /// user stays where they are.
    #[test]
    #[serial]
    fn an_agent_gets_a_workspace_of_its_own() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        request_seat(&handle, "Claude", ":1.10").expect("seat");
        let before = workspace_names(&handle);
        let current = handle.current_workspace_index();

        request_workspace(&handle, ":1.10").expect("workspace");
        let after = workspace_names(&handle);
        assert_eq!(after.len(), before.len() + 1);
        assert_eq!(after.last().map(String::as_str), Some("Claude"));
        assert_eq!(
            handle.current_workspace_index(),
            current,
            "the user was moved"
        );

        // Asking again gives the same one.
        request_workspace(&handle, ":1.10").expect("workspace");
        assert_eq!(workspace_names(&handle).len(), after.len());
        // No seat, no workspace.
        assert!(request_workspace(&handle, ":1.99").is_err());
        handle.stop();
    }

    /// The agent works on its hidden workspace: its click lands on its own
    /// window, not on the user's window drawn at the same place.
    #[test]
    #[serial]
    fn agent_input_reaches_its_hidden_workspace() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        request_seat(&handle, "Claude", ":1.10").expect("seat");
        request_workspace(&handle, ":1.10").expect("workspace");
        let mut user = TestClient::connect(&handle.socket_name).expect("client");
        let mut mine = TestClient::connect(&handle.socket_name).expect("client");
        let (x, y) = user_and_agent_windows(&handle, &mut user, &mut mine);

        let user_focus = || {
            handle.query(|state| {
                state
                    .seat
                    .get_keyboard()?
                    .current_focus()
                    .map(|f| format!("{f:?}"))
            })
        };
        let before = user_focus();

        let mut driver = Driver::connect_as_agent(&handle, ":1.10");
        let pointer = driver.pointer_on(driver.seat_index("agent-1").unwrap());
        move_to(&handle, &pointer, x, y);
        driver.settle(&handle);
        click(&pointer);
        driver.settle(&handle);

        assert_eq!(
            keyboard_focus_of(&handle, "agent-1").as_deref(),
            Some("Mine")
        );
        assert_eq!(user_focus(), before, "the agent moved the user's focus");
        assert_eq!(
            handle.current_workspace_index(),
            0,
            "the agent switched workspace"
        );

        // Watched by the agent: its window draws at full rate while hidden.
        let states = handle.window_throttle_states();
        assert_eq!(
            states.get("Mine"),
            Some(&otto::state::window_throttle::WindowThrottleState::Captured)
        );

        drop(driver);
        handle.stop();
    }

    /// A seat with no grant reaches nothing, and neither does one whose
    /// agent gave its workspace back — which stays, for the user.
    #[test]
    #[serial]
    fn an_agent_without_a_grant_reaches_nothing() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        request_seat(&handle, "Claude", ":1.10").expect("seat");
        let mut user = TestClient::connect(&handle.socket_name).expect("client");
        map_window(&handle, &mut user, "User");
        let (ux, uy, _, _) = handle.window_logical_geometry("User").expect("User");

        let mut driver = Driver::connect_as_agent(&handle, ":1.10");
        let pointer = driver.pointer_on(driver.seat_index("agent-1").unwrap());
        move_to(&handle, &pointer, ux + 20, uy + 20);
        driver.settle(&handle);
        click(&pointer);
        driver.settle(&handle);
        assert_eq!(keyboard_focus_of(&handle, "agent-1"), None);

        request_workspace(&handle, ":1.10").expect("workspace");
        let count = workspace_names(&handle).len();
        let released = handle.query(|state| state.release_own_workspace(":1.10"));
        assert!(released);
        assert_eq!(workspace_names(&handle).len(), count, "the workspace went");
        click(&pointer);
        driver.settle(&handle);
        assert_eq!(keyboard_focus_of(&handle, "agent-1"), None);

        drop(driver);
        handle.stop();
    }

    /// An agent that reconnects under its name gets its workspace back.
    #[test]
    #[serial]
    fn a_returning_agent_gets_its_workspace_back() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        request_seat(&handle, "Claude", ":1.10").expect("seat");
        request_workspace(&handle, ":1.10").expect("workspace");
        let count = workspace_names(&handle).len();
        release_seats(&handle, ":1.10");
        request_seat(&handle, "Claude", ":1.20").expect("seat");
        request_workspace(&handle, ":1.20").expect("workspace");
        assert_eq!(
            workspace_names(&handle).len(),
            count,
            "a second workspace was made"
        );
        handle.stop();
    }

    /// The border look on the agent's own workspace, if any.
    fn agent_frame(handle: &HeadlessHandle, owner: &str) -> Option<(Vec<String>, bool)> {
        let owner = owner.to_string();
        handle.query(move |state| {
            let agent = state
                .agent_seats
                .iter()
                .find(|agent| agent.owner.as_deref() == Some(owner.as_str()))?;
            let otto::state::agent_seats::Grant::Workspace {
                output, workspace, ..
            } = agent.grant.clone()?;
            state
                .workspaces
                .agent_frame_look(&output, workspace)
                .map(|look| (look.names.clone(), look.chip))
        })
    }

    /// A workspace an agent holds is framed, with its name on the chip; the
    /// frame goes when the agent lets go.
    #[test]
    #[serial]
    fn an_agent_workspace_is_framed() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        request_seat(&handle, "Claude", ":1.10").expect("seat");
        request_workspace(&handle, ":1.10").expect("workspace");
        handle.settle(200);
        assert_eq!(
            agent_frame(&handle, ":1.10"),
            Some((vec!["Claude".to_string()], true))
        );

        let frames = || {
            handle.query(|state| {
                let output = state.workspaces.outputs().next().unwrap().name();
                let ows = state.workspaces.output_workspaces.get(&output).unwrap();
                ows.workspace_views
                    .iter()
                    .filter(|view| {
                        state
                            .workspaces
                            .agent_frame_look(&output, view.index)
                            .is_some()
                    })
                    .count()
            })
        };
        assert_eq!(frames(), 1);
        // Its selector preview carries the agent's mark.
        let marks = || {
            handle.query(|state| {
                let ows = state.workspaces.primary_output_workspaces().unwrap();
                ows.workspace_views
                    .iter()
                    .filter_map(|view| view.agent_mark())
                    .map(|(_, name)| name)
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(marks(), vec!["Claude".to_string()]);

        handle.query(|state| state.release_own_workspace(":1.10"));
        handle.settle(200);
        assert_eq!(frames(), 0, "the frame outlived the grant");
        assert!(marks().is_empty(), "the preview kept the agent's mark");
        handle.stop();
    }

    /// Stop on the chip ends the grant: the agent reaches nothing after.
    #[test]
    #[serial]
    fn stop_on_the_chip_ends_the_seat() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        request_seat(&handle, "Claude", ":1.10").expect("seat");
        request_workspace(&handle, ":1.10").expect("workspace");
        handle.settle(200);
        let workspaces = workspace_names(&handle).len();

        let stopped = handle.query(|state| {
            let output = state.workspaces.outputs().next().cloned().unwrap();
            let geometry = state.workspaces.output_geometry(&output).unwrap();
            // Show the agent's workspace, where the chip is.
            let last = state.workspaces.output_workspaces[&output.name()]
                .workspace_views
                .len()
                - 1;
            state
                .workspaces
                .set_workspace_for_output(&output, last, None);
            let chip =
                otto::agent_cursor::chip_geometry(&["Claude".to_string()], geometry.size.w as f32);
            let at = |x: f32, y: f32| {
                smithay::utils::Point::<f64, smithay::utils::Logical>::from((
                    geometry.loc.x as f64 + x as f64,
                    geometry.loc.y as f64 + y as f64,
                ))
            };
            // Beside the chip: nothing happens.
            let missed = state.press_agent_stop(at(5.0, 5.0));
            let (x, y, w, h) = chip.stop;
            let hit = state.press_agent_stop(at(x + w / 2.0, y + h / 2.0));
            (missed, hit)
        });
        assert_eq!(stopped, (false, true));
        // The seat is gone; the workspace stays for the user.
        assert!(handle.query(|state| state.agent_seat("agent-1").is_none()));
        assert_eq!(
            workspace_names(&handle).len(),
            workspaces,
            "the workspace went with the agent"
        );
        handle.stop();
    }

    /// An agent can launch only onto a workspace of its own; the launch's
    /// token is tied to it for as long as it holds that workspace.
    #[test]
    #[serial]
    fn an_agent_launches_onto_its_own_workspace_only() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        request_seat(&handle, "Claude", ":1.10").expect("seat");
        let argv = vec!["true".to_string()];
        let launch = |argv: Vec<String>| {
            handle.query(move |state| {
                state
                    .launch_on_own_workspace(":1.10", &argv)
                    .map_err(|e| e.to_string())
            })
        };
        assert!(
            launch(argv.clone()).is_err(),
            "launched without a workspace"
        );

        request_workspace(&handle, ":1.10").expect("workspace");
        assert!(launch(argv.clone()).is_ok());
        let token_seat = || {
            handle.query(|state| {
                let token = state.agent_launch_tokens.keys().next().cloned()?;
                state.agent_seat_for_token(&token)
            })
        };
        assert_eq!(token_seat().as_deref(), Some("agent-1"));

        // Once the workspace is the user's, the token places nothing.
        handle.query(|state| state.release_own_workspace(":1.10"));
        assert_eq!(token_seat(), None);
        handle.stop();
    }

    fn capture(handle: &HeadlessHandle, owner: &str, selector: &str) -> Result<String, String> {
        let (owner, selector) = (owner.to_string(), selector.to_string());
        handle.query(move |state| {
            let (tx, mut rx) = tokio::sync::oneshot::channel();
            otto::screenshare::handle_screenshare_command(
                state,
                otto::screenshare::CompositorCommand::CaptureWorkspace {
                    owner,
                    workspace: selector,
                    response_tx: tx,
                },
            );
            rx.try_recv().expect("answered at once")
        })
    }

    /// An agent captures its own workspace by id or name, although it is not
    /// shown; the PNG is the size of its output.
    #[test]
    #[serial]
    fn an_agent_captures_its_own_workspace_by_id_name_or_none() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        request_seat(&handle, "Claude", ":1.10").expect("seat");
        request_workspace(&handle, ":1.10").expect("workspace");
        let (id, name, mode) = handle.query(|state| {
            let workspaces = state.workspaces_json();
            let own = workspaces
                .as_array()
                .unwrap()
                .iter()
                .find(|w| w["visible"] == false && w["name"] == "Claude")
                .cloned()
                .expect("the agent's hidden workspace");
            let output = state.workspaces.outputs().next().unwrap();
            let mode = output.current_mode().unwrap().size;
            (
                own["id"].as_u64().unwrap(),
                own["name"].as_str().unwrap().to_string(),
                (mode.w as u32, mode.h as u32),
            )
        });
        // By id, by name, or naming none for its own.
        for selector in [id.to_string(), name, String::new()] {
            let path =
                capture(&handle, ":1.10", &selector).unwrap_or_else(|e| panic!("{selector}: {e}"));
            let png = std::fs::read(&path).expect("the capture was written");
            assert_eq!(&png[1..4], b"PNG");
            let size = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
            assert_eq!((size(16), size(20)), mode, "{selector}: wrong size");
            let _ = std::fs::remove_file(path);
        }
        assert!(capture(&handle, ":1.10", "no such workspace").is_err());
        handle.stop();
    }

    /// No one captures a workspace that is not theirs: not an agent without
    /// one, not an agent naming the user's, not another agent.
    #[test]
    #[serial]
    fn only_the_granted_workspace_is_captured() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let users = handle.query(|state| {
            state.workspaces_json().as_array().unwrap()[0]["id"]
                .as_u64()
                .unwrap()
                .to_string()
        });
        assert!(capture(&handle, ":1.99", &users).is_err(), "no seat");
        request_seat(&handle, "Claude", ":1.10").expect("seat");
        assert!(capture(&handle, ":1.10", &users).is_err(), "no grant");
        assert!(
            capture(&handle, ":1.10", "").is_err(),
            "none, without a grant"
        );
        request_workspace(&handle, ":1.10").expect("workspace");
        assert!(capture(&handle, ":1.10", &users).is_err(), "the user's");
        request_seat(&handle, "Codex", ":1.11").expect("seat");
        request_workspace(&handle, ":1.11").expect("workspace");
        assert!(capture(&handle, ":1.11", "Claude").is_err(), "another's");
        assert!(capture(&handle, ":1.11", "Codex").is_ok());
        handle.stop();
    }

    /// Nothing is captured while the session is locked, not even an agent's
    /// own workspace.
    #[test]
    #[serial]
    fn a_locked_session_is_not_captured() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        request_seat(&handle, "Claude", ":1.10").expect("seat");
        request_workspace(&handle, ":1.10").expect("workspace");
        let mut locker = Driver::connect_as_locker(&handle);
        let lock = locker
            .state
            .lock_manager
            .as_ref()
            .expect("ext_session_lock_manager_v1 missing")
            .lock(&locker.qh, ());
        locker.settle(&handle);
        assert!(handle.query(|state| state.is_session_locked()));
        assert!(capture(&handle, ":1.10", "Claude").is_err());
        drop(lock);
        handle.stop();
    }
}
