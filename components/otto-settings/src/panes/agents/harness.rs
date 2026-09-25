//! The harnesses an agent can run on, and what each needs in its block.
//!
//! A harness is more than its command. Each takes the plugin agent it runs as
//! (the persona; none runs the harness as itself) by its own route, and
//! enters a session in a terminal its own way. The routes are the ones
//! `docs/developer/agents.md` lists under "One agent, every harness", and the
//! renderings they point at are what `otto-agents plugins install` writes.
//! Picking a harness writes all of it; typing a command writes the command
//! alone.

use toml_edit::{value, Array, InlineTable, Item, Table, Value};

/// A harness the pane knows how to set up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Harness {
    Claude,
    Codex,
    OpenCode,
    Hermes,
    Pi,
    /// A command the pane does not recognise. Nothing to set up: the command
    /// field is all there is.
    Custom,
}

/// Environment variables some harness is pointed at its rendering through.
/// Cleared on a switch so the new harness does not inherit the old one's.
const HARNESS_ENV: [&str; 3] = [
    "CODEX_CONFIG",
    "OPENCODE_CONFIG_CONTENT",
    "PI_ACP_PI_COMMAND",
];

/// Keys that only mean something to one harness, cleared on a switch.
const HARNESS_KEYS: [&str; 5] = ["skills", "agent", "enter", "enter_new", "mode"];

impl Harness {
    pub const ALL: [Harness; 6] = [
        Harness::Claude,
        Harness::Codex,
        Harness::OpenCode,
        Harness::Hermes,
        Harness::Pi,
        Harness::Custom,
    ];

    /// The name the pop-up shows. Product names, so only Custom is translated.
    pub fn label(self) -> &'static str {
        match self {
            Harness::Claude => "Claude Code",
            Harness::Codex => "Codex",
            Harness::OpenCode => "OpenCode",
            Harness::Hermes => "Hermes",
            Harness::Pi => "pi",
            Harness::Custom => otto_kit::t!("settings-agent-harness-custom"),
        }
    }

    /// The harness a command line runs, recognised by its adapter.
    pub fn detect(command: &[String]) -> Harness {
        let names = |needle: &str| command.iter().any(|word| word.contains(needle));
        let program = command
            .first()
            .map(|c| c.rsplit('/').next().unwrap_or(c))
            .unwrap_or_default();
        if names("claude-agent-acp") {
            Harness::Claude
        } else if names("codex-acp") {
            Harness::Codex
        } else {
            match program {
                "opencode" => Harness::OpenCode,
                "hermes" => Harness::Hermes,
                "pi-acp" => Harness::Pi,
                _ => Harness::Custom,
            }
        }
    }

    /// The command line that starts this harness's adapter for `persona`.
    /// Empty for [`Harness::Custom`], which has none of its own.
    pub fn command(self, persona: &str) -> Vec<String> {
        let words: &[&str] = match self {
            Harness::Claude => &["npx", "-y", "@agentclientprotocol/claude-agent-acp@latest"],
            Harness::Codex => &["npx", "-y", "@agentclientprotocol/codex-acp@latest"],
            Harness::OpenCode => &["opencode", "acp"],
            // The profile is named after the agent, and holds its rendering.
            // Without one, Hermes runs on its default profile.
            Harness::Hermes if persona.is_empty() => &["hermes", "acp"],
            Harness::Hermes => {
                return vec!["hermes".into(), "-p".into(), persona.into(), "acp".into()]
            }
            Harness::Pi => &["pi-acp"],
            Harness::Custom => &[],
        };
        words.iter().map(|word| word.to_string()).collect()
    }

    /// Write everything but the command that this harness needs into an
    /// agent's block, clearing what another harness left there. `home` is the
    /// home folder, for the one route that takes an absolute path.
    ///
    /// An empty `persona` writes no route at all: the harness then runs as
    /// itself, with its own instructions.
    pub fn write(self, table: &mut Table, persona: &str, home: &str) {
        if self == Harness::Custom {
            return;
        }
        for key in HARNESS_KEYS {
            table.remove(key);
        }
        for key in HARNESS_ENV {
            set_entry(table, "env", key, None);
        }
        set_entry(table, "config", "collaboration_mode", None);

        let words = |words: &[&str]| words.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        match self {
            Harness::Claude => {
                // The skills are the desktop's, not an identity: Claude keeps
                // them whoever it runs as.
                table.insert("skills", value("claude"));
                if !persona.is_empty() {
                    table.insert("agent", value(persona));
                }
                set_list(table, "enter", &words(&["claude", "--resume", "{session}"]));
                set_list(
                    table,
                    "enter_new",
                    &words(&["claude", "--session-id", "{session}"]),
                );
            }
            Harness::Codex => {
                if !persona.is_empty() {
                    let config =
                        format!("{{file:~/.local/share/otto/agents/codex/{persona}.json}}");
                    set_entry(table, "env", "CODEX_CONFIG", Some(&config));
                }
                // Codex offers its question tool in the plan collaboration
                // mode only.
                set_entry(table, "config", "collaboration_mode", Some("plan"));
                set_list(table, "enter", &words(&["codex", "resume", "{session}"]));
            }
            Harness::OpenCode => {
                if !persona.is_empty() {
                    let config = format!(r#"{{"default_agent":"{persona}"}}"#);
                    set_entry(table, "env", "OPENCODE_CONFIG_CONTENT", Some(&config));
                }
                set_list(
                    table,
                    "enter",
                    &words(&["opencode", "--session", "{session}"]),
                );
            }
            Harness::Hermes => {
                let enter: &[&str] = if persona.is_empty() {
                    &["hermes", "--resume", "{session}"]
                } else {
                    &["hermes", "-p", persona, "--resume", "{session}"]
                };
                set_list(table, "enter", &words(enter));
            }
            Harness::Pi => {
                // pi-acp spawns this wrapper instead of `pi`, and does not
                // expand `~`. pi has no way to resume one of pi-acp's sessions
                // by id, so there is no enter command.
                if !persona.is_empty() {
                    let wrapper = format!("{home}/.local/share/otto/agents/pi/{persona}-pi");
                    set_entry(table, "env", "PI_ACP_PI_COMMAND", Some(&wrapper));
                }
            }
            Harness::Custom => {}
        }
    }
}

