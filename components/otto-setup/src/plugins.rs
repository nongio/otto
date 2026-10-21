//! What `otto-agents plugins status` says: whether Otto's skills are linked
//! where agents look for them, and where each harness's rendering of a
//! plugin agent stands.
//!
//! Read from its output rather than worked out again here, so where each
//! harness keeps its copy stays `otto-agents`' to know (`vendors.rs` there).

// Rust guideline compliant 2026-02-21

use std::process::Command;

/// The plugin whose skills and agents setup looks after.
pub const PLUGIN: &str = "otto";

/// One of Otto's skills.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    /// Its name.
    pub name: String,
    /// What the status says: `linked`, `not linked`, `in the way`, …
    pub state: String,
    /// Where it is linked, as shown.
    pub path: String,
}

impl Skill {
    /// Linked where agents look for it.
    pub fn linked(&self) -> bool {
        self.state == "linked"
    }

    /// Something of the user's is where the link goes; the installer leaves
    /// it alone.
    pub fn in_the_way(&self) -> bool {
        self.state == "in the way"
    }
}

/// One file of a harness's rendering of a plugin agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Render {
    /// The harness, by `otto-agents`' id: `opencode`, `codex`, `pi`, `hermes`.
    pub vendor: String,
    /// The plugin agent it renders.
    pub agent: String,
    /// `current`, `stale`, `not there`, or `skipped` when the harness is
    /// not set up for it (a Hermes profile missing).
    pub state: String,
    /// The file, as shown.
    pub path: String,
}

impl Render {
    /// Up to date.
    pub fn current(&self) -> bool {
        self.state == "current"
    }
}

/// The status, as far as setup needs it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    /// Otto's skills.
    pub skills: Vec<Skill>,
    /// Every harness's rendering of every plugin agent.
    pub renders: Vec<Render>,
}

impl Status {
    /// Runs `otto-agents plugins status`; `None` when it cannot run.
    pub fn read() -> Option<Self> {
        let output = Command::new("otto-agents")
            .args(["plugins", "status"])
            .output()
            .ok()
            .filter(|output| output.status.success())?;
        Some(parse(&String::from_utf8_lossy(&output.stdout)))
    }

    /// The renderings of `agent` for `vendor`.
    pub fn renders_of<'a>(
        &'a self,
        vendor: &'a str,
        agent: &'a str,
    ) -> impl Iterator<Item = &'a Render> {
        self.renders
            .iter()
            .filter(move |r| r.vendor == vendor && r.agent == agent)
    }
}

/// The status from its output.
pub fn parse(text: &str) -> Status {
    let mut status = Status::default();
    let mut plugin = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("  ") {
            // A skill of the plugin above: `name  state  path`.
            if plugin != PLUGIN || rest.starts_with("agent:") || rest.starts_with(' ') {
                continue;
            }
            let Some((head, path)) = rest.rsplit_once(char::is_whitespace) else {
                continue;
            };
            let Some((name, state)) = head.trim().split_once(char::is_whitespace) else {
                continue;
            };
            status.skills.push(Skill {
                name: name.to_owned(),
                state: state.trim().to_owned(),
                path: path.to_owned(),
            });
        } else if let Some(rest) = line.strip_prefix("skipped ") {
            // `skipped hermes: no ~/.hermes/profiles/otto: …`
            let Some((vendor, rest)) = rest.split_once(": ") else {
                continue;
            };
            let path = rest
                .strip_prefix("no ")
                .and_then(|r| r.split(':').next())
                .unwrap_or_default();
            let agent = path.rsplit('/').next().unwrap_or_default();
            status.renders.push(Render {
                vendor: vendor.to_owned(),
                agent: agent.to_owned(),
                state: "skipped".to_owned(),
                path: path.to_owned(),
            });
        } else if let Some(render) = render_line(line) {
            status.renders.push(render);
        } else if !line.starts_with("Run ") && !line.starts_with("Claude ") && line.contains('/') {
            // A plugin's own line: `name [version]  dir`.
            if let Some(name) = line.split_whitespace().next() {
                plugin = name.to_owned();
            }
        }
    }
    status
}

/// A rendering line: `vendor  state  path`, the vendor padded to nine.
fn render_line(line: &str) -> Option<Render> {
    let (vendor, rest) = line.split_once(' ')?;
    if !["opencode", "codex", "pi", "hermes"].contains(&vendor) {
        return None;
    }
    let rest = rest.trim_start();
    let state = ["current", "stale", "not there"]
        .into_iter()
        .find(|state| rest.starts_with(state))?;
    let path = rest[state.len()..].trim().to_owned();
    // The agent is the file's name: `otto.md`, `otto.json`, `otto-pi`, or
    // the profile a `SOUL.md` sits in.
    let mut parts = path.rsplit('/');
    let file = parts.next().unwrap_or_default();
    let agent = if file == "SOUL.md" {
        parts.next().unwrap_or_default()
    } else {
        let stem = file.split('.').next().unwrap_or_default();
        stem.strip_suffix("-pi").unwrap_or(stem)
    };
    Some(Render {
        vendor: vendor.to_owned(),
        agent: agent.to_owned(),
        state: state.to_owned(),
        path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = "\
marketing 1.0.0  /home/u/.local/share/otto/plugins/marketing
  agent: nora — Otto's product marketing lead
otto 1.0.0  /usr/share/otto/plugins/otto
  otto-help                in the way  ~/.agents/skills/otto-help
  send-to-phone            not linked  ~/.agents/skills/send-to-phone
  agent: otto — Otto's own helper
Run `otto-agents plugins install` to link the rest.
Claude reads nora, otto from the plugin; the other harnesses get a rendering:
opencode  current    ~/.config/opencode/agents/nora.md
opencode  stale      ~/.config/opencode/agents/otto.md
skipped hermes: no ~/.hermes/profiles/nora: `hermes profile create nora` makes the profile
hermes    stale      ~/.hermes/profiles/otto/SOUL.md
codex     not there  ~/.local/share/otto/agents/codex/otto.json
pi        current    ~/.local/share/otto/agents/pi/otto-pi
Run `otto-agents plugins install` to write them.
";

    #[test]
    fn reads_otto_skills_only() {
        let status = parse(STATUS);
        let skills: Vec<(&str, &str)> = status
            .skills
            .iter()
            .map(|s| (s.name.as_str(), s.state.as_str()))
            .collect();
        assert_eq!(
            skills,
            [("otto-help", "in the way"), ("send-to-phone", "not linked")]
        );
        assert!(status.skills[0].in_the_way());
    }

    #[test]
    fn reads_each_rendering_and_its_agent() {
        let status = parse(STATUS);
        let of = |vendor: &str, agent: &str| -> Vec<String> {
            status
                .renders_of(vendor, agent)
                .map(|r| r.state.clone())
                .collect()
        };
        assert_eq!(of("opencode", "otto"), ["stale"]);
        assert_eq!(of("hermes", "otto"), ["stale"]);
        assert_eq!(of("hermes", "nora"), ["skipped"]);
        assert_eq!(of("codex", "otto"), ["not there"]);
        assert_eq!(of("pi", "otto"), ["current"]);
    }
}
