//! The file's versions, kept while an agent works on it.
//!
//! Once a chat about a file has started, Preview keeps a copy of the file as
//! it was before the first message, and another each time it changes after
//! that, whoever saved it. Stepping back copies an older version over the
//! file, which the window follows like any other change. The file matching a
//! version already kept is that version, not a new one, so a step back and
//! the watcher seeing it agree on where the file is.
//!
//! Kept under `$XDG_STATE_HOME/otto-preview/versions/<digest of the path>/`:
//! `index.json` and one copy per version, named by its number.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// How many versions of one file are kept. The first, the file as it was
/// before any of it, is never the one let go.
const KEEP: usize = 50;

/// One version of the file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Version {
    /// Its number, counting from 1 and never reused for the file.
    pub n: u32,
    /// The copy's file name in the store.
    name: String,
    /// When it was kept, in seconds since the epoch.
    pub time: u64,
    /// What changed, as the agent said when it asked for a reload.
    pub note: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Index {
    file: PathBuf,
    current: usize,
    next: u32,
    versions: Vec<Version>,
}

/// The versions kept for one file.
#[derive(Debug)]
pub struct Versions {
    dir: PathBuf,
    file: PathBuf,
    list: Vec<Version>,
    /// The version the file is at, an index into `list`.
    current: usize,
    next: u32,
}

impl Versions {
    /// Where every file's versions are kept.
    pub fn root() -> Option<PathBuf> {
        let state = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            })?;
        Some(state.join("otto-preview/versions"))
    }

    /// The versions a chat about `file` kept before, if any did.
    pub fn existing(file: &Path) -> Option<Self> {
        Self::existing_in(&Self::root()?, file)
    }

    /// The versions kept for `file`, starting them with the file as it is now
    /// when there are none yet.
    pub fn start(file: &Path) -> io::Result<Self> {
        let root = Self::root().ok_or_else(|| io::Error::other("no state folder"))?;
        Self::start_in(&root, file)
    }

    fn existing_in(root: &Path, file: &Path) -> Option<Self> {
        let dir = root.join(digest(file));
        let text = std::fs::read_to_string(dir.join("index.json")).ok()?;
        let index: Index = serde_json::from_str(&text).ok()?;
        if index.file != file || index.versions.is_empty() {
            return None;
        }
        Some(Self {
            dir,
            file: file.to_owned(),
            current: index.current.min(index.versions.len() - 1),
            next: index.next,
            list: index.versions,
        })
    }

    fn start_in(root: &Path, file: &Path) -> io::Result<Self> {
        let mut versions = match Self::existing_in(root, file) {
            Some(versions) => versions,
            None => Self {
                dir: root.join(digest(file)),
                file: file.to_owned(),
                list: Vec::new(),
                current: 0,
                next: 1,
            },
        };
        versions.record(None)?;
        Ok(versions)
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn can_undo(&self) -> bool {
        self.current > 0
    }

    pub fn can_redo(&self) -> bool {
        self.current + 1 < self.list.len()
    }

    /// Take the file as it is now: the version it matches, or a new one after
    /// the current, dropping any that were stepped back from. `note` names
    /// what changed, for a version that has no note yet. Returns whether the
    /// versions or the current one changed.
    pub fn record(&mut self, note: Option<&str>) -> io::Result<bool> {
        let bytes = match std::fs::read(&self.file) {
            Ok(bytes) => bytes,
            // A file gone keeps the versions it had.
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(err) => return Err(err),
        };
        let note = note.map(str::trim).filter(|note| !note.is_empty());
        if let Some(index) = self.matching(&bytes) {
            let moved = index != self.current;
            self.current = index;
            let version = &mut self.list[index];
            let noted = version.note.is_none() && note.is_some();
            if noted {
                version.note = note.map(str::to_owned);
            }
            if moved || noted {
                self.save()?;
            }
            return Ok(moved || noted);
        }
        std::fs::create_dir_all(&self.dir)?;
        if !self.list.is_empty() {
            for dropped in self.list.drain(self.current + 1..) {
                let _ = std::fs::remove_file(self.dir.join(&dropped.name));
            }
        }
        let n = self.next;
        self.next += 1;
        let extension = self
            .file
            .extension()
            .map(|extension| format!(".{}", extension.to_string_lossy()))
            .unwrap_or_default();
        let name = format!("{n:04}{extension}");
        std::fs::write(self.dir.join(&name), &bytes)?;
        self.list.push(Version {
            n,
            name,
            time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|since| since.as_secs())
                .unwrap_or(0),
            note: note.map(str::to_owned),
        });
        while self.list.len() > KEEP {
            let dropped = self.list.remove(1);
            let _ = std::fs::remove_file(self.dir.join(&dropped.name));
        }
        self.current = self.list.len() - 1;
        self.save()?;
        Ok(true)
    }

    /// Put version number `n` back in place of the file.
    pub fn revert(&mut self, n: u32) -> io::Result<()> {
        let index = self
            .list
            .iter()
            .position(|version| version.n == n)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no version {n}")))?;
        self.restore(index)
    }

    /// Step back one version. Returns whether there was one.
    pub fn undo(&mut self) -> io::Result<bool> {
        if !self.can_undo() {
            return Ok(false);
        }
        self.restore(self.current - 1)?;
        Ok(true)
    }

    /// Step forward again. Returns whether there was a version to go to.
    pub fn redo(&mut self) -> io::Result<bool> {
        if !self.can_redo() {
            return Ok(false);
        }
        self.restore(self.current + 1)?;
        Ok(true)
    }

    /// Every version, the current one marked, as the agent reads them.
    pub fn to_json(&self) -> Value {
        json!({
            "file": self.file,
            "current": self.list.get(self.current).map(|version| version.n),
            "versions": self.list.iter().enumerate().map(|(index, version)| json!({
                "version": version.n,
                "time": version.time,
                "note": version.note,
                "current": index == self.current,
            })).collect::<Vec<_>>(),
        })
    }

    /// Copy version `index` over the file, through a file beside it renamed
    /// into place, so nothing ever reads half of it.
    fn restore(&mut self, index: usize) -> io::Result<()> {
        let bytes = std::fs::read(self.dir.join(&self.list[index].name))?;
        let name = self
            .file
            .file_name()
            .ok_or_else(|| io::Error::other("the file has no name"))?;
        let temp = self.file.with_file_name(format!(
            ".{}.otto-preview-{}",
            name.to_string_lossy(),
            std::process::id()
        ));
        std::fs::write(&temp, &bytes)?;
        if let Ok(metadata) = std::fs::metadata(&self.file) {
            let _ = std::fs::set_permissions(&temp, metadata.permissions());
        }
        if let Err(err) = std::fs::rename(&temp, &self.file) {
            let _ = std::fs::remove_file(&temp);
            return Err(err);
        }
        self.current = index;
        self.save()
    }

    /// The version holding exactly `bytes`, the current one first.
    fn matching(&self, bytes: &[u8]) -> Option<usize> {
        let same = |index: usize| {
            let path = self.dir.join(&self.list[index].name);
            std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() == bytes.len() as u64)
                && std::fs::read(&path).is_ok_and(|kept| kept == bytes)
        };
        if self.current < self.list.len() && same(self.current) {
            return Some(self.current);
        }
        (0..self.list.len())
            .rev()
            .filter(|&index| index != self.current)
            .find(|&index| same(index))
    }

    fn save(&self) -> io::Result<()> {
        let index = Index {
            file: self.file.clone(),
            current: self.current,
            next: self.next,
            versions: self.list.clone(),
        };
        let text = serde_json::to_string_pretty(&index).map_err(io::Error::other)?;
        let temp = self.dir.join("index.json.new");
        std::fs::write(&temp, text)?;
        std::fs::rename(temp, self.dir.join("index.json"))
    }
}

