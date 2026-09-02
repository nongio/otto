//! Whether the wastebasket has anything in it.
//!
//! The dock draws a full bin when the trash is not empty, and it has to do so
//! whether or not the Trash window is open — the icon is on screen almost
//! always, the app almost never. So the compositor watches the directory
//! itself, with inotify: read once at startup, then only when something
//! changes, never polled.
//!
//! The trash directory does not have to exist. A session that has never thrown
//! anything away has no `Trash/files` at all, so the watch is placed on the
//! deepest ancestor that does exist and moves down as the directories appear.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

/// How long to wait after an event before looking: emptying the trash deletes
/// a burst of files, and one look per burst is the point of waiting.
const DEBOUNCE: Duration = Duration::from_millis(200);

/// Events that can change whether the can is empty. `DELETE_SELF`/`MOVE_SELF`
/// are here because the watch may be sitting on an ancestor that is about to
/// be replaced.
const INTEREST: u32 = libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_MOVED_FROM
    | libc::IN_MOVED_TO
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF;

/// The directory the icon watches.
///
/// `[dock] trash_path`, which defaults to the freedesktop location
/// `$XDG_DATA_HOME/Trash/files` — where the trash keeps what was thrown away,
/// and the same directory `otto-files --trash` lists. A desktop whose file
/// manager keeps its trash elsewhere points this at that instead.
pub fn files_dir() -> Option<PathBuf> {
    let configured = crate::config::Config::with(|config| config.dock.trash_path.clone());
    let configured = configured.trim();
    if configured.is_empty() {
        return Some(data_home()?.join("Trash/files"));
    }
    expand(configured)
}

/// `$XDG_DATA_HOME`, or the `~/.local/share` it defaults to.
fn data_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_DATA_HOME") {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => Some(PathBuf::from(std::env::var_os("HOME")?).join(".local/share")),
    }
}

/// Expand the handful of things a path in the config may start with: `~`,
/// `$HOME` and `$XDG_DATA_HOME` — the last of which is what the default is
/// written in terms of, so the setting says where it actually looks.
fn expand(path: &str) -> Option<PathBuf> {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    for (prefix, base) in [
        ("$XDG_DATA_HOME", data_home()),
        ("$HOME", home()),
        ("~", home()),
    ] {
        let Some(rest) = path.strip_prefix(prefix) else {
            continue;
        };
        // `$HOMEWORK` is not `$HOME`: only a whole leading segment counts.
        if !(rest.is_empty() || rest.starts_with('/')) {
            continue;
        }
        return Some(base?.join(rest.trim_start_matches('/')));
    }
    Some(PathBuf::from(path))
}

