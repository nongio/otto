//! Screencast control-plane tests (headless).
//!
//! The D-Bus service and the PipeWire stream are the two halves screen sharing
//! cannot have here: one needs a session bus, the other a daemon and a GPU.
//! Everything between them is exercised against a real compositor and real
//! Wayland clients — enumerating capturable sources, naming a window by its
//! `ext-foreign-toplevel-list-v1` identifier, sizing its stream, rejecting a
//! source that no longer exists, and the frame-callback throttling a live
//! capture forces.
//!
//! See `specs/screenshare.md` (Window identity, Window capture).

#[cfg(feature = "headless")]
mod screenshare_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto::screenshare::StreamTarget;
    use otto::state::window_throttle::WindowThrottleState;
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::time::Duration;

    /// The object path the D-Bus service would have minted for a session.
    const SESSION: &str = "/org/otto/ScreenCast/session/1";
    const OUTPUT: &str = "headless";

    fn start() -> HeadlessHandle {
        HeadlessHandle::start(HeadlessConfig::default())
    }

    fn connect(handle: &HeadlessHandle) -> TestClient {
        TestClient::connect(&handle.socket_name).expect("client failed to connect")
    }

    /// Map a toplevel and let the compositor settle so it is mapped, placed and
    /// registered with the foreign-toplevel protocols.
    fn map_window(handle: &HeadlessHandle, client: &mut TestClient, title: &str, app_id: &str) {
        client.create_toplevel_with_app_id(title, app_id, 640, 480);
        handle.wait(Duration::from_millis(100));
        let _ = client.roundtrip();
        handle.settle(200);
    }

    /// The `ext-foreign-toplevel-list-v1` identifier the portal would pass back
    /// to `RecordWindow` for the window with this title.
    fn identifier_of(handle: &HeadlessHandle, title: &str) -> String {
        handle
            .screencast_list_windows()
            .into_iter()
            .find(|w| w.title == title)
            .unwrap_or_else(|| panic!("window {title:?} not listed as capturable"))
            .id
    }

    fn output_scale(handle: &HeadlessHandle) -> f64 {
        handle.query(|state| {
            state
                .workspaces
                .outputs()
                .next()
                .map(|o| o.current_scale().fractional_scale())
                .unwrap_or(1.0)
        })
    }

    /// The window's geometry in logical pixels, straight from the space.
    fn logical_geometry(handle: &HeadlessHandle, title: &str) -> (i32, i32) {
        handle
            .window_logical_size(title)
            .unwrap_or_else(|| panic!("window {title:?} not mapped"))
    }

    // ── Source enumeration ───────────────────────────────────────────────

    /// `ListOutputs` answers with the connector name a session records by, and
    /// the mode that paces its stream.
    #[test]
    #[serial]
    fn list_outputs_reports_the_headless_output() {
        let handle = start();

        let outputs = handle.screencast_list_outputs();
        assert_eq!(outputs.len(), 1, "expected exactly one output");
        let output = &outputs[0];
        assert_eq!(output.connector, OUTPUT);
        assert_eq!((output.width, output.height), (1920, 1080));
        assert_eq!(output.refresh_rate, 60_000);

        handle.stop();
    }

    /// Every mapped toplevel is capturable, named by an identifier that is
    /// opaque, non-empty and unique — never a title or a surface id — and
    /// carries the app_id and title the picker shows.
    #[test]
    #[serial]
    fn list_windows_names_every_mapped_toplevel() {
        let handle = start();
        let mut client = connect(&handle);

        map_window(&handle, &mut client, "First", "org.otto.First");
        map_window(&handle, &mut client, "Second", "org.otto.Second");

        let windows = handle.screencast_list_windows();
        assert_eq!(windows.len(), 2, "both toplevels should be capturable");

        let first = windows.iter().find(|w| w.title == "First").unwrap();
        let second = windows.iter().find(|w| w.title == "Second").unwrap();
        assert_eq!(first.app_id, "org.otto.First");
        assert_eq!(second.app_id, "org.otto.Second");
        assert!(!first.id.is_empty(), "identifier must not be empty");
        assert_ne!(first.id, second.id, "identifiers must be unique per window");
        assert_ne!(first.id, first.title, "identifier is not the title");

        // Sizes are reported in physical pixels — logical geometry × the
        // output's scale, not the logical size itself.
        let scale = output_scale(&handle);
        let (logical_w, logical_h) = logical_geometry(&handle, "First");
        assert_eq!(first.width, ((logical_w as f64) * scale).round() as u32);
        assert_eq!(first.height, ((logical_h as f64) * scale).round() as u32);

        handle.stop();
    }

    /// A window whose client disconnected is no longer offered, and the
    /// identifier the picker handed out for it stops resolving — the portal may
    /// still be holding it when the user finally answers the dialog.
    #[test]
    #[serial]
    fn list_windows_forgets_a_window_whose_client_went_away() {
        let handle = start();
        let mut keeper = connect(&handle);
        let mut leaver = connect(&handle);

        map_window(&handle, &mut keeper, "Keeper", "org.otto.Keeper");
        map_window(&handle, &mut leaver, "Leaver", "org.otto.Leaver");
        let stale = identifier_of(&handle, "Leaver");

        drop(leaver);
        handle.wait(Duration::from_millis(200));
        handle.settle(200);

        let titles: Vec<String> = handle
            .screencast_list_windows()
            .into_iter()
            .map(|w| w.title)
            .collect();
        assert_eq!(titles, vec!["Keeper".to_string()]);

        handle.screencast_create_session(SESSION, 2);
        let err = handle
            .screencast_start_recording(SESSION, StreamTarget::Window(stale.clone()))
            .expect_err("a closed window must not be recordable");
        assert_eq!(err, format!("Window not found: {stale}"));

        handle.stop();
    }

    // ── Recording rejections ─────────────────────────────────────────────

    /// An identifier this compositor never issued fails outright, rather than
    /// falling back to some other window.
    #[test]
    #[serial]
    fn record_window_rejects_an_unknown_identifier() {
        let handle = start();
        let mut client = connect(&handle);
        map_window(&handle, &mut client, "Only", "org.otto.Only");
        handle.screencast_create_session(SESSION, 2);

        let err = handle
            .screencast_start_recording(SESSION, StreamTarget::Window("not-a-window".into()))
            .expect_err("unknown identifier must fail");
        assert_eq!(err, "Window not found: not-a-window");
        assert_eq!(handle.screencast_stream_keys(SESSION), Some(vec![]));

        handle.stop();
    }

    /// Same for a connector that is not plugged in.
    #[test]
    #[serial]
    fn record_monitor_rejects_an_unknown_connector() {
        let handle = start();
        handle.screencast_create_session(SESSION, 2);

        let err = handle
            .screencast_start_recording(SESSION, StreamTarget::Output("HDMI-A-9".into()))
            .expect_err("unknown connector must fail");
        assert_eq!(err, "Output not found: HDMI-A-9");

        handle.stop();
    }

    /// A stream can only be opened inside a session the compositor knows about
    /// — a resolvable target is not enough.
    #[test]
    #[serial]
    fn record_rejects_an_unknown_session() {
        let handle = start();
        let mut client = connect(&handle);
        map_window(&handle, &mut client, "Only", "org.otto.Only");
        let id = identifier_of(&handle, "Only");

        let err = handle
            .screencast_start_recording("/org/otto/ScreenCast/session/99", StreamTarget::Window(id))
            .expect_err("unknown session must fail");
        assert_eq!(err, "Session not found: /org/otto/ScreenCast/session/99");

        handle.stop();
    }

    /// The same source cannot be opened twice in one session.
    #[test]
    #[serial]
    fn record_rejects_a_source_already_being_recorded() {
        let handle = start();
        let mut client = connect(&handle);
        map_window(&handle, &mut client, "Shared", "org.otto.Shared");
        let id = identifier_of(&handle, "Shared");

        handle.screencast_create_session(SESSION, 2);
        handle
            .screencast_attach_stream(SESSION, StreamTarget::Window(id.clone()))
            .expect("window stream should attach");

        let err = handle
            .screencast_start_recording(SESSION, StreamTarget::Window(id.clone()))
            .expect_err("recording the same window twice must fail");
        assert_eq!(err, format!("Already recording: window:{id}"));

        handle.stop();
    }

    // ── Stream sizing and bookkeeping ────────────────────────────────────

    /// A window stream is fixed at the window's physical size, rounded down to
    /// even — odd dimensions are rejected by PipeWire consumers downstream.
    #[test]
    #[serial]
    fn window_capture_size_is_even_and_in_physical_pixels() {
        let handle = start();
        let mut client = connect(&handle);
        map_window(&handle, &mut client, "Sized", "org.otto.Sized");
        let id = identifier_of(&handle, "Sized");

        let scale = output_scale(&handle);
        let (logical_w, logical_h) = logical_geometry(&handle, "Sized");
        let (width, height, refresh) = handle
            .screencast_window_capture_size(&id)
            .expect("window should be sizable");

        assert_eq!(width % 2, 0, "width must be even");
        assert_eq!(height % 2, 0, "height must be even");
        assert_eq!(width, ((logical_w as f64) * scale).round() as u32 & !1);
        assert_eq!(height, ((logical_h as f64) * scale).round() as u32 & !1);
        assert_eq!(
            refresh, 60_000,
            "the stream is paced by the output hosting the window"
        );

        handle.stop();
    }

    /// An output stream and a window stream live in the same session under
    /// namespaced keys, so a window can never be mistaken for a connector.
    #[test]
    #[serial]
    fn a_session_holds_an_output_and_a_window_stream_at_once() {
        let handle = start();
        let mut client = connect(&handle);
        map_window(&handle, &mut client, "Shared", "org.otto.Shared");
        let id = identifier_of(&handle, "Shared");

        handle.screencast_create_session(SESSION, 2);
        let (window_w, window_h) = handle
            .screencast_attach_stream(SESSION, StreamTarget::Window(id.clone()))
            .expect("window stream should attach");
        let (output_w, output_h) = handle
            .screencast_attach_stream(SESSION, StreamTarget::Output(OUTPUT.into()))
            .expect("output stream should attach");

        assert_eq!((output_w, output_h), (1920, 1080));
        assert!(
            window_w < output_w && window_h < output_h,
            "a window stream is sized to the window ({window_w}x{window_h}), not the output"
        );
        assert_eq!(
            handle.screencast_stream_keys(SESSION),
            Some(vec![format!("output:{OUTPUT}"), format!("window:{id}"),])
        );

        handle.stop();
    }

    // ── What a live capture changes ──────────────────────────────────────

    /// A window being cast keeps receiving frame callbacks at full rate even
    /// with another window on top of it — the remote viewer is looking at it
    /// when the local user is not — but it is not reported as activated.
    #[test]
    #[serial]
    fn a_cast_window_keeps_full_frame_rate_behind_another() {
        let handle = start();
        let mut client = connect(&handle);

        map_window(&handle, &mut client, "Behind", "org.otto.Behind");
        map_window(&handle, &mut client, "InFront", "org.otto.InFront");

        // Before anything is cast, the window that is not on top is throttled.
        let before = handle.window_throttle_states();
        assert_eq!(before.get("InFront"), Some(&WindowThrottleState::Focused));
        assert_eq!(before.get("Behind"), Some(&WindowThrottleState::Secondary));

        let id = identifier_of(&handle, "Behind");
        handle.screencast_create_session(SESSION, 2);
        handle
            .screencast_attach_stream(SESSION, StreamTarget::Window(id))
            .expect("window stream should attach");

        let during = handle.window_throttle_states();
        let behind = during.get("Behind").copied().expect("Behind still mapped");
        assert_eq!(behind, WindowThrottleState::Captured);
        assert_eq!(
            behind.throttle(),
            Duration::ZERO,
            "a cast window must not be frame-throttled"
        );
        assert!(
            !behind.is_activated(),
            "capture does not give the window keyboard focus"
        );
        assert_eq!(
            during.get("InFront"),
            Some(&WindowThrottleState::Focused),
            "casting one window must not change the others"
        );

        handle.stop();
    }

    /// Switching to another workspace does not interrupt a window's capture:
    /// the identifier still resolves, the stream keeps its size, and the window
    /// stays at full frame rate while it is off screen.
    #[test]
    #[serial]
    fn a_cast_window_survives_a_workspace_switch() {
        let handle = start();
        let mut client = connect(&handle);
        map_window(&handle, &mut client, "Shared", "org.otto.Shared");
        let id = identifier_of(&handle, "Shared");

        handle.screencast_create_session(SESSION, 2);
        let (width, height) = handle
            .screencast_attach_stream(SESSION, StreamTarget::Window(id.clone()))
            .expect("window stream should attach");

        handle.with_state(|state| {
            state.workspaces.add_workspace_to_output(OUTPUT);
        });
        handle.settle(300);
        handle.set_workspace(1);
        handle.settle(300);

        assert_eq!(
            handle
                .screencast_window_capture_size(&id)
                .map(|(w, h, _)| (w, h)),
            Ok((width, height)),
            "the stream keeps the size it negotiated"
        );
        assert_eq!(
            handle.window_throttle_states().get("Shared"),
            Some(&WindowThrottleState::Captured),
            "a window off the current workspace is still being watched remotely"
        );

        handle.stop();
    }

    /// A minimized window should stay capturable — the spec asks for full frame
    /// rate "even when occluded, on an inactive workspace, or minimized".
    ///
    /// Ignored: minimizing unmaps the window from every space, and both
    /// `ListWindows` and `window_for_identifier` walk `spaces_elements()`, so
    /// today a minimized window disappears from the picker and its stream stops
    /// resolving. See `specs/screenshare.md` (Window capture).
    #[test]
    #[serial]
    #[ignore = "known gap: minimizing unmaps the window, so its capture stops resolving"]
    fn a_minimized_window_stays_capturable() {
        let handle = start();
        let mut client = connect(&handle);
        map_window(&handle, &mut client, "Shared", "org.otto.Shared");
        let id = identifier_of(&handle, "Shared");

        handle.screencast_create_session(SESSION, 2);
        handle
            .screencast_attach_stream(SESSION, StreamTarget::Window(id.clone()))
            .expect("window stream should attach");

        handle.with_state(|state| {
            let window = state
                .workspaces
                .spaces_elements()
                .find(|w| w.xdg_title() == "Shared")
                .cloned()
                .expect("window mapped");
            state.workspaces.minimize_window(&window);
        });
        handle.settle(400);

        assert!(
            handle.screencast_window_capture_size(&id).is_ok(),
            "a minimized window must still resolve for its stream"
        );
        assert_eq!(
            handle.window_throttle_states().get("Shared"),
            Some(&WindowThrottleState::Captured),
        );

        handle.stop();
    }

    /// Destroying the session drops its streams, and the window it was casting
    /// goes back to being throttled like any other background window.
    #[test]
    #[serial]
    fn destroying_a_session_ends_its_capture() {
        let handle = start();
        let mut client = connect(&handle);

        map_window(&handle, &mut client, "Behind", "org.otto.Behind");
        map_window(&handle, &mut client, "InFront", "org.otto.InFront");
        let id = identifier_of(&handle, "Behind");

        handle.screencast_create_session(SESSION, 2);
        handle
            .screencast_attach_stream(SESSION, StreamTarget::Window(id.clone()))
            .expect("window stream should attach");
        assert_eq!(
            handle.screencast_stream_keys(SESSION),
            Some(vec![format!("window:{id}")])
        );

        handle.screencast_destroy_session(SESSION);

        assert_eq!(
            handle.screencast_stream_keys(SESSION),
            None,
            "the session should be gone"
        );
        assert_eq!(
            handle.window_throttle_states().get("Behind"),
            Some(&WindowThrottleState::Secondary),
            "the window should be throttled again once nothing casts it"
        );

        handle.stop();
    }
}
