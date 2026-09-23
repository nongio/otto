//! The undo history, one for every file browser window in the session.
//!
//! Each window is its own process, but the files they change are the same
//! files: a drag from one window into another is one operation, and asking
//! which of the two remembers it is a question nobody should have to answer.
//! So the history lives in `$XDG_RUNTIME_DIR/otto/files-undo.json`, and every
//! window pushes onto and pops from that one stack. Ctrl+Z in any window takes
//! back the newest operation from any of them.
//!
//! The runtime directory is the right lifetime: it goes away at logout, and
//! the history is a session's worth of work, never something to carry into
//! the next one. Every read-modify-write holds an exclusive `flock` on the
//! file, so two windows finishing an operation at once both land.
//!
//! Without a runtime directory (or with the file unreadable) the history falls
//! back to this process alone, which is how it always used to work.

// Rust guideline compliant 2026-02-21

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::Change;

/// How many operations back Ctrl+Z can reach.
///
/// Deep enough that undo is a thing you can lean on, shallow enough that the
/// paths it holds cannot pile up: every step remembers where files went, and
/// an unbounded stack would keep the whole session's worth alive.
const DEPTH: usize = 32;

/// One undoable operation: what to call it, and everything it did.
///
/// Only operations that *change files* go on the stack — a move, a copy, a
/// paste, a delete, a rename, a new folder. Selecting and navigating are not
/// undoable and never were: Ctrl+Z that could take back a click would make
/// the ones that take back a delete unreliable, because the user would never
/// know which of the two the next press was going to reach.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoStep {
    /// Names the thing being taken back, for the status line: "Undid Move".
    /// Already translated, in the language of the window that recorded it.
    pub label: String,
    pub changes: Vec<Change>,
}

/// The stack of operations Ctrl+Z can take back, newest last.
#[derive(Debug)]
pub struct UndoHistory {
    /// The shared file, or `None` when this process keeps its own.
    path: Option<PathBuf>,
    /// The steps themselves when there is no shared file.
    local: Vec<UndoStep>,
}

impl UndoHistory {
    /// The history every window in this session shares.
    ///
    /// Tests get a private one instead, so that parallel tests and the
    /// session a developer runs them in never see each other's steps.
    pub fn for_session() -> Self {
        if cfg!(test) {
            return Self::private();
        }
        let path = shared_path();
        if let Some(parent) = path.as_ref().and_then(|p| p.parent()) {
            if let Err(err) = std::fs::create_dir_all(parent) {
                tracing::warn!(dir = %parent.display(), %err, "undo history not shared");
                return Self::private();
            }
        }
        Self {
            path,
            local: Vec::new(),
        }
    }

    /// A history this process keeps to itself.
    pub fn private() -> Self {
        Self {
            path: None,
            local: Vec::new(),
        }
    }

    /// Put an operation on top. Nothing is recorded for one that changed
    /// nothing.
    pub fn push(&mut self, label: &str, changes: Vec<Change>) {
        if changes.is_empty() {
            return;
        }
        let step = UndoStep {
            label: label.to_owned(),
            changes,
        };
        self.edit(|steps| {
            steps.push(step);
            if steps.len() > DEPTH {
                let excess = steps.len() - DEPTH;
                steps.drain(..excess);
            }
        });
    }

    /// Take the newest operation off, whichever window recorded it.
    pub fn pop(&mut self) -> Option<UndoStep> {
        self.edit(Vec::pop)
    }

    /// How many operations there are to take back.
    pub fn len(&self) -> usize {
        match &self.path {
            Some(path) => with_locked(path, |steps| steps.len()).unwrap_or(0),
            None => self.local.len(),
        }
    }

    /// Whether Ctrl+Z has anything to take back.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Change the steps under the lock and write them back.
    ///
    /// A shared file that cannot be opened makes this process fall back to
    /// its own history for good, rather than dropping the step it was handed.
    fn edit<T>(&mut self, change: impl FnOnce(&mut Vec<UndoStep>) -> T) -> T {
        let Some(path) = self.path.clone() else {
            return change(&mut self.local);
        };
        let mut change = Some(change);
        match with_locked(&path, |steps| change.take().map(|change| change(steps))) {
            Ok(Some(out)) => return out,
            Ok(None) => unreachable!("the closure is taken once"),
            Err(err) => {
                tracing::warn!(path = %path.display(), %err, "undo history not shared");
                self.path = None;
            }
        }
        // `with_locked` fails only before it runs the closure.
        let change = change.expect("the closure has not run");
        change(&mut self.local)
    }
}

