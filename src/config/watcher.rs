//! Detects edits to the config files backing [`crate::config::Config`].
//!
//! The watcher stats the config paths and compares (mtime, size) — no inotify,
//! no extra dependency. Editors that write in place, editors that
//! rename-replace, and a file that only appears later all look the same to it,
//! and a poll of a handful of paths is far cheaper than the reload it guards.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// How often the compositor re-stats the config files.
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Identity of a config file: missing, or (mtime, size).
type Stamp = Option<(SystemTime, u64)>;

#[derive(Debug, Default)]
pub struct ConfigWatcher {
    stamps: BTreeMap<PathBuf, Stamp>,
}

impl ConfigWatcher {
    /// Start watching from the current state on disk, so the first poll only
    /// reports edits made after the compositor read its config.
    pub fn new() -> Self {
        Self { stamps: snapshot() }
    }

    /// Whether any watched file changed since the previous poll.
    pub fn poll(&mut self) -> bool {
        let next = snapshot();
        if next == self.stamps {
            return false;
        }
        self.stamps = next;
        true
    }
}

fn snapshot() -> BTreeMap<PathBuf, Stamp> {
    super::watched_paths()
        .into_iter()
        .map(|path| {
            let stamp = std::fs::metadata(&path).ok().and_then(|meta| {
                let modified = meta.modified().ok()?;
                Some((modified, meta.len()))
            });
            (path, stamp)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// Point the user config at a temp dir so the watcher watches a file the
    /// test owns. `XDG_CONFIG_HOME` is process-global, hence `#[serial]`.
    fn with_config_home<R>(dir: &std::path::Path, f: impl FnOnce() -> R) -> R {
        let previous = std::env::var("XDG_CONFIG_HOME").ok();
        std::env::set_var("XDG_CONFIG_HOME", dir);
        let result = f();
        match previous {
            Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        result
    }

    #[test]
    #[serial]
    fn quiet_when_nothing_changes() {
        let dir = tempfile::tempdir().unwrap();
        with_config_home(dir.path(), || {
            let mut watcher = ConfigWatcher::new();
            assert!(!watcher.poll());
            assert!(!watcher.poll());
        });
    }

    #[test]
    #[serial]
    fn reports_a_created_config() {
        let dir = tempfile::tempdir().unwrap();
        with_config_home(dir.path(), || {
            let mut watcher = ConfigWatcher::new();
            assert!(!watcher.poll());

            let config_dir = dir.path().join("otto");
            std::fs::create_dir_all(&config_dir).unwrap();
            std::fs::write(config_dir.join("config.toml"), "screen_scale = 1.0\n").unwrap();

            assert!(watcher.poll(), "a new config file is a change");
            assert!(!watcher.poll(), "and only reported once");
        });
    }

    #[test]
    #[serial]
    fn reports_an_edit() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path().join("otto");
        std::fs::create_dir_all(&config_dir).unwrap();
        let config = config_dir.join("config.toml");
        std::fs::write(&config, "screen_scale = 1.0\n").unwrap();

        with_config_home(dir.path(), || {
            let mut watcher = ConfigWatcher::new();
            assert!(!watcher.poll());

            // Rewrite with a different length: filesystems with coarse mtime
            // granularity would otherwise hide an edit made in the same tick.
            std::fs::write(&config, "screen_scale = 1.25\n").unwrap();

            assert!(watcher.poll(), "an edited config file is a change");
            assert!(!watcher.poll());
        });
    }
}
