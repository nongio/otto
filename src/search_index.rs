//! Pushing the `[search]` configuration to LocalSearch, the file indexer.
//!
//! LocalSearch reads its settings from the desktop's settings store under
//! `org.freedesktop.Tracker3.Miner.Files`. Otto's configuration is the source
//! of truth for the three it exposes; this module writes them there with the
//! `gsettings` command, so the compositor links no GNOME library. The tool
//! ships with GLib, which LocalSearch depends on, so wherever there is an
//! indexer to configure there is a `gsettings` to configure it with.
//!
//! Only keys set in some configuration layer are written, and only when
//! LocalSearch holds something different: an untouched configuration never
//! writes to the settings store, and one that already agrees costs a read.
//! Everything runs on a thread of its own, since each step spawns a process.

// Rust guideline compliant 2026-02-21

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::config::{Config, SearchConfig};

/// LocalSearch's settings schema.
const SCHEMA: &str = "org.freedesktop.Tracker3.Miner.Files";

/// The folder whose presence marks a code repository, as LocalSearch's
/// `ignored-directories-with-content` names it.
const REPO_MARKER: &str = ".git";

/// The configuration keys this module pushes, with the LocalSearch key each
/// one lands in.
const FOLDERS: (&str, &str) = ("search.folders", "index-recursive-directories");
const SKIP_REPOS: (&str, &str) = (
    "search.skip_code_repositories",
    "ignored-directories-with-content",
);
const REMOVABLE: (&str, &str) = ("search.index_removable_drives", "index-removable-devices");

/// Serialises pushes, so two changes in quick succession cannot interleave
/// their reads and writes; each push reads the configuration afresh.
static PUSHING: Mutex<()> = Mutex::new(());

/// Set once LocalSearch's schema has been found missing, so that is said once.
static MISSING_LOGGED: AtomicBool = AtomicBool::new(false);

/// Push the `[search]` configuration to LocalSearch, in the background.
///
/// `changed` is a key just set through `org.otto.Settings`: it is pushed even
/// if the file that will hold it has not been written yet.
pub fn sync(changed: Option<&'static str>) {
    let spawned = std::thread::Builder::new()
        .name("search-index-sync".into())
        .spawn(move || {
            let _guard = PUSHING.lock().unwrap_or_else(|p| p.into_inner());
            let config = Config::with(|c| c.search.clone());
            let explicit = |key: &str| changed == Some(key) || crate::config::set_in_any_layer(key);
            let home = std::env::var_os("HOME").unwrap_or_default();
            push(&config, explicit, Path::new(&home));
        });
    if let Err(err) = spawned {
        tracing::warn!("Could not start the search index sync: {err}");
    }
}

/// Write every explicitly set key LocalSearch disagrees with.
fn push(config: &SearchConfig, explicit: impl Fn(&str) -> bool, home: &Path) {
    if explicit(FOLDERS.0) {
        let wanted = localsearch_folders(&config.folders, home);
        update(FOLDERS.1, |current| {
            (parse_list(current)? != wanted).then(|| format_list(&wanted))
        });
    }
    if explicit(SKIP_REPOS.0) {
        update(SKIP_REPOS.1, |current| {
            let current = parse_list(current)?;
            let wanted = with_repo_marker(&current, config.skip_code_repositories);
            (current != wanted).then(|| format_list(&wanted))
        });
    }
    if explicit(REMOVABLE.0) {
        let wanted = config.index_removable_drives.to_string();
        update(REMOVABLE.1, |current| {
            (current != wanted).then(|| wanted.clone())
        });
    }
}

/// Read `key`, and write what `edit` makes of it when that is `Some`.
fn update(key: &str, edit: impl FnOnce(&str) -> Option<String>) {
    let Some(current) = gsettings(&["get", SCHEMA, key]) else {
        if !MISSING_LOGGED.swap(true, Ordering::Relaxed) {
            tracing::debug!("LocalSearch's settings are not installed; [search] is not applied");
        }
        return;
    };
    let Some(text) = edit(&current) else {
        return;
    };
    if gsettings(&["set", SCHEMA, key, &text]).is_none() {
        tracing::warn!("Could not set LocalSearch's {key} to {text}");
    }
}

/// Run `gsettings`, returning what it printed when it succeeded.
fn gsettings(args: &[&str]) -> Option<String> {
    let output = Command::new("gsettings").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The configured folders as LocalSearch spells them: `~` as `$HOME`, a path
/// under it made absolute, anything else as written.
fn localsearch_folders(folders: &[String], home: &Path) -> Vec<String> {
    folders
        .iter()
        .map(|folder| match folder.as_str() {
            "~" => "$HOME".to_string(),
            other => match other.strip_prefix("~/") {
                Some(rest) => home.join(rest).display().to_string(),
                None => other.to_string(),
            },
        })
        .collect()
}

/// Parse a string list as `gsettings get` prints it: `['a', 'b']`, or
/// `@as []` when empty. `None` for anything else.
///
/// A string is quoted with `'`, or with `"` when it holds a `'`; a backslash
/// escapes the next character, with `\n`, `\t` and `\r` standing for control
/// characters.
fn parse_list(text: &str) -> Option<Vec<String>> {
    let text = text.trim();
    let text = text.strip_prefix("@as").unwrap_or(text).trim_start();
    let inner = text.strip_prefix('[')?.strip_suffix(']')?;
    let mut items = Vec::new();
    let mut chars = inner.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let Some(quote) = chars.next() else {
            return Some(items);
        };
        if quote != '\'' && quote != '"' {
            return None;
        }
        let mut item = String::new();
        loop {
            match chars.next()? {
                '\\' => item.push(match chars.next()? {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    other => other,
                }),
                c if c == quote => break,
                c => item.push(c),
            }
        }
        items.push(item);
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        match chars.next() {
            Some(',') => {}
            None => return Some(items),
            Some(_) => return None,
        }
    }
}

