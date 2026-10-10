//! Moving, copying and naming files the way a file manager does.
//!
//! Shared by everything that puts files somewhere on someone's behalf: a paste
//! in Files, a drop on the dock's Trash. Each helper works on one entry, a file,
//! a symlink or a whole directory tree, and reports failure as a message fit
//! to show.

use std::ffi::OsStr;
use std::io;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};

use rustix::fs::{self as rfs, AtFlags, RenameFlags};
use rustix::io::Errno;

/// What a move says when the name it was going to take is taken.
pub const NAME_TAKEN: &str = "something is already there";

/// Rename `from` to `to` without replacing anything already at `to`.
///
/// `rename(2)` quietly clobbers the destination, and nothing can bring that
/// back, so this asks the kernel for `RENAME_NOREPLACE` and reports a taken
/// name as [`io::ErrorKind::AlreadyExists`]. This is the one no-replace move
/// every path that puts a file under a name goes through: a rename, a batch
/// rename, an undo, a paste, the trash.
///
/// A filesystem without the flag (`EINVAL` or `ENOSYS`, as on some network
/// and FUSE mounts) falls back to a check, without following a symlink, then
/// a plain rename. That leaves a window, which is the best such a filesystem
/// allows.
///
/// On a case-insensitive filesystem `a` → `A` finds the file itself at the
/// destination: that is a rename, not a collision, and goes through.
///
/// # Errors
///
/// `AlreadyExists` when `to` is taken; otherwise the rename's own error
/// (`EXDEV` across filesystems among them, which this never works around).
pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    match rename_no_replace_at(rfs::CWD, from, rfs::CWD, to) {
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists && same_file(from, to) => {
            std::fs::rename(from, to)
        }
        other => other,
    }
}

