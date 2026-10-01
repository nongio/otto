//! Session lock lifecycle (`ext-session-lock-v1`, `src/lock.rs`), headless.
//!
//! The lock is the compositor's: the blank goes up before any locker has
//! asked, a locker that dies leaves the session locked, and the next locker
//! Otto starts takes the standing lock over instead of being refused.
//! Also: a popup grab takes the keyboard only after a press on its own
//! client, never while locked.

#[cfg(feature = "headless")]
mod session_lock_tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use wayland_client::{
        delegate_noop,
        protocol::{wl_output, wl_registry},
        Connection, Dispatch, EventQueue, QueueHandle,
    };
    use wayland_protocols::ext::session_lock::v1::client::{
        ext_session_lock_manager_v1, ext_session_lock_surface_v1, ext_session_lock_v1,
    };

    /// What a lock object has been told.
    #[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
    enum Told {
        #[default]
        Nothing,
        Locked,
        Finished,
    }

    #[derive(Default)]
    struct LockerState {
        manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
        output: Option<wl_output::WlOutput>,
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for LockerState {
        fn event(
            state: &mut Self,
            registry: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            _: &(),
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global {
                name, interface, ..
            } = event
            {
                if interface == "ext_session_lock_manager_v1" {
                    state.manager = Some(registry.bind(name, 1, qh, ()));
                }
                if interface == "wl_output" && state.output.is_none() {
                    state.output = Some(registry.bind(name, 1, qh, ()));
                }
            }
        }
    }

    impl Dispatch<ext_session_lock_v1::ExtSessionLockV1, Arc<Mutex<Told>>> for LockerState {
        fn event(
            _: &mut Self,
            _: &ext_session_lock_v1::ExtSessionLockV1,
            event: ext_session_lock_v1::Event,
            told: &Arc<Mutex<Told>>,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                ext_session_lock_v1::Event::Locked => *told.lock().unwrap() = Told::Locked,
                ext_session_lock_v1::Event::Finished => *told.lock().unwrap() = Told::Finished,
                _ => {}
            }
        }
    }
    delegate_noop!(LockerState: ext_session_lock_manager_v1::ExtSessionLockManagerV1);
    delegate_noop!(LockerState: ignore wl_output::WlOutput);
    delegate_noop!(LockerState: ignore ext_session_lock_surface_v1::ExtSessionLockSurfaceV1);

    /// A locker connected the way Otto connects the one it starts.
    struct Locker {
        conn: Connection,
        queue: EventQueue<LockerState>,
        qh: QueueHandle<LockerState>,
        state: LockerState,
    }

    impl Locker {
        fn connect(handle: &HeadlessHandle) -> Self {
            let stream =
                handle.query(|state| state.connect_locker_client().expect("connect the locker").1);
            let conn = Connection::from_socket(stream).expect("wayland connection");
            let mut queue = conn.new_event_queue();
            let qh = queue.handle();
            conn.display().get_registry(&qh, ());
            let mut state = LockerState::default();
            queue.roundtrip(&mut state).expect("bind globals");
            Self {
                conn,
                queue,
                qh,
                state,
            }
        }

        /// Ask for the lock; the returned cell records the answer.
        fn lock(
            &mut self,
            handle: &HeadlessHandle,
        ) -> (ext_session_lock_v1::ExtSessionLockV1, Arc<Mutex<Told>>) {
            let told = Arc::new(Mutex::new(Told::Nothing));
            let lock = self
                .state
                .manager
                .as_ref()
                .expect("ext_session_lock_manager_v1 missing")
                .lock(&self.qh, told.clone());
            self.settle(handle);
            (lock, told)
        }

        fn settle(&mut self, handle: &HeadlessHandle) {
            // The blank takes its slide to land before `locked` is sent.
            for _ in 0..8 {
                self.conn.flush().expect("flush");
                handle.wait(Duration::from_millis(120));
                handle.settle(20);
                self.queue.roundtrip(&mut self.state).expect("roundtrip");
            }
        }
    }

    /// A locker that also puts up a lock surface, with a keyboard of its own
    /// to tell which keys reach it.
    struct LockScreen {
        client: TestClient,
        queue: EventQueue<LockerState>,
        qh: QueueHandle<LockerState>,
        state: LockerState,
    }

    impl LockScreen {
        fn connect(handle: &HeadlessHandle) -> Self {
            let stream =
                handle.query(|state| state.connect_locker_client().expect("connect the locker").1);
            let client = TestClient::from_stream(stream).expect("locker client");
            let conn = client.conn.clone();
            let mut queue = conn.new_event_queue();
            let qh = queue.handle();
            conn.display().get_registry(&qh, ());
            let mut state = LockerState::default();
            queue.roundtrip(&mut state).expect("bind globals");
            Self {
                client,
                queue,
                qh,
                state,
            }
        }

        /// Lock, and put a lock surface on the output.
        fn lock(
            &mut self,
            handle: &HeadlessHandle,
        ) -> (
            ext_session_lock_v1::ExtSessionLockV1,
            ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
        ) {
            let told = Arc::new(Mutex::new(Told::Nothing));
            let lock = self
                .state
                .manager
                .as_ref()
                .expect("ext_session_lock_manager_v1 missing")
                .lock(&self.qh, told.clone());
            let surface = self.client.create_surface();
            let lock_surface = lock.get_lock_surface(
                &surface,
                self.state.output.as_ref().expect("wl_output"),
                &self.qh,
                (),
            );
            self.settle(handle);
            assert_eq!(*told.lock().unwrap(), Told::Locked);
            (lock, lock_surface)
        }

        fn settle(&mut self, handle: &HeadlessHandle) {
            for _ in 0..8 {
                self.client.conn.flush().expect("flush");
                handle.wait(Duration::from_millis(120));
                handle.settle(20);
                self.queue.roundtrip(&mut self.state).expect("roundtrip");
                let _ = self.client.roundtrip();
            }
        }
    }

    const KEY_A: u32 = 30;

    fn press_a(handle: &HeadlessHandle) {
        handle.key(KEY_A, true);
        handle.key(KEY_A, false);
        handle.wait(Duration::from_millis(50));
    }

    fn popup_has_keyboard(handle: &HeadlessHandle) -> bool {
        handle.query(|state| {
            matches!(
                state.seat.get_keyboard().and_then(|k| k.current_focus()),
                Some(otto::focus::KeyboardFocusTarget::Popup(_))
            )
        })
    }

    /// Map a window titled `title` and wait for it to be on screen.
    fn window(
        handle: &HeadlessHandle,
        client: &mut TestClient,
        title: &str,
    ) -> Arc<Mutex<otto_kit::testing::TestToplevel>> {
        let toplevel = client.create_toplevel(title, 300, 200);
        handle.wait(Duration::from_millis(120));
        client.roundtrip().expect("roundtrip");
        toplevel.lock().unwrap().commit_frame();
        client.roundtrip().expect("roundtrip");
        handle.wait(Duration::from_millis(120));
        handle.settle(20);
        toplevel
    }

    /// Click the middle of the window titled `title`; the press serial the
    /// client was sent.
    fn click(handle: &HeadlessHandle, client: &mut TestClient, title: &str) -> u32 {
        let (x, y, w, h) = handle
            .window_logical_geometry(title)
            .expect("the window is mapped");
        handle.pointer_move(x as f64 + w as f64 / 2.0, y as f64 + h as f64 / 2.0);
        handle.pointer_click();
        handle.wait(Duration::from_millis(50));
        handle.settle(20);
        client.roundtrip().expect("roundtrip");
        client
            .state
            .last_button_serial
            .expect("the press reached the client")
    }

    fn start() -> HeadlessHandle {
        // A lock the compositor starts runs `lock.locker_command`; pointing
        // it nowhere keeps a real otto-lock from being launched by a test.
        std::env::set_var("OTTO_LOCKER_COMMAND", "/nonexistent/otto-lock-under-test");
        HeadlessHandle::start(HeadlessConfig::default())
    }

    fn locked(handle: &HeadlessHandle) -> bool {
        handle.query(|state| state.is_session_locked())
    }

    fn blank_hidden(handle: &HeadlessHandle) -> Option<bool> {
        let name = handle.query(|state| {
            state
                .workspaces
                .outputs()
                .next()
                .map(|output| output.name())
                .expect("an output")
        });
        handle.is_layer_hidden(&format!("lock_plane_{name}"))
    }

    /// A locker that dies leaves the session locked, and the next one takes
    /// the lock over: it is told `locked`, not `finished`, and can unlock.
    #[test]
    #[serial]
    fn a_new_locker_takes_over_from_one_that_died() {
        let handle = start();

        let mut first = Locker::connect(&handle);
        let (_lock, told) = first.lock(&handle);
        assert_eq!(*told.lock().unwrap(), Told::Locked);

        drop(first);
        handle.wait(Duration::from_millis(200));
        handle.settle(20);
        assert!(locked(&handle), "the session unlocked when its locker died");
        assert_eq!(blank_hidden(&handle), Some(false), "the blank came down");

        let mut second = Locker::connect(&handle);
        let (lock, told) = second.lock(&handle);
        assert_eq!(
            *told.lock().unwrap(),
            Told::Locked,
            "the new locker was refused the standing lock"
        );

        lock.unlock_and_destroy();
        second.settle(&handle);
        assert!(!locked(&handle), "the new locker could not unlock");

        handle.stop();
    }

    /// While the locker holding the lock is alive, nobody else gets it.
    #[test]
    #[serial]
    fn a_second_locker_is_refused_while_the_first_lives() {
        let handle = start();

        let mut first = Locker::connect(&handle);
        let (first_lock, told) = first.lock(&handle);
        assert_eq!(*told.lock().unwrap(), Told::Locked);

        let mut second = Locker::connect(&handle);
        let (_lock, told) = second.lock(&handle);
        assert_eq!(*told.lock().unwrap(), Told::Finished);

        first_lock.unlock_and_destroy();
        first.settle(&handle);
        assert!(!locked(&handle));

        handle.stop();
    }

    /// Asking for a lock blanks the screen and cuts the session off at once,
    /// before any locker has connected — and a locker that fails to start
    /// leaves it blank rather than showing the desktop.
    #[test]
    #[serial]
    fn locking_blanks_before_the_locker_arrives() {
        let handle = start();
        assert_eq!(blank_hidden(&handle), Some(true));

        handle.query(|state| state.lock_session());
        assert!(locked(&handle));
        assert_eq!(blank_hidden(&handle), Some(false));

        // The configured locker does not exist; the session stays locked.
        handle.wait(Duration::from_millis(600));
        handle.settle(20);
        assert!(locked(&handle), "a locker that failed to start unlocked");
        assert_eq!(blank_hidden(&handle), Some(false));

        let mut locker = Locker::connect(&handle);
        let (lock, told) = locker.lock(&handle);
        assert_eq!(*told.lock().unwrap(), Told::Locked);

        lock.unlock_and_destroy();
        locker.settle(&handle);
        assert!(!locked(&handle));

        handle.stop();
    }

    /// A monitor plugged in while locked has its blank up as it is added,
    /// before any frame is drawn on it; it comes down
    /// with the others on unlock.
    #[test]
    #[serial]
    fn a_monitor_plugged_in_while_locked_is_blank_from_the_start() {
        use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};

        let handle = start();
        let mut locker = Locker::connect(&handle);
        let (lock, _told) = locker.lock(&handle);

        // Added and looked at in one turn of the loop: no frame in between.
        let hidden_at_birth = handle.query(|state| {
            let mode = Mode {
                size: (800, 600).into(),
                refresh: 60_000,
            };
            let output = Output::new(
                "HOTPLUG-1".into(),
                PhysicalProperties {
                    size: (0, 0).into(),
                    subpixel: Subpixel::Unknown,
                    make: "Otto".into(),
                    model: "Headless".into(),
                    serial_number: "2".into(),
                },
            );
            output.change_current_state(Some(mode), None, None, Some((10_000, 0).into()));
            output.set_preferred(mode);
            state.workspaces.map_output(&output, (10_000, 0));
            state
                .workspaces
                .output_workspaces
                .get("HOTPLUG-1")
                .map(|ows| ows.lock_plane.hidden())
        });
        assert_eq!(
            hidden_at_birth,
            Some(false),
            "the new monitor showed the desktop"
        );

        lock.unlock_and_destroy();
        locker.settle(&handle);
        assert!(!locked(&handle));
        let still_blanking = handle.query(|state| state.workspaces.blank_new_outputs);
        assert!(
            !still_blanking,
            "outputs added after the unlock would be blanked"
        );

        handle.stop();
    }

    /// A client that was not the one the user just pressed on cannot take
    /// the keyboard with a popup grab, whatever serial it names: it is sent
    /// `popup_done`, and the keys keep going where the user put them. The
    /// client the user did press on still can.
    #[test]
    #[serial]
    fn a_popup_grab_needs_a_press_on_its_own_client() {
        let handle = start();

        let mut intruder = TestClient::connect(&handle.socket_name).expect("client");
        let intruder_window = window(&handle, &mut intruder, "intruder");
        let mut owner = TestClient::connect(&handle.socket_name).expect("client");
        let owner_window = window(&handle, &mut owner, "owner");

        let serial = click(&handle, &mut owner, "owner");
        assert_eq!(handle.focused_window_title().as_deref(), Some("owner"));
        let _ = intruder.roundtrip();

        // The serial the owner was just sent, and one made up.
        for bogus in [serial, 1] {
            let popup = intruder.create_grabbing_popup(&intruder_window, 100, 80, bogus);
            handle.wait(Duration::from_millis(50));
            handle.settle(20);
            intruder.roundtrip().expect("roundtrip");
            assert!(
                popup.lock().unwrap().done,
                "a popup grab with serial {bogus} was not refused"
            );
            assert!(
                !popup_has_keyboard(&handle),
                "the refused popup took the keyboard"
            );
            assert_eq!(handle.focused_window_title().as_deref(), Some("owner"));
        }

        press_a(&handle);
        owner.roundtrip().expect("roundtrip");
        intruder.roundtrip().expect("roundtrip");
        assert!(owner.state.keys.contains(&(KEY_A, true)));
        assert!(
            intruder.state.keys.is_empty(),
            "the intruder received keys: {:?}",
            intruder.state.keys
        );

        // The owner's own menu, opened with the serial of the click that
        // opens it, grabs. (Not the first click's: a key pressed since is the
        // user's latest press, and the serial has to be no older than that.)
        let serial = click(&handle, &mut owner, "owner");
        let menu = owner.create_grabbing_popup(&owner_window, 100, 80, serial);
        handle.wait(Duration::from_millis(50));
        handle.settle(20);
        owner.roundtrip().expect("roundtrip");
        assert!(
            !menu.lock().unwrap().done,
            "a genuine menu grab was refused"
        );
        assert!(
            popup_has_keyboard(&handle),
            "a genuine menu did not get the keyboard"
        );

        handle.stop();
    }

    /// While locked no popup grab is honoured, not even with the serial of
    /// the user's last real click, and keys reach the lock surface.
    #[test]
    #[serial]
    fn a_locked_session_refuses_popup_grabs() {
        let handle = start();

        let mut app = TestClient::connect(&handle.socket_name).expect("client");
        let app_window = window(&handle, &mut app, "app");
        let serial = click(&handle, &mut app, "app");

        let mut locker = LockScreen::connect(&handle);
        let (lock, _lock_surface) = locker.lock(&handle);

        let popup = app.create_grabbing_popup(&app_window, 100, 80, serial);
        handle.wait(Duration::from_millis(50));
        handle.settle(20);
        app.roundtrip().expect("roundtrip");
        assert!(
            popup.lock().unwrap().done,
            "a popup grab was honoured while locked"
        );
        assert!(!popup_has_keyboard(&handle));

        let before = app.state.keys.len();
        press_a(&handle);
        locker.settle(&handle);
        app.roundtrip().expect("roundtrip");
        assert!(
            locker.client.state.keys.contains(&(KEY_A, true)),
            "the lock surface did not get the key"
        );
        assert_eq!(
            app.state.keys.len(),
            before,
            "the app got a key while locked"
        );

        lock.unlock_and_destroy();
        locker.settle(&handle);
        assert!(!locked(&handle));

        handle.stop();
    }
}
