use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use smithay::input::keyboard::ModifiersState;
use thiserror::Error;
use tracing::{info, warn};
use xkbcommon::xkb::{self, keysyms::KEY_NoSymbol};

/// Raw mapping loaded from configuration.
pub type ShortcutMap = BTreeMap<String, ShortcutActionConfig>;

/// Supported shortcut action encodings in the configuration file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ShortcutActionConfig {
    /// `action = "Quit"`
    BuiltinName(String),

    /// `action = { builtin = "Screen", index = 0 }`
    BuiltinDetailed {
        builtin: String,
        #[serde(default)]
        index: Option<usize>,
    },

    /// `action = { run = { cmd = "kitty", args = [] } }`
    RunCommand { run: RunCommandConfig },

    /// `action = { open_default = "browser" }`
    OpenDefault { open_default: OpenDefaultConfig },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OpenDefaultConfig {
    Role(String),
    Detailed {
        role: String,
        #[serde(default)]
        fallback: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCommandConfig {
    pub cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ShortcutBinding {
    pub trigger: ShortcutTrigger,
    pub action: ShortcutAction,
    #[allow(dead_code)]
    pub trigger_repr: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ShortcutTrigger {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
    pub keysym: xkb::Keysym,
}

impl ShortcutTrigger {
    pub fn matches(&self, modifiers: &ModifiersState, keysym: xkb::Keysym) -> bool {
        let normalized_keysym = normalize_alphanumeric_keysym(keysym);
        self.ctrl == modifiers.ctrl
            && self.alt == modifiers.alt
            && self.shift == modifiers.shift
            && self.logo == modifiers.logo
            && self.keysym == normalized_keysym
    }

    pub fn canonical_id(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".into());
        }
        if self.alt {
            parts.push("Alt".into());
        }
        if self.shift {
            parts.push("Shift".into());
        }
        if self.logo {
            parts.push("Logo".into());
        }
        parts.push(keysym_canonical_name(self.keysym));
        parts.join("+")
    }
}

#[derive(Debug, Clone)]
pub enum ShortcutAction {
    Builtin(BuiltinAction),
    RunCommand(RunCommandConfig),
    OpenDefaultApp {
        role: String,
        fallback: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub enum BuiltinAction {
    Quit,
    Screen { index: usize },
    ScaleUp,
    ScaleDown,
    RotateOutput,
    ToggleDecorations,
    ApplicationSwitchNext,
    ApplicationSwitchPrev,
    ApplicationSwitchNextWindow,
    ApplicationSwitchQuit,
    CloseWindow,
    ToggleMaximizeWindow,
    ExposeShowDesktop,
    ExposeShowAll,
    WorkspaceNum { index: usize },
    SceneSnapshot,
    BrightnessUp,
    BrightnessDown,
    VolumeUp,
    VolumeDown,
    VolumeMute,
    MediaPlayPause,
    MediaNext,
    MediaPrev,
    MediaStop,
}

#[derive(Debug, Error)]
pub enum ShortcutError {
    #[error("unknown modifier '{0}'")]
    UnknownModifier(String),
    #[error("unparsable trigger '{0}'")]
    InvalidTrigger(String),
    #[error("unknown keysym '{0}'")]
    UnknownKeysym(String),
    #[error("unknown builtin action '{0}'")]
    UnknownBuiltin(String),
    #[error("missing index for action that requires one: '{0}'")]
    MissingIndex(String),
}

pub fn build_bindings(map: &ShortcutMap) -> Vec<ShortcutBinding> {
    let mut bindings = Vec::new();
    let mut seen: HashMap<ShortcutTrigger, String> = HashMap::new();

    for (trigger_str, action_cfg) in map {
        match parse_trigger(trigger_str) {
            Ok(trigger) => match parse_action(action_cfg) {
                Ok(action) => {
                    if let Some(existing) = seen.insert(trigger.clone(), trigger_str.clone()) {
                        warn!(
                            trigger = %trigger.canonical_id(),
                            existing = %existing,
                            new = %trigger_str,
                            "duplicate shortcut definition, new entry replaces the previous one"
                        );
                    }
                    bindings.push(ShortcutBinding {
                        trigger,
                        action,
                        trigger_repr: trigger_str.clone(),
                    });
                }
                Err(err) => {
                    warn!(trigger = %trigger_str, error = %err, "skipping shortcut due to invalid action")
                }
            },
            Err(err) => {
                warn!(trigger = %trigger_str, error = %err, "skipping shortcut due to invalid trigger")
            }
        }
    }

    if !bindings.is_empty() {
        info!("loaded {} keyboard shortcut bindings", bindings.len());
    } else {
        info!("no keyboard shortcut bindings configured");
    }

    bindings
}

fn parse_action(cfg: &ShortcutActionConfig) -> Result<ShortcutAction, ShortcutError> {
    match cfg {
        ShortcutActionConfig::BuiltinName(name) => {
            let action = parse_builtin(name, None)?;
            Ok(ShortcutAction::Builtin(action))
        }
        ShortcutActionConfig::BuiltinDetailed { builtin, index } => {
            let action = parse_builtin(builtin, *index)?;
            Ok(ShortcutAction::Builtin(action))
        }
        ShortcutActionConfig::RunCommand { run } => Ok(ShortcutAction::RunCommand(run.clone())),
        ShortcutActionConfig::OpenDefault {
            open_default: OpenDefaultConfig::Role(role),
        } => Ok(ShortcutAction::OpenDefaultApp {
            role: role.clone(),
            fallback: None,
        }),
        ShortcutActionConfig::OpenDefault {
            open_default: OpenDefaultConfig::Detailed { role, fallback },
        } => Ok(ShortcutAction::OpenDefaultApp {
            role: role.clone(),
            fallback: fallback.clone(),
        }),
    }
}

fn parse_builtin(name: &str, index: Option<usize>) -> Result<BuiltinAction, ShortcutError> {
    Ok(match name {
        "Quit" => BuiltinAction::Quit,
        "ScaleUp" => BuiltinAction::ScaleUp,
        "ScaleDown" => BuiltinAction::ScaleDown,
        "RotateOutput" => BuiltinAction::RotateOutput,
        "ToggleDecorations" => BuiltinAction::ToggleDecorations,
        "ApplicationSwitchNext" => BuiltinAction::ApplicationSwitchNext,
        "ApplicationSwitchPrev" => BuiltinAction::ApplicationSwitchPrev,
        "ApplicationSwitchNextWindow" => BuiltinAction::ApplicationSwitchNextWindow,
        "ApplicationSwitchQuit" => BuiltinAction::ApplicationSwitchQuit,
        "CloseWindow" => BuiltinAction::CloseWindow,
        "ToggleMaximizeWindow" => BuiltinAction::ToggleMaximizeWindow,
        "ExposeShowDesktop" => BuiltinAction::ExposeShowDesktop,
        "ExposeShowAll" => BuiltinAction::ExposeShowAll,
        "SceneSnapshot" => BuiltinAction::SceneSnapshot,
        "BrightnessUp" => BuiltinAction::BrightnessUp,
        "BrightnessDown" => BuiltinAction::BrightnessDown,
        "VolumeUp" => BuiltinAction::VolumeUp,
        "VolumeDown" => BuiltinAction::VolumeDown,
        "VolumeMute" => BuiltinAction::VolumeMute,
        "MediaPlayPause" => BuiltinAction::MediaPlayPause,
        "MediaNext" => BuiltinAction::MediaNext,
        "MediaPrev" => BuiltinAction::MediaPrev,
        "MediaStop" => BuiltinAction::MediaStop,
        "Screen" => {
            let index = index.ok_or_else(|| ShortcutError::MissingIndex(name.to_string()))?;
            BuiltinAction::Screen { index }
        }
        "Workspace" => {
            let index = index.ok_or_else(|| ShortcutError::MissingIndex(name.to_string()))?;
            BuiltinAction::WorkspaceNum { index }
        }
        other => return Err(ShortcutError::UnknownBuiltin(other.to_string())),
    })
}

fn parse_trigger(trigger: &str) -> Result<ShortcutTrigger, ShortcutError> {
    let parts: Vec<&str> = trigger.split('+').collect();
    if parts.is_empty() {
        return Err(ShortcutError::InvalidTrigger(trigger.to_string()));
    }

    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut logo = false;

    for modifier in &parts[..parts.len() - 1] {
        let normalized = modifier.trim();

        match normalized.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "primary" => ctrl = true,
            "alt" => alt = true,
            "shift" => shift = true,
            "logo" | "super" | "meta" | "win" | "command" => logo = true,
            other => return Err(ShortcutError::UnknownModifier(other.to_string())),
        }
    }

    let key = parts[parts.len() - 1].trim();
    if key.is_empty() {
        return Err(ShortcutError::InvalidTrigger(trigger.to_string()));
    }

    let keysym = parse_keysym(key)?;

    Ok(ShortcutTrigger {
        ctrl,
        alt,
        shift,
        logo,
        keysym,
    })
}

fn parse_keysym(key: &str) -> Result<xkb::Keysym, ShortcutError> {
    let alias = match key {
        "ArrowUp" => Some("Up"),
        "ArrowDown" => Some("Down"),
        "ArrowLeft" => Some("Left"),
        "ArrowRight" => Some("Right"),
        _ => None,
    };

    let mut base_candidates = Vec::new();
    base_candidates.push(key.to_string());
    if let Some(alias) = alias {
        if alias != key {
            base_candidates.push(alias.to_string());
        }
    }

    let mut attempts: Vec<String> = Vec::new();
    let mut add_attempt = |value: String| {
        if !attempts.contains(&value) {
            attempts.push(value);
        }
    };

    for candidate in base_candidates {
        add_attempt(candidate.clone());
        if candidate.len() == 1 {
            add_attempt(candidate.to_ascii_uppercase());
        } else {
            add_attempt(candidate.to_ascii_lowercase());
            add_attempt(candidate.to_ascii_uppercase());
        }
    }

    for attempt in attempts {
        let sym = xkb::keysym_from_name(&attempt, xkb::KEYSYM_NO_FLAGS);
        if sym.raw() != KEY_NoSymbol {
            return Ok(normalize_alphanumeric_keysym(sym));
        }
    }

    Err(ShortcutError::UnknownKeysym(key.to_string()))
}

fn keysym_canonical_name(sym: xkb::Keysym) -> String {
    let name = xkb::keysym_get_name(sym);
    if name.is_empty() {
        format!("0x{:x}", sym.raw())
    } else {
        name
    }
}

fn normalize_alphanumeric_keysym(sym: xkb::Keysym) -> xkb::Keysym {
    if let Some(ch) = sym.key_char() {
        if ch.is_ascii_alphabetic() {
            return xkb::Keysym::from_char(ch.to_ascii_lowercase());
        }
    }
    sym
}

pub fn default_shortcut_map() -> ShortcutMap {
    ShortcutMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::input::keyboard::ModifiersState;

    #[test]
    fn parses_basic_shortcuts() {
        let mut map = ShortcutMap::new();
        map.insert(
            "Logo+Q".into(),
            ShortcutActionConfig::BuiltinName("Quit".into()),
        );
        map.insert(
            "Logo+Shift+Return".into(),
            ShortcutActionConfig::OpenDefault {
                open_default: OpenDefaultConfig::Role("terminal".into()),
            },
        );

        let bindings = build_bindings(&map);
        assert_eq!(bindings.len(), 2);

        let quit_binding = bindings
            .iter()
            .find(|binding| matches!(binding.action, ShortcutAction::Builtin(_)))
            .expect("quit binding present");
        assert!(quit_binding.trigger.logo);
        assert!(!quit_binding.trigger.ctrl);
        assert_eq!(
            quit_binding.trigger.keysym,
            xkb::keysym_from_name("q", xkb::KEYSYM_NO_FLAGS)
        );

        let open_default_binding = bindings
            .iter()
            .find(|binding| matches!(binding.action, ShortcutAction::OpenDefaultApp { .. }))
            .expect("open default binding present");
        assert!(open_default_binding.trigger.logo);
        assert!(open_default_binding.trigger.shift);
        assert_eq!(
            open_default_binding.trigger.keysym,
            xkb::keysym_from_name("Return", xkb::KEYSYM_NO_FLAGS)
        );
    }

    #[test]
    fn case_insensitive_letter_keysyms() {
        let mut map = ShortcutMap::new();
        map.insert(
            "Alt+Shift+W".into(),
            ShortcutActionConfig::BuiltinName("Quit".into()),
        );

        let bindings = build_bindings(&map);
        assert_eq!(bindings.len(), 1);
        let binding = &bindings[0];

        let mut modifiers = ModifiersState::default();
        modifiers.alt = true;
        modifiers.shift = true;

        let uppercase = xkb::keysym_from_name("W", xkb::KEYSYM_NO_FLAGS);
        let lowercase = xkb::keysym_from_name("w", xkb::KEYSYM_NO_FLAGS);

        assert!(binding.trigger.matches(&modifiers, uppercase));
        assert!(binding.trigger.matches(&modifiers, lowercase));
    }
}