/// Print a string list as GVariant text, which `gsettings set` parses.
///
/// Each string is single-quoted with `\` and `'` escaped, and control
/// characters written as escapes; everything else, spaces and non-ASCII
/// included, is written as it is. The text goes to `gsettings` as one
/// argument, with no shell in between to quote for.
fn format_list(items: &[String]) -> String {
    if items.is_empty() {
        return "@as []".to_string();
    }
    let quoted: Vec<String> = items
        .iter()
        .map(|item| {
            let mut text = String::with_capacity(item.len() + 2);
            text.push('\'');
            for c in item.chars() {
                match c {
                    '\\' => text.push_str("\\\\"),
                    '\'' => text.push_str("\\'"),
                    '\n' => text.push_str("\\n"),
                    '\t' => text.push_str("\\t"),
                    '\r' => text.push_str("\\r"),
                    c => text.push(c),
                }
            }
            text.push('\'');
            text
        })
        .collect();
    format!("[{}]", quoted.join(", "))
}

/// `list` with [`REPO_MARKER`] in it or not, everything else kept in order.
fn with_repo_marker(list: &[String], skip: bool) -> Vec<String> {
    let mut list: Vec<String> = list
        .iter()
        .filter(|name| name.as_str() != REPO_MARKER)
        .cloned()
        .collect();
    if skip {
        list.push(REPO_MARKER.to_string());
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_the_lists_gsettings_prints() {
        assert_eq!(parse_list("['$HOME']"), Some(strings(&["$HOME"])));
        assert_eq!(
            parse_list("['.trackerignore', '.git', '.hg', '.nomedia']"),
            Some(strings(&[".trackerignore", ".git", ".hg", ".nomedia"]))
        );
        assert_eq!(parse_list("@as []"), Some(Vec::new()));
        assert_eq!(parse_list("[]"), Some(Vec::new()));
        assert_eq!(
            parse_list(r#"["it's", 'a\\b', 'c\'d']"#),
            Some(strings(&["it's", "a\\b", "c'd"]))
        );
        assert_eq!(parse_list("['unterminated]"), None);
        assert_eq!(parse_list("true"), None);
        assert_eq!(parse_list("['a' 'b']"), None);
    }

    #[test]
    fn printed_lists_read_back_the_same() {
        for list in [
            strings(&[]),
            strings(&["$HOME", "&DOWNLOAD"]),
            strings(&[
                "/home/ada/it's mine",
                "/home/ada/back\\slash",
                "/home/ada/\"quoted\"",
                "/home/ada/Téléchargements",
                "/home/ada/写真",
                "/home/ada/odd\nname",
            ]),
        ] {
            assert_eq!(parse_list(&format_list(&list)), Some(list.clone()));
        }
    }

    #[test]
    fn printed_lists_are_gvariant_text() {
        assert_eq!(format_list(&[]), "@as []");
        assert_eq!(format_list(&strings(&["a", "b"])), "['a', 'b']");
        assert_eq!(
            format_list(&strings(&["/home/ada/My Files"])),
            "['/home/ada/My Files']"
        );
        assert_eq!(
            format_list(&strings(&["/home/ada/it's"])),
            r"['/home/ada/it\'s']"
        );
        assert_eq!(format_list(&strings(&[r"a\b"])), r"['a\\b']");
        assert_eq!(format_list(&strings(&["Ürün"])), "['Ürün']");
    }

    #[test]
    fn the_repo_switch_touches_only_git() {
        let list = strings(&[".trackerignore", ".git", ".nomedia"]);
        assert_eq!(
            with_repo_marker(&list, false),
            strings(&[".trackerignore", ".nomedia"])
        );
        assert_eq!(
            with_repo_marker(&strings(&[".trackerignore", ".nomedia"]), true),
            strings(&[".trackerignore", ".nomedia", ".git"])
        );
        assert_eq!(with_repo_marker(&list, true).len(), 3);
        assert_eq!(with_repo_marker(&[], true), strings(&[".git"]));
    }

    #[test]
    fn folders_are_spelled_the_way_localsearch_reads_them() {
        let home = Path::new("/home/ada");
        assert_eq!(
            localsearch_folders(
                &strings(&["~", "~/src", "/mnt/data", "$HOME", "&DESKTOP"]),
                home
            ),
            strings(&["$HOME", "/home/ada/src", "/mnt/data", "$HOME", "&DESKTOP"])
        );
    }

    #[test]
    fn the_defaults_are_localsearch_s_own() {
        let config = SearchConfig::default();
        assert_eq!(config.folders, strings(&["~"]));
        assert!(config.skip_code_repositories);
        assert!(!config.index_removable_drives);
    }
}