/// The plugin agent a block runs as, read back from whichever route its
/// harness takes; empty when it names none, and runs as the harness itself.
pub fn persona(table: &Table) -> String {
    let text = |key: &str| table.get(key).and_then(Item::as_str);
    let env = |key: &str| {
        table
            .get("env")
            .and_then(Item::as_table_like)
            .and_then(|env| env.get(key))
            .and_then(Item::as_str)
    };
    let args: Vec<&str> = table
        .get("args")
        .and_then(Item::as_array)
        .map(|args| args.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let found = text("agent")
        .map(str::to_string)
        .or_else(|| {
            env("OPENCODE_CONFIG_CONTENT").and_then(|json| {
                let rest = json.split("\"default_agent\"").nth(1)?;
                Some(rest.split('"').nth(1)?.to_string())
            })
        })
        .or_else(|| {
            env("CODEX_CONFIG")
                .and_then(|path| path.rsplit('/').next())
                .and_then(|file| file.strip_suffix(".json}"))
                .map(str::to_string)
        })
        .or_else(|| {
            env("PI_ACP_PI_COMMAND")
                .and_then(|path| path.rsplit('/').next())
                .and_then(|file| file.strip_suffix("-pi"))
                .map(str::to_string)
        })
        .or_else(|| {
            (text("command") == Some("hermes"))
                .then(|| args.windows(2).find(|pair| pair[0] == "-p"))
                .flatten()
                .map(|pair| pair[1].to_string())
        });
    found.unwrap_or_default()
}

/// Set `key` to a list of strings, or remove it when the list is empty. An
/// existing key keeps its place and the comment above it.
pub fn set_list(table: &mut Table, key: &str, words: &[String]) {
    if words.is_empty() {
        table.remove(key);
        return;
    }
    let list = value(words.iter().map(String::as_str).collect::<Array>());
    match table.get_mut(key) {
        Some(item) => *item = list,
        None => {
            table.insert(key, list);
        }
    }
}

/// Set or remove one entry of a small table such as `env`, written inline
/// when it has to be made. A table left empty is removed.
fn set_entry(table: &mut Table, table_key: &str, key: &str, text: Option<&str>) {
    match text {
        Some(text) => {
            if !table.get(table_key).is_some_and(Item::is_table_like) {
                table.insert(
                    table_key,
                    Item::Value(Value::InlineTable(InlineTable::new())),
                );
            }
            if let Some(entries) = table.get_mut(table_key).and_then(Item::as_table_like_mut) {
                entries.insert(key, value(text));
            }
        }
        None => {
            let emptied = match table.get_mut(table_key).and_then(Item::as_table_like_mut) {
                Some(entries) => {
                    entries.remove(key);
                    entries.is_empty()
                }
                None => false,
            };
            if emptied {
                table.remove(table_key);
            }
        }
    }
}

/// A command line split into words the way a shell would: spaces separate,
/// quotes group, a backslash escapes the next character.
pub fn split(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') => {
                quote = Some(c);
                started = true;
            }
            (Some('\''), c) => word.push(c),
            (_, '\\') => {
                if let Some(next) = chars.next() {
                    word.push(next);
                }
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (_, c) => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

/// Words joined back into a line [`split`] reads the same way.
pub fn join(words: &[String]) -> String {
    words
        .iter()
        .map(|word| {
            let plain = !word.is_empty()
                && word
                    .chars()
                    .all(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | '\\'));
            if plain {
                word.clone()
            } else {
                format!("'{}'", word.replace('\'', r"'\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split(' ').map(str::to_string).collect()
    }

    #[test]
    fn detects_each_harness_by_its_adapter() {
        assert_eq!(
            Harness::detect(&words(
                "npx -y @agentclientprotocol/claude-agent-acp@latest"
            )),
            Harness::Claude
        );
        assert_eq!(
            Harness::detect(&words("npx -y @agentclientprotocol/codex-acp@latest")),
            Harness::Codex
        );
        assert_eq!(Harness::detect(&words("opencode acp")), Harness::OpenCode);
        assert_eq!(
            Harness::detect(&words("/usr/bin/hermes -p otto acp")),
            Harness::Hermes
        );
        assert_eq!(Harness::detect(&words("pi-acp")), Harness::Pi);
        assert_eq!(Harness::detect(&words("copilot --acp")), Harness::Custom);
    }

    #[test]
    fn a_switch_carries_the_persona_and_drops_the_old_routes() {
        let mut table: Table = r#"
id = "review"
command = "npx"
skills = "claude"
agent = "reviewer"
enter_new = ["claude", "--session-id", "{session}"]
env = { KEEP = "1" }
"#
        .parse::<toml_edit::DocumentMut>()
        .unwrap()
        .as_table()
        .clone();
        assert_eq!(persona(&table), "reviewer");

        for harness in [
            Harness::Codex,
            Harness::OpenCode,
            Harness::Pi,
            Harness::Claude,
        ] {
            let carried = persona(&table);
            harness.write(&mut table, &carried, "/home/u");
            assert_eq!(persona(&table), "reviewer", "{harness:?}");
            let env = table.get("env").and_then(Item::as_table_like).unwrap();
            assert_eq!(env.get("KEEP").and_then(Item::as_str), Some("1"));
        }
        assert!(table.get("enter_new").is_some());
        assert!(table.get("config").is_none());

        Harness::Pi.write(&mut table, "reviewer", "/home/u");
        assert!(table.get("enter").is_none());
        assert!(table.get("skills").is_none());

        let mut hermes = Table::new();
        hermes.insert("command", value("hermes"));
        set_list(
            &mut hermes,
            "args",
            &Harness::Hermes.command("reviewer")[1..],
        );
        assert_eq!(persona(&hermes), "reviewer");
    }

    #[test]
    fn no_persona_writes_no_route() {
        for harness in [
            Harness::Claude,
            Harness::Codex,
            Harness::OpenCode,
            Harness::Hermes,
            Harness::Pi,
        ] {
            let mut table = Table::new();
            let command = harness.command("");
            table.insert("command", value(&command[0]));
            set_list(&mut table, "args", &command[1..]);
            harness.write(&mut table, "", "/home/u");
            assert_eq!(persona(&table), "", "{harness:?}");
            assert!(table.get("agent").is_none(), "{harness:?}");
            assert!(table.get("env").is_none(), "{harness:?}");
        }
    }

    #[test]
    fn a_command_line_splits_and_joins_back() {
        let line = r#"env FOO="a b" run 'it'\''s' x\ y"#;
        let split = split(line);
        assert_eq!(split, ["env", "FOO=a b", "run", "it's", "x y"]);
        assert_eq!(super::split(&join(&split)), split);
        assert_eq!(join(&words("pi-acp")), "pi-acp");
    }
}
