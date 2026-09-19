//! Skills the desktop offers its agents.
//!
//! Otto installs its own skills to `/usr/share/otto/plugins/<plugin>/`, in the
//! [Open Plugins](https://open-plugins.com/) shape: a
//! `.claude-plugin/plugin.json` manifest, a `skills/` directory with one
//! `SKILL.md` per skill, and optionally an `agents/` directory with one
//! Markdown file per agent — frontmatter and a system prompt. A person adds
//! their own under `$XDG_DATA_HOME/otto/plugins/`, which is searched first, so
//! a plugin of the same name shadows the packaged one.
//!
//! What is found reaches an agent by one of two routes, chosen per agent by
//! `skills` in `agents.toml` ([`crate::config::SkillDelivery`]):
//!
//! - **Claude loads them as a plugin.** With `skills = "claude"` the plugin
//!   directories go to claude-agent-acp in the session's `_meta`, together
//!   with a short Otto preamble appended to Claude's own system prompt
//!   ([`claude_session_meta`]). Claude then knows the skills as its own.
//! - **Every other agent reads them from disk.** `otto-agents plugins
//!   install` ([`install`]) links each skill into `~/.agents/skills`, the
//!   directory the Agent Skills convention names, and the session carries
//!   nothing. This is what the default, `skills` off, expects.
//!
//! An agent file goes the same two ways: Claude Code loads a plugin's
//! `agents/` itself and runs as one with `--agent plugin:name`
//! ([`claude_session_meta`]); every other harness gets the same file
//! rendered in its own dialect and place by `plugins install`
//! ([`crate::vendors`]), and `agents.toml` points each launch at it.
//!
//! Whatever the route, the same plugins are published to clients as AHP
//! [`Customization`] entries on the agent and on each session, read-only, with
//! their skills and agents as children — so a client can show exactly what a
//! session was given without asking the agent.
//!
//! Discovery happens once, at startup: the packaged skills change when the
//! package does, which is a restart either way.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use ahp_types::state::{
    AgentCustomization, ChildCustomization, Customization, CustomizationLoadState,
    CustomizationLoadedState, PluginCustomization, SkillCustomization,
};

use crate::uri;

/// Where Otto's packaged plugins live.
const SYSTEM_DIR: &str = "/usr/share/otto/plugins";

/// One skill: a `SKILL.md` and what its frontmatter says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    /// The frontmatter's `name`, or the directory's name when it has none.
    pub name: String,
    /// The frontmatter's `description`: one sentence saying when to use the
    /// skill. This is what an agent matches against, so a skill without one is
    /// a skill nothing will reach for.
    pub description: String,
    /// The `SKILL.md` itself, absolute.
    pub path: PathBuf,
}

/// One agent file: `<plugin>/agents/<name>.md`, in the shape Claude Code
/// plugins use — YAML frontmatter, then the agent's system prompt as the body.
/// Claude loads the file itself; the other harnesses get it re-rendered in
/// their own dialect by [`crate::vendors`], which is what `body` is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentFile {
    /// The frontmatter's `name`, or the file's stem when it has none.
    pub name: String,
    /// The frontmatter's `description`: what the agent is for and when to hand
    /// a request to it.
    pub description: String,
    /// The frontmatter's `tools`, comma-separated there. Empty means no
    /// restriction.
    pub tools: Vec<String>,
    /// The frontmatter's `model`, when the agent is pinned to one.
    pub model: Option<String>,
    /// The frontmatter's `skills`, comma-separated there: the skills the agent
    /// is given when it runs.
    pub skills: Vec<String>,
    /// The system prompt: everything under the frontmatter, trimmed.
    pub body: String,
    /// The file itself, absolute.
    pub path: PathBuf,
}

/// One plugin directory, with the skills and agents found inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plugin {
    /// The manifest's `name`, or the directory's name when there is no
    /// manifest.
    pub name: String,
    pub description: String,
    pub version: Option<String>,
    pub dir: PathBuf,
    pub skills: Vec<Skill>,
    pub agents: Vec<AgentFile>,
}

/// The directories searched, most specific first. `OTTO_AGENTS_PLUGINS` replaces
/// the list entirely — a colon-separated path, as `PATH` is — which is how a
/// plugin is tried out without installing it.
pub fn search_paths() -> Vec<PathBuf> {
    if let Some(overridden) = std::env::var_os("OTTO_AGENTS_PLUGINS") {
        tracing::warn!(path = %overridden.to_string_lossy(), "OTTO_AGENTS_PLUGINS is set: loading plugins from an override path, which trusts every directory in it");
        return std::env::split_paths(&overridden)
            .filter(|path| !path.as_os_str().is_empty())
            .collect();
    }
    let mut paths: Vec<PathBuf> = crate::xdg::data_home()
        .map(|dir| dir.join("otto/plugins"))
        .into_iter()
        .collect();
    paths.push(PathBuf::from(SYSTEM_DIR));
    paths
}

