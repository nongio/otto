use smithay::wayland::{
    compositor::with_states, keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitorSeat,
};
use smithay::{
    backend::input::{Event, InputBackend, InputTime, KeyState, KeyboardKeyEvent},
    desktop::layer_map_for_output,
    input::keyboard::{xkb, FilterResult, Keysym, ModifiersState},
    utils::{IsAlive, SERIAL_COUNTER as SCOUNTER},
    wayland::seat::WaylandFocus,
    wayland::shell::wlr_layer::{
        KeyboardInteractivity, Layer as WlrLayer, LayerSurfaceCachedState,
    },
};

use std::time::Duration;

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

/// The Cmd keys, in the xkb keycode convention (evdev code + 8).
const KEY_LEFTMETA: u32 = 125 + 8;
const KEY_RIGHTMETA: u32 = 126 + 8;

/// Whether a keycode is one of the physical Cmd keys.
pub fn is_cmd_keycode(keycode: u32) -> bool {
    keycode == KEY_LEFTMETA || keycode == KEY_RIGHTMETA
}

/// The xkb option that folds the Cmd keys into the Control modifier.
///
/// It maps `<LWIN>`/`<RWIN>` to `Control_L`/`Control_R` so that Cmd+C reaches
/// clients as Ctrl+C, the way every toolkit expects. The cost is that Cmd and
/// the real Ctrl key become the same event, which is what
/// [`shortcut_modifiers`] undoes for shortcut matching.
///
/// Otto never sets this — the layout is the config's to choose. This is only
/// the name to recognise when deciding whether that undoing applies.
const CMD_IS_CTRL_OPTION: &str = "altwin:ctrl_win";

/// Whether shortcuts should follow the Cmd key alone.
///
/// `input.mac_style_modifiers` decides, and when it is unset the layout does:
/// the behaviour is only meaningful under [`CMD_IS_CTRL_OPTION`], so a config
/// that asks for that option gets it without having to say so twice.
fn cmd_is_ctrl(config: &Config) -> bool {
    config.input.mac_style_modifiers.unwrap_or_else(|| {
        config
            .input
            .xkb_options
            .iter()
            .any(|option| option == CMD_IS_CTRL_OPTION)
    })
}

/// The modifiers a shortcut is matched against.
///
/// Under `altwin:ctrl_win` both Cmd and the real Ctrl key raise
/// `modifiers.ctrl`, so a binding like `Ctrl+W` fires from either — closing the
/// window when the user meant `^W` in a terminal. Otto's shortcuts belong to
/// Cmd; the real Ctrl key belongs to the focused client. Report `ctrl` only
/// while a Cmd key is physically held.
///
/// Only the *matching* is affected. The event forwarded to the client still
/// carries a plain Control modifier from either key, so `^W`/`^C` keep working
/// in a terminal and Cmd+C/V/X keep working everywhere else.
pub fn shortcut_modifiers(
    modifiers: ModifiersState,
    cmd_held: bool,
    cmd_is_ctrl: bool,
) -> ModifiersState {
    let mut modifiers = modifiers;
    if cmd_is_ctrl {
        modifiers.ctrl = modifiers.ctrl && cmd_held;
    }
    modifiers
}

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

/// Whether `action` still fires while a modal layer surface holds the keyboard.
///
/// Two kinds get through. The shortcuts whose UI draws above the overlay
/// layer — the app switcher, and the OSD for volume and brightness — so using
/// them never leaves something hidden behind the modal. And the ones that
/// leave the windows alone: the person's own commands (a screenshot, a
/// dictation toggle), the media keys, locking, and the debug snapshots. What
/// stays out is everything that acts on windows and workspaces, which the
/// modal is covering.
fn fires_over_modal_layers(action: &KeyAction) -> bool {
    matches!(
        action,
        KeyAction::ApplicationSwitchNext
            | KeyAction::ApplicationSwitchPrev
            | KeyAction::ApplicationSwitchNextWindow
            | KeyAction::VolumeUp
            | KeyAction::VolumeDown
            | KeyAction::VolumeMute
            | KeyAction::BrightnessUp
            | KeyAction::BrightnessDown
            | KeyAction::Run(_)
            | KeyAction::MediaPlayPause
            | KeyAction::MediaNext
            | KeyAction::MediaPrev
            | KeyAction::MediaStop
            | KeyAction::LockSession
            | KeyAction::SceneSnapshot
            | KeyAction::SkpSnapshot
    )
}

