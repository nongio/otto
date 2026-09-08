use smithay::backend::input::{KeyState, Keycode};
use smithay::input::keyboard::{FilterResult, KeyboardHandle, KeyboardTarget};
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
        // Smithay's virtual-keyboard dispatch sends the client the virtual
        // keyboard's keymap but does NOT deliver the key itself on the
        // non-IME path — the compositor must forward it (see smithay's anvil
        // example). Without this, every virtual-keyboard key (ydotool/wlrctl,
        // the otto-rdp bridge, otto-emoji) is silently dropped.
        //
        // It is forwarded from inside the filter rather than by letting
        // `input` forward it: the forwarding path "makes sure the keymap is
        // up to date" by re-sending the seat's own keymap, which lands
        // *between* the virtual keyboard's keymap and its key. A client then
        // decodes the key against the physical layout — an emoji typed
        // through a one-key keymap arrives as Escape. Sending the key to the
        // focus directly keeps the virtual keymap in force for it; the seat's
        // keymap is restored the next time a physical key is forwarded.
        //
        // Synthesized typing should reach apps, not run compositor shortcuts
        // (and never the Quit binding), so the shortcut filter is bypassed.
        let serial = SERIAL_COUNTER.next_serial();
        let seat = self.seat.clone();
        let focus = keyboard.current_focus();
        keyboard.input::<(), _>(self, keycode, state, serial, time, |otto, _, key| {
            if let Some(focus) = focus.as_ref() {
                KeyboardTarget::key(focus, &seat, otto, key, state, serial, time);
            }
            FilterResult::Intercept(())
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
