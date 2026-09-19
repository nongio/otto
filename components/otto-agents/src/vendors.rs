//! The plugin's agent, rendered for each harness that cannot load it itself.
//!
//! One source: `agents/<name>.md` in the plugin, in the Claude Code plugin
//! dialect ([`AgentFile`]). Claude reads that file where it lies, through
//! `--agent plugin:name` ([`crate::skills::claude_session_meta`]). Every other
//! harness wants the same instructions in a file of its own, in its own
//! dialect, in its own place — and selected per launch from `agents.toml`
//! rather than by editing the harness's configuration, so the harness in a
//! terminal is unchanged. [`Vendor`] is that table; [`install`] writes the
//! files and [`status`] reports them. The body is byte-identical across
//! vendors; only the frontmatter, a derived heading and the location differ.
//!
//! Each vendor was checked against the version installed when this was
//! written; the version and what was verified are in each variant's docs.
//!
//! - **OpenCode 1.18.31.** User agents live in
//!   `~/.config/opencode/agents/<name>.md` (`agent/` is read too), YAML
//!   frontmatter `description` and `mode: primary`, the body as the prompt;
//!   `tools` there is deprecated in favour of `permission` and a listed tool
//!   only ever denies, so it is not rendered. `opencode acp` has no `--agent`;
//!   `OPENCODE_CONFIG_CONTENT='{"default_agent":"<name>"}'` in the agent's
//!   `env` is a final config merge for that process alone, and `default_agent`
//!   is what the ACP server starts a session as. Verified with `opencode run
//!   --agent otto` and with that env var.
//! - **Hermes 0.16.0.** `hermes -p <profile>` makes `~/.hermes/profiles/<profile>/`
//!   its HERMES_HOME, and `SOUL.md` there is the identity slot of the system
//!   prompt — it replaces Hermes's own identity paragraph and nothing else;
//!   `hermes acp` builds the prompt the same way `hermes chat` does. The
//!   `agent.system_prompt` config key is ignored by the ACP adapter, so the
//!   file is the route. The profile is named after the agent, so
//!   `~/.hermes/SOUL.md`, the default profile's, is never touched; a profile
//!   that is not there is reported, not created. Verified with `hermes -p
//!   otto chat -Q -q`.
//! - **Codex, through codex-acp 1.12.0.** No agents.
//!   `developer_instructions` in its config is text appended to the system
//!   prompt (`model_instructions_file` takes a path but replaces the prompt,
//!   dropping Codex's own guidance; `experimental_instructions_file` is gone).
//!   This adapter takes its whole session config as one JSON object in
//!   `CODEX_CONFIG`, so the body is rendered twice under the data directory:
//!   Markdown for a person to read, and that JSON object. The agent's `env`
//!   carries `CODEX_CONFIG = "{file:<the json>}"`, which the service expands
//!   when it spawns the agent ([`crate::config::expand_env`]). A marked block
//!   in `~/.codex/AGENTS.md` would have reached Codex in a terminal too, in
//!   every project. Verified over codex-acp's stdio; the archived
//!   `@zed-industries/codex-acp` took `-c key=value` instead and bundled a
//!   Codex too old for current models.
//! - **pi 0.85.1, through pi-acp 0.0.33.** pi-acp spawns `pi --mode rpc` and
//!   drops its own arguments, but passes its environment on, and
//!   `PI_ACP_PI_COMMAND` names the executable it spawns. pi's
//!   `--append-system-prompt <file>` adds a file to the default prompt
//!   (`SYSTEM.md` would replace it). So two files are rendered under the data
//!   directory: the instructions, and a short `otto-pi` wrapper that adds
//!   the flag and hands over to `pi`; the agent's `env` points
//!   `PI_ACP_PI_COMMAND` at the wrapper. `~/.pi/agent/AGENTS.md` would reach
//!   pi in a terminal too. Verified with `pi -p --append-system-prompt`.
//!
//! Every rendered file carries a marker line, [`MARKER`], naming the source
//! it came from. A file without it is somebody else's and is left alone in
//! both directions; a file with it is rewritten only when the rendering
//! changed, so running [`install`] after every upgrade is free.

use std::io;
use std::path::{Path, PathBuf};

use crate::skills::AgentFile;

/// What every rendered file says about itself, followed by the source path.
pub const MARKER: &str = "rendered by otto-agents from";

