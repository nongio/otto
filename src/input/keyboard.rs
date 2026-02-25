use smithay::wayland::{
    compositor::with_states, keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitorSeat,
};
use smithay::{
    backend::input::{Event, InputBackend, KeyState, KeyboardKeyEvent},
    desktop::layer_map_for_output,
    input::keyboard::{FilterResult, Keysym, ModifiersState},
    utils::{IsAlive, SERIAL_COUNTER as SCOUNTER},
    wayland::shell::wlr_layer::{
        KeyboardInteractivity, Layer as WlrLayer, LayerSurfaceCachedState,
    },
};

use crate::{config::Config, state::Backend, Otto};

use super::actions::KeyAction;

pub fn capture_app_switcher_hold_modifiers(
    mut modifiers: ModifiersState,
) -> Option<ModifiersState> {
    modifiers.caps_lock = false;
    modifiers.num_lock = false;
    if modifiers.ctrl || modifiers.alt || modifiers.logo || modifiers.shift {
        Some(modifiers)
    } else {
        None
    }
}

pub fn app_switcher_hold_is_active(hold: Option<ModifiersState>, current: ModifiersState) -> bool {
    match hold {
        Some(hold_modifiers) => {
            let has_primary = hold_modifiers.ctrl || hold_modifiers.alt || hold_modifiers.logo;
            if has_primary {
                (hold_modifiers.ctrl && current.ctrl)
                    || (hold_modifiers.alt && current.alt)
                    || (hold_modifiers.logo && current.logo)
            } else if hold_modifiers.shift {
                current.shift
            } else {
                false
            }
        }
        None => current.ctrl || current.alt || current.logo || current.shift,
    }
}

pub fn process_keyboard_shortcut(
    config: &Config,
    modifiers: ModifiersState,
    keysym: Keysym,
) -> Option<KeyAction> {
    use smithay::input::keyboard::xkb::{self, keysyms::*};

    // Log the incoming key event for debugging
    let keysym_name = xkb::keysym_get_name(keysym);
    tracing::trace!(
        "Shortcut check: keysym={} (0x{:x}), ctrl={}, alt={}, shift={}, logo={}",
        keysym_name,
        keysym.raw(),
        modifiers.ctrl,
        modifiers.alt,
        modifiers.shift,
        modifiers.logo
    );

    if modifiers.ctrl && modifiers.alt && keysym == Keysym::BackSpace
        || modifiers.logo && keysym == Keysym::q
    {
        // ctrl+alt+backspace = quit
        // logo + q = quit
        tracing::info!("keyboard shortcut activated");
        return Some(KeyAction::Quit);
    }

    if (KEY_XF86Switch_VT_1..=KEY_XF86Switch_VT_12).contains(&keysym.raw()) {
        return Some(KeyAction::VtSwitch(
            (keysym.raw() - KEY_XF86Switch_VT_1 + 1) as i32,
        ));
    }

    let result = config
        .shortcut_bindings()
        .iter()
        .find(|binding| {
            let matches = binding.trigger.matches(&modifiers, keysym);
            matches
        })
        .and_then(|binding| super::actions::resolve_shortcut_action(config, &binding.action));

    result
}

