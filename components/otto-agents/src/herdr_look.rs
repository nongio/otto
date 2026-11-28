//! Otto's look for herdr-app, the terminal workspace for agents.
//!
//! The package installs a herdr layout, two themes and a sessions mod to
//! `/usr/share/otto/herdr/` ([`SYSTEM_DIR`]), laid out as herdr's own config
//! folder is. herdr reads only its config folder (`$HERDR_APP_HOME`, or
//! `~/.config/herdr-app`), so `otto-agents plugins install` copies them there
//! ([`install`]) once herdr has made that folder. The layout is offered in
//! herdr's list, not selected: picking it stays the person's choice.
//!
//! A copied file stays ours until someone edits it. A manifest in the herdr
//! folder, [`MANIFEST`], keeps a fingerprint of each file as it was written;
//! a file that still matches is replaced by a newer package's copy, and one
//! that does not is left alone. A file of the same name that was there before
//! Otto's, and differs from it, is never touched.
//!
//! The sessions mod writes the desktop's accent into the themes as it changes
//! (`cursor`, `colors.accent` and `colors.solid`), so those keys are left out
//! of a theme's fingerprint, and carried over when a theme is replaced.

// Rust guideline compliant 2026-02-21

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::vendors::Home;

/// Where the package installs the look.
pub const SYSTEM_DIR: &str = "/usr/share/otto/herdr";

/// The manifest's name inside herdr's config folder: one `<fingerprint>
/// <path>` line per file Otto wrote, the path relative to the folder.
pub const MANIFEST: &str = ".otto-look";

/// The theme keys the sessions mod rewrites with the desktop's accent.
const ACCENT_KEYS: [(&str, Option<&str>); 3] = [
    ("cursor", None),
    ("colors", Some("accent")),
    ("colors", Some("solid")),
];

/// Where one shipped file stands in herdr's config folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Not there yet.
    Absent,
    /// Ours and the same as the shipped copy.
    Current,
    /// Ours, unedited, and older than the shipped copy.
    Stale,
    /// Edited since Otto wrote it, or someone else's: left alone.
    Taken,
}

/// What [`install`] did to one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Done {
    Created,
    Updated,
    Unchanged,
    LeftAlone,
}

/// One shipped file and where it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The path relative to both the shipped folder and herdr's.
    pub path: PathBuf,
    pub state: State,
    /// What [`install`] did; `None` from [`status`].
    pub done: Option<Done>,
}

/// The look as found on disk and herdr's config folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Look {
    /// herdr's config folder.
    pub home: PathBuf,
    /// Whether herdr has made its config folder: nothing is copied before.
    pub herdr_set_up: bool,
    pub entries: Vec<Entry>,
}

/// herdr's config folder: `$HERDR_APP_HOME` when it names an absolute path
/// and `home` is the environment's, else `herdr-app` under the config home.
pub fn herdr_home(home: &Home, from_env: bool) -> PathBuf {
    if from_env {
        if let Some(dir) = std::env::var_os("HERDR_APP_HOME").map(PathBuf::from) {
            if dir.is_absolute() {
                return dir;
            }
        }
    }
    home.config.join("herdr-app")
}

/// Where each file of `source` stands under `herdr`, without writing.
///
/// `None` when `source` holds nothing, which is a package without the look.
pub fn status(source: &Path, herdr: &Path) -> Option<Look> {
    let files = shipped(source);
    if files.is_empty() {
        return None;
    }
    let manifest = read_manifest(herdr);
    let entries = files
        .into_iter()
        .map(|path| {
            let state = state_of(source, herdr, &path, manifest.get(&path));
            Entry {
                path,
                state,
                done: None,
            }
        })
        .collect();
    Some(Look {
        home: herdr.to_path_buf(),
        herdr_set_up: herdr.is_dir(),
        entries,
    })
}