/// Whether `action` still fires while the focused client holds a
/// keyboard-shortcuts inhibitor.
///
/// An inhibitor hands the compositor's shortcuts to the client — what a VM
/// or a remote desktop viewer wants. But every client that asks is granted
/// one, and locking must not be something an app can switch off: a window
/// that could keep the lock shortcut from working could keep the user from
/// locking the screen. Nor may it trap the user on the session, so VT
/// switching goes through too. (`Ctrl+Alt+Escape` and `Ctrl+Alt+F<n>` are
/// matched from raw keycodes before any of this; these are the configured
/// bindings, and the `XF86Switch_VT_<n>` keysyms.)
fn survives_shortcut_inhibition(action: &KeyAction) -> bool {
    matches!(action, KeyAction::LockSession | KeyAction::VtSwitch(_))
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

/// Whether `surface` belongs to the polkit agent Otto started.
pub fn is_authorize_surface(
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
) -> bool {
    use smithay::reexports::wayland_server::Resource;
    surface.client().is_some_and(|client| {
        crate::state::ClientState::component_of(&client)
            == Some(crate::state::OttoComponent::Authorize)
    })
}

impl<BackendData: Backend + 'static> Otto<BackendData> {
    /// The layer surface that takes every key: the newest mapped top or
    /// overlay surface asking for exclusive keyboard interactivity.
    ///
    /// Except while the password panel is up: then it is the panel, whatever
    /// else asks. An overlay mapped after the panel would otherwise be newer,
    /// take the keys, and receive the password typed into what looks like
    /// the panel. The locker needs no such rule: a locked session sends every
    /// key to the lock surface before any layer surface is looked at.
    pub fn modal_keyboard_layer(&self) -> Option<smithay::desktop::LayerSurface> {
        let modal: Vec<_> = self
            .layer_shell_state
            .layer_surfaces()
            .rev()
            .filter(|layer| {
                let data = with_states(layer.wl_surface(), |states| {
                    *states
                        .cached_state
                        .get::<LayerSurfaceCachedState>()
                        .current()
                });
                data.keyboard_interactivity == KeyboardInteractivity::Exclusive
                    && (data.layer == WlrLayer::Top || data.layer == WlrLayer::Overlay)
            })
            .filter_map(|layer| {
                self.workspaces.outputs().find_map(|o| {
                    let map = layer_map_for_output(o);
                    let cloned = map.layers().find(|l| l.layer_surface() == &layer).cloned();
                    cloned
                })
            })
            .collect();
        modal
            .iter()
            .find(|surface| is_authorize_surface(surface.wl_surface()))
            .or_else(|| modal.first())
            .cloned()
    }

    /// Whether the polkit agent has its password panel up.
    pub fn authorize_panel_up(&self) -> bool {
        self.layer_surfaces
            .values()
            .any(|layer| is_authorize_surface(layer.layer_surface().wl_surface()))
    }

    /// Resolve a key event to what Otto does with it, then announce the
    /// keyboard layout if the key switched it (a `grp:` option acts inside
    /// XKB, where no shortcut sees it).
    pub fn keyboard_key_to_action<B: InputBackend>(
        &mut self,
        evt: B::KeyboardKeyEvent,
    ) -> KeyAction {
        let action = self.key_to_action::<B>(evt);
        self.note_active_layout();
        action
    }

    fn key_to_action<B: InputBackend>(&mut self, evt: B::KeyboardKeyEvent) -> KeyAction {
        self.keycode_to_action(evt.key_code(), evt.state(), Event::time(&evt))
    }

    /// [`Self::keyboard_key_to_action`] for a key given by code rather than
    /// as a backend event. `pub(crate)` so the headless harness can press
    /// keys through the whole path.
    pub(crate) fn keycode_to_action(
        &mut self,
        keycode: smithay::input::keyboard::Keycode,
        state: KeyState,
        time: InputTime,
    ) -> KeyAction {
        let serial = SCOUNTER.next_serial();
        let mut suppressed_keys = self.suppressed_keys.clone();
        let keyboard = self.seat.get_keyboard().unwrap();
        let mut updated_modifiers: Option<ModifiersState> = None;
        // A key Otto keeps (a shortcut) still ends the last client's claim to
        // a popup grab; delivery overwrites this with the client that got it.
        // See `crate::input::popup_grab`.
        if matches!(state, KeyState::Pressed) {
            let seat = self.seat.clone();
            self.note_seat_press(&seat, serial, None);
        }

        // Self-heal a release we never saw — a VT switch or a focus change can
        // swallow one, and a Cmd key stuck down would keep every real Ctrl
        // press matching Otto's shortcuts. No key that raises Control is held
        // once the modifier bit clears, so the set has to be empty too.
        if !keyboard.modifier_state().ctrl {
            self.pressed_cmd_keys.clear();
        }

        // Which physical Cmd keys are down, tracked from raw keycodes because
        // the modifier bit alone cannot say which key produced it.
        if is_cmd_keycode(keycode.raw()) {
            match state {
                KeyState::Pressed => {
                    self.pressed_cmd_keys.insert(keycode.raw());
                }
                KeyState::Released => {
                    self.pressed_cmd_keys.remove(&keycode.raw());
                }
            }
        }
        let cmd_held = !self.pressed_cmd_keys.is_empty();

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
            // Nor through any grab: a popup's or an input method's would take
            // the key from the lock surface. Grabs are refused while
            // locked, and dropped here in case one got in anyway.
            let seat = self.seat.clone();
            let locker = self.lock_locker_client.as_ref().map(|c| c.id());
            self.release_grabs_not_held_by(&seat, locker.as_ref(), false);
            self.refresh_lock_focus();
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

        // An open app switcher owns the keys until its modifier is released —
        // the Tab that advances it and the release that commits it — even
        // over a modal layer surface. Committing focuses a window, and a modal
        // that loses the keyboard (the launcher) closes, so the switcher,
        // being the later of the two, wins.
        //
        // The password panel is the exception to the exception: while it is
        // up, the switcher does not get the keys either.
        let modal_layer = if self.workspaces.app_switcher.alive() && !self.authorize_panel_up() {
            None
        } else {
            self.modal_keyboard_layer()
        };
        if let Some(surface) = modal_layer {
            // The password panel's keys go to the panel whatever grab is in
            // place: a popup grab would ignore the focus change below, and an
            // input method's would be sent the password.
            if is_authorize_surface(surface.wl_surface()) {
                let seat = self.seat.clone();
                let panel =
                    smithay::reexports::wayland_server::Resource::client(surface.wl_surface())
                        .map(|c| c.id());
                self.release_grabs_not_held_by(&seat, panel.as_ref(), false);
            }
            keyboard.set_focus(self, Some(surface.into()), serial);
            // Every key is the surface's except the shortcuts that leave
            // the windows under it alone: see `fires_over_modal_layers`.
            let mut suppressed_keys = self.suppressed_keys.clone();
            let mut pressed_modifiers = None;
            let action = keyboard
                .input(
                    self,
                    keycode,
                    state,
                    serial,
                    time,
                    |_, modifiers, handle| {
                        let keysym = handle.modified_sym();
                        if let KeyState::Pressed = state {
                            let action = Config::with(|config| {
                                let modifiers =
                                    shortcut_modifiers(*modifiers, cmd_held, cmd_is_ctrl(config));
                                process_keyboard_shortcut(config, modifiers, keysym)
                            })
                            .filter(fires_over_modal_layers);
                            match action {
                                Some(action) => {
                                    suppressed_keys.push(keysym);
                                    pressed_modifiers = Some(*modifiers);
                                    FilterResult::Intercept(action)
                                }
                                None => FilterResult::Forward,
                            }
                        } else if suppressed_keys.contains(&keysym) {
                            suppressed_keys.retain(|k| *k != keysym);
                            FilterResult::Intercept(KeyAction::None)
                        } else {
                            FilterResult::Forward
                        }
                    },
                )
                .unwrap_or(KeyAction::None);
            if let Some(modifiers) = pressed_modifiers.filter(|_| {
                matches!(
                    action,
                    KeyAction::ApplicationSwitchNext
                        | KeyAction::ApplicationSwitchPrev
                        | KeyAction::ApplicationSwitchNextWindow
                )
            }) {
                self.app_switcher_hold_modifiers = capture_app_switcher_hold_modifiers(modifiers);
            }
            self.suppressed_keys = suppressed_keys;
            self.current_modifiers = keyboard.modifier_state();
            return action;
        }

        // The protocol ties an inhibitor to the surface with keyboard focus,
        // not the one under the pointer: a shortcut recorder keeps receiving
        // the compositor's own shortcuts after the mouse wanders off it.
        let inhibited = keyboard
            .current_focus()
            .and_then(|focus| {
                let surface = focus.wl_surface()?;
                self.seat.keyboard_shortcuts_inhibitor_for_surface(&surface)
            })
            .map(|inhibitor| inhibitor.is_active())
            .unwrap_or(false);

        // A drag out of a tree owns Escape, which cancels it.
        let tiling_drag_active = self.tiling_drag_is_active();

        // Assistive technologies are offered the key before anything else in
        // the session sees it. Cloned out of `self` because `keyboard.input`
        // borrows the state for the duration of the filter.
        let a11y = self.a11y.keyboard.clone();
        let repeat_delay = Duration::from_millis(
            Config::with(|config| config.keyboard_repeat_delay).max(0) as u64,
        );
        let event_time = Duration::from_millis(u64::from(time.millis()));

        let action = keyboard
            .input(
                self,
                keycode,
                state,
                serial,
                time,
                |_, modifiers, handle| {
                    let keysym = handle.modified_sym();

                    // A screen reader's own keys never reach the session: no
                    // shortcut fires, the focused client is not told, and a
                    // toggle does not flip. Its release is swallowed too, so a
                    // client never sees half of a keystroke.
                    let serialized = modifiers.serialized;
                    let grabbed = a11y.process_key(
                        repeat_delay,
                        event_time,
                        keycode.raw() as u16,
                        matches!(state, KeyState::Released),
                        serialized.depressed | serialized.latched | serialized.locked,
                        keysym,
                        xkb::keysym_to_utf32(keysym),
                    );
                    if grabbed {
                        return FilterResult::Intercept(KeyAction::None);
                    }

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

                    // Escape puts a window being dragged out of a tree back
                    // in the slot it came from.
                    if tiling_drag_active
                        && matches!(state, KeyState::Pressed)
                        && keysym == Keysym::Escape
                    {
                        suppressed_keys.push(keysym);
                        return FilterResult::Intercept(KeyAction::TilingDragCancel);
                    }

                    let shortcut_action = Config::with(|config| {
                        if matches!(state, KeyState::Pressed) {
                            let modifiers =
                                shortcut_modifiers(*modifiers, cmd_held, cmd_is_ctrl(config));
                            process_keyboard_shortcut(config, modifiers, keysym)
                                .filter(|action| !inhibited || survives_shortcut_inhibition(action))
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

    /// Commit the app switcher's selection: hide the panel and focus the app
    /// it landed on. `pub(crate)` so the headless harness can finish an
    /// alt-tab the way releasing the modifier does.
    pub(crate) fn dismiss_app_switcher(&mut self) {
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
                InputTime::from_millis(0),
                |_, _, _| FilterResult::Forward::<bool>,
            );
        }
        self.current_modifiers = keyboard.modifier_state();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `altwin:ctrl_win` makes Cmd and the real Ctrl key indistinguishable by
    /// modifier alone, so a `Ctrl+W` binding closed the window when the user
    /// meant `^W` in a terminal. Shortcuts follow Cmd; the real Ctrl key is the
    /// client's.
    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn cmd_drives_shortcuts_and_real_ctrl_does_not() {
        let mut mods = ModifiersState::default();
        mods.ctrl = true;

        // Cmd+W: the Control bit came from a Cmd key, so `Ctrl+W` matches.
        assert!(shortcut_modifiers(mods, true, true).ctrl);

        // Ctrl+W: no Cmd key held, so no binding matches and the key is
        // forwarded to the focused client as `^W`.
        assert!(!shortcut_modifiers(mods, false, true).ctrl);
    }

    /// Without the remap the Cmd keys raise `logo`, never `ctrl`, so gating on
    /// a held Cmd key would leave a stock config with no working Ctrl
    /// shortcuts at all.
    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn ctrl_is_untouched_without_the_remap() {
        let mut mods = ModifiersState::default();
        mods.ctrl = true;
        assert!(shortcut_modifiers(mods, false, false).ctrl);
    }

    /// Only `ctrl` is reinterpreted — the other modifiers still combine with it.
    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn other_modifiers_survive() {
        let mut mods = ModifiersState::default();
        mods.ctrl = true;
        mods.shift = true;
        mods.alt = true;
        let result = shortcut_modifiers(mods, true, true);
        assert!(result.ctrl && result.shift && result.alt);
    }

    #[test]
    fn cmd_keycodes_are_the_win_keys() {
        assert!(is_cmd_keycode(125 + 8));
        assert!(is_cmd_keycode(126 + 8));
        // The real Ctrl keys, which must keep falling through to the client.
        assert!(!is_cmd_keycode(29 + 8));
        assert!(!is_cmd_keycode(97 + 8));
    }

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
