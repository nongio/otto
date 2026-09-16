//! Skills the desktop offers its agents.
//!
//! Otto installs its own skills to `/usr/share/otto/plugins/<plugin>/`, in the
//! [Open Plugins](https://open-plugins.com/) shape: a
//! `.claude-plugin/plugin.json` manifest and a `skills/` directory with one
//! `SKILL.md` per skill. A person adds their own under
//! `$XDG_DATA_HOME/otto/plugins/`, which is searched first, so a plugin of the
//! same name shadows the packaged one.
//!
//! Two things happen with what is found here:
//!
//! - **The agent is told.** ACP has no field for skills, and no agent we run
//!   reads Otto's directory on its own, so the list goes to the agent as a
//!   briefing prepended to the first prompt of a new session: one line per
//!   skill, with its name, its description and the absolute path of its
//!   `SKILL.md`. The agent reads the file when — and only when — the
//!   description matches what it was asked. That is the whole point of the
//!   frontmatter, and it is why the briefing stays a few hundred bytes however
//!   long the skills grow.
//! - **The clients are shown.** The same plugins are published as AHP
//!   [`Customization`] entries on the agent and on each session, read-only,
//!   with their skills as children — so a client can show exactly what a
//!   session was given without asking the agent.
//!
//! Discovery happens once, at startup: the packaged skills change when the
//! package does, which is a restart either way.

use std::path::{Path, PathBuf};

use ahp_types::state::{
    ChildCustomization, Customization, CustomizationLoadState, CustomizationLoadedState,
    PluginCustomization, SkillCustomization,
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

/// One plugin directory, with the skills found inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plugin {
    /// The manifest's `name`, or the directory's name when there is no
    /// manifest.
    pub name: String,
    pub description: String,
    pub version: Option<String>,
    pub dir: PathBuf,
    pub skills: Vec<Skill>,
}