/// Every plugin in [`search_paths`], in that order. A plugin directory whose
/// name has already been seen is skipped, so the user's copy wins over the
/// packaged one rather than both being offered.
pub fn discover() -> Vec<Plugin> {
    discover_in(&search_paths())
}

pub fn discover_in(dirs: &[PathBuf]) -> Vec<Plugin> {
    let mut plugins: Vec<Plugin> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            // A directory that is not there is the normal case, not a fault:
            // nobody has to install plugins.
            continue;
        };
        let mut found: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        // read_dir gives whatever order the filesystem holds; the customization
        // list and the Claude preamble are both read by people, so sort them.
        found.sort();
        for path in found {
            let Some(plugin) = read_plugin(&path) else {
                continue;
            };
            let shadowed = plugins
                .iter()
                .any(|other| other.dir.file_name() == plugin.dir.file_name());
            if shadowed {
                tracing::debug!(dir = %plugin.dir.display(), "a plugin of this name is already loaded");
                continue;
            }
            plugins.push(plugin);
        }
    }
    plugins
}

/// Reads one plugin directory, or `None` when it holds neither skills nor
/// agents — an arbitrary directory under a search path is not a plugin, and a
/// plugin that contributes nothing is not worth telling anyone about.
fn read_plugin(dir: &Path) -> Option<Plugin> {
    let name = dir.file_name()?.to_string_lossy().into_owned();
    let manifest = read_manifest(&dir.join(".claude-plugin/plugin.json"));
    let skills = read_skills(&dir.join("skills"));
    let agents = read_agents(&dir.join("agents"));
    if skills.is_empty() && agents.is_empty() {
        return None;
    }
    let (manifest_name, description, version) = manifest.unwrap_or_default();
    Some(Plugin {
        name: path_safe(Some(manifest_name), name, dir),
        description,
        version,
        dir: dir.to_path_buf(),
        skills,
        agents,
    })
}

/// `(name, description, version)` from an Open Plugins manifest. A manifest
/// that will not parse is not fatal: the directory's own name and its skills
/// are enough to work with, and refusing the plugin would be a worse trade.
fn read_manifest(path: &Path) -> Option<(String, String, Option<String>)> {
    let text = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text)
        .inspect_err(|err| tracing::warn!(path = %path.display(), %err, "invalid plugin manifest"))
        .ok()?;
    let string = |key: &str| {
        json.get(key)
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_owned()
    };
    let version = json
        .get("version")
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    Some((string("name"), string("description"), version))
}

/// A name a path may be built from, or the file's own name instead.
///
/// A skill is linked as `<dir>/<name>` and an agent rendered to
/// `<dir>/<name>.md`, `<dir>/<name>.json` or `<dir>/<name>-pi`, so a name
/// carrying a separator or a parent link would write outside the directory it
/// was meant for — and a plugin directory is somewhere a person, or an agent
/// with one file write, can put one. The name a plugin declares is a
/// convenience; the name on disk is the authority.
fn path_safe(declared: Option<String>, on_disk: String, path: &Path) -> String {
    let Some(declared) = declared.filter(|name| !name.is_empty()) else {
        return on_disk;
    };
    let usable = declared != "."
        && declared != ".."
        && declared
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if usable {
        return declared;
    }
    tracing::warn!(
        path = %path.display(),
        name = %declared,
        "the declared name cannot be part of a path; using the name on disk"
    );
    on_disk
}

/// Every `<skills>/<name>/SKILL.md`, in name order.
fn read_skills(dir: &Path) -> Vec<Skill> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    dirs.iter()
        .filter_map(|path| {
            let file = path.join("SKILL.md");
            let text = std::fs::read_to_string(&file).ok()?;
            let mut keys = frontmatter(&text);
            let dir_name = path.file_name()?.to_string_lossy().into_owned();
            Some(Skill {
                name: path_safe(keys.remove("name"), dir_name, &file),
                description: keys.remove("description").unwrap_or_default(),
                path: file,
            })
        })
        .collect()
}

/// Every `<agents>/<name>.md`, in name order.
fn read_agents(dir: &Path) -> Vec<AgentFile> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|file| {
            let text = std::fs::read_to_string(&file).ok()?;
            let mut keys = frontmatter(&text);
            let stem = file.file_stem()?.to_string_lossy().into_owned();
            Some(AgentFile {
                name: path_safe(keys.remove("name"), stem, &file),
                description: keys.remove("description").unwrap_or_default(),
                tools: comma_list(keys.get("tools")),
                model: keys.remove("model").filter(|model| !model.is_empty()),
                skills: comma_list(keys.get("skills")),
                body: body(&text).to_owned(),
                path: file,
            })
        })
        .collect()
}

