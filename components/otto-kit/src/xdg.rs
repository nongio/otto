//! The XDG base directories, resolved one way.
//!
//! Each of these used to be worked out where it was needed, and no two
//! agreed: one checked that the variable named an absolute path, another that
//! it was not empty, and the rest checked nothing. An empty `XDG_DATA_HOME`
//! was honoured in one place and ignored in another, which is the sort of
//! difference that only shows up on someone else's machine. The rules live
//! here instead (and in `otto-agents`' copy, which does not link the toolkit).
//!
//! A variable is used when it names an absolute path — the spec says relative
//! values are invalid and must be ignored, and an empty value is how a shell
//! spells "unset" — and the usual place under the home folder is the fallback.

use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

/// The home folder, as `$HOME` gives it.
pub fn home() -> Option<PathBuf> {
    absolute(std::env::var_os("HOME")?)
}

/// `$XDG_CONFIG_HOME`, or `~/.config`.
pub fn config_home() -> Option<PathBuf> {
    base("XDG_CONFIG_HOME", ".config")
}

/// `$XDG_DATA_HOME`, or `~/.local/share`.
pub fn data_home() -> Option<PathBuf> {
    base("XDG_DATA_HOME", ".local/share")
}

/// `$XDG_STATE_HOME`, or `~/.local/state`.
pub fn state_home() -> Option<PathBuf> {
    base("XDG_STATE_HOME", ".local/state")
}

/// `$XDG_CACHE_HOME`, or `~/.cache`.
pub fn cache_home() -> Option<PathBuf> {
    base("XDG_CACHE_HOME", ".cache")
}

/// `$XDG_RUNTIME_DIR`, or `/run/user/<uid>` when that is a directory.
///
/// No fallback under the home folder: the runtime directory is the session's,
/// made by the login machinery, and inventing one would mean a socket
/// somewhere that is neither cleaned up nor private.
pub fn runtime_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .and_then(absolute)
        .filter(|dir| dir.is_dir())
        .or_else(|| {
            let uid = std::fs::metadata("/proc/self").ok()?.uid();
            Some(PathBuf::from(format!("/run/user/{uid}"))).filter(|dir| dir.is_dir())
        })
}

/// `$XDG_DATA_DIRS`, highest priority first, or `/usr/local/share` and
/// `/usr/share`.
pub fn data_dirs() -> Vec<PathBuf> {
    dirs(
        std::env::var("XDG_DATA_DIRS").ok(),
        "/usr/local/share:/usr/share",
    )
}

/// `$XDG_CONFIG_DIRS`, highest priority first, or `/etc/xdg`.
pub fn config_dirs() -> Vec<PathBuf> {
    dirs(std::env::var("XDG_CONFIG_DIRS").ok(), "/etc/xdg")
}

/// Where a user's Otto config file called `name` lives:
/// `$XDG_CONFIG_HOME/otto/<name>`.
pub fn otto_config_file(name: &str) -> Option<PathBuf> {
    Some(config_home()?.join("otto").join(name))
}

/// Every place an Otto config file called `name` is read from, in the order
/// they apply: the system's `/etc/otto/<name>`, then the user's, which
/// overrides it.
pub fn otto_config_paths(name: &str) -> Vec<PathBuf> {
    std::iter::once(PathBuf::from("/etc/otto").join(name))
        .chain(otto_config_file(name))
        .collect()
}

/// `var` when it names an absolute path, else `~/<fallback>`.
fn base(var: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .and_then(absolute)
        .or_else(|| home().map(|home| home.join(fallback)))
}

fn absolute(value: impl Into<PathBuf>) -> Option<PathBuf> {
    Some(value.into()).filter(|path| path.is_absolute())
}

/// A colon-separated list of directories, relative entries dropped, or
/// `fallback` when the variable is unset or empty.
fn dirs(value: Option<String>, fallback: &str) -> Vec<PathBuf> {
    let value = value.filter(|v| !v.is_empty());
    value
        .as_deref()
        .unwrap_or(fallback)
        .split(':')
        .filter_map(absolute)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rules, checked against the resolvers' parts so no environment is
    /// touched: tests share a process, and its variables with it.
    #[test]
    fn a_relative_or_empty_value_is_not_a_base_directory() {
        for value in ["", "relative/dir", "~/spelled-out"] {
            assert_eq!(absolute(value), None, "{value} would be a base directory");
        }
        assert_eq!(absolute("/srv/state"), Some(PathBuf::from("/srv/state")));
    }

    #[test]
    fn a_directory_list_keeps_its_order_and_drops_what_is_not_absolute() {
        assert_eq!(
            dirs(Some("/opt/share::rel:/usr/share".into()), "/x"),
            [PathBuf::from("/opt/share"), PathBuf::from("/usr/share")]
        );
        assert_eq!(
            dirs(Some(String::new()), "/a:/b"),
            [PathBuf::from("/a"), PathBuf::from("/b")]
        );
        assert_eq!(dirs(None, "/etc/xdg"), [PathBuf::from("/etc/xdg")]);
    }

    #[test]
    fn the_system_config_comes_before_the_users() {
        let paths = otto_config_paths("otto-bar.toml");
        assert_eq!(paths[0], PathBuf::from("/etc/otto/otto-bar.toml"));
        if let Some(user) = paths.get(1) {
            assert!(user.ends_with("otto/otto-bar.toml"));
        }
    }
}