/// The marker written while the service was called `otto-agentsd`. A file
/// carrying it is still ours, so an upgrade rewrites it rather than mistaking
/// it for the person's own and leaving their agent on stale instructions.
pub const LEGACY_MARKER: &str = "rendered by otto-agentsd from";

/// Whether a rendered file is one of ours, under either marker.
fn ours(text: &str) -> bool {
    text.contains(MARKER) || text.contains(LEGACY_MARKER)
}

/// A harness the agent is rendered for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Vendor {
    OpenCode,
    Hermes,
    Codex,
    Pi,
}

impl Vendor {
    pub const ALL: [Vendor; 4] = [Vendor::OpenCode, Vendor::Hermes, Vendor::Codex, Vendor::Pi];

    /// The name `--only` takes, and the agent id it usually goes with.
    pub fn id(self) -> &'static str {
        match self {
            Vendor::OpenCode => "opencode",
            Vendor::Hermes => "hermes",
            Vendor::Codex => "codex",
            Vendor::Pi => "pi",
        }
    }

    pub fn from_id(id: &str) -> Option<Vendor> {
        Vendor::ALL.into_iter().find(|vendor| vendor.id() == id)
    }

    /// Whether the harness is set up under `home`: the directory it keeps its
    /// own state in is there. A harness that is not is skipped, not
    /// installed for.
    pub fn present(self, home: &Home, agent: &AgentFile) -> bool {
        self.state_dir(home, agent).is_dir()
    }

    /// The directory whose absence means the harness is not set up, and what
    /// to do about it.
    fn state_dir(self, home: &Home, agent: &AgentFile) -> PathBuf {
        match self {
            Vendor::OpenCode => home.config.join("opencode"),
            Vendor::Hermes => home.root.join(".hermes/profiles").join(&agent.name),
            Vendor::Codex => home.root.join(".codex"),
            Vendor::Pi => home.root.join(".pi"),
        }
    }

    /// One line saying why the harness was skipped.
    pub fn absent_hint(self, home: &Home, agent: &AgentFile) -> String {
        let dir = home.tilde(&self.state_dir(home, agent));
        match self {
            Vendor::Hermes => format!(
                "no {dir}: `hermes profile create {}` makes the profile the agent runs in",
                agent.name
            ),
            _ => format!("no {dir}: the harness is not set up here"),
        }
    }

    /// The files this vendor gets for `agent`, rendered.
    pub fn files(self, home: &Home, agent: &AgentFile) -> Vec<Rendered> {
        let source = agent.path.display();
        match self {
            Vendor::OpenCode => vec![Rendered {
                path: home
                    .config
                    .join("opencode/agents")
                    .join(format!("{}.md", agent.name)),
                text: format!(
                    "---\n# {MARKER} {source}; edits are overwritten\ndescription: {}\nmode: primary\n---\n\n{}\n",
                    yaml_string(&agent.description),
                    agent.body
                ),
                executable: false,
            }],
            Vendor::Hermes => vec![Rendered {
                path: self.state_dir(home, agent).join("SOUL.md"),
                text: plain(agent),
                executable: false,
            }],
            Vendor::Codex => {
                let dir = home.rendered_dir(self);
                // codex-acp takes its session config as one JSON object in
                // `CODEX_CONFIG`, so the instructions ship as that object
                // ready to hand over; the Markdown beside it is what a person
                // reads and what `-c developer_instructions=` would take.
                let config = serde_json::json!({ "developer_instructions": plain(agent) });
                vec![
                    Rendered {
                        path: dir.join(format!("{}.md", agent.name)),
                        text: plain(agent),
                        executable: false,
                    },
                    Rendered {
                        path: dir.join(format!("{}.json", agent.name)),
                        text: format!("{config}\n"),
                        executable: false,
                    },
                ]
            }
            Vendor::Pi => {
                let dir = home.rendered_dir(self);
                let instructions = dir.join(format!("{}.md", agent.name));
                let wrapper = format!(
                    "#!/bin/sh\n# {MARKER} {source}; edits are overwritten\n\
                     # pi-acp spawns this in place of `pi` (PI_ACP_PI_COMMAND) and passes its\n\
                     # own arguments; this adds the agent's instructions and hands over.\n\
                     exec pi --append-system-prompt {} \"$@\"\n",
                    shell_word(&instructions)
                );
                vec![
                    Rendered {
                        path: instructions,
                        text: plain(agent),
                        executable: false,
                    },
                    Rendered {
                        path: dir.join(format!("{}-pi", agent.name)),
                        text: wrapper,
                        executable: true,
                    },
                ]
            }
        }
    }
}