/// A folder name for `file`'s versions: FNV-1a of its path, which stays the
/// same from one run to the next.
fn digest(file: &Path) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in file.as_os_str().as_encoded_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn each_change_is_a_version_and_a_step_back_puts_it_back() {
        let work = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        let file = work.path().join("note.md");
        write(&file, "first");

        let mut versions = Versions::start_in(store.path(), &file).unwrap();
        assert_eq!(versions.len(), 1);
        assert!(!versions.can_undo());

        write(&file, "second");
        assert!(versions.record(Some("said it again")).unwrap());
        assert_eq!(versions.len(), 2);
        // The same file again is the same version.
        assert!(!versions.record(None).unwrap());
        assert_eq!(versions.len(), 2);

        assert!(versions.undo().unwrap());
        assert_eq!(read(&file), "first");
        // The watcher sees the step back: still the first version.
        assert!(!versions.record(None).unwrap());
        assert_eq!(versions.len(), 2);
        assert!(versions.can_redo());

        assert!(versions.redo().unwrap());
        assert_eq!(read(&file), "second");
        versions.revert(1).unwrap();
        assert_eq!(read(&file), "first");

        // A change after stepping back drops what was stepped back from.
        write(&file, "third");
        assert!(versions.record(None).unwrap());
        assert_eq!(versions.len(), 2);
        assert!(!versions.can_redo());
        let json = versions.to_json();
        assert_eq!(json["current"], 3);
        assert_eq!(json["versions"][0]["version"], 1);

        // Kept between runs.
        let again = Versions::existing_in(store.path(), &file).unwrap();
        assert_eq!(again.len(), 2);
        assert!(again.can_undo());
        assert!(Versions::existing_in(store.path(), &work.path().join("other.md")).is_none());
    }

    #[test]
    fn a_note_names_the_version_it_came_with() {
        let work = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        let file = work.path().join("a.txt");
        write(&file, "one");
        let mut versions = Versions::start_in(store.path(), &file).unwrap();
        write(&file, "two");
        // The watcher kept it first; the agent's reload names it after.
        versions.record(None).unwrap();
        assert!(versions.record(Some("brighter")).unwrap());
        assert_eq!(versions.to_json()["versions"][1]["note"], "brighter");
        assert!(!versions.record(Some("again")).unwrap());
    }

    #[test]
    fn the_first_version_outlasts_the_limit() {
        let work = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        let file = work.path().join("a.txt");
        write(&file, "original");
        let mut versions = Versions::start_in(store.path(), &file).unwrap();
        for step in 0..KEEP + 5 {
            write(&file, &format!("edit {step}"));
            versions.record(None).unwrap();
        }
        assert_eq!(versions.len(), KEEP);
        assert_eq!(versions.to_json()["versions"][0]["version"], 1);
        versions.revert(1).unwrap();
        assert_eq!(read(&file), "original");
    }
}
