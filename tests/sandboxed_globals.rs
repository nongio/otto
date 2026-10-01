//! What a sandboxed Wayland client is not offered.
//!
//! A Flatpak connects through a `wp_security_context_v1` listener. Such a
//! client must not see the globals that would let it fake the lock screen,
//! read or inject input, snoop the clipboard, capture the screen, drive other
//! apps' windows or change the display's gamma. The user's own clients keep
//! seeing all of them.
//!
//! An agent's own connection is kept off them too, but for the virtual input
//! it drives its seat with. Session lock goes further: only the locker Otto
//! started is offered it.

#[cfg(feature = "headless")]
mod sandboxed_globals_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use serial_test::serial;
    use std::os::fd::AsFd;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;
    use wayland_client::{protocol::wl_registry, Connection, Dispatch, EventQueue, QueueHandle};
    use wayland_protocols::wp::security_context::v1::client::{
        wp_security_context_manager_v1::WpSecurityContextManagerV1,
        wp_security_context_v1::WpSecurityContextV1,
    };

    /// Never offered to a client that came in through a security context.
    const HIDDEN_FROM_SANDBOXED: &[&str] = &[
        "zwlr_layer_shell_v1",
        "zwp_keyboard_shortcuts_inhibit_manager_v1",
        "ext_foreign_toplevel_list_v1",
        "otto_dock_manager_v1",
        "otto_text_cursor_manager_v1",
        "zwlr_gamma_control_manager_v1",
        "zwlr_screencopy_manager_v1",
        "zwlr_virtual_pointer_manager_v1",
        "zwp_virtual_keyboard_manager_v1",
        "zwlr_data_control_manager_v1",
        "ext_data_control_manager_v1",
        "zwlr_foreign_toplevel_manager_v1",
        "wp_security_context_manager_v1",
    ];

    /// Not for sandboxed clients: an input method is sent every key,
    /// passwords included.
    const INPUT_METHOD: &str = "zwp_input_method_manager_v2";

    #[derive(Default)]
    struct Globals {
        names: Vec<(u32, String)>,
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for Globals {
        fn event(
            state: &mut Self,
            _: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global {
                name, interface, ..
            } = event
            {
                state.names.push((name, interface));
            }
        }
    }

    wayland_client::delegate_noop!(Globals: ignore WpSecurityContextManagerV1);
    wayland_client::delegate_noop!(Globals: ignore WpSecurityContextV1);

    struct Client {
        queue: EventQueue<Globals>,
        state: Globals,
        registry: wl_registry::WlRegistry,
    }

    impl Client {
        fn connect(path: &Path) -> Self {
            Self::from_stream(UnixStream::connect(path).expect("connect"))
        }

        fn from_stream(stream: UnixStream) -> Self {
            let conn = Connection::from_socket(stream).expect("wayland connection");
            let mut queue = conn.new_event_queue();
            let registry = conn.display().get_registry(&queue.handle(), ());
            let mut state = Globals::default();
            queue.roundtrip(&mut state).expect("registry roundtrip");
            Self {
                queue,
                state,
                registry,
            }
        }

        fn has(&self, interface: &str) -> bool {
            self.state.names.iter().any(|(_, i)| i == interface)
        }
    }

    #[test]
    #[serial]
    fn sandboxed_clients_do_not_see_privileged_globals() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let runtime = std::env::var("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR");
        let mut host = Client::connect(&Path::new(&runtime).join(&handle.socket_name));

        for interface in HIDDEN_FROM_SANDBOXED {
            assert!(host.has(interface), "the user's client lost {interface}");
        }

        // Open a security context the way Flatpak does.
        let (manager_name, _) = host
            .state
            .names
            .iter()
            .find(|(_, i)| i == "wp_security_context_manager_v1")
            .cloned()
            .unwrap();
        let qh = host.queue.handle();
        let manager: WpSecurityContextManagerV1 = host.registry.bind(manager_name, 1, &qh, ());
        let dir = tempfile::tempdir().unwrap();
        let listen_path = dir.path().join("sandboxed.sock");
        let listener = UnixListener::bind(&listen_path).unwrap();
        let (close_read, _close_write) = std::io::pipe().unwrap();
        let context = manager.create_listener(listener.as_fd(), close_read.as_fd(), &qh, ());
        context.set_sandbox_engine("org.flatpak".into());
        context.set_app_id("org.example.Sandboxed".into());
        context.set_instance_id("1".into());
        context.commit();
        host.queue.roundtrip(&mut host.state).unwrap();

        let sandboxed = Client::connect(&listen_path);
        for interface in HIDDEN_FROM_SANDBOXED.iter().chain([&INPUT_METHOD]) {
            assert!(
                !sandboxed.has(interface),
                "a sandboxed client was offered {interface}"
            );
        }
        // It is still an ordinary client otherwise. Idle inhibition stays: a
        // Flatpak video player needs it, and only an inhibitor on a surface
        // that is on screen counts.
        assert!(sandboxed.has("wl_compositor"));
        assert!(sandboxed.has("xdg_wm_base"));
        assert!(sandboxed.has("zwp_idle_inhibit_manager_v1"));

        drop(sandboxed);
        drop(host);
        handle.stop();
    }

    /// An agent's own connection (`ConnectAgent`) is kept off the same
    /// globals as a sandboxed client, except the virtual input it drives its
    /// seat with.
    #[test]
    #[serial]
    fn an_agents_connection_sees_only_virtual_input() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        handle
            .query(|state| state.request_agent_seat("Claude", ":1.10"))
            .expect("seat");
        let stream = handle.query(|state| {
            state
                .connect_agent_client(":1.10")
                .expect("connect the agent")
        });
        let agent = Client::from_stream(stream);
        const VIRTUAL_INPUT: &[&str] = &[
            "zwlr_virtual_pointer_manager_v1",
            "zwp_virtual_keyboard_manager_v1",
        ];
        for interface in HIDDEN_FROM_SANDBOXED.iter().chain([&INPUT_METHOD]) {
            if VIRTUAL_INPUT.contains(interface) {
                assert!(agent.has(interface), "an agent was not offered {interface}");
            } else {
                assert!(!agent.has(interface), "an agent was offered {interface}");
            }
        }
        assert!(agent.has("wl_seat"));

        drop(agent);
        handle.stop();
    }

    /// Whatever holds the session lock collects the password, so no client
    /// may lock but the locker Otto started itself — not a sandboxed app, and
    /// not any other program of the user's either.
    #[test]
    #[serial]
    fn only_ottos_locker_is_offered_session_lock() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let runtime = std::env::var("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR");
        let user = Client::connect(&Path::new(&runtime).join(&handle.socket_name));
        assert!(
            !user.has("ext_session_lock_manager_v1"),
            "an ordinary client was offered session lock"
        );

        let stream =
            handle.query(|state| state.connect_locker_client().expect("connect the locker").1);
        let locker = Client::from_stream(stream);
        assert!(locker.has("ext_session_lock_manager_v1"));

        drop(locker);
        drop(user);
        handle.stop();
    }
}