/// The plain rendering: marker, a heading derived from the frontmatter — the
/// name and what the agent is, since a file on its own has nothing else to
/// say so — and the body as it is.
fn plain(agent: &AgentFile) -> String {
    let role = first_sentence(&agent.description);
    let heading = if role.is_empty() {
        format!("# {}", agent.name)
    } else {
        format!("# {} — {role}", agent.name)
    };
    format!(
        "<!-- {MARKER} {}; edits are overwritten -->\n\n{heading}\n\n{}\n",
        agent.path.display(),
        agent.body
    )
}

/// The first sentence of a description, without its full stop: for an agent
/// file that is what the agent is, and the rest says when to use it, which is
/// the agent's own to read. The one definition — a heading and a `plugins
/// status` line that disagreed about the stop would be two renderings of one
/// description.
pub(crate) fn first_sentence(text: &str) -> &str {
    match text.split_once(". ") {
        Some((first, _)) => first.trim(),
        None => text.trim().trim_end_matches('.'),
    }
}

/// `text` as a double-quoted YAML scalar.
fn yaml_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// `path` single-quoted for `sh`.
fn shell_word(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

/// The directories the vendors' files are placed under. From the environment
/// by default; from one root for a dry run somewhere harmless.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Home {
    /// `$HOME`.
    pub root: PathBuf,
    /// `$XDG_CONFIG_HOME`.
    pub config: PathBuf,
    /// `$XDG_DATA_HOME`.
    pub data: PathBuf,
}

impl Home {
    /// From `HOME`, `XDG_CONFIG_HOME` and `XDG_DATA_HOME`; `None` without a
    /// home directory.
    pub fn from_env() -> Option<Home> {
        Some(Home {
            config: crate::xdg::config_home()?,
            data: crate::xdg::data_home()?,
            root: crate::xdg::home()?,
        })
    }

    /// Everything under `root`, with the XDG defaults.
    pub fn at(root: &Path) -> Home {
        Home {
            root: root.to_path_buf(),
            config: root.join(".config"),
            data: root.join(".local/share"),
        }
    }

    /// Where a vendor's files go when the vendor has no place of its own:
    /// `$XDG_DATA_HOME/otto/agents/<vendor>/`. Beside, not inside, the
    /// `otto/plugins/` search path, so a rendering is never mistaken for a
    /// plugin.
    pub fn rendered_dir(&self, vendor: Vendor) -> PathBuf {
        self.data.join("otto/agents").join(vendor.id())
    }

    /// `path` with `root` written as `~`.
    pub fn tilde(&self, path: &Path) -> String {
        crate::xdg::tilde(path, Some(&self.root))
    }
}

/// One file a vendor gets: where, what, and whether it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub path: PathBuf,
    pub text: String,
    pub executable: bool,
}

/// Where one rendered file stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Ours, and what the source renders to now.
    Current,
    /// Ours, but the source has moved on.
    Stale,
    /// Nothing there yet.
    Absent,
    /// A file without our marker: the person's, or the harness's own. Left
    /// alone in both directions.
    Taken,
    /// The harness is not set up here, so nothing is placed.
    NoHarness,
}

/// One vendor file and its state; `done` says what [`install`] did about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub vendor: Vendor,
    /// The agent's name.
    pub agent: String,
    pub path: PathBuf,
    pub state: State,
    pub done: Option<Done>,
}

/// What [`install`] did to a [`Target`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Done {
    Created,
    Updated,
    Unchanged,
    /// [`State::Taken`] or [`State::NoHarness`]: nothing written.
    LeftAlone,
}

/// Every vendor's files for every agent of `agents`, with their state, in
/// vendor order. `only` narrows it to one vendor.
pub fn status(agents: &[&AgentFile], home: &Home, only: Option<Vendor>) -> Vec<Target> {
    targets(agents, home, only)
        .into_iter()
        .map(|(vendor, agent, rendered, present)| Target {
            vendor,
            agent: agent.name.clone(),
            state: if present {
                state_of(&rendered)
            } else {
                State::NoHarness
            },
            path: rendered.path,
            done: None,
        })
        .collect()
}