impl<BackendData: Backend> Otto<BackendData> {
    pub fn keyboard_key_to_action<B: InputBackend>(
        &mut self,
        evt: B::KeyboardKeyEvent,
    ) -> KeyAction {
        let keycode = evt.key_code();
        let state = evt.state();
        let serial = SCOUNTER.next_serial();
        let time = Event::time_msec(&evt);
        let mut suppressed_keys = self.suppressed_keys.clone();
        let keyboard = self.seat.get_keyboard().unwrap();
        let mut updated_modifiers: Option<ModifiersState> = None;

        for layer in self.layer_shell_state.layer_surfaces().rev() {
            let data = with_states(layer.wl_surface(), |states| {
                *states
                    .cached_state
                    .get::<LayerSurfaceCachedState>()
                    .current()
            });
            if data.keyboard_interactivity == KeyboardInteractivity::Exclusive
                && (data.layer == WlrLayer::Top || data.layer == WlrLayer::Overlay)
            {
                let surface = self.workspaces.outputs().find_map(|o| {
                    let map = layer_map_for_output(o);
                    let cloned = map.layers().find(|l| l.layer_surface() == &layer).cloned();
                    cloned
                });
                if let Some(surface) = surface {
                    keyboard.set_focus(self, Some(surface.into()), serial);
                    keyboard.input::<(), _>(self, keycode, state, serial, time, |_, _, _| {
                        FilterResult::Forward
                    });
                    return KeyAction::None;
                };
            }
        }

        let inhibited = self
            .workspaces
            .element_under(self.pointer.current_location())
            .and_then(|(window, _)| {
                let surface = window.wl_surface()?;
                self.seat.keyboard_shortcuts_inhibitor_for_surface(&surface)
            })
            .map(|inhibitor| inhibitor.is_active())
            .unwrap_or(false);

        let action = keyboard
            .input(
                self,
                keycode,
                state,
                serial,
                time,
                |_, modifiers, handle| {
                    let keysym = handle.modified_sym();

                    let shortcut_action = Config::with(|config| {
                        if matches!(state, KeyState::Pressed) && !inhibited {
                            process_keyboard_shortcut(config, *modifiers, keysym)
                        } else {
                            None
                        }
                    });
                    updated_modifiers = Some(*modifiers);

                    // If the key is pressed and triggered an action
                    // we will not forward the key to the client.
                    // Additionally add the key to the suppressed keys
                    // so that we can decide on a release if the key
                    // should be forwarded to the client or not.
                    if let KeyState::Pressed = state {
                        if let Some(action) = shortcut_action {
                            suppressed_keys.push(keysym);
                            FilterResult::Intercept(action)
                        } else {
                            FilterResult::Forward
                        }
                    } else {
                        let suppressed = suppressed_keys.contains(&keysym);
                        if suppressed {
                            suppressed_keys.retain(|k| *k != keysym);
                            FilterResult::Intercept(KeyAction::None)
                        } else {
                            FilterResult::Forward
                        }
                    }
                },
            )
            .unwrap_or(KeyAction::None);

        // Capture modifiers when pressing app switcher actions
        if matches!(state, KeyState::Pressed)
            && matches!(
                action,
                KeyAction::ApplicationSwitchNext
                    | KeyAction::ApplicationSwitchPrev
                    | KeyAction::ApplicationSwitchNextWindow
            )
        {
            if let Some(modifiers) = updated_modifiers {
                self.app_switcher_hold_modifiers = capture_app_switcher_hold_modifiers(modifiers);
            }
        }

        // Check for app switcher dismissal on key release
        if KeyState::Released == state && self.workspaces.app_switcher.alive() {
            if let Some(modifiers) = updated_modifiers {
                if !app_switcher_hold_is_active(self.app_switcher_hold_modifiers, modifiers) {
                    self.dismiss_app_switcher();
                }
            }
        }

        // Update current modifiers state
        if let Some(modifiers) = updated_modifiers {
            self.current_modifiers = modifiers;
        }

        self.suppressed_keys = suppressed_keys;
        action
    }

    fn dismiss_app_switcher(&mut self) {
        if self.workspaces.app_switcher.alive() {
            self.workspaces.app_switcher.hide();
            if let Some(app_id) = self.workspaces.app_switcher.get_current_app_id() {
                self.focus_app(&app_id);
                self.workspaces.app_switcher.reset();
            }
        }
        self.app_switcher_hold_modifiers = None;
    }

    pub fn release_all_keys(&mut self) {
        let keyboard = self.seat.get_keyboard().unwrap();
        for keycode in keyboard.pressed_keys() {
            keyboard.input(
                self,
                keycode,
                KeyState::Released,
                SCOUNTER.next_serial(),
                0,
                |_, _, _| FilterResult::Forward::<bool>,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capture_app_switcher_hold_modifiers() {
        let mut mods = ModifiersState::default();
        mods.ctrl = true;
        let result = capture_app_switcher_hold_modifiers(mods);
        assert!(result.is_some());
        assert!(result.unwrap().ctrl);
    }

    #[test]
    fn test_capture_no_modifiers() {
        let mods = ModifiersState::default();
        let result = capture_app_switcher_hold_modifiers(mods);
        assert!(result.is_none());
    }

    #[test]
    fn test_app_switcher_hold_is_active_with_ctrl() {
        let mut hold = ModifiersState::default();
        hold.ctrl = true;
        let mut current = ModifiersState::default();
        current.ctrl = true;
        assert!(app_switcher_hold_is_active(Some(hold), current));
    }

    #[test]
    fn test_app_switcher_hold_not_active_when_released() {
        let mut hold = ModifiersState::default();
        hold.ctrl = true;
        let current = ModifiersState::default();
        assert!(!app_switcher_hold_is_active(Some(hold), current));
    }
}
