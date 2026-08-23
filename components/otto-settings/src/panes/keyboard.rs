//! The keyboard pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.

use std::sync::{OnceLock, RwLock};

use crate::model::{group, Control, Pane, Row};

/// Builtin actions a shortcut can be bound to.
///
/// **Hard-coded on purpose, and this is the one place.** Shortcuts are a
/// list, and `docs/developer/settings-dbus-api.md` says list-valued settings
/// have no identifier and are not in the served schema — so there is nothing
/// on the bus to ask for the choices. The names are exactly the ones
/// `parse_builtin` in `src/config/shortcuts.rs` accepts; anything else is
/// dropped with a warning when the compositor loads its config.
///
/// `Screen` and `Workspace` are missing because they are the two that need an
/// `index` alongside the name, which a pop-up button cannot carry. So are the
/// `run` and `open_default` forms, which are a command line rather than a
/// choice.
const BUILTIN_ACTIONS: &[&str] = &[
    "ApplicationSwitchNext",
    "ApplicationSwitchNextWindow",
    "ApplicationSwitchPrev",
    "ApplicationSwitchQuit",
    "BrightnessDown",
    "BrightnessUp",
    "CloseWindow",
    "ExposeShowAll",
    "ExposeShowDesktop",
    "LockSession",
    "MediaNext",
    "MediaPlayPause",
    "MediaPrev",
    "MediaStop",
    "Quit",
    "RotateOutput",
    "ScaleDown",
    "ScaleUp",
    "SceneSnapshot",
    "SkpSnapshot",
    "TileWindowLeft",
    "TileWindowRight",
    "ToggleDecorations",
    "ToggleMaximizeWindow",
    "VolumeDown",
    "VolumeMute",
    "VolumeUp",
];

/// How many shortcut lines the pane can hold.
///
/// A cap rather than an open list because every line's action pop-up needs a
/// `DropdownMenu`, and a `DropdownMenu` can only be built at window setup —
/// see `main.rs`. The pool is that size, so the list is too.
pub const MAX_SHORTCUTS: usize = 24;

/// One shortcut line: the action it runs and the combination that triggers it.
#[derive(Clone)]
pub struct Shortcut {
    pub action: String,
    pub keys: String,
}

/// The lines being edited.
///
/// Process-wide because [`crate::model::panes`] rebuilds every pane on every
/// frame, so a row cannot own anything that has to outlive one — the same
/// reason the settings values live in a store rather than in the model.
static SHORTCUTS: OnceLock<RwLock<Vec<Shortcut>>> = OnceLock::new();

fn shortcuts() -> &'static RwLock<Vec<Shortcut>> {
    SHORTCUTS.get_or_init(|| {
        // The set the shipped `otto_config.example.toml` binds. Nothing reads
        // the user's own config yet — see the module docs on the group below.
        RwLock::new(
            [
                ("Quit", "Ctrl+Esc"),
                ("ApplicationSwitchNext", "Ctrl+Tab"),
                ("ApplicationSwitchPrev", "Ctrl+Shift+ISO_Left_Tab"),
                ("ApplicationSwitchNextWindow", "Ctrl+grave"),
                ("ApplicationSwitchQuit", "Ctrl+q"),
                ("ToggleMaximizeWindow", "Ctrl+ArrowUp"),
                ("TileWindowLeft", "Ctrl+ArrowLeft"),
                ("TileWindowRight", "Ctrl+ArrowRight"),
                ("ExposeShowAll", "Prior"),
                ("ExposeShowDesktop", "Next"),
            ]
            .into_iter()
            .map(|(action, keys)| Shortcut {
                action: action.into(),
                keys: keys.into(),
            })
            .collect(),
        )
    })
}

/// The lines as they stand, for a pane build or a draw.
pub fn lines() -> Vec<Shortcut> {
    shortcuts().read().unwrap().clone()
}

