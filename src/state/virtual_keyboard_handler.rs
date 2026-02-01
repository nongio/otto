use smithay::backend::input::{KeyState, Keycode};
use smithay::input::keyboard::KeyboardHandle;
use smithay::wayland::virtual_keyboard::VirtualKeyboardHandler;
use xkbcommon::xkb::ModMask;

use crate::state::Backend;
use crate::state::Otto;

impl<BackendData: Backend> VirtualKeyboardHandler for Otto<BackendData> {
    fn on_keyboard_event(
        &mut self,
        _keycode: Keycode,
        _state: KeyState,
        _time: u32,
        _keyboard: KeyboardHandle<Self>,
    ) {
        // Smithay's protocol handler already forwards events to focused clients
        // No additional handling needed
    }

    fn on_keyboard_modifiers(
        &mut self,
        _depressed_mods: ModMask,
        _latched_mods: ModMask,
        _locked_mods: ModMask,
        _keyboard: KeyboardHandle<Self>,
    ) {
        // Smithay's protocol handler already updates modifier state
        // No additional handling needed
    }
}