/// `a, b, c` as a list, empty for `None` or a blank value.
fn comma_list(value: Option<&String>) -> Vec<String> {
    value
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The top-level scalar keys of a Markdown file's YAML frontmatter, by name.
///
/// Deliberately not a YAML parser: the keys a skill or agent file carries are
/// plain scalars on one line, and a dependency that can parse the rest of YAML
/// would only let a file do things nothing here reads. A folded or multi-line
/// value reads as far as the first line, which is enough to route on; a
/// nested block's own lines are skipped.
fn frontmatter(text: &str) -> BTreeMap<String, String> {
    let mut keys = BTreeMap::new();
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return keys;
    }
    for line in lines {
        let trimmed = line.trim_end();
        if trimmed.trim() == "---" {
            break;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        // Only top-level keys: an indented line belongs to something nested.
        if key.starts_with(char::is_whitespace) {
            continue;
        }
        keys.insert(key.trim().to_owned(), unquote(value.trim()));
    }
    keys
}

/// The text under the frontmatter, trimmed; the whole file when there is
/// none.
fn body(text: &str) -> &str {
    let Some((first, rest)) = text.split_once('\n') else {
        return text.trim();
    };
    if first.trim() != "---" {
        return text.trim();
    }
    let mut offset = first.len() + 1;
    for line in rest.split_inclusive('\n') {
        offset += line.len();
        if line.trim() == "---" {
            return text[offset..].trim();
        }
    }
    // An opening `---` that never closes: nothing is a body.
    ""
}

fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    let quoted = value.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'')
        && bytes[bytes.len() - 1] == bytes[0];
    if quoted {
        value[1..value.len() - 1].to_owned()
    } else {
        value.to_owned()
    }
}

/// What Claude is told about the desktop, appended to its own system prompt:
/// where it is, that the skills are loaded as plugins and how one is invoked,
/// that the plugins' agents are there to answer desktop questions, and when to
/// reach for one. Kept to a few hundred bytes: every session pays for it.
pub fn claude_system_prompt(plugins: &[Plugin]) -> String {
    let plugin_names: Vec<String> = plugins
        .iter()
        .map(|plugin| format!("`{}`", plugin.name))
        .collect();
    let skill_names: Vec<String> = plugins
        .iter()
        .flat_map(|plugin| &plugin.skills)
        .map(|skill| format!("`/{}`", skill.name))
        .collect();
    let agent_names: Vec<String> = plugins
        .iter()
        .flat_map(|plugin| &plugin.agents)
        .map(|agent| format!("`{}`", agent.name))
        .collect();
    let loaded = match plugin_names.len() {
        1 => format!("the {} plugin", plugin_names[0]),
        _ => format!("the {} plugins", join_and(&plugin_names)),
    };
    let invoked = match skill_names.len() {
        0 => String::new(),
        1 => format!("{} invokes it", skill_names[0]),
        _ => format!("{} invoke them", join_and(&skill_names)),
    };
    let answers = match agent_names.len() {
        0 => String::new(),
        1 => format!(
            "the {} agent answers questions about the desktop",
            agent_names[0]
        ),
        _ => format!(
            "the {} agents answer questions about the desktop",
            join_and(&agent_names)
        ),
    };
    let then = match (invoked.is_empty(), answers.is_empty()) {
        (false, false) => format!("; {invoked} and {answers}"),
        (false, true) => format!("; {invoked}"),
        (true, false) => format!("; {answers}"),
        (true, true) => String::new(),
    };
    format!(
        "You are running on Otto, a Wayland desktop. Otto's skills are loaded as \
         {loaded}{then}. When a request matches one, use it first and follow \
         it; when none matches, ignore this and carry on. A request that opens \
         with `/<name>` names the skill to use, and the rest of it is the request."
    )
}

/// `a`, `a and b`, `a, b and c`.
fn join_and(items: &[String]) -> String {
    match items.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// The session `_meta` that has claude-agent-acp load `plugins` as Claude Code
/// plugins and tell Claude where it is, or `None` when there are none. The
/// plugins go in `claudeCode.options`, which other agents ignore; Claude's
/// skills then come through its own skill machinery, `allowed-tools` included.
/// `systemPrompt.append` extends the adapter's `claude_code` preset rather
/// than replacing it, so Claude's own safety text stays. The adapter reads
/// both on `session/new`, `session/load` and `session/resume` alike.
pub fn claude_session_meta(
    plugins: &[Plugin],
    run_as: Option<&str>,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    if plugins.is_empty() {
        return None;
    }
    let dirs: Vec<serde_json::Value> = plugins
        .iter()
        .map(|plugin| serde_json::json!({ "type": "local", "path": plugin.dir }))
        .collect();
    let mut options = serde_json::json!({ "plugins": dirs });
    // `run_as` is Claude's own `--agent plugin:name`: Claude reads the agent
    // file itself and honours its instructions, tools, skills and model. It
    // travels as a CLI flag because the adapter drops the SDK's `agent`
    // option on purpose, and takes `extraArgs` as they are.
    if let Some(agent) = run_as {
        options["extraArgs"] = serde_json::json!({ "agent": agent });
    }
    let meta = serde_json::json!({
        "claudeCode": { "options": options },
        "systemPrompt": { "append": claude_system_prompt(plugins) },
    });
    match meta {
        serde_json::Value::Object(meta) => Some(meta),
        _ => None,
    }
}

/// The agent file called `name` in any plugin, the first plugin's winning,
/// with the `plugin:name` Claude knows it by.
pub fn agent_file<'a>(plugins: &'a [Plugin], name: &str) -> Option<(String, &'a AgentFile)> {
    plugins.iter().find_map(|plugin| {
        let agent = plugin.agents.iter().find(|agent| agent.name == name)?;
        Some((format!("{}:{}", plugin.name, agent.name), agent))
    })
}