/// Whether the trash holds anything. A directory that does not exist is an
/// empty trash, not an error: it is what a session that has never deleted
/// anything looks like.
pub fn has_content() -> bool {
    let Some(dir) = files_dir() else {
        return false;
    };
    std::fs::read_dir(dir)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

/// Call `on_change` with the trash's state now, and again every time it
/// changes, until the process ends.
///
/// One thread, blocking in `read(2)` on an inotify descriptor. If inotify is
/// unavailable the state is still reported once — the icon is right until
/// something changes it, which is better than no icon at all.
pub fn watch(on_change: impl Fn(bool) + Send + 'static) {
    on_change(has_content());

    let Some(files) = files_dir() else {
        return;
    };

    std::thread::Builder::new()
        .name("otto-trash-watch".into())
        .spawn(move || run(&files, on_change))
        .ok();
}

fn run(files: &Path, on_change: impl Fn(bool)) {
    // SAFETY: inotify_init1 takes flags and returns a descriptor or -1.
    let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
    if fd < 0 {
        tracing::warn!("trash watch: inotify unavailable, the dock icon will not follow the can");
        return;
    }

    let (tx, rx) = mpsc::channel::<()>();
    // The read blocks, so it lives on its own thread and pokes this one; this
    // one owns the debounce and the re-arming.
    std::thread::Builder::new()
        .name("otto-trash-inotify".into())
        .spawn(move || {
            let mut buffer = [0u8; 4096];
            loop {
                // SAFETY: reading into a buffer we own, of the size we pass.
                let read = unsafe {
                    libc::read(
                        fd,
                        buffer.as_mut_ptr() as *mut libc::c_void,
                        buffer.len() as libc::size_t,
                    )
                };
                if read <= 0 {
                    // EINTR is worth retrying; anything else means the
                    // descriptor is gone and so is the watch.
                    if read < 0
                        && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                    {
                        continue;
                    }
                    return;
                }
                if tx.send(()).is_err() {
                    return;
                }
            }
        })
        .ok();

    let mut armed: Option<(i32, PathBuf)> = None;
    let mut last = has_content();
    loop {
        // Watch the deepest directory that exists: `Trash/files` once it is
        // there, its parent while it is not, so its creation is itself an
        // event.
        let target = deepest_existing(files);
        match (&armed, &target) {
            (Some((_, current)), Some(target)) if current == target => {}
            _ => {
                if let Some((descriptor, _)) = armed.take() {
                    // SAFETY: a descriptor this thread added and has not removed.
                    unsafe { libc::inotify_rm_watch(fd, descriptor) };
                }
                if let Some(target) = target.clone() {
                    if let Ok(c_path) =
                        std::ffi::CString::new(target.as_os_str().as_encoded_bytes())
                    {
                        // SAFETY: a NUL-terminated path and a mask of inotify flags.
                        let descriptor =
                            unsafe { libc::inotify_add_watch(fd, c_path.as_ptr(), INTEREST) };
                        if descriptor >= 0 {
                            armed = Some((descriptor, target));
                        }
                    }
                }
            }
        }

        match rx.recv() {
            Ok(()) => {}
            // The reader thread is gone: the descriptor died with it.
            Err(_) => return,
        }
        // Drain the burst rather than looking once per file: keep waiting
        // until DEBOUNCE passes with nothing new.
        loop {
            match rx.recv_timeout(DEBOUNCE) {
                Ok(()) => continue,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }

        let now = has_content();
        if now != last {
            last = now;
            on_change(now);
        }
    }
}

/// `files`, or the nearest ancestor of it that exists.
fn deepest_existing(files: &Path) -> Option<PathBuf> {
    let mut candidate = Some(files);
    while let Some(path) = candidate {
        if path.is_dir() {
            return Some(path.to_path_buf());
        }
        candidate = path.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_configured_path_expands_the_usual_prefixes() {
        let home = PathBuf::from(std::env::var_os("HOME").expect("a home directory"));
        assert_eq!(expand("~/Rubbish"), Some(home.join("Rubbish")));
        assert_eq!(expand("$HOME/Rubbish"), Some(home.join("Rubbish")));
        assert_eq!(expand("/srv/bin"), Some(PathBuf::from("/srv/bin")));
        // A whole leading segment, not a prefix of a longer name.
        assert_eq!(expand("~weird"), Some(PathBuf::from("~weird")));
        assert_eq!(
            expand("$HOMEWORK/bin"),
            Some(PathBuf::from("$HOMEWORK/bin"))
        );
    }

    #[test]
    fn the_deepest_existing_ancestor_is_the_one_watched() {
        let root = std::env::temp_dir().join(format!("otto-trash-test-{}", std::process::id()));
        let files = root.join("Trash/files");
        std::fs::create_dir_all(&root).unwrap();

        // Nothing below `root` exists yet, so that is what gets watched.
        assert_eq!(deepest_existing(&files), Some(root.clone()));

        std::fs::create_dir_all(&files).unwrap();
        assert_eq!(deepest_existing(&files), Some(files.clone()));

        std::fs::remove_dir_all(&root).ok();
    }
}
