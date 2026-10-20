//! Replacing a file in one step.
//!
//! Every config and state file Otto writes is read by something else while
//! it runs: the desk watches `files.toml`, the agents service reads
//! `agents.toml` when it restarts. [`write_atomically`] writes beside the
//! file and renames over it, so a reader sees the old file or the new one,
//! never half of one, and a failed write leaves the old one alone.

use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// How many links are followed before giving up, as the kernel's `ELOOP`.
const MAX_LINKS: u32 = 40;

/// How many temporary names are drawn before giving up, should each one
/// already be taken.
const TEMP_TRIES: u32 = 16;

/// Replace the file at `path` with `bytes`, so a reader never sees it half
/// written.
///
/// The bytes go to a new file in the same folder, made fresh (never opened
/// through a link or over a file already there), synced, and renamed over
/// the old one. The folder is made when it is missing.
///
/// A symlinked file, such as a dotfiles checkout, is written where the link
/// points, so the link survives. The new file keeps the old one's
/// permissions and, where allowed, its owner; a file that was not there
/// gets the usual permissions under the umask.
///
/// # Errors
///
/// When the folder cannot be made or written, or `path` names no file. The
/// file is left as it was, and the temporary one removed.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let target = resolve_links(path)?;
    let name = target.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} names no file", path.display()),
        )
    })?;
    let dir = match target.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
        _ => PathBuf::from("."),
    };
    fs::create_dir_all(&dir)?;

    let old = fs::metadata(&target).ok();
    let mode = old
        .as_ref()
        .map_or(0o666, |old| old.permissions().mode() & 0o7777);
    let (temp, mut file) = create_temp(&dir, name, mode)?;
    let written = (|| {
        if let Some(old) = &old {
            // The umask may have narrowed the mode it was made with.
            file.set_permissions(fs::Permissions::from_mode(mode))?;
            // Best effort: only root may give a file away.
            let _ = std::os::unix::fs::fchown(&file, Some(old.uid()), Some(old.gid()));
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, &target)
    })();
    drop(file);
    if let Err(err) = written {
        let _ = fs::remove_file(&temp);
        return Err(err);
    }
    // The rename itself survives a crash once the folder is synced.
    if let Ok(dir) = fs::File::open(&dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// The file `path` ends up naming once every link on it is followed. A link
/// to a file not there yet resolves to where that file would be.
fn resolve_links(path: &Path) -> io::Result<PathBuf> {
    let mut current = path.to_path_buf();
    for _ in 0..MAX_LINKS {
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let link = fs::read_link(&current)?;
                current = match current.parent() {
                    Some(parent) if link.is_relative() => parent.join(link),
                    _ => link,
                };
            }
            _ => return Ok(current),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{}: too many levels of symbolic links", path.display()),
    ))
}

/// A new file in `dir` beside `name`, made with `mode` under the umask.
fn create_temp(dir: &Path, name: &OsStr, mode: u32) -> io::Result<(PathBuf, fs::File)> {
    let mut tries = 0;
    loop {
        let temp = dir.join(temp_name(name));
        // `create_new` is `O_CREAT | O_EXCL`, which also refuses to follow a
        // link planted on the name.
        let opened = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temp);
        match opened {
            Ok(file) => return Ok((temp, file)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists && tries + 1 < TEMP_TRIES => {
                tries += 1;
            }
            Err(err) => return Err(err),
        }
    }
}

/// A hidden name for the file written beside `name`, with a suffix nobody
/// can guess ahead of time: the clock, the process, a counter and the
/// standard library's per-process random hash keys.
fn temp_name(name: &OsStr) -> String {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    hasher.write_u128(now.as_nanos());
    hasher.write_u32(std::process::id());
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    format!(".{}.tmp-{:016x}", name.to_string_lossy(), hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder, swept up on drop.
    struct Dir(PathBuf);

    impl Dir {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "otto-foundations-fs-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        /// Everything in the folder but `keep`, to catch a temporary file
        /// left behind.
        fn others(&self, keep: &[&str]) -> Vec<String> {
            fs::read_dir(&self.0)
                .unwrap()
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| !keep.contains(&name.as_str()))
                .collect()
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn writes_a_new_file_and_its_folder() {
        let dir = Dir::new();
        let path = dir.0.join("a").join("b.toml");
        write_atomically(&path, b"one").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"one");
        assert!(dir.others(&["a"]).is_empty());
    }

    #[test]
    fn replaces_a_file_leaving_nothing_beside_it() {
        let dir = Dir::new();
        let path = dir.0.join("c.toml");
        fs::write(&path, b"old").unwrap();
        write_atomically(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert!(dir.others(&["c.toml"]).is_empty());
    }

    #[test]
    fn keeps_the_mode() {
        let dir = Dir::new();
        let path = dir.0.join("secret.toml");
        fs::write(&path, b"old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        write_atomically(&path, b"new").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn writes_through_a_link_and_keeps_it() {
        let dir = Dir::new();
        fs::create_dir(dir.0.join("dotfiles")).unwrap();
        let real = dir.0.join("dotfiles").join("files.toml");
        fs::write(&real, b"old").unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o640)).unwrap();
        let link = dir.0.join("files.toml");
        std::os::unix::fs::symlink("dotfiles/files.toml", &link).unwrap();

        write_atomically(&link, b"new").unwrap();
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&real).unwrap(), b"new");
        let mode = fs::metadata(&real).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o640);
        assert!(dir.others(&["dotfiles", "files.toml"]).is_empty());
    }

    #[test]
    fn a_dangling_link_makes_its_target() {
        let dir = Dir::new();
        let real = dir.0.join("later.toml");
        let link = dir.0.join("link.toml");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write_atomically(&link, b"made").unwrap();
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&real).unwrap(), b"made");
    }

    #[test]
    fn a_link_loop_is_refused() {
        let dir = Dir::new();
        let a = dir.0.join("a");
        let b = dir.0.join("b");
        std::os::unix::fs::symlink(&b, &a).unwrap();
        std::os::unix::fs::symlink(&a, &b).unwrap();
        assert!(write_atomically(&a, b"x").is_err());
    }

    #[test]
    fn a_failed_write_leaves_the_old_file_and_no_temporary() {
        let dir = Dir::new();
        // A folder where the file should be: the rename over it fails.
        let path = dir.0.join("taken");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("inside"), b"kept").unwrap();
        assert!(write_atomically(&path, b"x").is_err());
        assert_eq!(fs::read(path.join("inside")).unwrap(), b"kept");
        assert!(dir.others(&["taken"]).is_empty());
    }
}
