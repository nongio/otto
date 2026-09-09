//! What the user has said about the sidebar, in `~/.config/otto/files.toml`.
//!
//! The file is entirely optional and the defaults stand without it: Recent,
//! Home, and whichever XDG user directories actually exist. What it adds is
//! the two things a defaults-only sidebar cannot express — folders that are
//! nobody's idea of standard but are where *this* person works, and built-in
//! rows they never use taking up the list.
//!
//! ```toml
//! [sidebar]
//! # Built-in rows to leave out, by name.
//! hide = ["music", "videos"]
//!
//! # Folders to add, after the built-in ones.
//! [[sidebar.places]]
//! path = "~/dev/otto"
//! label = "Otto"          # optional: the folder's own name otherwise
//! icon = "folder-code"    # optional: a generic folder otherwise
//! ```
//!
//! A file that cannot be read or cannot be parsed is a warning in the log and
//! nothing more. A sidebar is how you get anywhere in this window, and losing
//! it over a stray bracket would be a bad trade for a strictness nobody asked
//! for.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The built-in rows, by the name the config file calls them.
///
/// Stable identifiers rather than labels: the label is translated, and a
/// config written on an English desktop must not stop working when the
/// language changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Recent,
    Home,
    Desktop,
    Documents,
    Downloads,
    Music,
    Pictures,
    Videos,
}

impl Builtin {
    /// The name this row answers to in `hide`.
    pub fn id(self) -> &'static str {
        match self {
            Builtin::Recent => "recent",
            Builtin::Home => "home",
            Builtin::Desktop => "desktop",
            Builtin::Documents => "documents",
            Builtin::Downloads => "downloads",
            Builtin::Music => "music",
            Builtin::Pictures => "pictures",
            Builtin::Videos => "videos",
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct File {
    #[serde(default)]
    sidebar: Sidebar,
}

#[derive(Debug, Default, Deserialize)]
struct Sidebar {
    #[serde(default)]
    hide: Vec<String>,
    #[serde(default)]
    places: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
struct Entry {
    path: String,
    label: Option<String>,
    icon: Option<String>,
}

/// One folder the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Custom {
    pub label: String,
    pub path: PathBuf,
    pub icon: String,
}

/// The sidebar as configured, or an empty one where nothing was said.
#[derive(Debug, Default, Clone)]
pub struct SidebarConfig {
    hidden: Vec<String>,
    pub extra: Vec<Custom>,
}

impl SidebarConfig {
    /// Read `~/.config/otto/files.toml`, if it is there.
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            // Absent is the ordinary case and says nothing worth logging;
            // unreadable is worth a line, because the person meant it to be
            // read.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(err) => {
                tracing::warn!(path = %path.display(), %err, "cannot read the sidebar config");
                return Self::default();
            }
        };
        match toml::from_str::<File>(&text) {
            Ok(file) => Self::from_file(file, home_dir().as_deref()),
            Err(err) => {
                tracing::warn!(path = %path.display(), %err, "ignoring the sidebar config");
                Self::default()
            }
        }
    }

    fn from_file(file: File, home: Option<&Path>) -> Self {
        Self {
            hidden: file
                .sidebar
                .hide
                .into_iter()
                .map(|name| name.trim().to_lowercase())
                .collect(),
            extra: file
                .sidebar
                .places
                .into_iter()
                .filter_map(|entry| {
                    let path = expand(&entry.path, home)?;
                    Some(Custom {
                        // A folder's own name is nearly always what you would
                        // have typed anyway, so the label is worth asking for
                        // only when it is not.
                        label: entry.label.unwrap_or_else(|| {
                            path.file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.to_string_lossy().into_owned())
                        }),
                        icon: entry.icon.unwrap_or_else(|| "folder".to_string()),
                        path,
                    })
                })
                .collect(),
        }
    }

    /// A config that hides these rows and says nothing else. For tests, and
    /// for anywhere the answer is known without reading a file.
    pub fn hiding(builtins: &[Builtin]) -> Self {
        Self {
            hidden: builtins.iter().map(|b| b.id().to_string()).collect(),
            extra: Vec::new(),
        }
    }

    /// Whether a built-in row was asked to stay out of the list.
    pub fn hides(&self, builtin: Builtin) -> bool {
        self.hidden.iter().any(|name| name == builtin.id())
    }
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".config")))?;
    Some(base.join("otto").join("files.toml"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

/// `~/dev` and `$HOME/dev` both mean the same folder to the person writing
/// them, and neither is a path any shell has already expanded by the time it
/// reaches a config file.
fn expand(raw: &str, home: Option<&Path>) -> Option<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let path = if let Some(rest) = raw
        .strip_prefix("~/")
        .or_else(|| raw.strip_prefix("$HOME/"))
    {
        home?.join(rest)
    } else if raw == "~" || raw == "$HOME" {
        home?.to_path_buf()
    } else {
        PathBuf::from(raw)
    };
    // A row leading nowhere is worse than its absence — the same rule the
    // built-in XDG folders follow.
    path.is_dir().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str, home: &Path) -> SidebarConfig {
        SidebarConfig::from_file(
            toml::from_str::<File>(text).expect("valid toml"),
            Some(home),
        )
    }

    /// The shape the documentation promises, read back.
    #[test]
    fn a_configured_sidebar_hides_and_adds() {
        let dir = std::env::temp_dir().join(format!("otto-files-cfg-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("dev/otto")).unwrap();

        let config = parse(
            r#"
            [sidebar]
            hide = ["music", "Videos"]

            [[sidebar.places]]
            path = "~/dev/otto"
            label = "Otto"
            icon = "folder-code"

            [[sidebar.places]]
            path = "~/dev"
            "#,
            &dir,
        );

        assert!(config.hides(Builtin::Music));
        // Names are matched case-insensitively: a config is prose, not code.
        assert!(config.hides(Builtin::Videos));
        assert!(!config.hides(Builtin::Home));

        assert_eq!(config.extra.len(), 2);
        assert_eq!(config.extra[0].label, "Otto");
        assert_eq!(config.extra[0].icon, "folder-code");
        assert_eq!(config.extra[0].path, dir.join("dev/otto"));
        // No label given, so the folder's own name; no icon, so a folder.
        assert_eq!(config.extra[1].label, "dev");
        assert_eq!(config.extra[1].icon, "folder");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A row that leads nowhere is dropped rather than drawn, the same as a
    /// missing XDG folder. A typo in a path should cost that row and no more.
    #[test]
    fn a_folder_that_is_not_there_is_not_listed() {
        let home = std::env::temp_dir();
        let config = parse(
            r#"
            [[sidebar.places]]
            path = "~/definitely-not-a-real-folder-9f3a"
            "#,
            &home,
        );
        assert!(config.extra.is_empty());
    }

    /// Saying nothing is the ordinary case and has to mean "the defaults".
    #[test]
    fn an_empty_config_changes_nothing() {
        let config = parse("", &std::env::temp_dir());
        assert!(config.extra.is_empty());
        for builtin in [
            Builtin::Recent,
            Builtin::Home,
            Builtin::Desktop,
            Builtin::Documents,
            Builtin::Downloads,
            Builtin::Music,
            Builtin::Pictures,
            Builtin::Videos,
        ] {
            assert!(!config.hides(builtin), "{}", builtin.id());
        }
    }
}
