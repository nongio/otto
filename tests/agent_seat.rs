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
    use wayland_protocols_wlr::virtual_pointer::v1::client::{
        zwlr_virtual_pointer_manager_v1, zwlr_virtual_pointer_v1,
    };

    const BTN_LEFT: u32 = 0x110;
    const AGENT: &str = otto::agent_cursor::AGENT_SEAT_NAME;

    /// Every seat, in the order the compositor advertised them.
    #[derive(Default)]
    struct DriverState {
        seats: Vec<wl_seat::WlSeat>,
        /// `wl_seat.name` of each seat, by the same index.
        seat_names: Vec<Option<String>>,
        pointer_manager: Option<zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1>,
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
    delegate_noop!(DriverState: ext_session_lock_manager_v1::ExtSessionLockManagerV1);
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
            let stream = std::os::unix::net::UnixStream::connect(path).expect("connect");
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

        fn settle(&mut self, handle: &HeadlessHandle) {
            self.conn.flush().expect("flush");
            handle.wait(Duration::from_millis(120));
            self.queue.roundtrip(&mut self.state).expect("roundtrip");
            handle.settle(200);
        }
    }

    /// A compositor with the agent seat advertised, as `enabled = true` does.
    fn start() -> HeadlessHandle {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        handle.query(|state| state.enable_agent_seat());
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

    /// The agent seat comes second, so clients that take the first seat keep
    /// the user's.
    #[test]
    #[serial]
    fn the_agent_seat_is_advertised_after_the_users() {
        let handle = start();
        let driver = Driver::connect(&handle);
        assert_eq!(
            driver.state.seats.len(),
            2,
            "the user's seat and the agent's"
        );
        drop(driver);
        handle.stop();
    }

    /// Motion on the agent seat moves the agent pointer and shows its cursor;
    /// the user's cursor stays where it was.
    #[test]
    #[serial]
    fn agent_motion_leaves_the_user_pointer_alone() {
        let handle = start();
        let mut driver = Driver::connect(&handle);
        let before = user_pointer(&handle);
        assert_eq!(
            agent_cursor_opacity(&handle),
            0.0,
            "the agent cursor is hidden until the agent first moves"
        );

        let pointer = driver.pointer_on(1);
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

    /// An agent click gives the agent seat's keyboard to the clicked window;
    /// the user's focused window keeps the user's keyboard.
    #[test]
    #[serial]
    fn agent_click_does_not_take_the_users_focus() {
        let handle = start();
        let mut background = TestClient::connect(&handle.socket_name).expect("client");
        let mut foreground = TestClient::connect(&handle.socket_name).expect("client");
        map_window(&handle, &mut background, "Background");
        map_window(&handle, &mut foreground, "Foreground");
        let _ = background.roundtrip();
        let _ = foreground.roundtrip();
        assert!(
            foreground.state.keyboard_focused,
            "last mapped starts focused"
        );

        let (bx, by, _, _) = handle
            .window_logical_geometry("Background")
            .expect("background window mapped");
        let (fx, fy, _, _) = handle
            .window_logical_geometry("Foreground")
            .expect("foreground window mapped");
        let (target_x, target_y) = (bx + 8, by + 8);
        assert!(
            target_x < fx || target_y < fy,
            "the background window must peek out from under the foreground one"
        );

        let mut driver = Driver::connect(&handle);
        let pointer = driver.pointer_on(1);
        move_to(&handle, &pointer, target_x, target_y);
        driver.settle(&handle);
        click(&pointer);
        driver.settle(&handle);

        let _ = background.roundtrip();
        let _ = foreground.roundtrip();
        assert!(
            foreground.state.keyboard_focused,
            "the user's window lost the user's keyboard"
        );
        assert!(!background.state.keyboard_focused);

        let agent_focus = agent_keyboard_focus(&handle);
        assert_eq!(agent_focus.as_deref(), Some("Background"));

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
        let mut driver = Driver::connect(&handle);
        let pointer = driver.pointer_on(1);
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
        let mut window = TestClient::connect(&handle.socket_name).expect("client");
        map_window(&handle, &mut window, "Target");
        let (x, y, _, _) = handle
            .window_logical_geometry("Target")
            .expect("window mapped");

        let mut driver = Driver::connect(&handle);
        let pointer = driver.pointer_on(1);
        move_to(&handle, &pointer, x + 20, y + 20);
        driver.settle(&handle);
        click(&pointer);
        driver.settle(&handle);
        assert_eq!(agent_keyboard_focus(&handle).as_deref(), Some("Target"));

        let lock = driver
            .state
            .lock_manager
            .as_ref()
            .expect("ext_session_lock_manager_v1 missing")
            .lock(&driver.qh, ());
        driver.settle(&handle);
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

        let driver = Driver::connect(&handle);
        assert_eq!(
            driver.state.seats.len(),
            3,
            "the user's seat and two agents'"
        );
        assert!(driver.seat_index("agent-1").is_some());
        assert!(driver.seat_index("agent-2").is_some());

        drop(driver);
        handle.stop();
    }

    /// A pointer on one agent's seat moves that agent's pointer only.
    #[test]
    #[serial]
    fn agents_move_their_own_pointers() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        request_seat(&handle, "Claude", ":1.10").expect("seat");
        request_seat(&handle, "Helper", ":1.11").expect("seat");
        let mut driver = Driver::connect(&handle);
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

    /// Releasing removes the seat; the same agent coming back in the same
    /// session gets the same seat name and colour.
    #[test]
    #[serial]
    fn a_returning_agent_gets_its_seat_back() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let seat = request_seat(&handle, "Claude", ":1.10").expect("seat");
        request_seat(&handle, "Helper", ":1.11").expect("seat");

        let mut driver = Driver::connect(&handle);
        let pointer = driver.pointer_on(driver.seat_index("agent-1").unwrap());
        release_seats(&handle, ":1.10");
        handle.settle(200);
        assert!(handle.query(|state| state.agent_seat("agent-1").is_none()));

        // A pointer left on the removed seat drives nothing, not the user's.
        let before = user_pointer(&handle);
        move_to(&handle, &pointer, 300, 300);
        driver.settle(&handle);
        assert_eq!(user_pointer(&handle), before);
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
}
