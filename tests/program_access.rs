//! Asking before one of the user's programs captures the screen or controls
//! the user's seat (`src/program_access.rs`, `specs/agent-seats.md` 5c).
//!
//! The headless compositor runs in the test's own process, whose clients are
//! trusted as Otto's own; each test turns that off before connecting, so its
//! client is a program nobody has answered for. The headless backend shows
//! no dialog: the tests answer through `answer_program_access`.

#[cfg(feature = "headless")]
mod program_access_tests {
    use std::collections::HashMap;
    use std::io::Write;
    use std::os::fd::AsFd;
    use std::time::Duration;

    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto::program_access::{self, Capability};
    use serial_test::serial;
    use wayland_client::{
        delegate_noop,
        protocol::{wl_buffer, wl_output, wl_registry, wl_seat, wl_shm, wl_shm_pool},
        Connection, Dispatch, EventQueue, QueueHandle,
    };
    use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
        zwp_virtual_keyboard_manager_v1, zwp_virtual_keyboard_v1,
    };
    use wayland_protocols_wlr::screencopy::v1::client::{
        zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1,
    };
    use wayland_protocols_wlr::virtual_pointer::v1::client::{
        zwlr_virtual_pointer_manager_v1, zwlr_virtual_pointer_v1,
    };

    #[derive(Default)]
    struct State {
        interfaces: Vec<String>,
        seat: Option<wl_seat::WlSeat>,
        output: Option<wl_output::WlOutput>,
        shm: Option<wl_shm::WlShm>,
        screencopy: Option<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1>,
        pointers: Option<zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1>,
        keyboards: Option<zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1>,
        /// The frame's buffer size, once announced.
        buffer: Option<(u32, u32, u32)>,
        failed: bool,
        ready: bool,
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
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind(name, version.min(5), qh, ()))
                }
                "wl_output" if state.output.is_none() => {
                    state.output = Some(registry.bind(name, version.min(3), qh, ()))
                }
                "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
                "zwlr_screencopy_manager_v1" => {
                    state.screencopy = Some(registry.bind(name, version.min(3), qh, ()))
                }
                "zwlr_virtual_pointer_manager_v1" => {
                    state.pointers = Some(registry.bind(name, version.min(2), qh, ()))
                }
                "zwp_virtual_keyboard_manager_v1" => {
                    state.keyboards = Some(registry.bind(name, 1, qh, ()))
                }
                _ => {}
            }
            state.interfaces.push(interface);
        }
    }

    impl Dispatch<zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, ()> for State {
        fn event(
            state: &mut Self,
            _: &zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
            event: zwlr_screencopy_frame_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                zwlr_screencopy_frame_v1::Event::Buffer {
                    width,
                    height,
                    stride,
                    ..
                } => state.buffer = Some((width, height, stride)),
                zwlr_screencopy_frame_v1::Event::Failed => state.failed = true,
                zwlr_screencopy_frame_v1::Event::Ready { .. } => state.ready = true,
                _ => {}
            }
        }
    }

    delegate_noop!(State: ignore wl_seat::WlSeat);
    delegate_noop!(State: ignore wl_output::WlOutput);
    delegate_noop!(State: ignore wl_shm::WlShm);
    delegate_noop!(State: wl_shm_pool::WlShmPool);
    delegate_noop!(State: ignore wl_buffer::WlBuffer);
    delegate_noop!(State: zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1);
    delegate_noop!(State: zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1);
    delegate_noop!(State: zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1);
    delegate_noop!(State: zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1);
    delegate_noop!(State: zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1);

    struct Program {
        conn: Connection,
        queue: EventQueue<State>,
        qh: QueueHandle<State>,
        state: State,
    }

    impl Program {
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

        fn has(&self, interface: &str) -> bool {
            self.state.interfaces.iter().any(|i| i == interface)
        }

        fn sync(&mut self, handle: &HeadlessHandle) -> Result<(), String> {
            self.conn.flush().map_err(|e| e.to_string())?;
            handle.wait(Duration::from_millis(100));
            handle.settle(150);
            self.queue
                .roundtrip(&mut self.state)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }

        /// Ask for a frame of the output and hand it a buffer to copy into.
        fn capture(&mut self, handle: &HeadlessHandle) {
            let manager = self.state.screencopy.clone().expect("screencopy offered");
            let output = self.state.output.clone().expect("an output");
            let frame = manager.capture_output(0, &output, &self.qh, ());
            self.sync(handle).expect("frame");
            let (width, height, stride) = self.state.buffer.expect("a buffer size");
            let size = (stride * height) as usize;
            let mut file = tempfile::tempfile().unwrap();
            file.write_all(&vec![0u8; size]).unwrap();
            let shm = self.state.shm.clone().expect("wl_shm");
            let pool = shm.create_pool(file.as_fd(), size as i32, &self.qh, ());
            let buffer = pool.create_buffer(
                0,
                width as i32,
                height as i32,
                stride as i32,
                wl_shm::Format::Argb8888,
                &self.qh,
                (),
            );
            frame.copy(&buffer);
            self.sync(handle).expect("copy");
        }
    }

    /// A compositor whose next clients are programs nobody answered for.
    fn start() -> (HeadlessHandle, std::path::PathBuf) {
        program_access::replace_all(HashMap::new());
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        handle.query(|state| state.program_access.trust_own_process = false);
        (handle, std::env::current_exe().unwrap())
    }

    fn asking(handle: &HeadlessHandle, capability: Capability) -> bool {
        let exe = std::env::current_exe().unwrap();
        handle.query(move |state| state.program_access.asking.contains(&(capability, exe)))
    }

    /// An unanswered program is offered capture and input, to be asked
    /// about when it uses them; the clipboard and window control are hidden.
    #[test]
    #[serial]
    fn an_unknown_program_is_offered_only_what_it_can_be_asked_about() {
        let (handle, _) = start();
        let program = Program::connect(&handle);
        assert!(program.has("zwlr_screencopy_manager_v1"));
        assert!(program.has("zwlr_virtual_pointer_manager_v1"));
        assert!(program.has("zwp_virtual_keyboard_manager_v1"));
        assert!(!program.has("zwlr_data_control_manager_v1"));
        assert!(!program.has("ext_data_control_manager_v1"));
        assert!(!program.has("zwlr_foreign_toplevel_manager_v1"));
        drop(program);
        handle.stop();
    }

    /// A capture waits for the user; allowed, it goes ahead.
    #[test]
    #[serial]
    fn a_capture_waits_for_the_answer() {
        let (handle, exe) = start();
        let mut program = Program::connect(&handle);
        program.capture(&handle);
        assert!(
            asking(&handle, Capability::ScreenCapture),
            "the user was not asked"
        );
        assert!(
            !program.state.failed && !program.state.ready,
            "the capture did not wait"
        );

        handle.query(move |state| {
            state.answer_program_access(Capability::ScreenCapture, exe, Some(true))
        });
        handle.settle(300);
        program.sync(&handle).unwrap();
        assert!(!program.state.failed, "an allowed capture failed");
        assert!(!asking(&handle, Capability::ScreenCapture));
        let waiting = handle.query(|state| state.program_access.awaiting_frames.len());
        assert_eq!(waiting, 0);

        drop(program);
        handle.stop();
    }

    /// Refused, the capture fails, and later ones fail without asking.
    #[test]
    #[serial]
    fn a_refused_capture_fails() {
        let (handle, exe) = start();
        let mut program = Program::connect(&handle);
        program.capture(&handle);
        handle.query(move |state| {
            state.answer_program_access(Capability::ScreenCapture, exe, Some(false))
        });
        program.sync(&handle).unwrap();
        assert!(program.state.failed, "a refused capture went ahead");

        let mut again = Program::connect(&handle);
        again.capture(&handle);
        assert!(again.state.failed);
        assert!(!asking(&handle, Capability::ScreenCapture), "asked twice");

        drop(program);
        drop(again);
        handle.stop();
    }

    fn user_pointer(handle: &HeadlessHandle) -> (f64, f64) {
        handle.query(|state| {
            let p = state.pointer.current_location();
            (p.x.round(), p.y.round())
        })
    }

    fn move_pointer(handle: &HeadlessHandle, program: &mut Program) {
        let manager = program.state.pointers.clone().expect("virtual pointer");
        let pointer = manager.create_virtual_pointer(program.state.seat.as_ref(), &program.qh, ());
        let (w, h) = handle.query(|state| {
            let output = state.workspaces.outputs().next().cloned().expect("output");
            let geo = state.workspaces.output_geometry(&output).expect("geometry");
            (geo.size.w as u32, geo.size.h as u32)
        });
        pointer.motion_absolute(0, 123, 77, w, h);
        pointer.frame();
        program.sync(handle).unwrap();
    }

    /// Input on the user's seat is refused and asked about; once allowed,
    /// the program's next pointer moves the user's.
    #[test]
    #[serial]
    fn input_on_the_users_seat_is_asked_about() {
        let (handle, exe) = start();
        let before = user_pointer(&handle);
        let mut program = Program::connect(&handle);
        move_pointer(&handle, &mut program);
        assert_eq!(
            user_pointer(&handle),
            before,
            "an unknown program moved the pointer"
        );
        assert!(asking(&handle, Capability::Input), "the user was not asked");

        handle.query(move |state| state.answer_program_access(Capability::Input, exe, Some(true)));
        move_pointer(&handle, &mut program);
        assert_eq!(user_pointer(&handle), (123.0, 77.0));

        drop(program);
        handle.stop();
    }

    /// A keyboard on the user's seat is refused until the program is
    /// allowed.
    #[test]
    #[serial]
    fn a_keyboard_on_the_users_seat_waits_for_permission() {
        let (handle, exe) = start();
        let mut program = Program::connect(&handle);
        let manager = program.state.keyboards.clone().expect("virtual keyboard");
        let seat = program.state.seat.clone().unwrap();
        let _keyboard = manager.create_virtual_keyboard(&seat, &program.qh, ());
        assert!(
            program.sync(&handle).is_err(),
            "an unknown program got a keyboard"
        );
        assert!(asking(&handle, Capability::Input));

        handle.query(move |state| state.answer_program_access(Capability::Input, exe, Some(true)));
        let mut again = Program::connect(&handle);
        let manager = again.state.keyboards.clone().unwrap();
        let seat = again.state.seat.clone().unwrap();
        let _keyboard = manager.create_virtual_keyboard(&seat, &again.qh, ());
        assert!(
            again.sync(&handle).is_ok(),
            "an allowed program was refused"
        );

        drop(again);
        handle.stop();
    }
}