/// Copies each file of `source` that is absent or ours into `herdr`.
///
/// Does nothing until herdr has made `herdr`. `None` when `source` holds
/// nothing.
///
/// # Errors
///
/// Fails when a file cannot be written or the manifest cannot be saved.
pub fn install(source: &Path, herdr: &Path) -> io::Result<Option<Look>> {
    let Some(mut look) = status(source, herdr) else {
        return Ok(None);
    };
    if !look.herdr_set_up {
        return Ok(Some(look));
    }
    let mut manifest = read_manifest(herdr);
    for entry in &mut look.entries {
        let from = source.join(&entry.path);
        let to = herdr.join(&entry.path);
        let done = match entry.state {
            State::Taken => Done::LeftAlone,
            State::Current => Done::Unchanged,
            State::Absent | State::Stale => {
                let mut text = fs::read(&from)?;
                if is_theme(&entry.path) {
                    if let Ok(old) = fs::read(&to) {
                        text = with_accent_of(&text, &old);
                    }
                }
                if let Some(parent) = to.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&to, &text)?;
                if entry.state == State::Absent {
                    Done::Created
                } else {
                    Done::Updated
                }
            }
        };
        if done != Done::LeftAlone {
            if let Ok(text) = fs::read(&to) {
                manifest.insert(entry.path.clone(), fingerprint(&entry.path, &text));
            }
        }
        entry.done = Some(done);
    }
    write_manifest(herdr, &manifest)?;
    Ok(Some(look))
}

fn state_of(source: &Path, herdr: &Path, path: &Path, recorded: Option<&u64>) -> State {
    let Ok(ours) = fs::read(source.join(path)) else {
        return State::Taken;
    };
    let Ok(theirs) = fs::read(herdr.join(path)) else {
        return State::Absent;
    };
    let now = fingerprint(path, &theirs);
    if now == fingerprint(path, &ours) {
        State::Current
    } else if recorded == Some(&now) {
        State::Stale
    } else {
        State::Taken
    }
}

/// Every file under `source`, relative to it, in path order.
fn shipped(source: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut pending = vec![source.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(read) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => pending.push(path),
                Ok(kind) if kind.is_file() => {
                    if let Ok(relative) = path.strip_prefix(source) {
                        out.push(relative.to_path_buf());
                    }
                }
                _ => {}
            }
        }
    }
    out.sort();
    out
}

fn is_theme(path: &Path) -> bool {
    path.starts_with("themes") && path.extension().is_some_and(|ext| ext == "json")
}

/// A file's fingerprint; a theme's leaves out the accent and the formatting.
fn fingerprint(path: &Path, text: &[u8]) -> u64 {
    if is_theme(path) {
        if let Ok(mut theme) = serde_json::from_slice::<serde_json::Value>(text) {
            for (key, inner) in ACCENT_KEYS {
                let (map, key) = match inner {
                    None => (theme.as_object_mut(), key),
                    Some(inner) => (
                        theme
                            .get_mut(key)
                            .and_then(serde_json::Value::as_object_mut),
                        inner,
                    ),
                };
                if let Some(map) = map {
                    map.remove(key);
                }
            }
            return fnv1a(theme.to_string().as_bytes());
        }
    }
    fnv1a(text)
}

/// `text`, a shipped theme, with the accent keys of `old` where it has them.
fn with_accent_of(text: &[u8], old: &[u8]) -> Vec<u8> {
    let (Ok(mut theme), Ok(old)) = (
        serde_json::from_slice::<serde_json::Value>(text),
        serde_json::from_slice::<serde_json::Value>(old),
    ) else {
        return text.to_vec();
    };
    for (key, inner) in ACCENT_KEYS {
        let value = match inner {
            None => old.get(key),
            Some(inner) => old.get(key).and_then(|map| map.get(inner)),
        };
        let Some(value) = value.cloned() else {
            continue;
        };
        let slot = match inner {
            None => theme.get_mut(key),
            Some(inner) => theme.get_mut(key).and_then(|map| map.get_mut(inner)),
        };
        if let Some(slot) = slot {
            *slot = value;
        }
    }
    match serde_json::to_string_pretty(&theme) {
        Ok(pretty) => format!("{pretty}\n").into_bytes(),
        Err(_) => text.to_vec(),
    }
}

/// 64-bit FNV-1a: stable across Rust releases, unlike `DefaultHasher`, which
/// matters because fingerprints outlive the binary that wrote them. Not for
/// security: it only tells an edited file from an unedited one.
fn fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    bytes.iter().fold(OFFSET, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(PRIME)
    })
}

fn read_manifest(herdr: &Path) -> BTreeMap<PathBuf, u64> {
    let Ok(text) = fs::read_to_string(herdr.join(MANIFEST)) else {
        return BTreeMap::new();
    };
    text.lines()
        .filter_map(|line| {
            let (hash, path) = line.split_once(' ')?;
            Some((PathBuf::from(path), u64::from_str_radix(hash, 16).ok()?))
        })
        .collect()
}