/// Where agents with skill discovery look for skills: `~/.agents/skills`, the
/// directory the Agent Skills convention names and Copilot, Codex and Claude
/// read. `None` without a home directory.
pub fn default_skills_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".agents/skills"))
}

/// One skill's place in a skills directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The plugin the skill belongs to.
    pub plugin: String,
    pub name: String,
    /// The skill's own directory, the one `SKILL.md` sits in.
    pub source: PathBuf,
    /// `<dir>/<name>`, where an agent would find it.
    pub link: PathBuf,
    pub state: LinkState,
}

/// What is at an [`Entry`]'s link path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    /// A symlink to the skill's source.
    Linked,
    /// Nothing.
    Unlinked,
    /// Something that is not ours: a directory, a file, or a symlink pointing
    /// elsewhere. Left alone in either direction.
    Taken,
}

/// Every skill of `plugins` and whether `dir` links to it.
pub fn status(plugins: &[Plugin], dir: &Path) -> Vec<Entry> {
    plugins
        .iter()
        .flat_map(|plugin| plugin.skills.iter().map(move |skill| (plugin, skill)))
        .filter_map(|(plugin, skill)| {
            let source = skill.path.parent()?.to_path_buf();
            let link = dir.join(&skill.name);
            Some(Entry {
                plugin: plugin.name.clone(),
                name: skill.name.clone(),
                state: link_state(&link, &source),
                source,
                link,
            })
        })
        .collect()
}

fn link_state(link: &Path, source: &Path) -> LinkState {
    match std::fs::symlink_metadata(link) {
        Err(_) => LinkState::Unlinked,
        Ok(metadata) if !metadata.file_type().is_symlink() => LinkState::Taken,
        Ok(_) => {
            let ours = std::fs::read_link(link).is_ok_and(|target| target == source)
                || matches!(
                    (std::fs::canonicalize(link), std::fs::canonicalize(source)),
                    (Ok(a), Ok(b)) if a == b
                );
            if ours {
                LinkState::Linked
            } else {
                LinkState::Taken
            }
        }
    }
}

/// An [`Entry`] after [`install`], and whether the link was made this time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub entry: Entry,
    pub created: bool,
}

/// Links every skill of `plugins` into `dir` as `<dir>/<name>`, a symlink to
/// the skill's directory, creating `dir` if need be. A path that already holds
/// something other than that symlink is left as it is and reported
/// [`LinkState::Taken`]: it is the person's, or another tool's, and not ours
/// to replace. Linking again is a no-op, so this is safe to run after every
/// upgrade.
/// Links in `dir` that point into a plugin search path at something no longer
/// there — what a renamed or dropped skill leaves behind. They are ours to
/// remove: the target is gone, so nothing can be reading them, and a harness
/// scanning the directory would otherwise see a skill that cannot be read.
/// A dangling link pointing anywhere else is somebody else's and is kept.
pub fn prune(dir: &Path) -> io::Result<Vec<PathBuf>> {
    prune_in(dir, &search_paths())
}

/// [`prune`] against an explicit set of search paths.
pub fn prune_in(dir: &Path, roots: &[PathBuf]) -> io::Result<Vec<PathBuf>> {
    let mut pruned = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(pruned),
        Err(err) => return Err(err),
    };
    for entry in entries {
        let path = entry?.path();
        if !path.is_symlink() || path.exists() {
            continue;
        }
        let Ok(target) = std::fs::read_link(&path) else {
            continue;
        };
        if roots.iter().any(|root| target.starts_with(root)) {
            std::fs::remove_file(&path)?;
            pruned.push(path);
        }
    }
    pruned.sort();
    Ok(pruned)
}

pub fn install(plugins: &[Plugin], dir: &Path) -> io::Result<Vec<Installed>> {
    std::fs::create_dir_all(dir)?;
    status(plugins, dir)
        .into_iter()
        .map(|mut entry| {
            let created = entry.state == LinkState::Unlinked;
            if created {
                std::os::unix::fs::symlink(&entry.source, &entry.link)?;
                entry.state = LinkState::Linked;
            }
            Ok(Installed { entry, created })
        })
        .collect()
}

