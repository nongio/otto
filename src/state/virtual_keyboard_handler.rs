use smithay::backend::input::{KeyState, Keycode};
use smithay::input::keyboard::{FilterResult, KeyboardHandle};
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::virtual_keyboard::VirtualKeyboardHandler;
use xkbcommon::xkb::ModMask;

use crate::state::Backend;
use crate::state::Otto;

impl<BackendData: Backend> VirtualKeyboardHandler for Otto<BackendData> {
    fn on_keyboard_event(
        &mut self,
        keycode: Keycode,
        state: KeyState,
        time: u32,
        keyboard: KeyboardHandle<Self>,
    ) {
        // Smithay's virtual-keyboard dispatch sends the client the keymap but
        // does NOT deliver the key itself on the non-IME path — the compositor
        // must forward it (see smithay's anvil example). Without this, every
        // virtual-keyboard key (ydotool/wlrctl and the otto-rdp bridge) is
        // silently dropped. Forward to the focused client rather than running
        // the shortcut filter: remote/synthesized typing should reach apps,
        // not trigger compositor shortcuts (and never the Quit binding).
        let serial = SERIAL_COUNTER.next_serial();
        keyboard.input::<(), _>(self, keycode, state, serial, time, |_, _, _| {
            FilterResult::Forward
        });
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
