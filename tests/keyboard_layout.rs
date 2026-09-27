//! End-to-end tests for the keyboard layout a status bar follows.
//!
//! The layouts are set the way Settings sets them, through the compositor's
//! own settings entry point, which rebuilds the seat's keymap live. Then the
//! `xkb_switch_layout` command moves between them, and `GetInputs` has to say
//! which one is active — the answer otto-bar draws its indicator from.

#[cfg(feature = "headless")]
mod keyboard_layout_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto::settings::value::SettingValue;
    use serial_test::serial;

    fn keyboard(handle: &HeadlessHandle) -> serde_json::Value {
        let inputs = handle.inputs_json();
        inputs
            .as_array()
            .and_then(|inputs| inputs.first())
            .cloned()
            .expect("GetInputs lists the keyboard")
    }

    fn active(handle: &HeadlessHandle) -> u64 {
        keyboard(handle)["xkb_active_layout_index"]
            .as_u64()
            .expect("an active index")
    }

    fn run(handle: &HeadlessHandle, command: &str) -> Vec<Result<(), String>> {
        let results = handle.run_command(command);
        handle.settle(50);
        results
    }

    #[test]
    #[serial]
    fn switching_layout_is_reported_by_get_inputs() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        handle
            .set_setting("input.xkb_layout", SettingValue::Str("us,it".into()))
            .expect("the keymap accepts us,it");
        handle.settle(50);

        let keyboard_now = keyboard(&handle);
        assert_eq!(
            keyboard_now["otto_layout_codes"],
            serde_json::json!(["us", "it"])
        );
        assert_eq!(
            keyboard_now["xkb_layout_names"]
                .as_array()
                .map(|names| names.len()),
            Some(2)
        );
        assert_eq!(keyboard_now["otto_show_in_bar"], serde_json::json!(true));
        assert_eq!(active(&handle), 0);

        assert_eq!(
            run(&handle, "input type:keyboard xkb_switch_layout next"),
            [Ok(())]
        );
        assert_eq!(active(&handle), 1);
        assert_eq!(
            keyboard(&handle)["xkb_active_layout_name"],
            keyboard(&handle)["xkb_layout_names"][1]
        );

        // Cycling wraps, an index goes straight there, and one past the end
        // is refused rather than ignored.
        assert_eq!(run(&handle, "input * xkb_switch_layout next"), [Ok(())]);
        assert_eq!(active(&handle), 0);
        assert_eq!(
            run(&handle, "input otto:keyboard xkb_switch_layout 1"),
            [Ok(())]
        );
        assert_eq!(active(&handle), 1);
        assert!(run(&handle, "input type:keyboard xkb_switch_layout 2")[0].is_err());
        assert_eq!(active(&handle), 1);
    }

    #[test]
    #[serial]
    fn the_bar_toggle_is_reported_and_applies_live() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        handle
            .set_setting("input.show_layout_in_bar", SettingValue::Bool(false))
            .expect("the toggle is live");
        assert_eq!(
            keyboard(&handle)["otto_show_in_bar"],
            serde_json::json!(false)
        );
    }
}
