//! Where Otto's logs are.
//!
//! In order: Otto's own log files; then, for each running Otto of this user,
//! wherever its stderr goes (a file, or the journal); then the files the
//! `otto-session` login script keeps; and always the panic log. Every place
//! is read through a root folder, so tests run the search over a fixture
//! `/proc` tree.

// Rust guideline compliant 2026-02-21

use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Where one log comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// A file in the state folder (or the one the compositor reported).
    File(PathBuf),
    /// A running Otto's stderr, a regular file: read through
    /// `/proc/<pid>/fd/2`, shown as the file it points to.
    Stderr {
        /// The process.
        pid: u32,
        /// `/proc/<pid>/fd/2`, the path to read.
        fd: PathBuf,
        /// The file stderr points to.
        target: PathBuf,
    },
    /// A running Otto whose stderr is a journal stream.
    Journal {
        /// The process.
        pid: u32,
    },
}

/// One log to include, with the words that say where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// A short name for the list summary: `otto.log`, `journal (pid 12)`.
    pub label: String,
    /// What the log is, for its heading.
    pub what: String,
    /// Where to read it.
    pub origin: Origin,
}

/// Where to look.
#[derive(Debug, Clone, Copy)]
pub struct Places<'a> {
    /// The `/proc` folder (a fixture in tests).
    pub proc_root: &'a Path,
    /// `$XDG_STATE_HOME/otto`.
    pub log_dir: Option<&'a Path>,
    /// The log file the running compositor reported, if it did.
    pub reported: Option<&'a Path>,
    /// Only processes of this user count.
    pub uid: u32,
}

/// Every log worth including, in the order the report shows them.
pub fn find(places: Places<'_>) -> Vec<Found> {
    let mut found: Vec<Found> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    let add_file = |found: &mut Vec<Found>, seen: &mut Vec<PathBuf>, path: PathBuf, what: &str| {
        let key = canonical(&path);
        if !path.is_file() || seen.contains(&key) {
            return;
        }
        seen.push(key);
        found.push(Found {
            label: file_name(&path),
            what: what.to_owned(),
            origin: Origin::File(path),
        });
    };

    // Otto's own log: the file the compositor named, then the usual names.
    if let Some(reported) = places.reported {
        add_file(
            &mut found,
            &mut seen,
            reported.to_owned(),
            "Otto's log, this session",
        );
        add_file(
            &mut found,
            &mut seen,
            with_suffix(reported, ".1"),
            "Otto's log, previous session",
        );
    }
    if let Some(dir) = places.log_dir {
        add_file(
            &mut found,
            &mut seen,
            dir.join("otto.log"),
            "Otto's log, this session",
        );
        add_file(
            &mut found,
            &mut seen,
            dir.join("otto.log.1"),
            "Otto's log, previous session",
        );
    }

    // Where a running Otto's stderr goes.
    for pid in otto_pids(places.proc_root, places.uid) {
        let proc_dir = places.proc_root.join(pid.to_string());
        let fd = proc_dir.join("fd/2");
        let Ok(target) = fs::read_link(&fd) else {
            continue;
        };
        if fs::metadata(&fd).is_ok_and(|m| m.is_file()) {
            let key = canonical(&target);
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            found.push(Found {
                label: format!("stderr → {} (pid {pid})", file_name(&target)),
                what: format!("stderr of Otto, pid {pid}"),
                origin: Origin::Stderr { pid, fd, target },
            });
        } else if target.to_string_lossy().starts_with("socket:")
            && has_env(&proc_dir.join("environ"), "JOURNAL_STREAM")
        {
            found.push(Found {
                label: format!("journal (pid {pid})"),
                what: format!("the journal, Otto pid {pid}"),
                origin: Origin::Journal { pid },
            });
        }
    }

    // What the otto-session login script keeps, unless a running Otto's
    // stderr already is that file.
    if let Some(dir) = places.log_dir {
        add_file(
            &mut found,
            &mut seen,
            dir.join("session.log"),
            "terminal output, from otto-session",
        );
        add_file(
            &mut found,
            &mut seen,
            dir.join("session.log.1"),
            "previous session's terminal output, from otto-session",
        );
        add_file(
            &mut found,
            &mut seen,
            dir.join("panic.log"),
            "every recorded crash",
        );
    }
    found
}

/// The ids of running Otto compositors owned by `uid`, in ascending order.
fn otto_pids(proc_root: &Path, uid: u32) -> Vec<u32> {
    let Ok(entries) = fs::read_dir(proc_root) else {
        return Vec::new();
    };
    let mut pids: Vec<u32> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
            let dir = entry.path();
            let owned = fs::metadata(&dir).is_ok_and(|m| m.uid() == uid);
            (owned && is_otto(&dir)).then_some(pid)
        })
        .collect();
    pids.sort_unstable();
    pids
}

/// Whether the process in `dir` is the compositor: named `otto` by its
/// `comm`, or by its executable.
fn is_otto(dir: &Path) -> bool {
    let comm = fs::read_to_string(dir.join("comm")).unwrap_or_default();
    if comm.trim_end() == "otto" {
        return true;
    }
    fs::read_link(dir.join("exe")).is_ok_and(|exe| {
        // An upgraded binary shows as "otto (deleted)".
        let name = file_name(&exe);
        name.strip_suffix(" (deleted)").unwrap_or(&name) == "otto"
    })
}

