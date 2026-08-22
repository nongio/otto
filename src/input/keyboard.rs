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

// ── Debug plane PNG dumps (debug-kms feature only) ───────────────────────────
// Shift+6/7/8/9 — save the bg / windows / expose / overlay plane to PNG.
#[cfg(feature = "debug-kms")]
pub static DBG_SAVE_BG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "debug-kms")]
pub static DBG_SAVE_WIN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "debug-kms")]
pub static DBG_SAVE_EXPOSE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "debug-kms")]
pub static DBG_SAVE_OVERLAY: std::sync::atomic::AtomicBool =
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

/// Escape, in the xkb keycode convention (evdev code + 8). Ctrl+Alt+Escape
/// locks the session, and is read from the raw code for the same reason VT
/// switching is: it must work whatever the layout, and whatever has the
/// keyboard.
const KEY_ESC: u32 = 1 + 8;

/// The hardware power button, in the xkb keycode convention. Read raw for the
/// same reason as Escape above: on a laptop it is the one key that has to work
/// whatever holds the keyboard — a lock screen, a greeter, a fullscreen game.
const KEY_POWER: u32 = 116 + 8;

/// Map a raw evdev function-key code to the VT it switches to.
///
/// Read from raw keycodes rather than keysyms so the mapping holds regardless
/// of the active layout, and so it can be checked without consuming the event
/// through the xkb state.
fn function_key_vt(keycode: u32) -> Option<i32> {
    // Keycodes arrive in the xkb convention (evdev + 8), as produced by
    // `KeyboardKeyEvent::key_code`. KEY_F1..KEY_F10 are contiguous; F11 and
    // F12 sit elsewhere in the evdev table.
    const KEY_F1: u32 = 59 + 8;
    const KEY_F10: u32 = 68 + 8;
    const KEY_F11: u32 = 87 + 8;
    const KEY_F12: u32 = 88 + 8;

    match keycode {
        KEY_F1..=KEY_F10 => Some((keycode - KEY_F1 + 1) as i32),
        KEY_F11 => Some(11),
        KEY_F12 => Some(12),
        _ => None,
    }
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

        // VT switching must never be swallowable. An exclusive layer surface
        // (a greeter, a lock screen) otherwise receives every key including
        // Ctrl+Alt+F<n> and Ctrl+Alt+Backspace, leaving no way off the session
        // short of cutting the power. Checked against raw keycodes before the
        // grab below, since the grab forwards unconditionally and returns.
        if matches!(state, KeyState::Pressed) {
            let mods = keyboard.modifier_state();
            if mods.ctrl && mods.alt {
                if let Some(vt) = function_key_vt(keycode.raw()) {
                    self.current_modifiers = mods;
                    return KeyAction::VtSwitch(vt);
                }
                if keycode.raw() == KEY_ESC {
                    self.current_modifiers = mods;
                    return KeyAction::LockSession;
                }
            }

            // The power button, when Otto is configured to act on it. Checked
            // here, before the lock/greeter grabs below, so it keeps working
            // from a locked session — and left untouched (delivered as a plain
            // keysym, logind's `HandlePowerKey` deciding) when the action is
            // `ignore`.
            if keycode.raw() == KEY_POWER {
                let action = crate::config::Config::with(|c| c.power_management.on_power_button);
                if action != crate::config::PowerButtonAction::Ignore {
                    return KeyAction::PowerButton;
                }
            }
        }

        // A locked session has no shortcuts: every key belongs to the locker,
        // which owns keyboard focus. VT switching above is the exception, and
        // is checked before this. The key still has to go through
        // `keyboard.input` so the focused lock surface receives it.
        if self.is_session_locked() {
            keyboard.input::<(), _>(self, keycode, state, serial, time, |_, _, _| {
                FilterResult::Forward
            });
            self.current_modifiers = keyboard.modifier_state();
            return KeyAction::None;
        }

        // Renaming a workspace label grabs the keyboard the same way: every
        // key is text, so no shortcut fires and nothing reaches a client. The
        // selector view owns focus and gets the key through `keyboard.input`.
        if self.workspaces.is_label_editing() {
            keyboard.input::<(), _>(self, keycode, state, serial, time, |_, _, _| {
                FilterResult::Forward
            });
            self.current_modifiers = keyboard.modifier_state();
            return KeyAction::None;
        }

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
                    self.current_modifiers = keyboard.modifier_state();
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

                    // Debug plane PNG dumps. Shift-modified so clients still
                    // receive plain digit keys even in debug-kms builds.
                    #[cfg(feature = "debug-kms")]
                    if matches!(state, KeyState::Pressed)
                        && modifiers.shift
                        && !modifiers.ctrl
                        && !modifiers.alt
                        && !modifiers.logo
                    {
                        use std::sync::atomic::Ordering;
                        let toggled = match keysym {
                            Keysym::_6 | Keysym::asciicircum => {
                                DBG_SAVE_BG.store(true, Ordering::Relaxed);
                                Some("save bg")
                            }
                            Keysym::_7 | Keysym::ampersand => {
                                DBG_SAVE_WIN.store(true, Ordering::Relaxed);
                                Some("save win")
                            }
                            Keysym::_8 | Keysym::asterisk => {
                                DBG_SAVE_EXPOSE.store(true, Ordering::Relaxed);
                                Some("save expose")
                            }
                            Keysym::_9 | Keysym::parenleft => {
                                DBG_SAVE_OVERLAY.store(true, Ordering::Relaxed);
                                Some("save overlay")
                            }
                            _ => None,
                        };
                        if let Some(msg) = toggled {
                            tracing::info!(target: "otto::planes", "debug plane dump: {msg}");
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

        // Update current modifiers state. Read it back from the keyboard rather
        // than only from the filter's snapshot: the filter is skipped on some
        // paths (debug dumps), and the handle is authoritative after `input`.
        self.current_modifiers = keyboard.modifier_state();

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

    /// Release every key the keyboard still believes is held.
    ///
    /// Needed whenever key events can be missed — leaving the session for
    /// another VT, losing X11 focus — otherwise a modifier that was pressed
    /// before we left stays latched forever and silently arms every
    /// modifier-gated interaction (window-drag tiling, shortcuts).
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
        self.current_modifiers = keyboard.modifier_state();
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

    /// Ctrl+Alt+F<n> is the only guaranteed way off a session that an exclusive
    /// layer surface (greeter, lock screen) has grabbed the keyboard on. An
    /// off-by-8 here silently switches to the wrong VT, so pin the mapping.
    #[test]
    fn function_keys_map_to_their_vt() {
        // Keycodes are xkb-style: evdev code + 8.
        assert_eq!(function_key_vt(59 + 8), Some(1), "F1 -> vt1");
        assert_eq!(function_key_vt(60 + 8), Some(2), "F2 -> vt2");
        assert_eq!(function_key_vt(68 + 8), Some(10), "F10 -> vt10");
        assert_eq!(function_key_vt(87 + 8), Some(11), "F11 -> vt11");
        assert_eq!(function_key_vt(88 + 8), Some(12), "F12 -> vt12");
    }

    #[test]
    fn other_keys_do_not_switch_vt() {
        // Raw evdev values (without the +8) must not be mistaken for F-keys,
        // nor should ordinary letters.
        assert_eq!(function_key_vt(59), None, "raw evdev F1 is not a keycode");
        assert_eq!(function_key_vt(30 + 8), None, "'a' must not switch VT");
        assert_eq!(function_key_vt(1 + 8), None, "Escape must not switch VT");
        assert_eq!(function_key_vt(0), None);
    }
}
