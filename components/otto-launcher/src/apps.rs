//! Installed applications, from their `.desktop` files.
//!
//! The launcher opens onto the last few applications started from it rather
//! than onto all of them, so the list also remembers what was launched. That
//! history is the only state the launcher keeps between runs.

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use freedesktop_desktop_entry::{default_paths, DesktopEntry, Iter};

use crate::source::{Item, Origin, Source};

/// How many recently launched applications the resting list shows.
const RECENT_SHOWN: usize = 3;

/// How many are remembered. More than are shown, so a launch that scrolls off
/// the visible three is still there when the ones above it are uninstalled.
const RECENT_KEPT: usize = 20;

struct Entry {
    /// The desktop file's stem, which is what the history records.
    id: String,
    name: String,
    comment: Option<String>,
    icon: Option<String>,
    exec: String,
    terminal: bool,
    /// `Path=` — the directory the entry asks to be started in.
    working_dir: Option<String>,
    keywords: Vec<String>,
}

pub struct Apps {
    index: usize,
    entries: Vec<Entry>,
}

impl Apps {
    /// Scan the XDG data directories. Entries that are hidden, that say they
    /// do not belong in a menu, or that have nothing to run are left out —
    /// they are all things that would only ever be picked by mistake.
    pub fn load(index: usize) -> Self {
        let locales = locales();
        let locale_refs: Vec<&str> = locales.iter().map(String::as_str).collect();

        let mut entries: Vec<Entry> = Vec::new();
        let mut seen: Vec<String> = Vec::new();

        for path in Iter::new(default_paths()) {
            // Earlier XDG directories win: a user's ~/.local/share override
            // must not appear twice alongside the system copy it replaces.
            let id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_string();
            if id.is_empty() || seen.contains(&id) {
                continue;
            }

            let Ok(entry) = DesktopEntry::from_path(path, Some(&locale_refs)) else {
                continue;
            };
            if entry.no_display() || entry.type_() != Some("Application") {
                continue;
            }
            if entry.desktop_entry("Hidden").is_some_and(|v| v == "true") {
                continue;
            }
            let Some(exec) = entry.exec() else { continue };
            let Some(name) = entry.name(&locale_refs) else {
                continue;
            };

            seen.push(id.clone());
            entries.push(Entry {
                id,
                name: name.to_string(),
                comment: entry.comment(&locale_refs).map(|c| c.to_string()),
                icon: entry.icon().map(|i| i.to_string()),
                exec: exec.to_string(),
                terminal: entry.terminal(),
                working_dir: entry.path().map(|p| p.to_string()),
                keywords: entry
                    .keywords(&locale_refs)
                    .map(|words| words.iter().map(|w| w.to_string()).collect())
                    .unwrap_or_default(),
            });
        }

        entries.sort_by_key(|a| a.name.to_lowercase());
        Self { index, entries }
    }
}

impl Source for Apps {
    fn label(&self) -> &'static str {
        "App"
    }

    fn items(&mut self) -> Vec<Item> {
        self.entries
            .iter()
            .enumerate()
            .map(|(index, entry)| Item {
                title: entry.name.clone(),
                subtitle: entry.comment.clone(),
                icon: entry.icon.clone(),
                search_terms: entry
                    .keywords
                    .iter()
                    .cloned()
                    .chain(binary_name(&entry.exec))
                    .collect(),
                origin: Origin {
                    source: self.index,
                    index,
                },
            })
            .collect()
    }

    /// The last few applications started from the launcher, most recent first.
    ///
    /// Empty until something has been launched — a launcher that has never been
    /// used has nothing to say about what you want, and says nothing.
    fn resting(&mut self) -> Vec<Item> {
        let items = self.items();
        history()
            .iter()
            .filter_map(|id| {
                let index = self.entries.iter().position(|entry| &entry.id == id)?;
                items.get(index).cloned()
            })
            .take(RECENT_SHOWN)
            .collect()
    }

    fn activate(&mut self, index: usize) -> Result<(), String> {
        let entry = self.entries.get(index).ok_or("no such app")?;
        spawn(entry)?;
        // Only once it has actually started: an application that could not be
        // launched should not be what the launcher offers next time.
        remember(&entry.id);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Launch history
// ---------------------------------------------------------------------------

/// Where the history lives — state rather than config: it is written by the
/// program, and losing it costs nothing.
fn history_path() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
        })?;
    Some(dir.join("otto").join("launcher-history"))
}

/// Desktop file ids, most recently launched first.
fn history() -> Vec<String> {
    let Some(path) = history_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .take(RECENT_KEPT)
        .collect()
}

/// `id` moved to the front, keeping the rest in order and dropping the oldest
/// past [`RECENT_KEPT`].
fn promote(id: &str, existing: Vec<String>) -> Vec<String> {
    std::iter::once(id.to_string())
        .chain(existing.into_iter().filter(|other| other != id))
        .take(RECENT_KEPT)
        .collect()
}

/// Move `id` to the front of the history.
///
/// Written whole and moved into place, so a launcher killed mid-write leaves
/// the previous history rather than half of a new one.
fn remember(id: &str) {
    let Some(path) = history_path() else {
        return;
    };
    let ids = promote(id, history());

    let Some(dir) = path.parent() else { return };
    if let Err(err) = std::fs::create_dir_all(dir) {
        tracing::warn!(%err, "could not create the state directory");
        return;
    }
    let temporary = path.with_extension("tmp");
    let written = std::fs::File::create(&temporary).and_then(|mut file| {
        file.write_all(ids.join("\n").as_bytes())?;
        file.write_all(b"\n")
    });
    match written {
        Ok(()) => {
            if let Err(err) = std::fs::rename(&temporary, &path) {
                tracing::warn!(%err, "could not save the launch history");
            }
        }
        Err(err) => tracing::warn!(%err, "could not write the launch history"),
    }
}

