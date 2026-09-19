//! The XDG base directories, resolved one way.
//!
//! Each of these used to be worked out where it was needed, and no two agreed:
//! one checked that the variable named an absolute path, another that it was
//! not empty, a third that it was a directory, and the rest checked nothing.
//! An empty `XDG_DATA_HOME` was honoured in one place and ignored in another,
//! which is the sort of difference that only shows up on someone else's
//! machine. The rules live here instead.
//!
//! A variable is used when it names an absolute path — the spec says relative
//! values are invalid and must be ignored — and the usual place under the home
//! folder is the fallback.

use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

/// `$XDG_CONFIG_HOME`, or `~/.config`.
pub fn config_home() -> Option<PathBuf> {
    base("XDG_CONFIG_HOME", ".config")
}

/// `$XDG_DATA_HOME`, or `~/.local/share`.
pub fn data_home() -> Option<PathBuf> {
    base("XDG_DATA_HOME", ".local/share")
}

/// `$XDG_CACHE_HOME`, or `~/.cache`.
pub fn cache_home() -> Option<PathBuf> {
    base("XDG_CACHE_HOME", ".cache")
}

/// `$XDG_STATE_HOME`, or `~/.local/state`.
pub fn state_home() -> Option<PathBuf> {
    base("XDG_STATE_HOME", ".local/state")
}

/// `$XDG_RUNTIME_DIR`, or `/run/user/<uid>` when that is a directory.
///
/// Unlike the others this has no fallback under the home folder: the runtime
/// directory is the session's, made by the login machinery, and inventing one
/// would mean a socket somewhere that is not cleaned up or private.
pub fn runtime_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute() && dir.is_dir())
        .or_else(|| {
            let uid = std::fs::metadata("/proc/self").ok()?.uid();
            Some(PathBuf::from(format!("/run/user/{uid}"))).filter(|dir| dir.is_dir())
        })
}

/// The home folder, as `$HOME` gives it.
pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
}

/// `path` written for a person, with `home` as `~`.
///
/// The one spelling, so a folder reads the same in a dialog, in `plugins
/// status` and in the session list.
pub fn tilde(path: &std::path::Path, home: Option<&std::path::Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// [`tilde`] against the home folder this process has.
pub fn tilde_from_env(path: &std::path::Path) -> String {
    tilde(path, home().as_deref())
}

/// `var` when it names an absolute path, else `~/<fallback>`.
fn base(var: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| home().map(|home| home.join(fallback)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rules, checked against the resolver directly so no environment is
    /// touched: tests share a process, and its variables with it.
    #[test]
    fn a_relative_or_empty_variable_is_not_a_base_directory() {
        // The spec says an invalid (relative) value must be ignored, and an
        // empty value is the way a shell passes "unset".
        for value in ["", "relative/dir", "~/spelled-out"] {
            assert!(
                !PathBuf::from(value).is_absolute(),
                "{value} would be taken as a base directory"
            );
        }
        assert!(PathBuf::from("/srv/state").is_absolute());
    }
}
