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

// ── Debug plane toggles (debug-kms feature only) ─────────────────────────────
// 1/2/3/4/5 — toggle background / windows / expose / overlay / top-window planes.
// 6/7/8/9/0 — save those same planes to PNG.
#[cfg(feature = "debug-kms")]
pub static DBG_PLANE_BG: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "debug-kms")]
pub static DBG_PLANE_WIN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "debug-kms")]
pub static DBG_PLANE_EXPOSE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "debug-kms")]
pub static DBG_PLANE_OVERLAY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "debug-kms")]
pub static DBG_PLANE_TOP_WIN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "debug-kms")]
pub static DBG_SAVE_BG: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "debug-kms")]
pub static DBG_SAVE_WIN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "debug-kms")]
pub static DBG_SAVE_EXPOSE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "debug-kms")]
pub static DBG_SAVE_OVERLAY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "debug-kms")]
pub static DBG_SAVE_TOP_WIN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

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
        .find(|binding| binding.trigger.matches(&modifiers, keysym))
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

                    // Debug plane toggles — highest priority, always intercept.
                    #[cfg(feature = "debug-kms")]
                    if matches!(state, KeyState::Pressed)
                        && !modifiers.ctrl && !modifiers.alt && !modifiers.logo && !modifiers.shift
                    {
                        use std::sync::atomic::Ordering;
                        let toggled = match keysym {
                            Keysym::_1 => { let v = !DBG_PLANE_BG.load(Ordering::Relaxed); DBG_PLANE_BG.store(v, Ordering::Relaxed); Some(format!("bg={v}")) }
                            Keysym::_2 => { let v = !DBG_PLANE_WIN.load(Ordering::Relaxed); DBG_PLANE_WIN.store(v, Ordering::Relaxed); Some(format!("win={v}")) }
                            Keysym::_3 => { let v = !DBG_PLANE_EXPOSE.load(Ordering::Relaxed); DBG_PLANE_EXPOSE.store(v, Ordering::Relaxed); Some(format!("expose={v}")) }
                            Keysym::_4 => { let v = !DBG_PLANE_OVERLAY.load(Ordering::Relaxed); DBG_PLANE_OVERLAY.store(v, Ordering::Relaxed); Some(format!("overlay={v}")) }
                            Keysym::_5 => { let v = !DBG_PLANE_TOP_WIN.load(Ordering::Relaxed); DBG_PLANE_TOP_WIN.store(v, Ordering::Relaxed); Some(format!("top_win={v}")) }
                            Keysym::_6 => { DBG_SAVE_BG.store(true, Ordering::Relaxed); Some("save bg".into()) }
                            Keysym::_7 => { DBG_SAVE_WIN.store(true, Ordering::Relaxed); Some("save win".into()) }
                            Keysym::_8 => { DBG_SAVE_EXPOSE.store(true, Ordering::Relaxed); Some("save expose".into()) }
                            Keysym::_9 => { DBG_SAVE_OVERLAY.store(true, Ordering::Relaxed); Some("save overlay".into()) }
                            Keysym::_0 => { DBG_SAVE_TOP_WIN.store(true, Ordering::Relaxed); Some("save top_win".into()) }
                            _ => None,
                        };
                        if let Some(msg) = toggled {
                            tracing::info!(target: "otto::planes", "debug plane toggle: {msg}");
                            suppressed_keys.push(keysym);
                            return FilterResult::Intercept(KeyAction::None);
                        }
                    }

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
    #[allow(clippy::field_reassign_with_default)]
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
    #[allow(clippy::field_reassign_with_default)]
    fn test_app_switcher_hold_is_active_with_ctrl() {
        let mut hold = ModifiersState::default();
        hold.ctrl = true;
        let mut current = ModifiersState::default();
        current.ctrl = true;
        assert!(app_switcher_hold_is_active(Some(hold), current));
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn test_app_switcher_hold_not_active_when_released() {
        let mut hold = ModifiersState::default();
        hold.ctrl = true;
        let current = ModifiersState::default();
        assert!(!app_switcher_hold_is_active(Some(hold), current));
    }
}