/// Writes every vendor's files for every agent of `agents` where the vendor
/// is set up, creating directories as needed, and says what happened to each.
/// A file that is current is not touched, so a run with nothing to do writes
/// nothing; a file that is not ours is left as it is.
pub fn install(
    agents: &[&AgentFile],
    home: &Home,
    only: Option<Vendor>,
) -> io::Result<Vec<Target>> {
    targets(agents, home, only)
        .into_iter()
        .map(|(vendor, agent, rendered, present)| {
            let state = if present {
                state_of(&rendered)
            } else {
                State::NoHarness
            };
            let done = match state {
                State::Absent => {
                    write(&rendered)?;
                    Done::Created
                }
                State::Stale => {
                    write(&rendered)?;
                    Done::Updated
                }
                State::Current => Done::Unchanged,
                State::Taken | State::NoHarness => Done::LeftAlone,
            };
            Ok(Target {
                vendor,
                agent: agent.name.clone(),
                path: rendered.path,
                state: match done {
                    Done::Created | Done::Updated => State::Current,
                    _ => state,
                },
                done: Some(done),
            })
        })
        .collect()
}

#[allow(clippy::type_complexity)]
fn targets<'a>(
    agents: &[&'a AgentFile],
    home: &Home,
    only: Option<Vendor>,
) -> Vec<(Vendor, &'a AgentFile, Rendered, bool)> {
    Vendor::ALL
        .into_iter()
        .filter(|vendor| only.is_none_or(|only| only == *vendor))
        .flat_map(|vendor| {
            agents.iter().flat_map(move |agent| {
                let present = vendor.present(home, agent);
                vendor
                    .files(home, agent)
                    .into_iter()
                    .map(move |rendered| (vendor, *agent, rendered, present))
            })
        })
        .collect()
}

fn state_of(rendered: &Rendered) -> State {
    match std::fs::read_to_string(&rendered.path) {
        Err(_) if !rendered.path.exists() => State::Absent,
        Err(_) => State::Taken,
        Ok(text) if !ours(&text) => State::Taken,
        Ok(text) if text == rendered.text => State::Current,
        Ok(_) => State::Stale,
    }
}