/// The plugins as AHP customizations: one container per plugin, its skills
/// and then its agents as children. Read-only — these are the desktop's, and a
/// client cannot write into `/usr/share`.
pub fn customizations(plugins: &[Plugin]) -> Vec<Customization> {
    plugins
        .iter()
        .map(|plugin| {
            let skills = plugin.skills.iter().map(|skill| {
                ChildCustomization::Skill(SkillCustomization {
                    id: format!("otto-skill:{}", skill.path.display()),
                    uri: uri::from_path(&skill.path),
                    name: skill.name.clone(),
                    icons: None,
                    range: None,
                    meta: None,
                    enabled: None,
                    description: Some(skill.description.clone()).filter(|text| !text.is_empty()),
                    // Otto's skills are for the agent to reach for when a
                    // request matches, and there is no slash-command
                    // surface here for a person to invoke one from.
                    disable_model_invocation: None,
                    disable_user_invocation: None,
                })
            });
            let agents = plugin.agents.iter().map(|agent| {
                ChildCustomization::Agent(AgentCustomization {
                    id: format!("otto-agent:{}", agent.path.display()),
                    uri: uri::from_path(&agent.path),
                    name: agent.name.clone(),
                    icons: None,
                    range: None,
                    meta: None,
                    enabled: None,
                    description: Some(agent.description.clone()).filter(|text| !text.is_empty()),
                    model: agent.model.clone(),
                    // The spec reads an empty list as no restriction, the
                    // same as absence; absence is the form it asks for.
                    tools: Some(agent.tools.clone()).filter(|tools| !tools.is_empty()),
                    disable_model_invocation: None,
                    disable_user_invocation: None,
                })
            });
            let children = skills.chain(agents).collect();
            Customization::Plugin(PluginCustomization {
                id: format!("otto-plugin:{}", plugin.dir.display()),
                uri: uri::from_path(&plugin.dir),
                name: plugin.name.clone(),
                icons: None,
                range: None,
                meta: None,
                client_id: None,
                load: Some(CustomizationLoadState::Loaded(CustomizationLoadedState {})),
                children: Some(children),
                enablement: None,
                version: plugin.version.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a plugin directory and returns its root.
    fn plugin_dir(root: &Path, name: &str, skills: &[(&str, &str)]) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join(".claude-plugin")).unwrap();
        std::fs::write(
            dir.join(".claude-plugin/plugin.json"),
            format!(r#"{{ "name": "{name}", "description": "d", "version": "1.0.0" }}"#),
        )
        .unwrap();
        for (skill, description) in skills {
            let skill_dir = dir.join("skills").join(skill);
            std::fs::create_dir_all(&skill_dir).unwrap();
            std::fs::write(
                skill_dir.join("SKILL.md"),
                format!("---\nname: {skill}\ndescription: {description}\n---\n\n# {skill}\n"),
            )
            .unwrap();
        }
        dir
    }

    /// Writes `<plugin>/agents/<name>.md` with the given frontmatter lines.
    fn agent_file(plugin: &Path, name: &str, frontmatter: &str) -> PathBuf {
        let dir = plugin.join("agents");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(format!("{name}.md"));
        std::fs::write(
            &file,
            format!("---\n{frontmatter}\n---\n\nYou help people use their desktop.\n"),
        )
        .unwrap();
        file
    }

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("otto-agents-skills-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_plugin_directory_is_read_with_its_skills() {
        let root = temp();
        plugin_dir(
            &root,
            "otto",
            &[
                ("configure-otto", "Change settings"),
                ("extend-otto-files", "Write a Files command"),
            ],
        );
        let plugins = discover_in(std::slice::from_ref(&root));

        assert_eq!(plugins.len(), 1);
        let plugin = &plugins[0];
        assert_eq!(plugin.name, "otto");
        assert_eq!(plugin.version.as_deref(), Some("1.0.0"));
        // Sorted, so the list reads the same way twice.
        let names: Vec<&str> = plugin.skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["configure-otto", "extend-otto-files"]);
        assert_eq!(plugin.skills[0].description, "Change settings");
        assert!(plugin.skills[0].path.ends_with("configure-otto/SKILL.md"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_name_that_would_escape_its_directory_is_refused() {
        let root = temp();
        let dir = plugin_dir(&root, "otto", &[("otto", "Use the desktop")]);
        // Renderings build paths from these names, so a separator or a parent
        // link in one would write outside the directory it was meant for.
        agent_file(
            &dir,
            "escapes",
            "name: ../../../../.config/systemd/user/evil.service\ndescription: Nothing good\n",
        );
        std::fs::create_dir_all(dir.join("skills/sneaky")).unwrap();
        std::fs::write(
            dir.join("skills/sneaky/SKILL.md"),
            "---\nname: ../../evil\ndescription: Nothing good\n---\n",
        )
        .unwrap();

        let plugins = discover_in(std::slice::from_ref(&root));
        let plugin = &plugins[0];
        let agents: Vec<&str> = plugin.agents.iter().map(|a| a.name.as_str()).collect();
        assert!(agents.contains(&"escapes"), "{agents:?}");
        let skills: Vec<&str> = plugin.skills.iter().map(|s| s.name.as_str()).collect();
        assert!(skills.contains(&"sneaky"), "{skills:?}");
        assert!(
            !skills.iter().chain(&agents).any(|name| name.contains('/')),
            "a name reached a path: {skills:?} {agents:?}"
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_directory_with_no_skills_is_not_a_plugin() {
        let root = temp();
        std::fs::create_dir_all(root.join("not-a-plugin/docs")).unwrap();
        assert!(discover_in(std::slice::from_ref(&root)).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn agent_files_are_read_from_the_plugin() {
        let root = temp();
        let dir = plugin_dir(&root, "otto", &[("otto", "Use the desktop")]);
        let file = agent_file(
            &dir,
            "otto",
            "name: otto\ndescription: \"Otto's own helper. Use it for the desktop.\"\ntools: Read, Grep, Bash\nskills: otto\n",
        );
        // No name, no tools: the stem names it and nothing is restricted.
        agent_file(&dir, "quiet", "description: Says little\nmodel: haiku\n");

        let plugins = discover_in(std::slice::from_ref(&root));
        let agents = &plugins[0].agents;
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].name, "otto");
        assert_eq!(
            agents[0].description,
            "Otto's own helper. Use it for the desktop."
        );
        assert_eq!(agents[0].tools, ["Read", "Grep", "Bash"]);
        assert_eq!(agents[0].skills, ["otto"]);
        assert_eq!(agents[0].model, None);
        assert_eq!(agents[0].body, "You help people use their desktop.");
        assert_eq!(agents[0].path, file);
        assert_eq!(agents[1].name, "quiet");
        assert!(agents[1].tools.is_empty());
        assert_eq!(agents[1].model.as_deref(), Some("haiku"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A plugin can ship an agent and nothing else.
    #[test]
    fn a_plugin_with_agents_but_no_skills_is_still_a_plugin() {
        let root = temp();
        agent_file(&root.join("helper"), "helper", "description: Helps");
        let plugins = discover_in(std::slice::from_ref(&root));
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "helper");
        assert!(plugins[0].skills.is_empty());
        assert_eq!(plugins[0].agents[0].name, "helper");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// The point of searching the user's directory first.
    #[test]
    fn a_user_plugin_shadows_the_packaged_one_of_the_same_name() {
        let user = temp();
        let system = temp();
        plugin_dir(&user, "otto", &[("configure-otto", "mine")]);
        plugin_dir(&system, "otto", &[("configure-otto", "the packaged one")]);

        let plugins = discover_in(&[user.clone(), system.clone()][..]);
        assert_eq!(plugins.len(), 1, "one plugin, not two of the same name");
        assert_eq!(plugins[0].skills[0].description, "mine");

        std::fs::remove_dir_all(&user).unwrap();
        std::fs::remove_dir_all(&system).unwrap();
    }

    /// A renamed skill leaves a link pointing at a path the upgrade removed.
    /// Nothing else revisits it, so `install` prunes it first.
    #[test]
    fn a_link_to_a_skill_that_is_gone_is_pruned() {
        let root = temp();
        let links = temp();
        let plugins = root.join("otto").join("skills");
        std::fs::create_dir_all(plugins.join("otto-help")).unwrap();

        let live = links.join("otto-help");
        let renamed = links.join("otto");
        let mine = links.join("something-else");
        std::os::unix::fs::symlink(plugins.join("otto-help"), &live).unwrap();
        std::os::unix::fs::symlink(plugins.join("otto"), &renamed).unwrap();
        std::os::unix::fs::symlink(Path::new("/nowhere/of/mine"), &mine).unwrap();

        let pruned = prune_in(&links, std::slice::from_ref(&root)).unwrap();

        assert_eq!(pruned, vec![renamed.clone()], "only the stale one");
        assert!(!renamed.exists() && !renamed.is_symlink(), "it is gone");
        assert!(live.exists(), "a live link is kept");
        assert!(
            mine.is_symlink(),
            "a dangling link of someone else's is kept"
        );

        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&links).unwrap();
    }

    #[test]
    fn frontmatter_takes_every_top_level_key() {
        let keys = frontmatter(
            "---\nname: configure-otto\ndescription: \"Configure the desktop\"\nother: kept\nnested:\n  inner: skipped\n---\nbody: not frontmatter\n",
        );
        assert_eq!(keys["name"], "configure-otto");
        assert_eq!(keys["description"], "Configure the desktop");
        assert_eq!(keys["other"], "kept");
        assert_eq!(keys["nested"], "");
        assert!(!keys.contains_key("inner"));
        assert!(!keys.contains_key("body"));
        assert!(frontmatter("# No frontmatter\n").is_empty());
    }

    #[test]
    fn the_body_is_what_follows_the_frontmatter() {
        assert_eq!(body("---\nname: a\n---\n\nHello.\n\n"), "Hello.");
        assert_eq!(body("---\nname: a\n---\n"), "");
        assert_eq!(body("# Just text\n"), "# Just text");
        assert_eq!(
            body("---\nname: a\n"),
            "",
            "an unclosed frontmatter has no body"
        );
        assert_eq!(
            body("---\nname: a\n---\nOne --- dash line\n---\nTwo\n"),
            "One --- dash line\n---\nTwo",
            "a later `---` is the body's"
        );
    }

    #[test]
    fn a_file_without_frontmatter_falls_back_to_the_directory_name() {
        let root = temp();
        let skill = root.join("plug/skills/quiet");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "# No frontmatter here\n").unwrap();

        let plugins = discover_in(std::slice::from_ref(&root));
        assert_eq!(plugins[0].skills[0].name, "quiet");
        assert_eq!(plugins[0].skills[0].description, "");
        // No manifest either: the directory names the plugin.
        assert_eq!(plugins[0].name, "plug");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn customizations_carry_the_skills_as_children() {
        let root = temp();
        plugin_dir(&root, "otto", &[("configure-otto", "Change settings")]);
        let plugins = discover_in(std::slice::from_ref(&root));

        let published = customizations(&plugins);
        assert_eq!(published.len(), 1);
        let Customization::Plugin(plugin) = &published[0] else {
            panic!("a plugin directory publishes as a plugin customization");
        };
        assert_eq!(plugin.name, "otto");
        assert_eq!(plugin.version.as_deref(), Some("1.0.0"));
        let children = plugin.children.as_ref().expect("children were parsed");
        let ChildCustomization::Skill(skill) = &children[0] else {
            panic!("a SKILL.md publishes as a skill customization");
        };
        assert_eq!(skill.name, "configure-otto");
        assert_eq!(skill.description.as_deref(), Some("Change settings"));
        assert!(skill.uri.starts_with("file:///"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn customizations_carry_the_agents_after_the_skills() {
        let root = temp();
        let dir = plugin_dir(&root, "otto", &[("otto", "Use the desktop")]);
        let file = agent_file(
            &dir,
            "otto",
            "name: otto\ndescription: Otto's own helper\ntools: Read, Bash\nskills: otto\n",
        );
        agent_file(&dir, "plain", "description: No tools listed\n");
        let plugins = discover_in(std::slice::from_ref(&root));

        let published = customizations(&plugins);
        let Customization::Plugin(plugin) = &published[0] else {
            panic!("a plugin directory publishes as a plugin customization");
        };
        let children = plugin.children.as_ref().expect("children were parsed");
        assert_eq!(children.len(), 3);
        assert!(matches!(children[0], ChildCustomization::Skill(_)));
        let ChildCustomization::Agent(agent) = &children[1] else {
            panic!("an agent file publishes as an agent customization");
        };
        assert_eq!(agent.id, format!("otto-agent:{}", file.display()));
        assert_eq!(agent.uri, uri::from_path(&file));
        assert_eq!(agent.name, "otto");
        assert_eq!(agent.description.as_deref(), Some("Otto's own helper"));
        assert_eq!(
            agent.tools,
            Some(vec!["Read".to_owned(), "Bash".to_owned()])
        );
        assert_eq!(agent.model, None);
        assert_eq!(agent.disable_model_invocation, None);
        assert_eq!(agent.disable_user_invocation, None);
        let ChildCustomization::Agent(plain) = &children[2] else {
            panic!("an agent file publishes as an agent customization");
        };
        assert_eq!(plain.name, "plain");
        assert_eq!(plain.tools, None, "no restriction is absence, not []");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn claude_is_handed_the_plugin_directories_and_a_system_prompt() {
        let root = temp();
        let dir = plugin_dir(&root, "otto", &[("otto", "Use the desktop")]);
        let plugins = discover_in(std::slice::from_ref(&root));
        let meta = claude_session_meta(&plugins, None).expect("one plugin to load");
        assert_eq!(
            serde_json::Value::Object(meta),
            serde_json::json!({
                "claudeCode": { "options": { "plugins": [{ "type": "local", "path": dir }] } },
                "systemPrompt": { "append": claude_system_prompt(&plugins) },
            }),
            "`append` extends the adapter's claude_code preset; a string would replace it"
        );
        assert_eq!(claude_session_meta(&[], None), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_system_prompt_names_the_plugin_and_how_to_invoke_it() {
        let root = temp();
        plugin_dir(&root, "otto", &[("otto", "Use the desktop")]);
        let plugins = discover_in(std::slice::from_ref(&root));

        let prompt = claude_system_prompt(&plugins);
        assert!(prompt.starts_with("You are running on Otto, a Wayland desktop."));
        assert!(
            prompt.contains("loaded as the `otto` plugin; `/otto` invokes it."),
            "{prompt}"
        );
        assert!(prompt.contains("when none matches, ignore this and carry on"));
        assert!(
            prompt.len() < 600,
            "{} bytes: every session pays for it",
            prompt.len()
        );

        plugin_dir(&root, "mine", &[("a", "A"), ("b", "B")]);
        let plugins = discover_in(std::slice::from_ref(&root));
        let prompt = claude_system_prompt(&plugins);
        assert!(
            prompt.contains("the `mine` and `otto` plugins; `/a`, `/b` and `/otto` invoke them."),
            "{prompt}"
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn running_as_a_plugin_agent_is_claudes_own_flag() {
        let root = temp();
        let dir = plugin_dir(&root, "otto", &[("otto", "Use the desktop")]);
        agent_file(
            &dir,
            "otto",
            "name: otto\ndescription: Otto's own helper.\n",
        );
        let plugins = discover_in(std::slice::from_ref(&root));
        let (run_as, file) = super::agent_file(&plugins, "otto").expect("the agent file is there");
        assert_eq!(run_as, "otto:otto");
        assert!(file.path.ends_with("agents/otto.md"));
        assert!(super::agent_file(&plugins, "nobody").is_none());

        let meta = claude_session_meta(&plugins, Some(&run_as)).expect("one plugin to load");
        assert_eq!(
            meta["claudeCode"]["options"]["extraArgs"],
            serde_json::json!({ "agent": "otto:otto" })
        );
        let plain = claude_session_meta(&plugins, None).expect("one plugin to load");
        assert!(plain["claudeCode"]["options"].get("extraArgs").is_none());
    }

    #[test]
    fn the_system_prompt_names_the_agent_when_the_plugin_has_one() {
        let root = temp();
        let dir = plugin_dir(&root, "otto", &[("otto", "Use the desktop")]);
        agent_file(&dir, "otto", "name: otto\ndescription: Otto's own helper\n");
        let plugins = discover_in(std::slice::from_ref(&root));

        let prompt = claude_system_prompt(&plugins);
        assert!(
            prompt.contains(
                "loaded as the `otto` plugin; `/otto` invokes it and the `otto` agent \
                 answers questions about the desktop."
            ),
            "{prompt}"
        );
        assert!(
            prompt.len() < 600,
            "{} bytes: every session pays for it",
            prompt.len()
        );

        // An agent-only plugin has nothing to invoke.
        let helper = root.join("helper");
        agent_file(&helper, "helper", "description: Helps\n");
        agent_file(&helper, "other", "description: Also helps\n");
        std::fs::remove_dir_all(dir.join("skills")).unwrap();
        let plugins = discover_in(std::slice::from_ref(&root));
        let prompt = claude_system_prompt(&plugins);
        assert!(
            prompt.contains(
                "the `helper` and `otto` plugins; the `helper`, `other` and `otto` agents \
                 answer questions about the desktop."
            ),
            "{prompt}"
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn install_links_each_skill_by_name_and_leaves_what_is_not_ours() {
        let root = temp();
        plugin_dir(
            &root,
            "otto",
            &[("otto", "Use the desktop"), ("taken", "In the way")],
        );
        let plugins = discover_in(std::slice::from_ref(&root));
        let dir = root.join("agents/skills");
        // Something of the person's already sits where `taken` would go.
        std::fs::create_dir_all(dir.join("taken")).unwrap();

        let before = status(&plugins, &dir);
        assert_eq!(before.len(), 2);
        assert_eq!(before[0].state, LinkState::Unlinked);
        assert_eq!(before[1].state, LinkState::Taken);

        let installed = install(&plugins, &dir).unwrap();
        assert_eq!(installed.len(), 2);
        assert!(installed[0].created);
        assert_eq!(installed[0].entry.state, LinkState::Linked);
        assert_eq!(installed[0].entry.link, dir.join("otto"));
        assert_eq!(
            std::fs::read_link(dir.join("otto")).unwrap(),
            root.join("otto/skills/otto"),
            "a symlink to the skill's directory, so upgrades reach it"
        );
        assert!(dir.join("otto/SKILL.md").is_file());
        assert!(!installed[1].created);
        assert_eq!(installed[1].entry.state, LinkState::Taken);
        assert!(
            dir.join("taken").is_dir() && !dir.join("taken").is_symlink(),
            "what was there stays"
        );

        // Again: nothing to do, nothing broken.
        let again = install(&plugins, &dir).unwrap();
        assert!(again.iter().all(|installed| !installed.created));
        assert_eq!(again[0].entry.state, LinkState::Linked);
        assert_eq!(status(&plugins, &dir)[0].state, LinkState::Linked);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_symlink_that_points_elsewhere_is_taken_not_linked() {
        let root = temp();
        plugin_dir(&root, "otto", &[("otto", "Use the desktop")]);
        let plugins = discover_in(std::slice::from_ref(&root));
        let dir = root.join("agents/skills");
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(root.join("somewhere-else"), dir.join("otto")).unwrap();

        assert_eq!(status(&plugins, &dir)[0].state, LinkState::Taken);
        let installed = install(&plugins, &dir).unwrap();
        assert!(!installed[0].created);
        assert_eq!(
            std::fs::read_link(dir.join("otto")).unwrap(),
            root.join("somewhere-else"),
            "left pointing where it pointed"
        );

        std::fs::remove_dir_all(&root).unwrap();
    }
}
