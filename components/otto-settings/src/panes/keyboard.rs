//! The keyboard pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.

use std::sync::{OnceLock, RwLock};

use crate::model::{group, untitled, Control, Pane, Row};

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
/// choice. A line loaded with one of those shows it as the compositor
/// describes it (`Workspace 2`, `run kitty`); its pop-up offers only these.
const BUILTIN_ACTIONS: &[&str] = &[
    "ApplicationSwitchNext",
    "ApplicationSwitchNextWindow",
    "ApplicationSwitchPrev",
    "ApplicationSwitchQuit",
    "BrightnessDown",
    "BrightnessUp",
    "CloseWindow",
    "EqualizeContainer",
    "ExposeShowAll",
    "ExposeShowDesktop",
    "FloatingToggle",
    "FocusDown",
    "FocusLeft",
    "FocusModeToggle",
    "FocusRight",
    "FocusUp",
    "LockSession",
    "MediaNext",
    "MediaPlayPause",
    "MediaPrev",
    "MediaStop",
    "MoveContainerDown",
    "MoveContainerLeft",
    "MoveContainerRight",
    "MoveContainerUp",
    "Quit",
    "ResizeGrowHeight",
    "ResizeGrowWidth",
    "ResizeShrinkHeight",
    "ResizeShrinkWidth",
    "RotateOutput",
    "ScaleDown",
    "ScaleUp",
    "SceneSnapshot",
    "SkpSnapshot",
    "SplitHorizontal",
    "SplitVertical",
    "TileWindowLeft",
    "TileWindowRight",
    "ToggleDecorations",
    "TilingToggle",
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
pub const MAX_SHORTCUTS: usize = 96;

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
        // The set the shipped `otto_config.example.toml` binds, for when the
        // compositor cannot be asked — offline, or one without
        // `ListShortcuts`. Otherwise [`load`] replaces it at startup.
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

/// Replace the lines with the shortcuts the compositor has in force.
///
/// Anything past [`MAX_SHORTCUTS`] is dropped, since the pane has no pop-up
/// for it; the compositor still honours it.
pub fn load(pairs: Vec<(String, String)>) {
    if pairs.len() > MAX_SHORTCUTS {
        eprintln!(
            "{} shortcuts, showing the first {MAX_SHORTCUTS}",
            pairs.len()
        );
    }
    *shortcuts().write().unwrap() = pairs
        .into_iter()
        .take(MAX_SHORTCUTS)
        .map(|(keys, action)| Shortcut { action, keys })
        .collect();
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

/// A line listening for its combination to be pressed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Recording {
    pub index: usize,
    /// The modifiers held so far, shown in the field until a key completes
    /// the combination.
    pub held: Modifiers,
}

/// The line listening for a combination, if any.
///
/// Process-wide for the same reason [`SHORTCUTS`] is, and so the draw can show
/// the line listening without being handed the state.
static RECORDING: RwLock<Option<Recording>> = RwLock::new(None);

/// The line whose record button is listening, if any.
pub fn recording() -> Option<Recording> {
    *RECORDING.read().unwrap()
}

/// Start listening on `index`, or stop when `None`.
pub fn set_recording(index: Option<usize>) {
    *RECORDING.write().unwrap() = index.map(|index| Recording {
        index,
        held: Modifiers::default(),
    });
}

/// Update the modifiers a listening line shows as held.
pub fn set_held(held: Modifiers) {
    if let Some(recording) = RECORDING.write().unwrap().as_mut() {
        recording.held = held;
    }
}

/// Held modifiers, in the order [`combination`] writes them.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
}

impl Modifiers {
    /// The modifiers as the leading part of a combination, `Ctrl+Shift+`,
    /// which is what a listening field shows until a key completes it.
    pub fn prefix(self) -> String {
        [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.logo, "Logo"),
        ]
        .into_iter()
        .filter(|(held, _)| *held)
        .map(|(_, name)| format!("{name}+"))
        .collect()
    }
}

/// A pressed combination written the way `parse_trigger` in
/// `src/config/shortcuts.rs` reads it.
///
/// `key` is the xkb name of the keysym the press produced. Letters are
/// written lower case: the compositor folds a letter's case before matching,
/// and the shipped config spells them that way (`Ctrl+q`). Everything else
/// keeps the name xkb gave it, since that is also what the compositor
/// compares against — Shift+Tab arrives as `ISO_Left_Tab`, and a trigger
/// written `Shift+Tab` would never match.
pub fn combination(modifiers: Modifiers, key: &str) -> String {
    let key = if key.len() == 1 && key.chars().all(|c| c.is_ascii_alphabetic()) {
        key.to_ascii_lowercase()
    } else {
        key.to_string()
    };
    format!("{}{key}", modifiers.prefix())
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
    let mut shortcut_rows = vec![Row::new(
        otto_kit::t!("settings-key-combination"),
        Control::Value(String::new()),
    )
    .detail(otto_kit::t!("settings-key-combination-detail"))];
    shortcut_rows.extend((0..lines().len()).map(|index| Row::new("", Control::Shortcut { index })));
    if lines().len() < MAX_SHORTCUTS {
        shortcut_rows.push(Row::new("", Control::AddShortcut));
    }

    Pane {
        name: otto_kit::t!("settings-pane-keyboard"),
        icon: "keyboard",
        intro: None,
        groups: vec![
            untitled(vec![
                Row::new(
                    otto_kit::t!("settings-key-repeat-delay"),
                    Control::Slider {
                        value: 300.0,
                        min: 100.0,
                        max: 1000.0,
                        readout: "300 ms".into(),
                    },
                )
                .id("keyboard_repeat_delay"),
                Row::new(
                    otto_kit::t!("settings-key-repeat-rate"),
                    Control::Slider {
                        value: 30.0,
                        min: 5.0,
                        max: 60.0,
                        readout: "30 / s".into(),
                    },
                )
                .id("keyboard_repeat_rate"),
            ]),
            group(
                otto_kit::t!("settings-group-input-source"),
                vec![
                    // Shown, not editable: these are free text with no
                    // discoverable choice list, and the app has no text entry
                    // yet. Binding them at least stops the pane from hiding
                    // what the session is actually using.
                    Row::new(
                        otto_kit::t!("settings-xkb-layout"),
                        Control::Text(String::new()),
                    )
                    .id("input.xkb_layout"),
                    Row::new(
                        otto_kit::t!("settings-xkb-variant"),
                        Control::Text(String::new()),
                    )
                    .id("input.xkb_variant"),
                    Row::new(
                        otto_kit::t!("settings-xkb-options"),
                        Control::Text(String::new()),
                    )
                    .id("input.xkb_options"),
                ],
            ),
            // Read from the compositor's merged config, editable, but not
            // persisted: `[keyboard_shortcuts]` is a table, and the settings
            // contract has no identifier for a list. Adding, removing and
            // retyping lines all work; nothing leaves the process.
            group(otto_kit::t!("settings-group-shortcuts"), shortcut_rows),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combination_matches_what_the_compositor_parses() {
        let ctrl_shift = Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(combination(ctrl_shift, "Q"), "Ctrl+Shift+q");
        assert_eq!(
            combination(ctrl_shift, "ISO_Left_Tab"),
            "Ctrl+Shift+ISO_Left_Tab"
        );
        assert_eq!(combination(Modifiers::default(), "Prior"), "Prior");
        let all = Modifiers {
            ctrl: true,
            alt: true,
            shift: true,
            logo: true,
        };
        assert_eq!(combination(all, "Return"), "Ctrl+Alt+Shift+Logo+Return");
    }
}
