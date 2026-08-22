//! Drag-icon lifecycle: what the compositor leaves behind between drags.
//!
//! A drag icon is a client surface the compositor carries under the cursor,
//! and it outlives its own drag by design — a refused drop flies the picture
//! home, so its layers cannot be torn down at the moment the button comes up.
//! That makes "taken down before the next drag" a property nothing enforces
//! implicitly, and one that has already regressed once: each drag drew its
//! picture over the pile the previous ones left.

#[cfg(feature = "headless")]
mod dnd_icon_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::time::Duration;
    use wayland_client::protocol::wl_surface::WlSurface;

    /// How many drags a test performs. Enough that a per-drag leak is
    /// unmistakable rather than arguable.
    const ROUNDS: usize = 4;

    fn start() -> (HeadlessHandle, TestClient) {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let client = TestClient::connect(&handle.socket_name).expect("connect");
        (handle, client)
    }

    /// A mapped window with the pointer resting in the middle of it, which is
    /// where a drag has to begin: the press serial the compositor hands out is
    /// what authorises `start_drag`.
    fn window_with_pointer(handle: &HeadlessHandle, client: &mut TestClient) -> WlSurface {
        let toplevel = client.create_toplevel("dnd-source", 400, 300);
        handle.wait(Duration::from_millis(120));
        client.roundtrip().expect("roundtrip");
        toplevel.lock().unwrap().commit_frame();
        client.roundtrip().expect("roundtrip");
        handle.wait(Duration::from_millis(120));

        let (x, y, w, h) = handle
            .window_logical_geometry("dnd-source")
            .expect("the window is mapped");
        handle.pointer_move(x as f64 + w as f64 / 2.0, y as f64 + h as f64 / 2.0);
        handle.wait(Duration::from_millis(50));
        client.roundtrip().expect("roundtrip");

        let surface = toplevel.lock().unwrap().surface.clone();
        surface
    }

    /// Press, start a drag carrying `icon`, and let the compositor settle.
    fn drag_with_icon(
        handle: &HeadlessHandle,
        client: &mut TestClient,
        origin: &WlSurface,
        icon: &WlSurface,
    ) {
        handle.pointer_press();
        handle.wait(Duration::from_millis(50));
        client.roundtrip().expect("roundtrip");
        assert!(
            client.state.last_button_serial.is_some(),
            "the press never reached the client, so no drag can be authorised"
        );

        client
            .start_drag(origin, Some(icon), &["text/uri-list"])
            .expect("the drag started");
        client.roundtrip().expect("roundtrip");
        handle.wait(Duration::from_millis(120));
        handle.settle(120);
    }

    /// Let go, and let the refusal animation run out.
    fn drop_it(handle: &HeadlessHandle, client: &mut TestClient) {
        handle.pointer_release();
        handle.wait(Duration::from_millis(120));
        let _ = client.roundtrip();
        // The snap-back home is 0.35s; run well past it so nothing is still
        // being animated when the next drag starts.
        handle.settle(200);
        handle.wait(Duration::from_millis(120));
        let _ = client.roundtrip();
    }

    /// Consecutive drags must not pile their pictures up. The client here
    /// keeps every icon surface alive, which is the harder case: nothing is
    /// destroyed, so only the compositor's own sweep can take them down.
    #[test]
    #[serial]
    fn consecutive_drags_do_not_pile_up_icons() {
        let (handle, mut client) = start();
        let origin = window_with_pointer(&handle, &mut client);

        // Kept alive deliberately — see above.
        let mut icons = Vec::new();

        for round in 0..ROUNDS {
            let (icon, buffer) = client.create_drag_icon(64, 64);
            client.roundtrip().expect("roundtrip");

            drag_with_icon(&handle, &mut client, &origin, &icon);

            assert!(
                handle.dnd_icon_present(),
                "round {round}: the compositor is not carrying an icon"
            );
            let carried = handle.dnd_icon_layer_count();
            assert_eq!(
                carried, 1,
                "round {round}: {carried} pictures under the cursor — \
                 every drag but the last is a leftover"
            );

            drop_it(&handle, &mut client);
            icons.push((icon, buffer));
        }

        handle.stop();
    }

    /// The same, with the client destroying each icon when it is done with it
    /// — what otto-kit actually does. The compositor's own teardown has to
    /// cope with the surface being gone before it looks at it.
    #[test]
    #[serial]
    fn a_destroyed_icon_leaves_nothing_behind() {
        let (handle, mut client) = start();
        let origin = window_with_pointer(&handle, &mut client);

        for round in 0..ROUNDS {
            let (icon, _buffer) = client.create_drag_icon(64, 64);
            client.roundtrip().expect("roundtrip");

            drag_with_icon(&handle, &mut client, &origin, &icon);
            drop_it(&handle, &mut client);

            icon.destroy();
            client.roundtrip().expect("roundtrip");
            handle.wait(Duration::from_millis(80));
            handle.settle(120);

            let left = handle.dnd_icon_layer_count();
            assert_eq!(
                left, 0,
                "round {round}: {left} layers left under the drag view after the \
                 icon's own surface was destroyed"
            );
        }

        handle.stop();
    }

    /// Nothing about a drag may accumulate. Counts every surface layer the
    /// compositor holds and every node in the scene, before and after a run
    /// of drags: a per-drag leak of any shape shows up as growth here, even
    /// one that leaves the drag view itself looking tidy.
    ///
    /// One round of settling separates the baseline from the measurement, so
    /// what is compared is drag-to-drag steady state rather than the cost of
    /// the first one.
    #[test]
    #[serial]
    fn a_run_of_drags_leaves_the_scene_the_size_it_started() {
        let (handle, mut client) = start();
        let origin = window_with_pointer(&handle, &mut client);

        let one_drag = |handle: &HeadlessHandle, client: &mut TestClient| {
            let (icon, buffer) = client.create_drag_icon(64, 64);
            client.roundtrip().expect("roundtrip");
            drag_with_icon(handle, client, &origin, &icon);
            drop_it(handle, client);
            // What otto-kit does: the icon is kept until the next drag needs
            // one, then destroyed.
            icon.destroy();
            client.roundtrip().expect("roundtrip");
            handle.settle(120);
            drop(buffer);
        };

        // Baseline after a few full drags, so what is compared is steady
        // state. The first drags pay one-off costs — a compositor lazily
        // builds scene furniture the first time a drag needs it — and those
        // are not what this test is looking for.
        for _ in 0..3 {
            one_drag(&handle, &mut client);
        }
        handle.wait(Duration::from_millis(120));
        handle.settle(200);
        let layers_before = handle.surface_layer_count();
        let nodes_before = handle.scene_node_count();

        for _ in 0..ROUNDS {
            one_drag(&handle, &mut client);
        }
        handle.wait(Duration::from_millis(120));
        handle.settle(200);

        let layers_after = handle.surface_layer_count();
        let nodes_after = handle.scene_node_count();
        assert_eq!(
            layers_after,
            layers_before,
            "{ROUNDS} drags left {} surface layers behind",
            layers_after as i64 - layers_before as i64
        );
        assert_eq!(
            nodes_after,
            nodes_before,
            "{ROUNDS} drags left {} scene nodes behind",
            nodes_after as i64 - nodes_before as i64
        );

        handle.stop();
    }

    /// A refused drop flies the picture home — and then the picture has to
    /// go, on its own, without waiting for another drag to sweep it up.
    ///
    /// The client keeps its icon surface alive here, exactly as otto-kit does,
    /// so nothing but the compositor's own teardown can take these layers
    /// down. Waiting for the next drag would mean an icon (and the buffer
    /// behind it) sitting in the scene for the rest of a session in which the
    /// user drags once.
    #[test]
    #[serial]
    fn a_refused_drop_takes_its_picture_down_when_the_flight_lands() {
        let (handle, mut client) = start();
        let origin = window_with_pointer(&handle, &mut client);

        let (icon, _buffer) = client.create_drag_icon(64, 64);
        client.roundtrip().expect("roundtrip");
        drag_with_icon(&handle, &mut client, &origin, &icon);
        assert_eq!(
            handle.dnd_icon_layer_count(),
            1,
            "the picture is not up in the first place"
        );

        drop_it(&handle, &mut client);

        let left = handle.dnd_icon_layer_count();
        assert_eq!(
            left, 0,
            "{left} layers still under the drag view after the flight home;              nothing else will take them down until the next drag"
        );

        handle.stop();
    }

    /// The sweep list is bookkeeping, not a cache: it must not grow across
    /// drags either, or the compositor holds ids for surfaces long gone.
    #[test]
    #[serial]
    fn the_sweep_list_does_not_grow_across_drags() {
        let (handle, mut client) = start();
        let origin = window_with_pointer(&handle, &mut client);
        let mut icons = Vec::new();

        for round in 0..ROUNDS {
            let (icon, buffer) = client.create_drag_icon(64, 64);
            client.roundtrip().expect("roundtrip");
            drag_with_icon(&handle, &mut client, &origin, &icon);

            let tracked = handle.dnd_tracked_layer_count();
            assert_eq!(
                tracked, 1,
                "round {round}: {tracked} icon surfaces tracked for one drag"
            );

            drop_it(&handle, &mut client);
            icons.push((icon, buffer));
        }

        handle.stop();
    }
}
