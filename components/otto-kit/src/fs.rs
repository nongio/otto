//! Moving, copying and naming files the way a file manager does.
//!
//! Shared by everything that puts files somewhere on someone's behalf: a paste
//! in Files, a drop on the dock's Trash. Each helper works on one entry, a file,
//! a symlink or a whole directory tree, and reports failure as a message fit
//! to show.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Move one entry, falling back to copy-then-delete across filesystems.
///
/// The source is unlinked only after the destination is fully written. That
/// ordering is the whole guarantee that a failed move cannot lose data.
///
/// # Errors
///
/// The rename, copy or removal that failed, as text.
pub fn move_entry(source: &Path, target: &Path) -> Result<(), String> {
    match std::fs::rename(source, target) {
        Ok(()) => Ok(()),
        // EXDEV: a rename cannot cross filesystems, so do it the long way.
        Err(err) if err.raw_os_error() == Some(libc::EXDEV) => {
            copy_entry(source, target)?;
            remove_entry(source)
        }
        Err(err) => Err(err.to_string()),
    }
}

/// Copy a file or a directory tree.
///
/// A symlink is copied as a link, never followed.
///
/// # Errors
///
/// The first read, write or rename that failed, as text.
pub fn copy_entry(source: &Path, target: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(source).map_err(|e| e.to_string())?;

    if meta.is_symlink() {
        // Copy the link itself, not what it points at: following it could
        // duplicate a whole tree the user did not ask for.
        let link = std::fs::read_link(source).map_err(|e| e.to_string())?;
        return std::os::unix::fs::symlink(link, target).map_err(|e| e.to_string());
    }

    if meta.is_dir() {
        std::fs::create_dir_all(target).map_err(|e| e.to_string())?;
        for entry in std::fs::read_dir(source)
            .map_err(|e| e.to_string())?
            .flatten()
        {
            copy_entry(&entry.path(), &target.join(entry.file_name()))?;
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
    if let Err(err) = std::fs::rename(&temp, target) {
        std::fs::remove_file(&temp).ok();
        return Err(err.to_string());
    }
    Ok(())
}

/// Remove a file, a symlink or a whole directory tree.
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