/// Whether the NUL-separated environment in `path` sets `var`.
fn has_env(path: &Path, var: &str) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    let prefix = format!("{var}=");
    bytes
        .split(|b| *b == 0)
        .any(|entry| entry.starts_with(prefix.as_bytes()))
}

/// The last `lines` journal lines of process `pid`, from the user journal or,
/// failing that, the system one.
pub fn read_journal(pid: u32, lines: usize) -> Option<String> {
    for user in [true, false] {
        let mut command = Command::new("journalctl");
        if user {
            command.arg("--user");
        }
        let output = command
            .arg(format!("_PID={pid}"))
            .args(["-n", &lines.to_string(), "--no-pager", "-o", "short-iso"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        let Ok(output) = output else {
            return None;
        };
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        // "-- No entries --" and similar markers are not log lines.
        if output.status.success() && text.lines().any(|l| !l.starts_with("-- ")) {
            return Some(text);
        }
    }
    None
}

/// `path` with symlinks resolved, or as it is when that fails.
fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

/// `path` with `suffix` added to its file name.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// The last component of `path`, for display.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn write(path: &Path, content: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    /// A process directory under `proc` with a `comm`, an environment and
    /// `fd/2` pointing at `stderr`.
    fn process(proc_root: &Path, pid: u32, comm: &str, environ: &[u8], stderr: &Path) {
        let dir = proc_root.join(pid.to_string());
        write(&dir.join("comm"), format!("{comm}\n").as_bytes());
        write(&dir.join("environ"), environ);
        fs::create_dir_all(dir.join("fd")).unwrap();
        symlink(stderr, dir.join("fd/2")).unwrap();
    }

    fn uid() -> u32 {
        // SAFETY: getuid has no preconditions.
        unsafe { libc::getuid() }
    }

    fn labels(found: &[Found]) -> Vec<&str> {
        found.iter().map(|f| f.label.as_str()).collect()
    }

    #[test]
    fn otto_logs_come_first_then_stderr_then_legacy_and_panics() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state/otto");
        let proc_root = dir.path().join("proc");
        for name in [
            "otto.log",
            "otto.log.1",
            "session.log",
            "session.log.1",
            "panic.log",
        ] {
            write(&state.join(name), b"line\n");
        }
        // Started by otto-session: stderr is session.log.
        process(
            &proc_root,
            4242,
            "otto",
            b"HOME=/h\0",
            &state.join("session.log"),
        );
        // Something else whose stderr is a file: not Otto, not listed.
        process(&proc_root, 77, "otto-bar", b"", &state.join("otto.log"));

        let found = find(Places {
            proc_root: &proc_root,
            log_dir: Some(&state),
            reported: Some(&state.join("otto.log")),
            uid: uid(),
        });

        assert_eq!(
            labels(&found),
            [
                "otto.log",
                "otto.log.1",
                "stderr → session.log (pid 4242)",
                "session.log.1",
                "panic.log",
            ]
        );
        assert!(matches!(
            &found[2].origin,
            Origin::Stderr { pid: 4242, fd, .. } if fd.ends_with("4242/fd/2")
        ));
    }

    #[test]
    fn a_journal_stream_is_read_from_the_journal() {
        let dir = tempfile::tempdir().unwrap();
        let proc_root = dir.path().join("proc");
        process(
            &proc_root,
            900,
            "otto",
            b"LANG=C\0JOURNAL_STREAM=8:12345\0",
            Path::new("socket:[12345]"),
        );
        // A socket without JOURNAL_STREAM is some other pipe: nothing to read.
        process(
            &proc_root,
            901,
            "otto",
            b"LANG=C\0",
            Path::new("socket:[6789]"),
        );
        // A terminal: its output is gone.
        process(&proc_root, 902, "otto", b"", Path::new("/dev/pts/3"));

        let found = find(Places {
            proc_root: &proc_root,
            log_dir: None,
            reported: None,
            uid: uid(),
        });

        assert_eq!(labels(&found), ["journal (pid 900)"]);
        assert_eq!(found[0].origin, Origin::Journal { pid: 900 });
    }

    #[test]
    fn a_reported_log_with_another_name_is_included_with_its_previous_one() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state/otto");
        write(&state.join("otto-winit.log"), b"nested\n");
        write(&state.join("otto-winit.log.1"), b"nested before\n");
        write(&state.join("otto.log"), b"session\n");

        let found = find(Places {
            proc_root: &dir.path().join("proc"),
            log_dir: Some(&state),
            reported: Some(&state.join("otto-winit.log")),
            uid: uid(),
        });

        assert_eq!(
            labels(&found),
            ["otto-winit.log", "otto-winit.log.1", "otto.log"]
        );
    }

    #[test]
    fn an_executable_named_otto_counts_even_when_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let proc_root = dir.path().join("proc");
        let log = dir.path().join("out.log");
        write(&log, b"x\n");
        process(&proc_root, 5, "tokio-runtime-w", b"", &log);
        symlink("/usr/bin/otto (deleted)", proc_root.join("5/exe")).unwrap();

        let found = find(Places {
            proc_root: &proc_root,
            log_dir: None,
            reported: None,
            uid: uid(),
        });
        assert_eq!(labels(&found), ["stderr → out.log (pid 5)"]);
    }
}