fn write(rendered: &Rendered) -> io::Result<()> {
    if let Some(dir) = rendered.path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&rendered.path, &rendered.text)?;
    if rendered.executable {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&rendered.path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(root: &Path) -> AgentFile {
        AgentFile {
            name: "otto".into(),
            description: "Otto's own helper. Use it when someone asks how to do something on their \"Otto\" desktop.".into(),
            tools: vec!["Read".into(), "Bash".into()],
            model: None,
            skills: vec!["otto".into()],
            body: "You are Otto, the desktop's own helper.\n\n## How you talk\n\n- Short sentences.".into(),
            path: root.join("plugins/otto/agents/otto.md"),
        }
    }

    /// A temp root with every harness set up under it.
    fn home_with_harnesses() -> (tempfile::TempDir, Home) {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at(dir.path());
        for path in [
            home.config.join("opencode"),
            home.root.join(".hermes/profiles/otto"),
            home.root.join(".codex"),
            home.root.join(".pi/agent"),
        ] {
            std::fs::create_dir_all(path).unwrap();
        }
        (dir, home)
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn opencode_gets_its_frontmatter_and_the_body_as_it_is() {
        let (_dir, home) = home_with_harnesses();
        let agent = agent(&home.root);
        let files = Vendor::OpenCode.files(&home, &agent);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, home.config.join("opencode/agents/otto.md"));
        assert_eq!(
            files[0].text,
            format!(
                "---\n# rendered by otto-agents from {}; edits are overwritten\n\
                 description: \"Otto's own helper. Use it when someone asks how to do something on their \\\"Otto\\\" desktop.\"\n\
                 mode: primary\n---\n\n{}\n",
                agent.path.display(),
                agent.body
            )
        );
        assert!(!files[0].executable);
    }

    #[test]
    fn hermes_codex_and_pi_get_the_plain_rendering_in_their_own_places() {
        let (_dir, home) = home_with_harnesses();
        let agent = agent(&home.root);
        let plain = format!(
            "<!-- rendered by otto-agents from {}; edits are overwritten -->\n\n\
             # otto — Otto's own helper\n\n{}\n",
            agent.path.display(),
            agent.body
        );

        let hermes = Vendor::Hermes.files(&home, &agent);
        assert_eq!(hermes.len(), 1);
        assert_eq!(
            hermes[0].path,
            home.root.join(".hermes/profiles/otto/SOUL.md")
        );
        assert_eq!(hermes[0].text, plain);

        let codex = Vendor::Codex.files(&home, &agent);
        assert_eq!(codex.len(), 2);
        assert_eq!(codex[0].path, home.data.join("otto/agents/codex/otto.md"));
        assert_eq!(codex[0].text, plain);
        assert_eq!(codex[1].path, home.data.join("otto/agents/codex/otto.json"));
        let config: serde_json::Value = serde_json::from_str(&codex[1].text).expect("json");
        assert_eq!(config["developer_instructions"], plain);

        let pi = Vendor::Pi.files(&home, &agent);
        assert_eq!(pi.len(), 2);
        assert_eq!(pi[0].path, home.data.join("otto/agents/pi/otto.md"));
        assert_eq!(pi[0].text, plain);
        assert_eq!(pi[1].path, home.data.join("otto/agents/pi/otto-pi"));
        assert!(pi[1].executable);
        assert!(
            pi[1]
                .text
                .starts_with("#!/bin/sh\n# rendered by otto-agents from ")
        );
        assert!(pi[1].text.ends_with(&format!(
            "exec pi --append-system-prompt '{}' \"$@\"\n",
            pi[0].path.display()
        )));
    }

    /// The point of one source: what each harness reads is the same text.
    #[test]
    fn the_body_is_identical_across_vendors() {
        let (_dir, home) = home_with_harnesses();
        let agent = agent(&home.root);
        for vendor in Vendor::ALL {
            let files = vendor.files(&home, &agent);
            assert!(
                files[0].text.ends_with(&format!("\n\n{}\n", agent.body)),
                "{vendor:?} changes the body"
            );
        }
    }

    #[test]
    fn install_creates_then_leaves_alone_then_updates() {
        let (_dir, home) = home_with_harnesses();
        let mut agent = agent(&home.root);

        let before = status(&[&agent], &home, None);
        assert_eq!(before.len(), 6);
        assert!(before.iter().all(|target| target.state == State::Absent));

        let first = install(&[&agent], &home, None).unwrap();
        assert!(
            first
                .iter()
                .all(|target| target.done == Some(Done::Created))
        );
        assert!(first.iter().all(|target| target.state == State::Current));
        let wrapper = home.data.join("otto/agents/pi/otto-pi");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&wrapper).unwrap().permissions().mode() & 0o111,
            0o111
        );
        let mtime = |path: &Path| std::fs::metadata(path).unwrap().modified().unwrap();
        let soul = home.root.join(".hermes/profiles/otto/SOUL.md");
        let written_at = mtime(&soul);

        // Nothing changed: nothing written.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let again = install(&[&agent], &home, None).unwrap();
        assert!(
            again
                .iter()
                .all(|target| target.done == Some(Done::Unchanged))
        );
        assert_eq!(
            mtime(&soul),
            written_at,
            "an unchanged file is not rewritten"
        );

        // The source moved on: every file that carries the body follows; the
        // pi wrapper only names the file, so it stays.
        agent.body.push_str("\n- No emoji.");
        let is_wrapper = |target: &Target| target.path == wrapper;
        assert!(status(&[&agent], &home, None).iter().all(|t| t.state
            == if is_wrapper(t) {
                State::Current
            } else {
                State::Stale
            }));
        let updated = install(&[&agent], &home, None).unwrap();
        assert!(updated.iter().all(|t| {
            t.done
                == Some(if is_wrapper(t) {
                    Done::Unchanged
                } else {
                    Done::Updated
                })
        }));
        assert!(read(&soul).ends_with("- No emoji.\n"));
    }

    /// The service was once called `otto-agentsd` and wrote that name into
    /// every file it rendered. An upgrade must still recognise its own work,
    /// or it leaves every existing user on the instructions they had.
    #[test]
    fn a_file_rendered_under_the_old_name_is_still_ours() {
        let (_dir, home) = home_with_harnesses();
        let agent = agent(&home.root);
        let soul = home.root.join(".hermes/profiles/otto/SOUL.md");
        std::fs::write(
            &soul,
            format!("<!-- {LEGACY_MARKER} /usr/share/otto/x.md; edits are overwritten -->\n\nold\n"),
        )
        .unwrap();

        let installed = install(&[&agent], &home, None).unwrap();
        let target = installed.iter().find(|t| t.path == soul).unwrap();

        assert_ne!(target.state, State::Taken, "ours, under the old marker");
        assert_eq!(target.done, Some(Done::Updated));
        assert!(read(&soul).contains(MARKER), "rewritten under the new one");
    }

    #[test]
    fn a_file_that_is_not_ours_is_left_alone_and_said_so() {
        let (_dir, home) = home_with_harnesses();
        let agent = agent(&home.root);
        let soul = home.root.join(".hermes/profiles/otto/SOUL.md");
        std::fs::write(&soul, "# Fred — Your CLI Agent\n").unwrap();
        let opencode = home.config.join("opencode/agents/otto.md");
        std::fs::create_dir_all(opencode.parent().unwrap()).unwrap();
        std::fs::write(&opencode, "---\ndescription: mine\n---\nMy own otto.\n").unwrap();

        let installed = install(&[&agent], &home, None).unwrap();
        let by_path = |path: &Path| installed.iter().find(|t| t.path == path).unwrap();
        assert_eq!(by_path(&soul).state, State::Taken);
        assert_eq!(by_path(&soul).done, Some(Done::LeftAlone));
        assert_eq!(read(&soul), "# Fred — Your CLI Agent\n");
        assert_eq!(by_path(&opencode).state, State::Taken);
        assert_eq!(
            read(&opencode),
            "---\ndescription: mine\n---\nMy own otto.\n"
        );
        // The others still went in.
        assert_eq!(
            by_path(&home.data.join("otto/agents/codex/otto.md")).done,
            Some(Done::Created)
        );
    }

    #[test]
    fn a_harness_that_is_not_set_up_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at(dir.path());
        let agent = agent(&home.root);
        std::fs::create_dir_all(home.root.join(".codex")).unwrap();
        // Hermes is there, but not the agent's profile.
        std::fs::create_dir_all(home.root.join(".hermes")).unwrap();

        let installed = install(&[&agent], &home, None).unwrap();
        let of = |vendor: Vendor| {
            installed
                .iter()
                .filter(|t| t.vendor == vendor)
                .collect::<Vec<_>>()
        };
        assert_eq!(of(Vendor::Codex)[0].done, Some(Done::Created));
        for vendor in [Vendor::OpenCode, Vendor::Hermes, Vendor::Pi] {
            for target in of(vendor) {
                assert_eq!(target.state, State::NoHarness, "{vendor:?}");
                assert_eq!(target.done, Some(Done::LeftAlone));
                assert!(
                    !target.path.exists(),
                    "{vendor:?} wrote {}",
                    target.path.display()
                );
            }
        }
        assert_eq!(
            Vendor::Hermes.absent_hint(&home, &agent),
            "no ~/.hermes/profiles/otto: `hermes profile create otto` makes the profile the agent runs in"
        );
        assert!(!home.data.join("otto/agents/pi").exists());
    }

    #[test]
    fn only_narrows_to_one_vendor() {
        let (_dir, home) = home_with_harnesses();
        let agent = agent(&home.root);
        let installed = install(&[&agent], &home, Some(Vendor::Pi)).unwrap();
        assert_eq!(installed.len(), 2);
        assert!(installed.iter().all(|t| t.vendor == Vendor::Pi));
        assert!(!home.config.join("opencode/agents/otto.md").exists());
        assert_eq!(Vendor::from_id("codex"), Some(Vendor::Codex));
        assert_eq!(Vendor::from_id("gemini"), None);
    }

    #[test]
    fn home_comes_from_the_environment_or_one_root() {
        let home = Home::at(Path::new("/home/me"));
        assert_eq!(home.config, PathBuf::from("/home/me/.config"));
        assert_eq!(home.data, PathBuf::from("/home/me/.local/share"));
        assert_eq!(
            home.rendered_dir(Vendor::Codex),
            PathBuf::from("/home/me/.local/share/otto/agents/codex")
        );
        assert_eq!(home.tilde(Path::new("/home/me/.codex")), "~/.codex");
        assert_eq!(home.tilde(Path::new("/etc/otto")), "/etc/otto");
    }

    #[test]
    fn yaml_and_shell_quoting_survive_awkward_characters() {
        assert_eq!(yaml_string(r#"a "b": c\d"#), r#""a \"b\": c\\d""#);
        assert_eq!(
            shell_word(Path::new("/tmp/it's here")),
            r#"'/tmp/it'\''s here'"#
        );
        assert_eq!(first_sentence("One. Two."), "One");
        assert_eq!(first_sentence("No stop"), "No stop");
        assert_eq!(first_sentence("One sentence."), "One sentence");
        assert_eq!(first_sentence("  padded. Rest."), "padded");
    }
}
