//! The XDG base directories, resolved one way.
//!
//! Each of these used to be worked out where it was needed, and no two
//! agreed: one checked that the variable named an absolute path, another that
//! it was not empty, and the rest checked nothing. An empty `XDG_DATA_HOME`
//! was honoured in one place and ignored in another, which is the sort of
//! difference that only shows up on someone else's machine. The rules live
//! here instead, for the toolkit (`otto_kit::xdg`) and for every program that
//! does not link it.
//!
//! A variable is used when it names an absolute path — the spec says relative
//! values are invalid and must be ignored, and an empty value is how a shell
//! spells "unset" — and the usual place under the home folder is the fallback.
//!
//! Two neighbours of the base directories live here too: the user's named
//! folders from `user-dirs.dirs` ([`user_dirs`]), and the `~/…` spelling of a
//! path under the home folder ([`tilde`]).

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

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

/// The user's named folders, as `$XDG_CONFIG_HOME/user-dirs.dirs` lists
/// them: `("XDG_DESKTOP_DIR", "/home/me/Scrivania")` and so on, in the
/// file's order. Empty when there is no file or no home folder.
///
/// See [`parse_user_dirs`] for what counts as an entry.
pub fn user_dirs() -> Vec<(String, PathBuf)> {
    let (Some(config), Some(home)) = (config_home(), home()) else {
        return Vec::new();
    };
    match std::fs::read_to_string(config.join("user-dirs.dirs")) {
        Ok(text) => parse_user_dirs(&text, &home),
        Err(_) => Vec::new(),
    }
}

/// The folder `user-dirs.dirs` names for `key` (`XDG_PICTURES_DIR`, say), if
/// it names one.
pub fn user_dir(key: &str) -> Option<PathBuf> {
    user_dirs()
        .into_iter()
        .find_map(|(k, path)| (k == key).then_some(path))
}

/// The entries of a `user-dirs.dirs` file's text, `$HOME` expanded to `home`.
///
/// The format is xdg-user-dirs': one `XDG_<NAME>_DIR="<path>"` per line,
/// where the path is `$HOME`, `$HOME/<rest>` or absolute. Blank lines and `#`
/// comments are skipped, and so is a line whose path ends up relative — the
/// spec calls it invalid, and a relative folder would mean something
/// different in every working directory.
pub fn parse_user_dirs(text: &str, home: &Path) -> Vec<(String, PathBuf)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            let value = value.trim().trim_matches('"');
            let path = match value.strip_prefix("$HOME") {
                Some("") => home.to_path_buf(),
                Some(rest) => home.join(rest.strip_prefix('/')?),
                None => PathBuf::from(value),
            };
            path.is_absolute().then(|| (key.trim().to_owned(), path))
        })
        .collect()
}

/// `path` written for a person, with the home folder as `~`:
/// `~/Documents` rather than `/home/me/Documents`, and `~` for the home
/// folder itself. A path elsewhere is written out whole.
///
/// The one spelling, so a folder reads the same in a dialog, a settings pane
/// and a session list.
pub fn tilde(path: &Path) -> String {
    tilde_in(path, home().as_deref())
}

/// [`tilde`] against a given home folder — a test's, or another user's.
/// `None` writes every path out whole.
pub fn tilde_in(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
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
    fn user_dirs_expand_home_and_keep_the_files_order() {
        let home = Path::new("/home/me");
        let text = "# This file is written by xdg-user-dirs-update\n\
                    \n\
                    XDG_DESKTOP_DIR=\"$HOME/Scrivania\"\n\
                    XDG_PUBLICSHARE_DIR=\"$HOME\"\n\
                    XDG_MUSIC_DIR=\"/srv/music\"\n\
                    \x20 XDG_PICTURES_DIR = \"$HOME/Immagini\" \n";
        assert_eq!(
            parse_user_dirs(text, home),
            [
                (
                    "XDG_DESKTOP_DIR".to_owned(),
                    PathBuf::from("/home/me/Scrivania")
                ),
                ("XDG_PUBLICSHARE_DIR".to_owned(), PathBuf::from("/home/me")),
                ("XDG_MUSIC_DIR".to_owned(), PathBuf::from("/srv/music")),
                (
                    "XDG_PICTURES_DIR".to_owned(),
                    PathBuf::from("/home/me/Immagini")
                ),
            ]
        );
    }

    #[test]
    fn a_relative_or_malformed_user_dir_is_skipped() {
        let home = Path::new("/home/me");
        let text = "XDG_DESKTOP_DIR=\"Desktop\"\n\
                    XDG_MUSIC_DIR=\"$HOMEMusic\"\n\
                    no equals sign here\n\
                    XDG_VIDEOS_DIR=\"\"\n";
        assert_eq!(parse_user_dirs(text, home), []);
        assert_eq!(parse_user_dirs("", home), []);
    }

    #[test]
    fn a_path_under_home_is_written_from_a_tilde() {
        let home = Some(Path::new("/home/me"));
        assert_eq!(tilde_in(Path::new("/home/me/Desktop"), home), "~/Desktop");
        assert_eq!(tilde_in(Path::new("/home/me/a/b"), home), "~/a/b");
        assert_eq!(tilde_in(Path::new("/home/me"), home), "~");
        assert_eq!(tilde_in(Path::new("/home/me/"), home), "~");
        // A name that only starts like the home folder is not inside it.
        assert_eq!(tilde_in(Path::new("/home/meg/x"), home), "/home/meg/x");
        assert_eq!(tilde_in(Path::new("/srv/desk"), home), "/srv/desk");
        assert_eq!(tilde_in(Path::new("/home/me/x"), None), "/home/me/x");
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
