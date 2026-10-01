//! Capture through `ext-image-copy-capture-v1` (`src/state/image_capture.rs`,
//! `specs/security-model.md`): an agent's output is its workspace, and it
//! is told of, and captures, its own windows alone.
//!
//! The headless backend has no render loop, so a capture that rides it (a
//! user's program capturing the screen) is checked for being queued; the
//! agent's and toplevel captures, drawn off screen, complete at once.

#[cfg(feature = "headless")]
mod image_capture_tests {
    use std::io::Write;
    use std::os::fd::AsFd;
    use std::time::Duration;

    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use wayland_client::{
        delegate_noop,
        protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool},
        Connection, Dispatch, EventQueue, QueueHandle,
    };
    use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
        ext_foreign_toplevel_handle_v1, ext_foreign_toplevel_list_v1,
    };
    use wayland_protocols::ext::image_capture_source::v1::client::{
        ext_foreign_toplevel_image_capture_source_manager_v1 as toplevel_sources,
        ext_image_capture_source_v1, ext_output_image_capture_source_manager_v1 as output_sources,
    };
    use wayland_protocols::ext::image_copy_capture::v1::client::{
        ext_image_copy_capture_frame_v1 as frame_v1,
        ext_image_copy_capture_manager_v1 as manager_v1,
        ext_image_copy_capture_session_v1 as session_v1,
    };

    #[derive(Default)]
    struct State {
        output: Option<wl_output::WlOutput>,
        shm: Option<wl_shm::WlShm>,
        manager: Option<manager_v1::ExtImageCopyCaptureManagerV1>,
        output_sources: Option<output_sources::ExtOutputImageCaptureSourceManagerV1>,
        toplevel_sources: Option<toplevel_sources::ExtForeignToplevelImageCaptureSourceManagerV1>,
        toplevels: Option<ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1>,
        windows: Vec<(
            ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
            String,
        )>,
        /// The session's buffer size, once announced.
        size: Option<(i32, i32)>,
        formats: Vec<u32>,
        done: bool,
        stopped: bool,
        ready: bool,
        failed: Option<frame_v1::FailureReason>,
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
            let wl_registry::Event::Global {
                name,
                interface,
                version,
            } = event
            else {
                return;
            };
            match interface.as_str() {
                "wl_output" if state.output.is_none() => {
                    state.output = Some(registry.bind(name, version.min(3), qh, ()))
                }
                "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
                "ext_image_copy_capture_manager_v1" => {
                    state.manager = Some(registry.bind(name, 1, qh, ()))
                }
                "ext_output_image_capture_source_manager_v1" => {
                    state.output_sources = Some(registry.bind(name, 1, qh, ()))
                }
                "ext_foreign_toplevel_image_capture_source_manager_v1" => {
                    state.toplevel_sources = Some(registry.bind(name, 1, qh, ()))
                }
                "ext_foreign_toplevel_list_v1" => {
                    state.toplevels = Some(registry.bind(name, 1, qh, ()))
                }
                _ => {}
            }
        }
    }

    impl Dispatch<ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1, ()> for State {
        fn event(
            state: &mut Self,
            _: &ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1,
            event: ext_foreign_toplevel_list_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
                state.windows.push((toplevel, String::new()));
            }
        }

        wayland_client::event_created_child!(State, ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1, [
            ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1, ()),
        ]);
    }

    impl Dispatch<ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1, ()> for State {
        fn event(
            state: &mut Self,
            handle: &ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
            event: ext_foreign_toplevel_handle_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                ext_foreign_toplevel_handle_v1::Event::Title { title } => {
                    if let Some(entry) = state.windows.iter_mut().find(|(h, _)| h == handle) {
                        entry.1 = title;
                    }
                }
                ext_foreign_toplevel_handle_v1::Event::Closed => {
                    state.windows.retain(|(h, _)| h != handle);
                }
                _ => {}
            }
        }
    }

    impl Dispatch<session_v1::ExtImageCopyCaptureSessionV1, ()> for State {
        fn event(
            state: &mut Self,
            _: &session_v1::ExtImageCopyCaptureSessionV1,
            event: session_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                session_v1::Event::BufferSize { width, height } => {
                    state.size = Some((width as i32, height as i32))
                }
                session_v1::Event::ShmFormat { format } => state.formats.push(format.into()),
                session_v1::Event::Done => state.done = true,
                session_v1::Event::Stopped => state.stopped = true,
                _ => {}
            }
        }
    }

    impl Dispatch<frame_v1::ExtImageCopyCaptureFrameV1, ()> for State {
        fn event(
            state: &mut Self,
            _: &frame_v1::ExtImageCopyCaptureFrameV1,
            event: frame_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                frame_v1::Event::Ready => state.ready = true,
                frame_v1::Event::Failed { reason } => {
                    state.failed = reason.into_result().ok();
                }
                _ => {}
            }
        }
    }

    delegate_noop!(State: ignore wl_output::WlOutput);
    delegate_noop!(State: ignore wl_shm::WlShm);
    delegate_noop!(State: wl_shm_pool::WlShmPool);
    delegate_noop!(State: ignore wl_buffer::WlBuffer);
    delegate_noop!(State: manager_v1::ExtImageCopyCaptureManagerV1);
    delegate_noop!(State: output_sources::ExtOutputImageCaptureSourceManagerV1);
    delegate_noop!(State: toplevel_sources::ExtForeignToplevelImageCaptureSourceManagerV1);
    delegate_noop!(State: ext_image_capture_source_v1::ExtImageCaptureSourceV1);

    struct Capturer {
        conn: Connection,
        queue: EventQueue<State>,
        qh: QueueHandle<State>,
        state: State,
    }

    impl Capturer {
        fn from_stream(stream: std::os::unix::net::UnixStream) -> Self {
            let conn = Connection::from_socket(stream).expect("wayland connection");
            let mut queue = conn.new_event_queue();
            let qh = queue.handle();
            conn.display().get_registry(&qh, ());
            let mut state = State::default();
            queue.roundtrip(&mut state).expect("bind globals");
            queue.roundtrip(&mut state).expect("settle");
            Self {
                conn,
                queue,
                qh,
                state,
            }
        }

        fn connect(handle: &HeadlessHandle) -> Self {
            let path = format!(
                "{}/{}",
                std::env::var("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR"),
                handle.socket_name
            );
            Self::from_stream(std::os::unix::net::UnixStream::connect(path).expect("connect"))
        }

        fn as_agent(handle: &HeadlessHandle, owner: &str) -> Self {
            let owner = owner.to_string();
            Self::from_stream(handle.query(move |state| {
                state
                    .connect_agent_client(&owner)
                    .expect("connect the agent")
            }))
        }

        fn sync(&mut self, handle: &HeadlessHandle) {
            self.conn.flush().expect("flush");
            handle.wait(Duration::from_millis(100));
            handle.settle(150);
            self.queue.roundtrip(&mut self.state).expect("roundtrip");
        }

        fn window_titles(&self) -> Vec<String> {
            let mut titles: Vec<String> =
                self.state.windows.iter().map(|(_, t)| t.clone()).collect();
            titles.sort();
            titles
        }

        fn output_source(&self) -> ext_image_capture_source_v1::ExtImageCaptureSourceV1 {
            let manager = self.state.output_sources.as_ref().expect("output sources");
            manager.create_source(self.state.output.as_ref().expect("an output"), &self.qh, ())
        }

        fn toplevel_source(
            &self,
            title: &str,
        ) -> ext_image_capture_source_v1::ExtImageCaptureSourceV1 {
            let manager = self
                .state
                .toplevel_sources
                .as_ref()
                .expect("toplevel sources");
            let handle = &self
                .state
                .windows
                .iter()
                .find(|(_, t)| t == title)
                .expect("window listed")
                .0;
            manager.create_source(handle, &self.qh, ())
        }

        /// Open a session on `source`, wait for its constraints, and capture
        /// one frame into a matching wl_shm buffer.
        fn capture(
            &mut self,
            handle: &HeadlessHandle,
            source: &ext_image_capture_source_v1::ExtImageCaptureSourceV1,
        ) -> session_v1::ExtImageCopyCaptureSessionV1 {
            let manager = self.state.manager.clone().expect("capture manager");
            let session =
                manager.create_session(source, manager_v1::Options::empty(), &self.qh, ());
            self.sync(handle);
            if self.state.stopped {
                return session;
            }
            assert!(self.state.done, "no constraints came");
            let (width, height) = self.state.size.expect("a buffer size");
            let stride = width * 4;
            let size = (stride * height) as usize;
            let mut file = tempfile::tempfile().unwrap();
            file.write_all(&vec![0u8; size]).unwrap();
            let shm = self.state.shm.clone().expect("wl_shm");
            let pool = shm.create_pool(file.as_fd(), size as i32, &self.qh, ());
            let buffer = pool.create_buffer(
                0,
                width,
                height,
                stride,
                wl_shm::Format::Argb8888,
                &self.qh,
                (),
            );
            let frame = session.create_frame(&self.qh, ());
            frame.attach_buffer(&buffer);
            frame.damage_buffer(0, 0, width, height);
            frame.capture();
            self.sync(handle);
            session
        }
    }

    fn request_seat_and_workspace(handle: &HeadlessHandle, owner: &str) {
        let owner = owner.to_string();
        handle.query(move |state| {
            state.request_agent_seat("Claude", &owner).expect("seat");
            state.request_own_workspace(&owner).expect("workspace");
        });
    }

    fn map_window(handle: &HeadlessHandle, client: &mut TestClient, title: &str) {
        client.create_toplevel_with_app_id(title, &format!("org.otto.{title}"), 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(200);
    }

    /// An agent capturing "the output" gets its workspace, at the output's
    /// size, drawn at once.
    #[test]
    #[serial]
    fn an_agent_captures_its_workspace_as_its_output() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        request_seat_and_workspace(&handle, ":1.10");
        let mode = handle.query(|state| {
            let output = state.workspaces.outputs().next().cloned().unwrap();
            let mode = output.current_mode().unwrap().size;
            (mode.w, mode.h)
        });

        let mut agent = Capturer::as_agent(&handle, ":1.10");
        let source = agent.output_source();
        let _session = agent.capture(&handle, &source);
        assert_eq!(agent.state.size, Some(mode), "not the output's size");
        assert!(
            agent.state.ready,
            "the frame was not drawn: {:?}",
            agent.state.failed
        );

        drop(agent);
        handle.stop();
    }

    /// An agent's window list holds its own windows alone, and capturing one
    /// of them works.
    #[test]
    #[serial]
    fn an_agent_is_told_of_and_captures_its_own_windows() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        request_seat_and_workspace(&handle, ":1.10");
        let mut user = TestClient::connect(&handle.socket_name).expect("client");
        map_window(&handle, &mut user, "User");
        let stream = handle.query(|state| {
            state
                .connect_agent_client(":1.10")
                .expect("connect the agent")
        });
        let mut mine = TestClient::from_stream(stream).expect("agent client");
        map_window(&handle, &mut mine, "Mine");

        let mut agent = Capturer::as_agent(&handle, ":1.10");
        agent.sync(&handle);
        assert_eq!(agent.window_titles(), vec!["Mine".to_string()]);
        let everyone = Capturer::connect(&handle);
        assert_eq!(
            everyone.window_titles(),
            vec!["Mine".to_string(), "User".to_string()]
        );

        let source = agent.toplevel_source("Mine");
        let _session = agent.capture(&handle, &source);
        assert!(agent.state.size.is_some_and(|(w, h)| w > 0 && h > 0));
        assert!(
            agent.state.ready,
            "the window was not drawn: {:?}",
            agent.state.failed
        );

        drop(agent);
        drop(everyone);
        drop(mine);
        drop(user);
        handle.stop();
    }

    /// The user's own programs capture the screen as the render loop draws
    /// it: the frame waits for the next drawn frame.
    #[test]
    #[serial]
    fn a_users_program_captures_the_screen_from_the_render_loop() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut program = Capturer::connect(&handle);
        let source = program.output_source();
        let _session = program.capture(&handle, &source);
        assert!(!program.state.ready && program.state.failed.is_none());
        let queued = handle.query(|state| state.pending_screencopy_frames.len());
        assert_eq!(queued, 1, "the capture did not wait for the render loop");

        drop(program);
        handle.stop();
    }
}