/// Identifiers for the action pop-up of each line.
///
/// Leaked rather than formatted per call: the ids key the `DropdownMenu` pool,
/// which is `&'static str` because those menus live as long as the window
/// does. There are [`MAX_SHORTCUTS`] of them and they are made once.
pub fn slot_ids() -> &'static [&'static str] {
    static IDS: OnceLock<Vec<&'static str>> = OnceLock::new();
    IDS.get_or_init(|| {
        (0..MAX_SHORTCUTS)
            .map(|i| &*format!("shortcut.{i}").leak())
            .collect()
    })
}

/// The pop-up identifier for the line at `index`.
pub fn slot_id(index: usize) -> Option<&'static str> {
    slot_ids().get(index).copied()
}

/// The line a pop-up identifier belongs to, or `None` for anything that is not
/// one of ours — which is how `main.rs` tells a shortcut's menu apart from a
/// menu over a real setting.
pub fn slot_index(id: &str) -> Option<usize> {
    slot_ids().iter().position(|slot| *slot == id)
}

/// The actions a line's pop-up offers.
pub fn actions() -> &'static [&'static str] {
    BUILTIN_ACTIONS
}

/// Append a line, unless the pool is full.
pub fn add() {
    let mut lines = shortcuts().write().unwrap();
    if lines.len() >= MAX_SHORTCUTS {
        return;
    }
    lines.push(Shortcut {
        // The first action rather than an empty one: a line with no action is
        // not a shortcut, and the pop-up has to show something.
        action: BUILTIN_ACTIONS[0].to_string(),
        keys: String::new(),
    });
}

pub fn remove(index: usize) {
    let mut lines = shortcuts().write().unwrap();
    if index < lines.len() {
        lines.remove(index);
    }
}

pub fn set_action(index: usize, action: String) {
    if let Some(line) = shortcuts().write().unwrap().get_mut(index) {
        line.action = action;
    }
}

pub fn set_keys(index: usize, keys: String) {
    if let Some(line) = shortcuts().write().unwrap().get_mut(index) {
        line.keys = keys;
    }
}

/// The combination on a line, for opening its field on the value it shows.
pub fn keys(index: usize) -> Option<String> {
    shortcuts()
        .read()
        .unwrap()
        .get(index)
        .map(|l| l.keys.clone())
}

pub fn build() -> Pane {
    // One row per line, plus the explainer above them and the "+" below.
    let mut shortcut_rows = vec![Row::new("Key combination", Control::Value(String::new()))
        .detail("Ctrl, Alt, Shift or Logo joined by +, then one key: Ctrl+Shift+Return")];
    shortcut_rows.extend((0..lines().len()).map(|index| Row::new("", Control::Shortcut { index })));
    if lines().len() < MAX_SHORTCUTS {
        shortcut_rows.push(Row::new("", Control::AddShortcut));
    }

    Pane {
        name: "Keyboard",
        icon: "keyboard",
        groups: vec![
            group(
                None,
                vec![
                    Row::new(
                        "Key repeat delay",
                        Control::Slider {
                            value: 300.0,
                            min: 100.0,
                            max: 1000.0,
                            readout: "300 ms".into(),
                        },
                    )
                    .id("keyboard_repeat_delay"),
                    Row::new(
                        "Key repeat rate",
                        Control::Slider {
                            value: 30.0,
                            min: 5.0,
                            max: 60.0,
                            readout: "30 / s".into(),
                        },
                    )
                    .id("keyboard_repeat_rate"),
                ],
            ),
            group(
                Some("Input source"),
                vec![
                    // Shown, not editable: these are free text with no
                    // discoverable choice list, and the app has no text entry
                    // yet. Binding them at least stops the pane from hiding
                    // what the session is actually using.
                    Row::new("Layout", Control::Text(String::new())).id("input.xkb_layout"),
                    Row::new("Variant", Control::Text(String::new())).id("input.xkb_variant"),
                    Row::new("Options", Control::Text(String::new())).id("input.xkb_options"),
                ],
            ),
            // Editable, but not persisted: `[keyboard_shortcuts]` is a table in
            // the config file, and the settings contract has no identifier for
            // a list. Adding, removing and retyping lines all work; nothing
            // leaves the process.
            group(Some("Shortcuts"), shortcut_rows),
        ],
    }
}
