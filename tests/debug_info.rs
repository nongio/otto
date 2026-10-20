//! `org.otto.Compositor.DebugInfo` reports the session a bug happened in.
//!
//! otto-msg doctor reads it to fill the "Otto" and "Screens" sections of a bug
//! report, so the fields it reads must be there and match the outputs.

#[cfg(feature = "headless")]
mod debug_info_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use serial_test::serial;

    #[test]
    #[serial]
    fn reports_version_backend_and_every_output() {
        let config = HeadlessConfig::default();
        let (width_px, height_px) = (config.width, config.height);
        let handle = HeadlessHandle::start(config);
        handle.add_output("headless-2", 800, 600, 4000, 0);
        handle.settle(10);

        let info = handle.debug_info_json();

        assert_eq!(info["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(info["backend"], "headless");
        let outputs = info["outputs"].as_array().expect("outputs is an array");
        assert_eq!(outputs.len(), 2, "both outputs are listed: {info}");
        let second = outputs
            .iter()
            .find(|o| o["name"] == "headless-2")
            .expect("the added output is listed");
        assert_eq!(second["width"], 800);
        assert_eq!(second["height"], 600);
        assert_eq!(second["refresh_mhz"], 60_000);
        assert_eq!(second["make"], "Otto");
        assert_eq!(second["x"], 4000);
        let first = outputs
            .iter()
            .find(|o| o["name"] != "headless-2")
            .expect("the headless output is listed");
        assert_eq!(first["width"], width_px);
        assert_eq!(first["height"], height_px);
        assert!(first["scale"].as_f64().is_some_and(|s| s > 0.0));

        handle.stop();
    }
}