/// Start the app, detached.
///
/// The launcher exits immediately afterwards, so the child is put in its own
/// process group: a session that reaps the launcher must not take the app with
/// it, and a terminal app must not end up sharing our controlling terminal.
fn spawn(entry: &Entry) -> Result<(), String> {
    let line = strip_field_codes(&entry.exec);
    let mut parts = shell_words::split(&line).map_err(|err| err.to_string())?;
    if parts.is_empty() {
        return Err("nothing to run".to_string());
    }

    if entry.terminal {
        let mut wrapped = terminal_command();
        wrapped.append(&mut parts);
        parts = wrapped;
    }

    let mut command = Command::new(&parts[0]);
    command.args(&parts[1..]);
    if let Some(dir) = &entry.working_dir {
        command.current_dir(dir);
    }
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    command
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("could not start {}: {err}", parts[0]))
}

/// `Terminal=true` entries are a command, not a command line: something has to
/// supply the terminal to run them in.
fn terminal_command() -> Vec<String> {
    let terminal = std::env::var("TERMINAL").unwrap_or_default();
    let terminal = if terminal.is_empty() {
        ["ghostty", "alacritty", "foot", "kitty", "xterm"]
            .into_iter()
            .find(|candidate| which(candidate))
            .unwrap_or("xterm")
            .to_string()
    } else {
        terminal
    };
    vec![terminal, "-e".to_string()]
}

fn which(program: &str) -> bool {
    std::env::var("PATH")
        .is_ok_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

/// Drop the `%f`/`%U`/… placeholders. The launcher opens applications with no
/// arguments, so every field code expands to nothing — except `%%`, which is a
/// literal percent sign.
fn strip_field_codes(exec: &str) -> String {
    let mut out = String::with_capacity(exec.len());
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some(_) => {}
            None => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn binary_name(exec: &str) -> Option<String> {
    let line = strip_field_codes(exec);
    let first = line.split_whitespace().next()?;
    let name = first.rsplit('/').next()?;
    // `env FOO=bar app` and friends would otherwise contribute a search term
    // that matches half the menu.
    if matches!(name, "env" | "sh" | "bash" | "flatpak") {
        return None;
    }
    Some(name.to_string())
}

fn locales() -> Vec<String> {
    let mut locales = Vec::new();
    for var in ["LC_MESSAGES", "LC_ALL", "LANG"] {
        let Ok(value) = std::env::var(var) else {
            continue;
        };
        if value.is_empty() || value == "C" || value == "POSIX" {
            continue;
        }
        let base = value.split('.').next().unwrap_or(&value).to_string();
        let language = base.split('_').next().unwrap_or(&base).to_string();
        for candidate in [base, language] {
            if !locales.contains(&candidate) {
                locales.push(candidate);
            }
        }
    }
    if locales.is_empty() {
        locales.push("en".to_string());
    }
    locales
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launching_moves_an_app_to_the_front_without_duplicating_it() {
        let existing = vec!["b".to_string(), "a".to_string()];
        assert_eq!(promote("a", existing.clone()), ["a", "b"]);
        assert_eq!(promote("c", existing), ["c", "b", "a"]);
    }

    #[test]
    fn the_history_forgets_the_oldest_past_its_limit() {
        let existing: Vec<String> = (0..RECENT_KEPT).map(|n| n.to_string()).collect();
        let promoted = promote("new", existing);
        assert_eq!(promoted.len(), RECENT_KEPT);
        assert_eq!(promoted[0], "new");
        assert!(!promoted.contains(&(RECENT_KEPT - 1).to_string()));
    }

    /// Reading and writing the file, end to end, in a directory of its own.
    /// One test rather than several: they would be sharing an environment
    /// variable.
    #[test]
    fn the_history_survives_a_round_trip_through_the_state_file() {
        let dir = std::env::temp_dir().join(format!("otto-launcher-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("XDG_STATE_HOME", &dir);

        assert!(history().is_empty(), "nothing has been launched yet");
        remember("code");
        remember("ghostty");
        remember("code");
        assert_eq!(history(), ["code", "ghostty"]);

        let _ = std::fs::remove_dir_all(&dir);
        std::env::remove_var("XDG_STATE_HOME");
    }

    #[test]
    fn field_codes_are_dropped_and_double_percent_survives() {
        assert_eq!(strip_field_codes("gimp %U"), "gimp");
        assert_eq!(strip_field_codes("app -f %f --x %i"), "app -f --x");
        assert_eq!(strip_field_codes("printf 100%%"), "printf 100%");
    }

    #[test]
    fn the_binary_name_becomes_a_search_term() {
        assert_eq!(
            binary_name("/usr/bin/gimp-2.10 %U").as_deref(),
            Some("gimp-2.10")
        );
    }

    #[test]
    fn wrappers_do_not_become_search_terms() {
        assert_eq!(binary_name("env GDK_BACKEND=x11 inkscape"), None);
        assert_eq!(binary_name("flatpak run org.gimp.GIMP"), None);
    }
}
