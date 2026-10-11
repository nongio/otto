//! Directory watching, event-driven.
//!
//! Every directory on screen watches itself with inotify, so a file appearing
//! or vanishing under a pane — whoever made it happen, this app or another —
//! shows up without anybody asking. See `specs/file-picker.md` under
//! *Filesystem watching*: only what is displayed is watched, nothing is
//! watched recursively, and nothing polls the filesystem.
//!
//! One inotify instance serves the whole process. A single reader thread owns
//! it, blocking in `poll(2)` until either an event lands or a debounce comes
//! due; watches are added and removed from the UI thread, which never blocks.
//! Filesystems inotify cannot watch (many network mounts) simply produce no
//! events — the listing is right when it is read and refreshes on navigation.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How long a directory stays dirty before it is re-read.
///
/// Anything that writes files writes a burst of them; one re-read per burst is
/// the point of waiting at all.
const DEBOUNCE: Duration = Duration::from_millis(100);

/// What happened to a watched directory since the last check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Its contents moved. Re-read it.
    Modified,
    /// The directory itself was deleted or moved away.
    Gone,
}

/// Events worth a re-read. Writes are taken on close rather than on every
/// `write(2)`, so copying a large file costs one refresh, not thousands.
const INTEREST: u32 = libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_MOVED_FROM
    | libc::IN_MOVED_TO
    | libc::IN_CLOSE_WRITE
    | libc::IN_ATTRIB
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF;

/// A watch on one directory, held for as long as that directory is displayed.
///
/// Dropping it drops the kernel watch, once the last holder of that path is
/// gone — two panes on the same directory share one inotify descriptor. Each
/// holder is told of a change on its own: two windows on one folder both
/// refresh.
pub struct DirWatch {
    path: PathBuf,
    /// This holder's own, to take its changes by. `None` when inotify was
    /// unavailable, so `Drop` has nothing to undo and [`Self::take`] has
    /// nothing to report.
    id: Option<u64>,
}

impl DirWatch {
    /// Start watching `path`. A failure here is not an error the user needs to
    /// see: the pane keeps working, it just will not notice changes on its own.
    pub fn new(path: &Path) -> Self {
        let id = watcher().and_then(|w| w.add(path));
        Self {
            path: path.to_path_buf(),
            id,
        }
    }

    /// The change to act on, if the debounce has elapsed. Never blocks.
    pub fn take(&self) -> Option<Change> {
        watcher()?.take(self.id?)
    }
}

impl Drop for DirWatch {
    fn drop(&mut self) {
        if let (Some(id), Some(w)) = (self.id, watcher()) {
            w.remove(&self.path, id);
        }
    }
}

// ---------------------------------------------------------------------------
// The process-wide watcher
// ---------------------------------------------------------------------------

struct Entry {
    wd: i32,
    /// The `DirWatch` handles that want this path.
    holders: Vec<u64>,
}

#[derive(Default)]
struct Inner {
    by_path: HashMap<PathBuf, Entry>,
    by_wd: HashMap<i32, PathBuf>,
    /// Holders whose directory is dirty, each remembering when it first went
    /// dirty — that is what the debounce is measured from, so a steady stream
    /// of writes still refreshes every 100 ms instead of never.
    dirty: HashMap<u64, Instant>,
    /// Holders whose directory went away.
    gone: HashSet<u64>,
    /// The id the next holder gets.
    next: u64,
}

struct Watcher {
    fd: i32,
    inner: Mutex<Inner>,
}

fn watcher() -> Option<&'static Watcher> {
    static WATCHER: OnceLock<Option<&'static Watcher>> = OnceLock::new();
    *WATCHER.get_or_init(|| {
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if fd < 0 {
            tracing::warn!("inotify unavailable; directories will not refresh on their own");
            return None;
        }
        let watcher: &'static Watcher = Box::leak(Box::new(Watcher {
            fd,
            inner: Mutex::new(Inner::default()),
        }));
        std::thread::Builder::new()
            .name("otto-files-watch".into())
            .spawn(move || watcher.run())
            .ok()?;
        Some(watcher)
    })
}

