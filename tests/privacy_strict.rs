//! `[privacy] strict` (`src/sandbox.rs`, `specs/security-model.md`): the
//! user's programs are kept off the privileged interfaces unless allowed.
//!
//! The headless compositor runs in the test's own process, whose clients
//! count as Otto's own; each test turns that off, so its client is an
//! ordinary program of the user's, named by this test binary.

#[cfg(feature = "headless")]
mod privacy_strict_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto::sandbox::{set_policy, Policy};
    use serial_test::serial;
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use wayland_client::{protocol::wl_registry, Connection, Dispatch, EventQueue, QueueHandle};

    /// The interfaces the policy is about.
    const PRIVILEGED: &[&str] = &[
        "zwlr_screencopy_manager_v1",
        "zwlr_virtual_pointer_manager_v1",
        "zwp_virtual_keyboard_manager_v1",
        "zwlr_data_control_manager_v1",
        "ext_data_control_manager_v1",
        "zwlr_foreign_toplevel_manager_v1",
        "ext_image_copy_capture_manager_v1",
        "ext_output_image_capture_source_manager_v1",
        "ext_foreign_toplevel_image_capture_source_manager_v1",
    ];

    #[derive(Default)]
    struct Globals {
        names: Vec<String>,
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
            if let wl_registry::Event::Global { interface, .. } = event {
                state.names.push(interface);
            }
        }
    }

    struct Client {
        _queue: EventQueue<Globals>,
        state: Globals,
    }

    impl Client {
        fn connect(handle: &HeadlessHandle) -> Self {
            let runtime = std::env::var("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR");
            let stream = UnixStream::connect(Path::new(&runtime).join(&handle.socket_name))
                .expect("connect");
            let conn = Connection::from_socket(stream).expect("wayland connection");
            let mut queue = conn.new_event_queue();
            conn.display().get_registry(&queue.handle(), ());
            let mut state = Globals::default();
            queue.roundtrip(&mut state).expect("registry roundtrip");
            Self {
                _queue: queue,
                state,
            }
        }

        fn has(&self, interface: &str) -> bool {
            self.state.names.iter().any(|i| i == interface)
        }
    }

    /// A compositor under `policy`, whose next clients are the user's
    /// programs rather than Otto's own.
    fn start(policy: Policy) -> HeadlessHandle {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        set_policy(Policy {
            trust_own_process: false,
            ..policy
        });
        handle
    }

    fn stop(handle: HeadlessHandle) {
        set_policy(Policy::default());
        handle.stop();
    }

    #[test]
    #[serial]
    fn by_default_the_users_programs_get_the_privileged_interfaces() {
        let handle = start(Policy::default());
        let client = Client::connect(&handle);
        for interface in PRIVILEGED {
            assert!(client.has(interface), "{interface} was not offered");
        }
        drop(client);
        stop(handle);
    }

    #[test]
    #[serial]
    fn strict_keeps_them_off_the_users_programs() {
        let handle = start(Policy {
            strict: true,
            ..Policy::default()
        });
        let client = Client::connect(&handle);
        for interface in PRIVILEGED {
            assert!(
                !client.has(interface),
                "{interface} was offered under strict"
            );
        }
        // Still an ordinary client otherwise.
        assert!(client.has("wl_compositor"));
        assert!(client.has("xdg_wm_base"));
        assert!(client.has("wl_seat"));
        drop(client);
        stop(handle);
    }

    #[test]
    #[serial]
    fn allow_names_the_exceptions() {
        let handle = start(Policy {
            strict: true,
            allow: vec![std::env::current_exe().unwrap()],
            ..Policy::default()
        });
        let client = Client::connect(&handle);
        for interface in PRIVILEGED {
            assert!(client.has(interface), "an allowed program lost {interface}");
        }
        drop(client);
        stop(handle);
    }
}