/// Run `f` over the steps in `path` while holding an exclusive lock on it,
/// and write them back if `f` changed them.
///
/// Fails only before `f` runs, when the file cannot be opened, locked or
/// read. A failure to write back is logged rather than returned: by then the
/// operation has happened and `f` has already answered.
///
/// A file that does not parse is read as empty rather than refused: it is a
/// session's undo history, and losing it costs less than an undo that never
/// works again.
fn with_locked<T>(
    path: &std::path::Path,
    f: impl FnOnce(&mut Vec<UndoStep>) -> T,
) -> std::io::Result<T> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    let _lock = Lock::exclusive(&file)?;
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    let mut steps: Vec<UndoStep> = serde_json::from_str(&text).unwrap_or_default();
    let before = steps.clone();
    let out = f(&mut steps);
    if steps != before {
        if let Err(err) = write_steps(&mut file, &steps) {
            tracing::warn!(path = %path.display(), %err, "could not save the undo history");
        }
    }
    Ok(out)
}

fn write_steps(file: &mut File, steps: &[UndoStep]) -> std::io::Result<()> {
    let text = serde_json::to_string(steps).map_err(std::io::Error::other)?;
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(text.as_bytes())
}

/// An exclusive `flock`, released when dropped.
///
/// Declared after the file it locks, so it drops first; the file closing
/// would release the lock anyway.
struct Lock(RawFd);

impl Lock {
    fn exclusive(file: &File) -> std::io::Result<Self> {
        let fd = file.as_raw_fd();
        // SAFETY: `fd` is open for as long as `file` is, and `flock` only
        // reads it.
        if unsafe { libc::flock(fd, libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self(fd))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        // SAFETY: as in `exclusive`. A failed unlock is not worth reporting:
        // closing the file releases the lock regardless.
        unsafe { libc::flock(self.0, libc::LOCK_UN) };
    }
}

/// Where the shared history lives. `OTTO_FILES_UNDO` overrides the whole path,
/// for running two separate sessions side by side.
fn shared_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("OTTO_FILES_UNDO") {
        return Some(PathBuf::from(path));
    }
    let base = std::env::var_os("XDG_RUNTIME_DIR").filter(|value| !value.is_empty())?;
    Some(PathBuf::from(base).join("otto").join("files-undo.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moved(n: usize) -> Vec<Change> {
        vec![Change::Moved {
            from: PathBuf::from(format!("/a/{n}")),
            to: PathBuf::from(format!("/b/{n}")),
        }]
    }

    fn shared(path: &std::path::Path) -> UndoHistory {
        UndoHistory {
            path: Some(path.to_path_buf()),
            local: Vec::new(),
        }
    }

    fn temp_file(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("otto-files-undo-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("files-undo.json");
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn two_windows_share_one_stack() {
        let path = temp_file("share");
        let mut first = shared(&path);
        let mut second = shared(&path);

        first.push("Move", moved(1));
        second.push("Copy", moved(2));
        assert_eq!(first.len(), 2);

        // The newest step comes off whichever window asks.
        assert_eq!(first.pop().map(|s| s.label).as_deref(), Some("Copy"));
        assert_eq!(second.pop().map(|s| s.label).as_deref(), Some("Move"));
        assert!(first.is_empty());
        assert!(second.pop().is_none());
    }

    #[test]
    fn empty_operations_are_not_recorded() {
        let mut history = UndoHistory::private();
        history.push("Move", Vec::new());
        assert!(history.is_empty());
    }

    #[test]
    fn the_stack_keeps_only_the_newest_steps() {
        let path = temp_file("depth");
        let mut history = shared(&path);
        for n in 0..DEPTH + 5 {
            history.push("Move", moved(n));
        }
        assert_eq!(history.len(), DEPTH);
        assert_eq!(history.pop().map(|s| s.changes), Some(moved(DEPTH + 4)));
    }

    #[test]
    fn a_corrupt_file_reads_as_empty() {
        let path = temp_file("corrupt");
        std::fs::write(&path, "not json").unwrap();
        let mut history = shared(&path);
        assert!(history.is_empty());
        history.push("Move", moved(1));
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn an_unopenable_file_falls_back_to_this_process() {
        let mut history = shared(std::path::Path::new("/nonexistent/dir/undo.json"));
        history.push("Move", moved(1));
        assert_eq!(history.len(), 1);
        assert!(history.path.is_none());
    }
}