impl Watcher {
    /// Watch `path` for a new holder, and return the holder's id.
    fn add(&self, path: &Path) -> Option<u64> {
        let mut inner = self.inner.lock().unwrap();
        let id = inner.next;
        if let Some(entry) = inner.by_path.get_mut(path) {
            entry.holders.push(id);
            inner.next += 1;
            return Some(id);
        }
        let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
        let wd = unsafe { libc::inotify_add_watch(self.fd, c_path.as_ptr(), INTEREST) };
        if wd < 0 {
            // An unwatchable path is normal — a network mount, or a directory
            // that vanished between the listing and this call.
            return None;
        }
        inner.by_path.insert(
            path.to_path_buf(),
            Entry {
                wd,
                holders: vec![id],
            },
        );
        inner.by_wd.insert(wd, path.to_path_buf());
        inner.next += 1;
        Some(id)
    }

    fn remove(&self, path: &Path, id: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.dirty.remove(&id);
        inner.gone.remove(&id);
        let Some(entry) = inner.by_path.get_mut(path) else {
            return;
        };
        entry.holders.retain(|holder| *holder != id);
        if !entry.holders.is_empty() {
            return;
        }
        let wd = entry.wd;
        inner.by_path.remove(path);
        inner.by_wd.remove(&wd);
        unsafe { libc::inotify_rm_watch(self.fd, wd) };
    }

    fn take(&self, id: u64) -> Option<Change> {
        let mut inner = self.inner.lock().unwrap();
        if inner.gone.remove(&id) {
            return Some(Change::Gone);
        }
        let since = *inner.dirty.get(&id)?;
        if since.elapsed() < DEBOUNCE {
            return None;
        }
        inner.dirty.remove(&id);
        Some(Change::Modified)
    }

    /// The reader thread: block until inotify has something or a debounce
    /// comes due, then wake the UI thread so it can take the change.
    fn run(&'static self) {
        loop {
            let timeout = self.wait_for_events();
            self.drain();
            if timeout {
                // A debounce came due while nothing else happened; the UI
                // thread is idle and commits no frames, so it needs telling.
                crate::AppContext::request_wakeup();
            }
        }
    }

