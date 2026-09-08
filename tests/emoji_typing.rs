//! otto-emoji types its pick through a virtual keyboard with a keymap of its
//! own. This runs that path against the real compositor: a client takes the
//! keyboard, the picker's typing routine runs, and the client is asked what
//! it received — the keymap and the keys, decoded with libxkbcommon exactly
//! as a toolkit would decode them.

#[cfg(feature = "headless")]
mod headless_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::time::Duration;
    use xkbcommon::xkb;

    /// What `keymap` and `keys` spell, the way a client would work it out.
    fn decode(keymap: &str, keys: &[(u32, bool)]) -> String {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_string(
            &context,
            keymap.to_string(),
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .expect("the client can compile the keymap it was sent");
        let state = xkb::State::new(&keymap);
        keys.iter()
            .filter(|(_, pressed)| *pressed)
            .map(|(key, _)| state.key_get_utf8(xkb::Keycode::new(key + 8)))
            .collect()
    }

    #[test]
    #[serial]
    fn the_focused_client_receives_the_emoji_as_keys() {
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        let mut client = TestClient::connect(&handle.socket_name).expect("connect");
        let _window = client.create_toplevel("emoji-target", 640, 480);
        handle.wait(Duration::from_millis(150));
        client.roundtrip().expect("roundtrip");
        assert!(
            client.state.keyboard_focused,
            "the window should have the keyboard before anything is typed"
        );
        let keymaps_before = client.state.keymaps.len();

        // A joiner sequence and a tone: two glyphs' worth of codepoints, so
        // the keymap has to carry more than one key.
        let text = "👋🏽❤️";
        std::env::set_var("WAYLAND_DISPLAY", &handle.socket_name);
        otto_emoji::typing::type_text(text).expect("typing");

        handle.wait(Duration::from_millis(150));
        client.roundtrip().expect("roundtrip");

        assert!(
            client.state.keymaps.len() > keymaps_before,
            "the virtual keyboard's keymap should reach the focused client"
        );
        let keymap = client.state.keymaps.last().unwrap();
        let typed = decode(keymap, &client.state.keys);
        assert_eq!(typed, text);

        handle.stop();
    }
}