/// The directories searched, most specific first. `OTTO_AGENTS_PLUGINS` replaces
/// the list entirely — a colon-separated path, as `PATH` is — which is how a
/// plugin is tried out without installing it.
pub fn search_paths() -> Vec<PathBuf> {
    if let Some(overridden) = std::env::var_os("OTTO_AGENTS_PLUGINS") {
        return std::env::split_paths(&overridden)
            .filter(|path| !path.as_os_str().is_empty())
            .collect();
    }
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    let mut paths: Vec<PathBuf> = data_home
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
        // read_dir gives whatever order the filesystem holds; the briefing and
        // the customization list are both read by people, so sort them.
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

/// Reads one plugin directory, or `None` when it holds no skills — an
/// arbitrary directory under a search path is not a plugin, and a plugin that
/// contributes nothing is not worth telling anyone about.
fn read_plugin(dir: &Path) -> Option<Plugin> {
    let name = dir.file_name()?.to_string_lossy().into_owned();
    let manifest = read_manifest(&dir.join(".claude-plugin/plugin.json"));
    let skills = read_skills(&dir.join("skills"));
    if skills.is_empty() {
        return None;
    }
    let (manifest_name, description, version) = manifest.unwrap_or_default();
    Some(Plugin {
        name: if manifest_name.is_empty() {
            name
        } else {
            manifest_name
        },
        description,
        version,
        dir: dir.to_path_buf(),
        skills,
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
            let (name, description) = frontmatter(&text);
            Some(Skill {
                name: if name.is_empty() {
                    path.file_name()?.to_string_lossy().into_owned()
                } else {
                    name
                },
                description,
                path: file,
            })
        })
        .collect()
}

/// `name` and `description` out of a `SKILL.md`'s YAML frontmatter.
///
/// Deliberately not a YAML parser: the two keys a skill must carry are plain
/// scalars on one line, and a dependency that can parse the rest of YAML would
/// only let a skill do things nothing here reads. A folded or multi-line value
/// reads as far as the first line, which is enough to route on.
fn frontmatter(text: &str) -> (String, String) {
    let mut name = String::new();
    let mut description = String::new();
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (name, description);
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
        let value = unquote(value.trim());
        match key.trim() {
            "name" => name = value,
            "description" => description = value,
            _ => {}
        }
    }
    (name, description)
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

/// What a new session's agent is told, or `None` when there is nothing to say.
///
/// Every line is one skill: its name, its description, and where to read it.
/// The agent is told to read the file rather than being given its contents,
/// because a briefing that grew with the skills would cost every session the
/// whole guide to answer "what time is it".
pub fn briefing(plugins: &[Plugin]) -> Option<String> {
    let skills: Vec<&Skill> = plugins.iter().flat_map(|plugin| &plugin.skills).collect();
    if skills.is_empty() {
        return None;
    }
    let mut text = String::from(
        "You are running on Otto, a Wayland desktop. Otto provides the skills \
         below. Each is a Markdown file you can read with your own tools. When \
         a request matches one, read that file first and follow it; when none \
         matches, ignore this and carry on. A request that opens with `/<name>` \
         names the skill to use, and the rest of it is the request.\n\n",
    );
    for skill in skills {
        text.push_str("- ");
        text.push_str(&skill.name);
        if !skill.description.is_empty() {
            text.push_str(" — ");
            text.push_str(&skill.description);
        }
        text.push_str("\n  ");
        text.push_str(&skill.path.to_string_lossy());
        text.push('\n');
    }
    Some(text)
}

/// The session options that have claude-agent-acp load `plugins` as Claude
/// Code plugins, or `None` when there are none. Claude reads them from the
/// session's `_meta.claudeCode.options`, which other agents ignore; its skills
/// then come through Claude's own skill machinery, `allowed-tools` included.
pub fn claude_session_meta(
    plugins: &[Plugin],
) -> Option<serde_json::Map<String, serde_json::Value>> {
    if plugins.is_empty() {
        return None;
    }
    let plugins: Vec<serde_json::Value> = plugins
        .iter()
        .map(|plugin| serde_json::json!({ "type": "local", "path": plugin.dir }))
        .collect();
    match serde_json::json!({ "claudeCode": { "options": { "plugins": plugins } } }) {
        serde_json::Value::Object(meta) => Some(meta),
        _ => None,
    }
}

/// The plugins as AHP customizations: one container per plugin, its skills as
/// children. Read-only — these are the desktop's, and a client cannot write
/// into `/usr/share`.
pub fn customizations(plugins: &[Plugin]) -> Vec<Customization> {
    plugins
        .iter()
        .map(|plugin| {
            let children = plugin
                .skills
                .iter()
                .map(|skill| {
                    ChildCustomization::Skill(SkillCustomization {
                        id: format!("otto-skill:{}", skill.path.display()),
                        uri: uri::from_path(&skill.path),
                        name: skill.name.clone(),
                        icons: None,
                        range: None,
                        meta: None,
                        enabled: None,
                        description: Some(skill.description.clone())
                            .filter(|text| !text.is_empty()),
                        // Otto's skills are for the agent to reach for when a
                        // request matches, and there is no slash-command
                        // surface here for a person to invoke one from.
                        disable_model_invocation: None,
                        disable_user_invocation: None,
                    })
                })
                .collect();
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

    fn temp() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("otto-agentsd-skills-{}", uuid::Uuid::new_v4()));
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
        // Sorted, so the briefing reads the same way twice.
        let names: Vec<&str> = plugin.skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["configure-otto", "extend-otto-files"]);
        assert_eq!(plugin.skills[0].description, "Change settings");
        assert!(plugin.skills[0].path.ends_with("configure-otto/SKILL.md"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_directory_with_no_skills_is_not_a_plugin() {
        let root = temp();
        std::fs::create_dir_all(root.join("not-a-plugin/docs")).unwrap();
        assert!(discover_in(std::slice::from_ref(&root)).is_empty());
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

    #[test]
    fn frontmatter_takes_the_two_keys_that_matter() {
        let (name, description) = frontmatter(
            "---\nname: configure-otto\ndescription: \"Configure the desktop\"\nother: ignored\n---\nbody: not frontmatter\n",
        );
        assert_eq!(name, "configure-otto");
        assert_eq!(description, "Configure the desktop");
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

    /// The briefing names every skill and where to read it, and nothing else:
    /// it is prepended to a real prompt, so every word is paid for.
    #[test]
    fn the_briefing_lists_each_skill_with_its_path() {
        let root = temp();
        plugin_dir(&root, "otto", &[("configure-otto", "Change settings")]);
        let plugins = discover_in(std::slice::from_ref(&root));

        let briefing = briefing(&plugins).expect("skills were found");
        assert!(briefing.contains("configure-otto — Change settings"));
        assert!(briefing.contains(&format!(
            "{}/otto/skills/configure-otto/SKILL.md",
            root.display()
        )));
        assert!(
            !briefing.contains("# configure-otto"),
            "the briefing points at the file, it does not inline it"
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn no_plugins_means_no_briefing() {
        assert_eq!(briefing(&[]), None);
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
    fn claude_is_handed_the_plugin_directories() {
        let root = temp();
        let dir = plugin_dir(&root, "otto", &[("configure", "Change settings")]);
        let plugins = discover_in(std::slice::from_ref(&root));
        let meta = claude_session_meta(&plugins).expect("one plugin to load");
        assert_eq!(
            serde_json::Value::Object(meta),
            serde_json::json!({
                "claudeCode": { "options": { "plugins": [{ "type": "local", "path": dir }] } }
            })
        );
        assert_eq!(claude_session_meta(&[]), None);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
