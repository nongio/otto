//! Which idle inhibitors hold off auto-lock (`idle-inhibit-unstable-v1`).
//!
//! An inhibitor only counts while its surface is on screen: a window that is
//! not minimized, on the workspace its output is showing. A surface with no
//! role, or a window on a workspace nobody is looking at, could otherwise keep
//! the session from ever locking without showing anything.

#[cfg(feature = "headless")]
mod idle_inhibit_tests {
    use std::time::Duration;

    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use smithay::wayland::idle_inhibit::IdleInhibitHandler;
    use wayland_client::{
        delegate_noop,
        protocol::{wl_compositor, wl_registry, wl_surface},
        Connection, Dispatch, EventQueue, QueueHandle,
    };
    use wayland_protocols::wp::idle_inhibit::zv1::client::{
        zwp_idle_inhibit_manager_v1, zwp_idle_inhibitor_v1,
    };

    #[derive(Default)]
    struct State {
        compositor: Option<wl_compositor::WlCompositor>,
        manager: Option<zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1>,
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for State {
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
                match interface.as_str() {
                    "wl_compositor" => state.compositor = Some(registry.bind(name, 4, qh, ())),
                    "zwp_idle_inhibit_manager_v1" => {
                        state.manager = Some(registry.bind(name, 1, qh, ()))
                    }
                    _ => {}
                }
            }
        }
    }
    delegate_noop!(State: wl_compositor::WlCompositor);
    delegate_noop!(State: ignore wl_surface::WlSurface);
    delegate_noop!(State: zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1);
    delegate_noop!(State: zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1);

    fn start() -> HeadlessHandle {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        handle.settle(20);
        handle
    }

    fn map_window(handle: &HeadlessHandle, client: &mut TestClient, title: &str) {
        client.create_toplevel_with_app_id(title, &format!("org.otto.{title}"), 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(200);
    }

    /// Take an inhibitor on the window titled `title`, the way the protocol
    /// handler does when its client asks.
    fn inhibit_window(handle: &HeadlessHandle, title: &str) {
        let title = title.to_string();
        handle.with_state(move |state| {
            let surface = state
                .workspaces
                .spaces_elements()
                .find(|w| w.xdg_title() == title)
                .and_then(|w| w.wl_surface().map(|s| s.into_owned()))
                .expect("window mapped");
            state.inhibit(surface);
        });
    }

    fn idle_inhibited(handle: &HeadlessHandle) -> bool {
        handle.query(|state| state.idle_inhibited())
    }

    #[test]
    #[serial]
    fn a_surface_with_no_role_does_not_hold_off_the_lock() {
        let handle = start();
        let path = format!(
            "{}/{}",
            std::env::var("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR"),
            handle.socket_name
        );
        let stream = std::os::unix::net::UnixStream::connect(path).expect("connect");
        let conn = Connection::from_socket(stream).expect("wayland connection");
        let mut queue: EventQueue<State> = conn.new_event_queue();
        let qh = queue.handle();
        conn.display().get_registry(&qh, ());
        let mut state = State::default();
        queue.roundtrip(&mut state).expect("bind globals");

        let surface = state
            .compositor
            .as_ref()
            .expect("wl_compositor")
            .create_surface(&qh, ());
        let _inhibitor = state
            .manager
            .as_ref()
            .expect("zwp_idle_inhibit_manager_v1")
            .create_inhibitor(&surface, &qh, ());
        surface.commit();
        queue.roundtrip(&mut state).expect("create inhibitor");
        handle.settle(10);

        assert_eq!(
            handle.query(|state| state.idle_inhibitors.len()),
            1,
            "the inhibitor never reached the compositor"
        );
        assert!(
            !idle_inhibited(&handle),
            "a surface nobody can see kept the session awake"
        );
        handle.stop();
    }

    #[test]
    #[serial]
    fn only_a_window_on_screen_holds_off_the_lock() {
        let handle = start();
        let mut client = TestClient::connect(&handle.socket_name).expect("client");
        map_window(&handle, &mut client, "Video");
        inhibit_window(&handle, "Video");
        assert!(idle_inhibited(&handle), "a visible window's inhibitor");

        handle.with_state(|state| {
            state.workspaces.add_workspace_to_output("headless");
        });
        handle.settle(100);
        handle.move_window_to_workspace("Video", 1);
        handle.settle(300);
        assert!(
            !idle_inhibited(&handle),
            "a window on a workspace nobody is looking at kept the session awake"
        );

        handle.set_workspace(1);
        handle.settle(300);
        assert!(idle_inhibited(&handle), "the window is on screen again");

        handle.minimize_window("Video");
        handle.settle(300);
        assert!(!idle_inhibited(&handle), "a minimized window's inhibitor");

        drop(client);
        handle.stop();
    }
}
