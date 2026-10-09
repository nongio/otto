//! The directory each application last accepted from, so its next request
//! opens there. See `specs/file-picker.md`, *Starting directory*.
//!
//! `$XDG_STATE_HOME/otto/file-picker-dirs`: one `app_id<TAB>path` per line,
//! least recently used first, at most [`CAPACITY`] lines. State, not
//! configuration — nobody is invited to edit it, and losing it costs a
//! remembered folder and nothing else.
//!
//! A request with no `app_id` — what an unsandboxed application sends through
//! the portal — is remembered under the empty key. Every such application
//! shares that one slot: a shared memory is still where the user last was,
//! and no memory at all would send most of their applications home every time.

use std::path::{Path, PathBuf};

/// How many applications are remembered before the oldest is forgotten.
pub const CAPACITY: usize = 64;

/// The table, least recently used first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PickerDirs {
    entries: Vec<(String, PathBuf)>,
}

impl PickerDirs {
    /// Read the table from `text`. A line that does not parse is skipped
    /// rather than costing the rest of the file.
    pub fn parse(text: &str) -> Self {
        let mut dirs = Self::default();
        for line in text.lines() {
            let Some((app_id, path)) = line.split_once('\t') else {
                continue;
            };
            if path.starts_with('/') {
                dirs.remember(app_id, Path::new(path));
            }
        }
        dirs
    }

    pub fn to_text(&self) -> String {
        self.entries
            .iter()
            .filter_map(|(app_id, path)| Some(format!("{app_id}\t{}\n", path.to_str()?)))
            .collect()
    }

    /// The directory `app_id` last accepted from.
    pub fn get(&self, app_id: &str) -> Option<&Path> {
        self.entries
            .iter()
            .find(|(id, _)| id == app_id)
            .map(|(_, path)| path.as_path())
    }

    /// Record `dir` as where `app_id` last accepted, making it the most
    /// recent entry. A path the one-line format cannot carry is not recorded.
    pub fn remember(&mut self, app_id: &str, dir: &Path) {
        let Some(text) = dir.to_str() else {
            return;
        };
        if !dir.is_absolute() || text.contains(['\n', '\r']) || app_id.contains(['\t', '\n']) {
            return;
        }
        self.entries.retain(|(id, _)| id != app_id);
        self.entries.push((app_id.to_string(), dir.to_path_buf()));
        if self.entries.len() > CAPACITY {
            let excess = self.entries.len() - CAPACITY;
            self.entries.drain(..excess);
        }
    }

    /// The table as it is on disk; empty when there is none.
    pub fn load() -> Self {
        let Some(path) = state_path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(err) => {
                tracing::warn!("could not read {}: {err}", path.display());
                Self::default()
            }
        }
    }

    /// Write the table back. Quiet on failure, like the window's own state.
    pub fn save(&self) {
        let Some(path) = state_path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            if let Err(err) = std::fs::create_dir_all(dir) {
                tracing::warn!("could not create {}: {err}", dir.display());
                return;
            }
        }
        if let Err(err) = std::fs::write(&path, self.to_text()) {
            tracing::warn!("could not write {}: {err}", path.display());
        }
    }
}

/// Where `app_id` last accepted from, read fresh from disk: requests from
/// different applications may be served by different picker processes.
pub fn remembered(app_id: &str) -> Option<PathBuf> {
    PickerDirs::load().get(app_id).map(Path::to_path_buf)
}

/// Record an accept on disk.
pub fn record(app_id: &str, dir: &Path) {
    let mut dirs = PickerDirs::load();
    dirs.remember(app_id, dir);
    dirs.save();
}

/// Honours `XDG_STATE_HOME`; `OTTO_PICKER_DIRS` overrides the whole path, for
/// tests and for running two copies side by side.
fn state_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("OTTO_PICKER_DIRS") {
        return Some(PathBuf::from(path));
    }
    // The browser's tests answer pickers; none of them may touch the real file.
    if cfg!(test) {
        return None;
    }
    let base = std::env::var("XDG_STATE_HOME")
        .ok()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".local").join("state"))
        })?;
    Some(base.join("otto").join("file-picker-dirs"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_application_gets_its_own_directory_back() {
        let mut dirs = PickerDirs::default();
        dirs.remember("org.gimp.GIMP", Path::new("/home/me/Pictures"));
        dirs.remember("org.mozilla.firefox", Path::new("/home/me/Downloads"));
        let back = PickerDirs::parse(&dirs.to_text());
        assert_eq!(
            back.get("org.gimp.GIMP"),
            Some(Path::new("/home/me/Pictures"))
        );
        assert_eq!(
            back.get("org.mozilla.firefox"),
            Some(Path::new("/home/me/Downloads"))
        );
        assert_eq!(back.get("org.other.App"), None);
    }

    #[test]
    fn an_application_without_an_id_is_remembered_in_the_shared_slot() {
        let mut dirs = PickerDirs::default();
        dirs.remember("", Path::new("/home/me/Documents"));
        let back = PickerDirs::parse(&dirs.to_text());
        assert_eq!(back.get(""), Some(Path::new("/home/me/Documents")));
    }

    #[test]
    fn remembering_again_replaces_and_refreshes() {
        let mut dirs = PickerDirs::default();
        dirs.remember("a", Path::new("/one"));
        dirs.remember("b", Path::new("/two"));
        dirs.remember("a", Path::new("/three"));
        assert_eq!(dirs.get("a"), Some(Path::new("/three")));
        assert_eq!(dirs.to_text(), "b\t/two\na\t/three\n");
    }

    #[test]
    fn the_least_recently_used_application_is_forgotten_first() {
        let mut dirs = PickerDirs::default();
        for i in 0..=CAPACITY {
            dirs.remember(&format!("app{i}"), Path::new("/tmp"));
        }
        assert_eq!(dirs.get("app0"), None);
        assert!(dirs.get("app1").is_some());
        assert!(dirs.get(&format!("app{CAPACITY}")).is_some());
    }

    #[test]
    fn a_path_the_format_cannot_carry_is_not_recorded() {
        let mut dirs = PickerDirs::default();
        dirs.remember("a", Path::new("relative/dir"));
        dirs.remember("b", Path::new("/line\nbreak"));
        assert_eq!(dirs, PickerDirs::default());
    }

    #[test]
    fn a_broken_line_costs_only_itself() {
        let back = PickerDirs::parse("garbage\nx\tnot-absolute\napp\t/ok\n");
        assert_eq!(back.get("app"), Some(Path::new("/ok")));
        assert_eq!(back.get("x"), None);
    }
}
