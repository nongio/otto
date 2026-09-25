//! The instructions an agent can run as: the agent files in Otto's plugins.
//!
//! Found the way otto-agents finds them (`skills.rs` there): plugins under
//! `$XDG_DATA_HOME/otto/plugins/` first, then the packaged ones under
//! `/usr/share/otto/plugins/`, a user plugin shadowing a packaged one of the
//! same name. Each plugin's `agents/*.md` is one set of instructions, known by
//! the `name` in its frontmatter.

use std::path::{Path, PathBuf};

/// One agent file.
#[derive(Clone, Debug, PartialEq)]
pub struct Instructions {
    /// The `name` in its frontmatter, which is what an agent block names.
    pub name: String,
    pub path: PathBuf,
}

/// Where plugins are looked for, most important first.
fn plugin_dirs() -> Vec<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    data.map(|dir| dir.join("otto/plugins"))
        .into_iter()
        .chain([PathBuf::from("/usr/share/otto/plugins")])
        .collect()
}

/// Every agent file the plugins hold, the first of each name winning.
pub fn discover() -> Vec<Instructions> {
    discover_in(&plugin_dirs())
}

fn discover_in(dirs: &[PathBuf]) -> Vec<Instructions> {
    let mut plugins_seen: Vec<std::ffi::OsString> = Vec::new();
    let mut found: Vec<Instructions> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut plugins: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        plugins.sort();
        for plugin in plugins {
            let Some(plugin_name) = plugin.file_name().map(|n| n.to_os_string()) else {
                continue;
            };
            if plugins_seen.contains(&plugin_name) {
                continue;
            }
            plugins_seen.push(plugin_name);
            let Ok(files) = std::fs::read_dir(plugin.join("agents")) else {
                continue;
            };
            let mut files: Vec<PathBuf> = files
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
                .collect();
            files.sort();
            for path in files {
                let Some(name) = name_of(&path) else {
                    continue;
                };
                if !found.iter().any(|known| known.name == name) {
                    found.push(Instructions { name, path });
                }
            }
        }
    }
    found
}

/// The `name` in an agent file's frontmatter, or its file name without one.
fn name_of(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let from_frontmatter = text.strip_prefix("---").and_then(|rest| {
        rest.lines()
            .take_while(|line| line.trim() != "---")
            .find_map(|line| line.strip_prefix("name:"))
            .map(|name| name.trim().trim_matches(['"', '\'']).to_string())
            .filter(|name| !name.is_empty())
    });
    from_frontmatter.or_else(|| Some(path.file_stem()?.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_plugin_shadows_a_packaged_one() {
        let root =
            std::env::temp_dir().join(format!("otto-settings-plugins-{}", std::process::id()));
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write("user/otto/agents/otto.md", "---\nname: otto\n---\nmine");
        write(
            "user/review/agents/reviewer.md",
            "---\nname: \"reviewer\"\nmodel: opus\n---\n",
        );
        write(
            "system/otto/agents/otto.md",
            "---\nname: otto\n---\npackaged",
        );
        write("system/otto/agents/helper.md", "no frontmatter");

        let found = discover_in(&[root.join("user"), root.join("system")]);
        let names: Vec<&str> = found.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["otto", "reviewer"]);
        assert!(found[0].path.starts_with(root.join("user")));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
