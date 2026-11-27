//! Where an app's sound plays, and what happens to other apps during a call.
//!
//! Both are kept in the routing file, next to the saved links:
//!
//! ```toml
//! [[rule]]
//! app = "Spotify"
//! output = "alsa_output.usb-Focusrite_Scarlett_2i2-00.analog-stereo"
//!
//! [[rule]]
//! app = "firefox"          # no output: follow the default
//!
//! [calls]
//! lower_others = true
//! level = 30
//! headset = ["bluez_card.AC_80_0A_00_00_00"]
//! ```
//!
//! A rule names an app by its `application.name` or its program's binary,
//! either way round and in any case, and its output by `node.name`, which
//! stays the same across plugs and boots. The first rule that matches wins.
//! `otto-mixer-service` moves a matching stream when it appears and when
//! the rule's output comes back.
//!
//! `[calls]` says whether other apps are lowered while an app records from
//! a microphone, to what share of their volume, and which Bluetooth
//! headsets switch to their headset profile for it.

// Rust guideline compliant 2026-02-21

use serde::{Deserialize, Serialize};

use crate::model::Node;
use crate::routing::Routing;

/// The share of their volume other apps keep during a call, in percent,
/// until a person picks another: low enough to hear a voice over music.
pub const DEFAULT_DUCK_LEVEL: u32 = 30;

/// Where one app's sound plays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRule {
    /// The app: its `application.name` or its program's binary.
    pub app: String,
    /// The `node.name` of the output it plays on; `None` follows the
    /// default output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

impl AppRule {
    /// Whether `stream` is one of this rule's app's streams.
    #[must_use]
    pub fn matches(&self, stream: &Node) -> bool {
        [
            stream.prop("application.name"),
            stream.prop("application.process.binary"),
        ]
        .into_iter()
        .flatten()
        .any(|name| name.eq_ignore_ascii_case(&self.app))
    }
}

fn duck_level() -> u32 {
    DEFAULT_DUCK_LEVEL
}

/// What happens while an app records from a microphone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Calls {
    /// Whether other apps are lowered meanwhile.
    #[serde(default)]
    pub lower_others: bool,
    /// The share of their volume they keep, in percent.
    #[serde(default = "duck_level")]
    pub level: u32,
    /// The Bluetooth devices, by `device.name`, that switch to their
    /// headset profile while an app records, and back afterwards.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headset: Vec<String>,
}

impl Default for Calls {
    fn default() -> Self {
        Self {
            lower_others: false,
            level: DEFAULT_DUCK_LEVEL,
            headset: Vec::new(),
        }
    }
}

impl Calls {
    /// Whether nothing differs from the default, so the file leaves it out.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The share of their volume lowered apps keep, as a fraction.
    #[must_use]
    pub fn fraction(&self) -> f32 {
        self.level.min(100) as f32 / 100.0
    }
}

impl Routing {
    /// The app rules, in the order they are tried.
    #[must_use]
    pub fn rules(&self) -> &[AppRule] {
        &self.rules
    }

    /// The first rule that matches `stream`.
    #[must_use]
    pub fn rule_for(&self, stream: &Node) -> Option<&AppRule> {
        self.rules.iter().find(|r| r.matches(stream))
    }

    /// The rule for `app` (compared in any case).
    #[must_use]
    pub fn rule(&self, app: &str) -> Option<&AppRule> {
        self.rules.iter().find(|r| r.app.eq_ignore_ascii_case(app))
    }

    /// Have `app`'s sound play on `output` (a `node.name`), or follow the
    /// default with `None`, replacing any rule it had. `false` when that
    /// was the rule already.
    pub fn set_rule(&mut self, app: &str, output: Option<String>) -> bool {
        let app = app.trim();
        match self
            .rules
            .iter_mut()
            .find(|r| r.app.eq_ignore_ascii_case(app))
        {
            Some(existing) if existing.output == output => false,
            Some(existing) => {
                existing.output = output;
                true
            }
            None => {
                self.rules.push(AppRule {
                    app: app.to_string(),
                    output,
                });
                true
            }
        }
    }

    /// Forget `app`'s rule. `false` when it had none.
    pub fn remove_rule(&mut self, app: &str) -> bool {
        let before = self.rules.len();
        self.rules.retain(|r| !r.app.eq_ignore_ascii_case(app));
        self.rules.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NodeState;

    fn stream(name: &str, binary: &str) -> Node {
        Node {
            id: 1,
            props: [
                ("application.name".to_string(), name.to_string()),
                ("application.process.binary".to_string(), binary.to_string()),
            ]
            .into(),
            state: NodeState::Running,
        }
    }

    #[test]
    fn a_rule_matches_by_name_or_binary_in_any_case() {
        let rule = AppRule {
            app: "spotify".into(),
            output: None,
        };
        assert!(rule.matches(&stream("Spotify", "spotify-launcher")));
        assert!(rule.matches(&stream("Music", "SPOTIFY")));
        assert!(!rule.matches(&stream("Firefox", "firefox")));
    }

    #[test]
    fn the_first_matching_rule_wins_and_setting_replaces() {
        let mut routing = Routing::default();
        assert!(routing.set_rule("Spotify", Some("usb".into())));
        assert!(routing.set_rule("spotify-launcher", Some("hdmi".into())));
        let found = routing.rule_for(&stream("Spotify", "spotify-launcher"));
        assert_eq!(found.and_then(|r| r.output.as_deref()), Some("usb"));
        assert!(!routing.set_rule("SPOTIFY", Some("usb".into())));
        assert!(routing.set_rule("spotify", None));
        assert_eq!(routing.rules().len(), 2);
        assert_eq!(routing.rule("Spotify").unwrap().output, None);
        assert!(routing.remove_rule("Spotify"));
        assert!(!routing.remove_rule("Spotify"));
    }

    #[test]
    fn rules_and_calls_round_trip_and_older_files_still_read() {
        let mut routing = Routing::default();
        routing.set_rule("mpv", Some("speakers".into()));
        routing.set_rule("firefox", None);
        routing.calls.lower_others = true;
        routing.calls.level = 20;
        routing.calls.headset.push("bluez_card.AA".into());
        let text = toml::to_string(&routing).unwrap();
        assert!(text.contains("[[rule]]"), "{text}");
        assert!(text.contains("[calls]"), "{text}");
        let back: Routing = toml::from_str(&text).unwrap();
        assert_eq!(back.rules(), routing.rules());
        assert_eq!(back.calls, routing.calls);

        // Left out while at their defaults, and missing from older files.
        let text = toml::to_string(&Routing::default()).unwrap();
        assert!(!text.contains("calls") && !text.contains("rule"), "{text}");
        let old: Routing = toml::from_str("version = 1\n").unwrap();
        assert!(old.rules().is_empty());
        assert_eq!(old.calls, Calls::default());
        let partial: Routing = toml::from_str("[calls]\nlower_others = true\n").unwrap();
        assert_eq!(partial.calls.level, DEFAULT_DUCK_LEVEL);
    }
}