fn write_manifest(herdr: &Path, manifest: &BTreeMap<PathBuf, u64>) -> io::Result<()> {
    let mut text = String::from("# Files otto-agents wrote; an edited file is left alone.\n");
    for (path, hash) in manifest {
        text.push_str(&format!("{hash:016x} {}\n", path.display()));
    }
    fs::write(herdr.join(MANIFEST), text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const THEME: &str = r##"{"cursor":"#0a84ff","colors":{"accent":"#0a84ff","solid":"#0a84ff","chrome":"#252527b3"}}"##;

    fn source(root: &Path, theme: &str, main: &str) -> PathBuf {
        let source = root.join("share");
        fs::create_dir_all(source.join("themes")).unwrap();
        fs::create_dir_all(source.join("mods/otto-sessions")).unwrap();
        fs::write(source.join("themes/otto-dark.json"), theme).unwrap();
        fs::write(source.join("mods/otto-sessions/main.js"), main).unwrap();
        source
    }

    fn done(look: &Look) -> Vec<Option<Done>> {
        look.entries.iter().map(|entry| entry.done).collect()
    }

    #[test]
    fn waits_for_herdr_then_copies_everything() {
        let root = tempfile::tempdir().unwrap();
        let source = source(root.path(), THEME, "v1");
        let herdr = root.path().join("herdr-app");
        let look = install(&source, &herdr).unwrap().unwrap();
        assert!(!look.herdr_set_up);
        assert!(!herdr.exists(), "nothing is written before herdr runs");

        fs::create_dir(&herdr).unwrap();
        let look = install(&source, &herdr).unwrap().unwrap();
        assert_eq!(done(&look), [Some(Done::Created), Some(Done::Created)]);
        let again = install(&source, &herdr).unwrap().unwrap();
        assert_eq!(done(&again), [Some(Done::Unchanged), Some(Done::Unchanged)]);
    }

    #[test]
    fn updates_unedited_files_and_keeps_edited_ones() {
        let root = tempfile::tempdir().unwrap();
        let herdr = root.path().join("herdr-app");
        fs::create_dir(&herdr).unwrap();
        install(&source(root.path(), THEME, "v1"), &herdr).unwrap();
        fs::write(herdr.join("themes/otto-dark.json"), "{\"mine\":true}").unwrap();

        let newer = THEME.replace("252527b3", "303030b3");
        let look = install(&source(root.path(), &newer, "v2"), &herdr)
            .unwrap()
            .unwrap();
        assert_eq!(done(&look), [Some(Done::Updated), Some(Done::LeftAlone)]);
        assert_eq!(
            fs::read_to_string(herdr.join("mods/otto-sessions/main.js")).unwrap(),
            "v2"
        );
        assert_eq!(
            fs::read_to_string(herdr.join("themes/otto-dark.json")).unwrap(),
            "{\"mine\":true}"
        );
    }

    #[test]
    fn a_rewritten_accent_is_not_an_edit_and_survives_an_update() {
        let root = tempfile::tempdir().unwrap();
        let herdr = root.path().join("herdr-app");
        fs::create_dir(&herdr).unwrap();
        install(&source(root.path(), THEME, "v1"), &herdr).unwrap();
        let theme = herdr.join("themes/otto-dark.json");
        let accented = THEME.replace("#0a84ff", "#ff9f0a");
        fs::write(&theme, &accented).unwrap();
        let look = status(&root.path().join("share"), &herdr).unwrap();
        assert_eq!(look.entries[1].state, State::Current);

        let newer = THEME.replace("252527b3", "303030b3");
        install(&source(root.path(), &newer, "v1"), &herdr).unwrap();
        let written: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&theme).unwrap()).unwrap();
        assert_eq!(written["colors"]["chrome"], "#303030b3");
        assert_eq!(written["colors"]["accent"], "#ff9f0a");
        assert_eq!(written["cursor"], "#ff9f0a");
    }

    #[test]
    fn someone_elses_file_of_the_same_name_is_left_alone() {
        let root = tempfile::tempdir().unwrap();
        let herdr = root.path().join("herdr-app");
        fs::create_dir_all(herdr.join("mods/otto-sessions")).unwrap();
        fs::write(herdr.join("mods/otto-sessions/main.js"), "theirs").unwrap();
        let look = install(&source(root.path(), THEME, "v1"), &herdr)
            .unwrap()
            .unwrap();
        assert_eq!(look.entries[0].state, State::Taken);
        assert_eq!(
            fs::read_to_string(herdr.join("mods/otto-sessions/main.js")).unwrap(),
            "theirs"
        );
    }

    #[test]
    fn no_shipped_look_is_none() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(status(&root.path().join("missing"), root.path()), None);
    }
}