/// [`rename_no_replace`] relative to directory descriptors, for a caller that
/// has opened and checked the directories and must not go back through their
/// paths (the trash).
///
/// # Errors
///
/// As [`rename_no_replace`], without its same-file allowance.
pub fn rename_no_replace_at(
    from_dir: impl AsFd,
    from: &Path,
    to_dir: impl AsFd,
    to: &Path,
) -> io::Result<()> {
    match rfs::renameat_with(&from_dir, from, &to_dir, to, RenameFlags::NOREPLACE) {
        Ok(()) => Ok(()),
        Err(Errno::EXIST) => Err(io::ErrorKind::AlreadyExists.into()),
        Err(Errno::INVAL | Errno::NOSYS) => {
            if rfs::statat(&to_dir, to, AtFlags::SYMLINK_NOFOLLOW).is_ok() {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            rfs::renameat(&from_dir, from, &to_dir, to).map_err(io::Error::from)
        }
        Err(err) => Err(err.into()),
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (a.symlink_metadata(), b.symlink_metadata()) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// Move one entry to a name that must be free, falling back to
/// copy-then-delete across filesystems.
///
/// Never replaces what is at `target`: see [`rename_no_replace`]. The source
/// is unlinked only after the destination is fully written. That ordering is
/// the whole guarantee that a failed move cannot lose data.
///
/// # Errors
///
/// `AlreadyExists` when `target` is taken, or the rename, copy or removal
/// that failed.
pub fn move_no_replace(source: &Path, target: &Path) -> io::Result<()> {
    match rename_no_replace(source, target) {
        // EXDEV: a rename cannot cross filesystems, so do it the long way.
        Err(err) if err.raw_os_error() == Some(libc::EXDEV) => {
            if target.symlink_metadata().is_ok() {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            copy_entry_with(source, target, false).map_err(|err| {
                if err == NAME_TAKEN {
                    io::ErrorKind::AlreadyExists.into()
                } else {
                    io::Error::other(err)
                }
            })?;
            remove_entry(source).map_err(io::Error::other)
        }
        other => other,
    }
}

/// [`move_no_replace`] with its failure as text fit to show.
///
/// # Errors
///
/// The name is taken ([`NAME_TAKEN`]), or the rename, copy or removal that
/// failed, as text.
pub fn move_entry(source: &Path, target: &Path) -> Result<(), String> {
    move_no_replace(source, target).map_err(|err| match err.kind() {
        io::ErrorKind::AlreadyExists => NAME_TAKEN.to_string(),
        _ => err.to_string(),
    })
}

/// Move one entry onto `target`, replacing a file there: for a paste the
/// user explicitly asked to replace with. Everything else wants
/// [`move_entry`].
///
/// # Errors
///
/// The rename, copy or removal that failed, as text.
pub fn move_entry_replacing(source: &Path, target: &Path) -> Result<(), String> {
    match std::fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(err) if err.raw_os_error() == Some(libc::EXDEV) => {
            copy_entry(source, target)?;
            remove_entry(source)
        }
        Err(err) => Err(err.to_string()),
    }
}

/// Copy a file or a directory tree.
///
/// A symlink is copied as a link, never followed. A file already at `target`
/// is replaced, and a directory already there is merged into.
///
/// # Errors
///
/// The first read, write or rename that failed, as text.
pub fn copy_entry(source: &Path, target: &Path) -> Result<(), String> {
    copy_entry_with(source, target, true)
}

/// [`copy_entry`], or with `replace` false one that never replaces or merges
/// into anything at `target`, nor anywhere under it, and says [`NAME_TAKEN`]
/// when it would have had to.
fn copy_entry_with(source: &Path, target: &Path, replace: bool) -> Result<(), String> {
    let taken = |err: io::Error| {
        if err.kind() == io::ErrorKind::AlreadyExists {
            NAME_TAKEN.to_string()
        } else {
            err.to_string()
        }
    };
    let meta = std::fs::symlink_metadata(source).map_err(|e| e.to_string())?;

    if meta.is_symlink() {
        // Copy the link itself, not what it points at: following it could
        // duplicate a whole tree the user did not ask for.
        let link = std::fs::read_link(source).map_err(|e| e.to_string())?;
        return std::os::unix::fs::symlink(link, target).map_err(taken);
    }

    if meta.is_dir() {
        if replace {
            std::fs::create_dir_all(target).map_err(|e| e.to_string())?;
        } else {
            std::fs::create_dir(target).map_err(taken)?;
        }
        for entry in std::fs::read_dir(source)
            .map_err(|e| e.to_string())?
            .flatten()
        {
            copy_entry_with(&entry.path(), &target.join(entry.file_name()), replace)?;
        }
        return Ok(());
    }

    // Write to a temporary name in the destination directory and rename into
    // place, so an interrupted copy never leaves a truncated file wearing the
    // real name.
    let parent = target.parent().unwrap_or(Path::new("."));
    let temp = parent.join(format!(
        ".{}.otto-{}",
        target
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default(),
        std::process::id()
    ));

    let copy = std::fs::copy(source, &temp).map_err(|e| e.to_string());
    if let Err(err) = copy {
        std::fs::remove_file(&temp).ok();
        return Err(err);
    }
    let landed = if replace {
        std::fs::rename(&temp, target)
    } else {
        rename_no_replace(&temp, target)
    };
    if let Err(err) = landed {
        std::fs::remove_file(&temp).ok();
        return Err(taken(err));
    }
    Ok(())
}

/// Remove a file, a symlink or a whole directory tree.
///
/// Only the last component of `path` is checked for being a symlink: a
/// directory on the way that is one is followed. That is fine for a path the
/// user chose, and wrong for one under a directory somebody else could have
/// planted — the trash deletes through [`crate::trash::delete_forever`]
/// instead, which walks by descriptor.
///
/// # Errors
///
/// The removal that failed, as text.
pub fn remove_entry(path: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.is_dir() && !meta.is_symlink() {
        std::fs::remove_dir_all(path).map_err(|e| e.to_string())
    } else {
        std::fs::remove_file(path).map_err(|e| e.to_string())
    }
}

/// `name 2`, `name 3`… : the first that does not exist in `dir`.
///
/// The suffix goes before the extension, so `photo.png` becomes `photo 2.png`
/// rather than `photo.png 2`.
pub fn unique_name(dir: &Path, name: &OsStr) -> PathBuf {
    let name = name.to_string_lossy();
    let (stem, ext) = match name.rsplit_once('.') {
        // A leading dot is the whole name of a hidden file, not an extension.
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name.as_ref(), String::new()),
    };

    for n in 2..10_000 {
        let candidate = dir.join(format!("{stem} {n}{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem} {}{ext}", std::process::id()))
}

/// `dir.join(name)`, or the next free numbered variant if that is taken.
///
/// Unlike [`unique_name`], which always numbers because every caller of it
/// already knows the plain name collides, this checks first, so a fresh
/// "untitled folder" doesn't open as "untitled folder 2" for no reason.
pub fn first_free_name(dir: &Path, name: &OsStr) -> PathBuf {
    let plain = dir.join(name);
    if plain.exists() {
        unique_name(dir, name)
    } else {
        plain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of our own under the system temp dir, removed on drop.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "otto-kit-fs-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_no_replace_rename_refuses_a_taken_name() {
        let dir = TempDir::new("rename");
        let (a, b) = (dir.0.join("a.txt"), dir.0.join("b.txt"));
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();

        let err = rename_no_replace(&a, &b).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "a");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "b");

        let c = dir.0.join("c.txt");
        rename_no_replace(&a, &c).unwrap();
        assert!(!a.exists());
        assert_eq!(std::fs::read_to_string(&c).unwrap(), "a");
    }

    #[test]
    fn a_dangling_symlink_is_a_taken_name() {
        let dir = TempDir::new("dangling");
        let (a, link) = (dir.0.join("a.txt"), dir.0.join("link"));
        std::fs::write(&a, "a").unwrap();
        std::os::unix::fs::symlink(dir.0.join("nowhere"), &link).unwrap();

        assert_eq!(move_entry(&a, &link).unwrap_err(), NAME_TAKEN);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "a");
        assert!(link.symlink_metadata().unwrap().is_symlink());
    }

    #[test]
    fn a_move_never_replaces_but_an_asked_for_replace_does() {
        let dir = TempDir::new("move");
        let (a, b) = (dir.0.join("a.txt"), dir.0.join("b.txt"));
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();

        assert_eq!(move_entry(&a, &b).unwrap_err(), NAME_TAKEN);
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "b");

        move_entry_replacing(&a, &b).unwrap();
        assert!(!a.exists());
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "a");
    }

    #[test]
    fn a_no_replace_copy_does_not_merge_into_a_folder() {
        let dir = TempDir::new("copy");
        let (src, dst) = (dir.0.join("src"), dir.0.join("dst"));
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("f"), "new").unwrap();
        std::fs::create_dir(&dst).unwrap();
        std::fs::write(dst.join("f"), "old").unwrap();

        assert_eq!(copy_entry_with(&src, &dst, false).unwrap_err(), NAME_TAKEN);
        assert_eq!(std::fs::read_to_string(dst.join("f")).unwrap(), "old");
    }
}
