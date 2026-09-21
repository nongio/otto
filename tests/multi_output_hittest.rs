//! A layer-shell surface answers the pointer only on its own output.
//!
//! Every output's scene subtree sits at the scene origin, so the pointer hit
//! test works in coordinates local to the output under it. Surfaces on other
//! outputs share that local space, and must not be tested against it: a panel
//! at the top-left of one monitor would otherwise take the clicks at the
//! top-left of every other monitor.

#[cfg(feature = "headless")]
mod multi_output_hittest_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::{KeyboardInteractivity, Layer, TestClient};
    use serial_test::serial;
    use std::time::Duration;

    #[test]
    #[serial]
    fn a_panel_is_not_hit_from_another_output() {
        let config = HeadlessConfig::default();
        let width_px = config.width;
        let height_px = config.height;
        let handle = HeadlessHandle::start(config);
        let mut client =
            TestClient::connect(&handle.socket_name).expect("Failed to connect to compositor");

        let _panel = client.create_layer_surface(
            "test-bar",
            Layer::Top,
            200,
            30,
            KeyboardInteractivity::None,
        );
        handle.wait(Duration::from_millis(200));
        let _ = client.roundtrip();
        handle.settle(100);

        // A second monitor to the right of the first, touching its edge.
        let scale = handle.output_scale();
        let first_width = (width_px as f64 / scale).round() as i32;
        handle.add_output("headless-2", width_px, height_px, first_width, 0);
        handle.settle(100);

        assert_eq!(
            handle.layer_namespace_under(10.0, 10.0).as_deref(),
            Some("test-bar"),
            "the panel answers on its own output"
        );
        assert_eq!(
            handle.layer_namespace_under(first_width as f64 + 10.0, 10.0),
            None,
            "the same local point on the other output must miss the panel"
        );

        handle.stop();
    }
}