    /// Block until the inotify fd is readable or the next debounce is due.
    /// Returns whether the wait ended on the deadline rather than on an event.
    fn wait_for_events(&self) -> bool {
        let deadline = {
            let inner = self.inner.lock().unwrap();
            inner.dirty.values().min().map(|t| *t + DEBOUNCE)
        };
        let timeout_ms = match deadline {
            Some(deadline) => deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(i32::MAX as u128) as i32,
            None => -1,
        };
        let mut fds = libc::pollfd {
            fd: self.fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut fds, 1, timeout_ms) };
        ready == 0
    }

    /// Read every queued event and fold it into the dirty set.
    fn drain(&self) {
        // `u64` backing so the buffer is aligned for `inotify_event`, whatever
        // the compiler would have given a byte array.
        let mut buf = [0u64; 512];
        let mut woke = false;
        loop {
            let n = unsafe {
                libc::read(
                    self.fd,
                    buf.as_mut_ptr() as *mut libc::c_void,
                    std::mem::size_of_val(&buf),
                )
            };
            if n <= 0 {
                break;
            }
            let mut offset = 0usize;
            let base = buf.as_ptr() as *const u8;
            let header = std::mem::size_of::<libc::inotify_event>();
            while offset + header <= n as usize {
                // Safe: the kernel writes whole records, each header followed
                // by `len` bytes of name, and the buffer is aligned for one.
                let event = unsafe { &*(base.add(offset) as *const libc::inotify_event) };
                woke |= self.record(event.wd, event.mask);
                offset += header + event.len as usize;
            }
        }
        if woke {
            crate::AppContext::request_wakeup();
        }
    }

    /// Fold one event in. Returns whether it is worth waking the UI thread —
    /// a directory going away is acted on at once, while an ordinary change
    /// waits out its debounce and is woken for by the deadline.
    fn record(&self, wd: i32, mask: u32) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if mask & libc::IN_Q_OVERFLOW != 0 {
            // Events were lost, so nothing on screen can be trusted: every
            // watched directory is re-read.
            let now = Instant::now();
            let holders: Vec<u64> = inner
                .by_path
                .values()
                .flat_map(|entry| entry.holders.iter().copied())
                .collect();
            for id in holders {
                inner.dirty.entry(id).or_insert(now);
            }
            return false;
        }
        let Some(path) = inner.by_wd.get(&wd).cloned() else {
            return false;
        };
        if mask & (libc::IN_DELETE_SELF | libc::IN_MOVE_SELF | libc::IN_IGNORED) != 0 {
            // The kernel drops the watch itself for these, so the bookkeeping
            // has to follow — and IN_IGNORED lands for an unmounted
            // filesystem too, which is just as gone as far as a pane is
            // concerned.
            if let Some(entry) = inner.by_path.remove(&path) {
                inner.by_wd.remove(&entry.wd);
                for id in entry.holders {
                    inner.dirty.remove(&id);
                    inner.gone.insert(id);
                }
            }
            return true;
        }
        let holders = inner
            .by_path
            .get(&path)
            .map(|entry| entry.holders.clone())
            .unwrap_or_default();
        let now = Instant::now();
        for id in holders {
            inner.dirty.entry(id).or_insert(now);
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory that removes itself, so a failing test does not leave one
    /// behind in the temp directory.
    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "otto-files-watch-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temp dir");
            Self(path)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Wait for a change, up to two seconds — long enough that a loaded
    /// machine does not make this flaky, short enough to fail a test rather
    /// than hang it.
    fn wait(watch: &DirWatch) -> Option<Change> {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Some(change) = watch.take() {
                return Some(change);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }

    #[test]
    fn a_new_file_marks_its_directory() {
        let dir = Tmp::new("create");
        let watch = DirWatch::new(&dir.0);
        std::fs::write(dir.0.join("landed"), b"x").expect("write");
        assert_eq!(wait(&watch), Some(Change::Modified));
        // Taken once and only once: nothing has changed since.
        assert_eq!(watch.take(), None);
    }

    #[test]
    fn a_burst_of_writes_is_one_change() {
        let dir = Tmp::new("burst");
        let watch = DirWatch::new(&dir.0);
        for i in 0..50 {
            std::fs::write(dir.0.join(format!("f{i}")), b"x").expect("write");
        }
        assert_eq!(wait(&watch), Some(Change::Modified));
        assert_eq!(watch.take(), None);
    }

    #[test]
    fn nothing_is_reported_while_nothing_happens() {
        let dir = Tmp::new("quiet");
        let watch = DirWatch::new(&dir.0);
        std::thread::sleep(DEBOUNCE * 3);
        assert_eq!(watch.take(), None);
    }

    #[test]
    fn losing_the_directory_is_reported_as_gone() {
        let dir = Tmp::new("gone");
        let watch = DirWatch::new(&dir.0);
        std::fs::remove_dir_all(&dir.0).expect("remove");
        assert_eq!(wait(&watch), Some(Change::Gone));
    }

    #[test]
    fn two_panes_on_one_directory_share_a_watch() {
        let dir = Tmp::new("shared");
        let first = DirWatch::new(&dir.0);
        let second = DirWatch::new(&dir.0);
        // Dropping one must not take the other's watch with it.
        drop(first);
        std::fs::write(dir.0.join("landed"), b"x").expect("write");
        assert_eq!(wait(&second), Some(Change::Modified));
    }

    #[test]
    fn each_holder_is_told_of_a_change() {
        let dir = Tmp::new("both");
        let first = DirWatch::new(&dir.0);
        let second = DirWatch::new(&dir.0);
        std::fs::write(dir.0.join("landed"), b"x").expect("write");
        // Two windows on one folder: the first to look must not take the
        // change from the other.
        assert_eq!(wait(&first), Some(Change::Modified));
        assert_eq!(wait(&second), Some(Change::Modified));
    }
}
