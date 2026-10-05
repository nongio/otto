//! Throwing files away into the freedesktop trash can.
//!
//! An item in the trash lives under `files/`, and beside it under `info/` sits
//! a `.trashinfo` sidecar recording where it came from and when it left, so
//! any spec-compliant file manager can put it back.
//!
//! There is more than one can. The home trash, `$XDG_DATA_HOME/Trash`, takes
//! everything from the filesystem it lives on. A file on another filesystem (a
//! USB stick, a second disk) goes to a can at the top of *that* filesystem
//! instead, `$topdir/.Trash/$uid` or `$topdir/.Trash-$uid`, so trashing it is a
//! rename rather than a copy of the whole tree into the home folder. Only when
//! neither can be used does it fall back to the home trash. [`cans`] lists
//! every can that exists, for whoever shows or empties the trash.
//!
//! # Topdir cans are not trusted as found
//!
//! The home trash is in the user's own home and is taken as it stands. A
//! topdir can is on a filesystem somebody else may have written: a USB stick
//! prepared on another machine, or `/tmp`, which every user can write to. A
//! `.Trash-1000/files` planted there as a symlink to `/home/victim` would make
//! Empty Trash delete the victim's home. So a topdir can is only used —
//! trashed into, listed, counted, emptied or restored from — when it passes
//! one check, [`Can::open`]:
//!
//! - `.Trash-$uid`, or `$uid` under `.Trash`, is a real directory (not a
//!   symlink), owned by the user, and not writable by group or others;
//! - `.Trash` itself, for the shared layout, is a real, sticky directory;
//! - `files/` and `info/`, when present, pass the same test as the can, and
//!   `directorysizes`, when present, is a regular file of the user's.
//!
//! Every operation then works relative to directory descriptors opened with
//! `O_NOFOLLOW` along that walk, so nothing can be swapped for a symlink
//! between the check and the use.
//!
//! The spec is <https://specifications.freedesktop.org/trash/latest/>.

use std::ffi::{OsStr, OsString};
use std::fs::{DirBuilder, File};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rustix::fs::{self as rfs, AtFlags, FileType, Mode, OFlags};
use rustix::io::Errno;

use crate::fs::move_entry;

/// `$XDG_DATA_HOME/Trash`, falling back to `~/.local/share/Trash`: the
/// "home trashcan" the freedesktop Trash spec describes.
pub fn home_trash_dir() -> Option<PathBuf> {
    Some(crate::xdg::data_home()?.join("Trash"))
}

/// One trash can: a directory holding `files/` and `info/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Can {
    dir: PathBuf,
    kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    /// The home trash, or a can a test made somewhere private: trusted as it
    /// stands.
    Home,
    /// A can at the top of a mounted filesystem, `$topdir/.Trash/$uid` when
    /// `shared`, `$topdir/.Trash-$uid` otherwise, which must belong to `uid`.
    Topdir {
        topdir: PathBuf,
        shared: bool,
        uid: u32,
    },
}

/// Why an item cannot be put back where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadOrigin {
    /// No sidecar, or one that records no `Path=`.
    Unknown,
    /// A topdir can's sidecar records a path outside its own filesystem:
    /// absolute, or climbing out with `..`.
    Outside,
}

/// How a directory on the way to a can is opened: never by following a
/// symlink in its last component.
const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NOFOLLOW);

impl Can {
    /// The home trash.
    pub fn home() -> Option<Self> {
        home_trash_dir().map(Self::trusted)
    }

    /// A can at `dir` treated like the home trash: created on demand, its
    /// layout taken as it stands.
    fn trusted(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            kind: Kind::Home,
        }
    }

    /// `$topdir/.Trash/$uid` (when `shared`) or `$topdir/.Trash-$uid`.
    fn topdir(topdir: &Path, shared: bool, uid: u32) -> Self {
        let dir = if shared {
            topdir.join(".Trash").join(uid.to_string())
        } else {
            topdir.join(format!(".Trash-{uid}"))
        };
        Self {
            dir,
            kind: Kind::Topdir {
                topdir: topdir.to_path_buf(),
                shared,
                uid,
            },
        }
    }

    /// What `dir` is, by its name alone: the home trash, a topdir can of the
    /// user `uid`, or not a can at all. Nothing on disk is looked at.
    fn classify(dir: &Path, uid: u32) -> Option<Self> {
        if home_trash_dir().as_deref() == Some(dir) {
            return Some(Self::trusted(dir));
        }
        let name = dir.file_name()?;
        let parent = dir.parent()?;
        if name.as_bytes() == format!(".Trash-{uid}").as_bytes() {
            return Some(Self::topdir(parent, false, uid));
        }
        if name.as_bytes() == uid.to_string().as_bytes()
            && parent.file_name() == Some(OsStr::new(".Trash"))
        {
            return Some(Self::topdir(parent.parent()?, true, uid));
        }
        None
    }

    /// The can an item in the trash belongs to: `item` is `<can>/files/<name>`.
    ///
    /// Only a can that passes [`Self::open`]'s checks is returned: an item in
    /// a can that does not is not something this module will act on.
    pub fn of_item(item: &Path) -> Option<Self> {
        let files = item.parent()?;
        if files.file_name()? != "files" {
            return None;
        }
        let can = Self::classify(files.parent()?, uid())?;
        can.open(false).is_ok().then_some(can)
    }

    /// The can holding `item`, which is either one of its items or anything
    /// inside a trashed folder, and `item`'s path relative to `files/`.
    fn containing(item: &Path, uid: u32) -> Option<(Self, PathBuf)> {
        let ancestors: Vec<&Path> = item.ancestors().skip(1).collect();
        ancestors.into_iter().rev().find_map(|files| {
            if files.file_name()? != "files" {
                return None;
            }
            let can = Self::classify(files.parent()?, uid)?;
            let relative = item.strip_prefix(files).ok()?;
            let plain = relative
                .components()
                .all(|part| matches!(part, Component::Normal(_)));
            (plain && !relative.as_os_str().is_empty()).then(|| (can, relative.to_path_buf()))
        })
    }

    /// The can's own directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Where the trashed items are.
    pub fn files_dir(&self) -> PathBuf {
        self.dir.join("files")
    }

    /// Where their sidecars are.
    pub fn info_dir(&self) -> PathBuf {
        self.dir.join("info")
    }

    /// The sidecar of the item called `name` in this can.
    pub fn sidecar(&self, name: &OsStr) -> PathBuf {
        self.info_dir().join(sidecar_name(name))
    }

    /// The directory a relative `Path=` is relative to: the one the can
    /// resides in, which for `$topdir/.Trash/$uid` is `$topdir`, not
    /// `$topdir/.Trash`.
    pub fn base(&self) -> Option<&Path> {
        match &self.kind {
            Kind::Home => self.dir.parent(),
            Kind::Topdir { topdir, .. } => Some(topdir),
        }
    }

    /// Where the item called `name` came from, from its sidecar.
    ///
    /// # Errors
    ///
    /// As [`Self::origin_in`], and [`BadOrigin::Unknown`] when there is no
    /// sidecar to read.
    pub fn origin(&self, name: &OsStr) -> Result<PathBuf, BadOrigin> {
        let body = std::fs::read_to_string(self.sidecar(name)).map_err(|_| BadOrigin::Unknown)?;
        self.origin_in(&body)
    }

    /// The origin a sidecar's body records, resolved against [`Self::base`]
    /// when it is relative.
    ///
    /// A topdir can's sidecars are as trustworthy as the filesystem they are
    /// on, which may be a stick somebody else prepared, so only a relative
    /// path that stays under the topdir is taken from one: plain names, no
    /// `..`, nothing absolute. Put Back would otherwise move a file, and
    /// make directories, wherever the sidecar said.
    ///
    /// # Errors
    ///
    /// [`BadOrigin::Unknown`] when no `Path=` is recorded,
    /// [`BadOrigin::Outside`] when a topdir can records one it may not.
    pub fn origin_in(&self, body: &str) -> Result<PathBuf, BadOrigin> {
        let recorded = path_key(body).ok_or(BadOrigin::Unknown)?;
        if self.owner().is_some() {
            let plain = !recorded.as_os_str().is_empty()
                && recorded
                    .components()
                    .all(|part| matches!(part, Component::Normal(_)));
            if !plain {
                return Err(BadOrigin::Outside);
            }
        }
        if recorded.is_absolute() {
            Ok(recorded)
        } else {
            Ok(self.base().ok_or(BadOrigin::Unknown)?.join(recorded))
        }
    }

    /// The user a topdir can has to belong to; `None` for a trusted one.
    fn owner(&self) -> Option<u32> {
        match self.kind {
            Kind::Home => None,
            Kind::Topdir { uid, .. } => Some(uid),
        }
    }

    /// Open the can's directory, checking a topdir can on the way (see the
    /// module documentation), and making it first when `create` is set.
    ///
    /// This is the one check every use of a can goes through.
    fn open(&self, create: bool) -> io::Result<OwnedFd> {
        let Kind::Topdir {
            topdir,
            shared,
            uid,
        } = &self.kind
        else {
            if create {
                DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(&self.dir)?;
            }
            // The user's own: a symlinked home trash is their choice.
            return Ok(rfs::open(
                &self.dir,
                DIR_FLAGS.difference(OFlags::NOFOLLOW),
                Mode::empty(),
            )?);
        };
        // The mount point itself comes from the mount table, not from the
        // filesystem being checked, so it may be reached through a symlink.
        let top = rfs::open(
            topdir.as_path(),
            DIR_FLAGS.difference(OFlags::NOFOLLOW),
            Mode::empty(),
        )?;
        let parent = if *shared {
            let shared = rfs::openat(&top, ".Trash", DIR_FLAGS, Mode::empty())?;
            if !is_sticky_dir(&rfs::fstat(&shared)?) {
                return Err(refused("$topdir/.Trash is not a sticky directory"));
            }
            shared
        } else {
            top
        };
        let name = self
            .dir
            .file_name()
            .ok_or_else(|| refused("a can without a name"))?;
        if create {
            match rfs::mkdirat(&parent, name, Mode::RWXU) {
                Ok(()) | Err(Errno::EXIST) => {}
                Err(err) => return Err(err.into()),
            }
        }
        let can = rfs::openat(&parent, name, DIR_FLAGS, Mode::empty())?;
        check_private(&rfs::fstat(&can)?, *uid)?;
        for child in ["files", "info"] {
            match rfs::statat(&can, child, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat) => check_private(&stat, *uid)?,
                Err(Errno::NOENT) => {}
                Err(err) => return Err(err.into()),
            }
        }
        match rfs::statat(&can, "directorysizes", AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat)
                if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
                    || stat.st_uid != *uid =>
            {
                return Err(refused("directorysizes is not a file of the user's"));
            }
            Ok(_) | Err(Errno::NOENT) => {}
            Err(err) => return Err(err.into()),
        }
        Ok(can)
    }

    /// `files` or `info` in the can open as `can`, made (`0700`, not
    /// recursively) first when `create` is set, and checked like the can.
    fn subdir(&self, can: &OwnedFd, name: &str, create: bool) -> io::Result<OwnedFd> {
        if create {
            match rfs::mkdirat(can, name, Mode::RWXU) {
                Ok(()) | Err(Errno::EXIST) => {}
                Err(err) => return Err(err.into()),
            }
        }
        let Some(uid) = self.owner() else {
            return Ok(rfs::openat(
                can,
                name,
                DIR_FLAGS.difference(OFlags::NOFOLLOW),
                Mode::empty(),
            )?);
        };
        let dir = rfs::openat(can, name, DIR_FLAGS, Mode::empty())?;
        check_private(&rfs::fstat(&dir)?, uid)?;
        Ok(dir)
    }

    /// Drop the `directorysizes` entry for `name`, once it has left the can.
    /// The cache is advisory, so a failure is not reported.
    pub fn forget_directory_size(&self, name: &OsStr) {
        if let Ok(can) = self.open(false) {
            update_directory_sizes(&can, name, None);
        }
    }

    /// Record the size of the trashed directory `name` in `directorysizes`.
    ///
    /// The size is what `du -B1` would say. Walking a huge tree on the
    /// trashing thread would cost far more than the rename did, so past
    /// [`SIZE_WALK_LIMIT`] entries nothing is written: a missing entry is what
    /// any reader of the cache already copes with, by computing it.
    fn record_directory_size(&self, can: &OwnedFd, name: &OsStr) {
        let Ok(info) = std::fs::symlink_metadata(self.sidecar(name)) else {
            return;
        };
        let mut budget = SIZE_WALK_LIMIT;
        let Some(size) = disk_usage(&self.files_dir().join(name), &mut budget) else {
            return;
        };
        update_directory_sizes(can, name, Some((size, info.mtime())));
    }
}

/// Why a can was not used, as an error.
fn refused(why: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, why)
}

/// A directory that is the user's alone: real, owned by `uid`, and not
/// writable by anybody else.
fn check_private(stat: &rfs::Stat, uid: u32) -> io::Result<()> {
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
        return Err(refused("not a directory"));
    }
    if stat.st_uid != uid {
        return Err(refused("owned by another user"));
    }
    if stat.st_mode & 0o022 != 0 {
        return Err(refused("writable by group or others"));
    }
    Ok(())
}

fn is_sticky_dir(stat: &rfs::Stat) -> bool {
    FileType::from_raw_mode(stat.st_mode) == FileType::Directory && stat.st_mode & 0o1000 != 0
}

fn sidecar_name(name: &OsStr) -> OsString {
    let mut file = name.to_os_string();
    file.push(".trashinfo");
    file
}

/// Rewrite `directorysizes` in the can open as `can` without `name`, plus
/// `entry` for it when given: to a temporary file, renamed into place, as the
/// spec requires.
///
/// The temporary is created with `O_EXCL | O_NOFOLLOW`, so a name planted in
/// advance (a symlink to a file of the user's) is never written through.
fn update_directory_sizes(can: &OwnedFd, name: &OsStr, entry: Option<(u64, i64)>) {
    let read = rfs::openat(
        can,
        "directorysizes",
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    );
    let existing = match read {
        Ok(fd) => {
            let mut text = String::new();
            if File::from(fd).read_to_string(&mut text).is_err() {
                return;
            }
            text
        }
        Err(Errno::NOENT) if entry.is_none() => return,
        Err(Errno::NOENT) => String::new(),
        Err(_) => return,
    };
    let mut out = String::with_capacity(existing.len() + 64);
    let mut changed = false;
    for line in existing.lines() {
        let listed = line
            .splitn(3, ' ')
            .nth(2)
            .map(|encoded| crate::uri::decode_path(encoded.trim_end()));
        if listed.as_deref() == Some(Path::new(name)) {
            changed = true;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if let Some((size, mtime)) = entry {
        let encoded = crate::uri::encode_path(Path::new(name));
        out.push_str(&format!("{size} {mtime} {encoded}\n"));
        changed = true;
    }
    if !changed {
        return;
    }

    for _ in 0..8 {
        let temp = format!(
            ".directorysizes.{}.{}",
            std::process::id(),
            DIRECTORYSIZES_SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        let file = match rfs::openat(
            can,
            temp.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        ) {
            Ok(fd) => File::from(fd),
            // Taken, by a leftover or by something planted: try another.
            Err(Errno::EXIST) => continue,
            Err(_) => return,
        };
        let written = (&file)
            .write_all(out.as_bytes())
            .is_ok_and(|()| rfs::renameat(can, temp.as_str(), can, "directorysizes").is_ok());
        if !written {
            rfs::unlinkat(can, temp.as_str(), AtFlags::empty()).ok();
        }
        return;
    }
}

/// Numbers the temporary files [`update_directory_sizes`] writes.
static DIRECTORYSIZES_SERIAL: AtomicU64 = AtomicU64::new(0);

/// How many entries [`Can::record_directory_size`] will stat before giving up.
const SIZE_WALK_LIMIT: usize = 10_000;

/// Disk usage of `path` and everything under it, in bytes of allocated
/// blocks, or `None` once more than `budget` entries have been looked at.
fn disk_usage(path: &Path, budget: &mut usize) -> Option<u64> {
    *budget = budget.checked_sub(1)?;
    let meta = std::fs::symlink_metadata(path).ok()?;
    let mut total = meta.blocks() * 512;
    if meta.is_dir() {
        for entry in std::fs::read_dir(path).ok()?.flatten() {
            total += disk_usage(&entry.path(), budget)?;
        }
    }
    Some(total)
}

/// The first `Path=` key of a `.trashinfo`, percent-decoded, as recorded
/// (relative or absolute).
fn path_key(body: &str) -> Option<PathBuf> {
    body.lines()
        .find_map(|line| line.strip_prefix("Path="))
        .map(|encoded| crate::uri::decode_path(encoded.trim()))
}

/// Every can that exists: the home trash first, then the topdir cans of the
/// mounted filesystems — `$topdir/.Trash/$uid` (only when `$topdir/.Trash`
/// passes the spec's checks) and `$topdir/.Trash-$uid`, both when both are
/// there. A topdir can that fails [`Can::open`]'s checks is left out: it is
/// not listed, counted or emptied.
///
/// The mounts are read from `/proc/self/mountinfo`, leaving out the kernel's
/// pseudo-filesystems and autofs (looking into an autofs mount point would
/// mount it). A can that is not mounted right now is not listed.
pub fn cans() -> Vec<Can> {
    let mut found: Vec<Can> = Can::home().into_iter().collect();
    let uid = uid();
    for topdir in mount_points() {
        for can in existing_topdir_cans(&topdir, uid) {
            if !found.contains(&can) {
                found.push(can);
            }
        }
    }
    found
}

/// Move `source` into the trash, choosing the can the spec says it belongs
/// in: the home trash for anything on the home trash's filesystem, the
/// topdir can of its own filesystem otherwise, and the home trash again when
/// no topdir can could be used.
///
/// Returns where the item landed and where its sidecar went.
///
/// # Errors
///
/// The directory creation, sidecar creation or move that failed, as text.
pub fn trash(source: &Path) -> Result<(PathBuf, PathBuf), String> {
    let home = home_trash_dir().ok_or_else(|| "no home folder to trash into".to_string())?;
    let source_dir = source
        .parent()
        .map(|dir| std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf()));
    if let (Some(dir), Some(name)) = (&source_dir, source.file_name()) {
        let home_dev = nearest_existing(&home).and_then(|p| device(&p));
        let source_dev = device(dir);
        if source_dev.is_some() && source_dev != home_dev {
            let topdir = topdir_of(dir, device);
            if let Some(can) = topdir_can(&topdir, uid()) {
                let resolved = dir.join(name);
                match place(source, &resolved, &can, Some(&topdir)) {
                    Ok(placed) => return Ok(placed),
                    Err(err) => tracing::debug!(
                        "trash: {} could not take {}, using the home trash: {err}",
                        can.dir().display(),
                        source.display()
                    ),
                }
            }
        }
    }
    trash_into(source, &home)
}

/// Move `source` into the trash can at `trash`, with its sidecar.
///
/// `trash` is the can itself, the directory holding `files/` and `info/`;
/// both are created if missing. A name already in the can gets a numbered
/// variant. The sidecar records the absolute path, which is what the home
/// trash takes. Returns where the item landed and where its sidecar went.
///
/// The name is reserved by creating the sidecar with `O_EXCL` before anything
/// moves, so two processes or threads trashing the same name at once each get
/// their own. If the move then fails, the sidecar is removed again.
fn trash_into(source: &Path, trash: &Path) -> Result<(PathBuf, PathBuf), String> {
    place(source, source, &Can::trusted(trash), None)
}

/// Trash `source` into `can`, recording `recorded` (the same file, perhaps
/// spelled through a canonical parent) relative to `relative_to` when given.
fn place(
    source: &Path,
    recorded: &Path,
    can: &Can,
    relative_to: Option<&Path>,
) -> Result<(PathBuf, PathBuf), String> {
    let name = source
        .file_name()
        .ok_or_else(|| format!("{} has no name to trash it under", source.display()))?;
    let dir = can.open(true).map_err(|e| e.to_string())?;
    let files = can.subdir(&dir, "files", true).map_err(|e| e.to_string())?;
    let info = can.subdir(&dir, "info", true).map_err(|e| e.to_string())?;

    let path_value = match relative_to.and_then(|base| recorded.strip_prefix(base).ok()) {
        Some(relative) if !relative.as_os_str().is_empty() => relative,
        _ => recorded,
    };
    let body = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        crate::uri::encode_path(path_value),
        deletion_date(),
    );

    for candidate in candidate_names(name) {
        let sidecar = sidecar_name(&candidate);
        let file = match rfs::openat(
            &info,
            sidecar.as_os_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        ) {
            Ok(fd) => File::from(fd),
            Err(Errno::EXIST) => continue,
            Err(err) => return Err(io::Error::from(err).to_string()),
        };
        let drop_sidecar = || {
            rfs::unlinkat(&info, sidecar.as_os_str(), AtFlags::empty()).ok();
        };
        // An item without a sidecar (left by a crash, or another program)
        // still owns its name: never move on top of it.
        if rfs::statat(&files, candidate.as_os_str(), AtFlags::SYMLINK_NOFOLLOW).is_ok() {
            drop(file);
            drop_sidecar();
            continue;
        }
        if let Err(err) = (&file).write_all(body.as_bytes()) {
            drop(file);
            drop_sidecar();
            return Err(err.to_string());
        }
        drop(file);
        let target = can.files_dir().join(&candidate);
        let moved = if can.owner().is_some() {
            // Into the directory that was checked, not whatever its path
            // names by now. A topdir can is on the source's own filesystem,
            // so this is always a rename.
            rfs::renameat(rfs::CWD, source, &files, candidate.as_os_str())
                .map_err(|e| io::Error::from(e).to_string())
        } else {
            move_entry(source, &target)
        };
        if let Err(err) = moved {
            drop_sidecar();
            return Err(err);
        }
        if rfs::statat(&files, candidate.as_os_str(), AtFlags::SYMLINK_NOFOLLOW)
            .is_ok_and(|stat| FileType::from_raw_mode(stat.st_mode) == FileType::Directory)
        {
            can.record_directory_size(&dir, &candidate);
        }
        return Ok((target, can.sidecar(&candidate)));
    }
    Err(format!("no free name in {}", can.files_dir().display()))
}

/// Delete `item` from the trash for good: one of a can's items, with its
/// sidecar and `directorysizes` line, or anything inside a trashed folder.
///
/// The can is checked first and the deletion walks down from its checked
/// `files/` by descriptor, never following a symlink, so it cannot reach
/// outside the can whatever has been planted in it.
///
/// # Errors
///
/// `item` is not in a usable can, or the removal failed, as text.
pub fn delete_forever(item: &Path) -> Result<(), String> {
    delete_forever_as(item, uid())
}

fn delete_forever_as(item: &Path, uid: u32) -> Result<(), String> {
    let (can, relative) =
        Can::containing(item, uid).ok_or_else(|| "not in the trash".to_string())?;
    let dir = can.open(false).map_err(|e| e.to_string())?;
    let files = can
        .subdir(&dir, "files", false)
        .map_err(|e| e.to_string())?;
    let mut parts: Vec<&OsStr> = relative.iter().collect();
    let name = parts.pop().ok_or_else(|| "not in the trash".to_string())?;
    let mut parent = files;
    for part in parts {
        parent = rfs::openat(&parent, part, DIR_FLAGS, Mode::empty())
            .map_err(|e| io::Error::from(e).to_string())?;
    }
    remove_tree_at(parent.as_fd(), name).map_err(|e| e.to_string())?;
    if relative.components().count() == 1 {
        if let Ok(info) = can.subdir(&dir, "info", false) {
            rfs::unlinkat(&info, sidecar_name(name).as_os_str(), AtFlags::empty()).ok();
        }
        update_directory_sizes(&dir, name, None);
    }
    Ok(())
}

/// Remove `name` under `parent`, and everything under it when it is a real
/// directory. A symlink is removed, never followed.
fn remove_tree_at(parent: BorrowedFd<'_>, name: &OsStr) -> io::Result<()> {
    let stat = rfs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
        return Ok(rfs::unlinkat(parent, name, AtFlags::empty())?);
    }
    let dir = rfs::openat(parent, name, DIR_FLAGS, Mode::empty())?;
    let mut children = Vec::new();
    for entry in rfs::Dir::read_from(&dir)? {
        let entry = entry?;
        let child = entry.file_name().to_bytes();
        if child != b"." && child != b".." {
            children.push(OsString::from_vec(child.to_vec()));
        }
    }
    for child in children {
        remove_tree_at(dir.as_fd(), &child)?;
    }
    Ok(rfs::unlinkat(parent, name, AtFlags::REMOVEDIR)?)
}

/// `name`, then `name 2`, `name 3`…, numbered before the extension the way
/// [`crate::fs::unique_name`] does, ending in a name no one else would pick.
fn candidate_names(name: &OsStr) -> impl Iterator<Item = OsString> + '_ {
    let text = name.to_string_lossy().into_owned();
    let (stem, ext) = match text.rsplit_once('.') {
        // A leading dot is the whole name of a hidden file, not an extension.
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
        _ => (text.clone(), String::new()),
    };
    // The plain name keeps its exact bytes; only the numbered ones go through
    // the lossy rendering.
    std::iter::once(name.to_os_string())
        .chain((2..10_000).map(move |n| OsString::from(format!("{stem} {n}{ext}"))))
        .chain(std::iter::once_with(move || {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default();
            OsString::from(format!("{} {}-{nanos}", text, std::process::id()))
        }))
}

/// The top directory of the filesystem `dir` is on: the highest ancestor
/// whose device is still `dir`'s. `device` is how a path's device is read,
/// passed in so the walk can be tested without mounting anything.
fn topdir_of(dir: &Path, device: impl Fn(&Path) -> Option<u64>) -> PathBuf {
    let dev = device(dir);
    let mut top = dir;
    while let Some(up) = top.parent() {
        if device(up) != dev {
            break;
        }
        top = up;
    }
    top.to_path_buf()
}

/// The can to trash into on the filesystem whose top is `topdir`, creating
/// it as the spec says: `$topdir/.Trash/$uid` when an administrator made a
/// `.Trash` that passes the checks, `$topdir/.Trash-$uid` otherwise. `None`
/// when neither could be made, or neither passes [`Can::open`]'s checks —
/// one somebody else made for this user is not used.
fn topdir_can(topdir: &Path, uid: u32) -> Option<Can> {
    let shared = topdir.join(".Trash");
    if std::fs::symlink_metadata(&shared).is_ok() {
        let can = Can::topdir(topdir, true, uid);
        match can.open(true) {
            Ok(_) => return Some(can),
            Err(err) => tracing::warn!(
                can = %can.dir().display(),
                error = %err,
                "trash: not using the shared topdir can"
            ),
        }
    }
    let can = Can::topdir(topdir, false, uid);
    match can.open(true) {
        Ok(_) => Some(can),
        Err(err) => {
            tracing::warn!(
                can = %can.dir().display(),
                error = %err,
                "trash: not using the topdir can"
            );
            None
        }
    }
}

/// The topdir cans already on the filesystem whose top is `topdir` that pass
/// [`Can::open`]'s checks.
fn existing_topdir_cans(topdir: &Path, uid: u32) -> Vec<Can> {
    [true, false]
        .into_iter()
        .map(|shared| Can::topdir(topdir, shared, uid))
        .filter(|can| match can.open(false) {
            Ok(_) => true,
            Err(err) if err.kind() == io::ErrorKind::NotFound => false,
            Err(err) => {
                tracing::debug!(
                    can = %can.dir().display(),
                    error = %err,
                    "trash: passing over a topdir can"
                );
                false
            }
        })
        .collect()
}

fn device(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|meta| meta.dev())
}

/// `path`, or its nearest ancestor that exists: the home trash may not have
/// been made yet, but its filesystem is already decided.
fn nearest_existing(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|candidate| candidate.exists())
        .map(Path::to_path_buf)
}

fn uid() -> u32 {
    rustix::process::getuid().as_raw()
}

/// Filesystem types with no user files on them, never worth a look.
const PSEUDO_FILESYSTEMS: &[&str] = &[
    "autofs",
    "binfmt_misc",
    "bpf",
    "cgroup",
    "cgroup2",
    "configfs",
    "debugfs",
    "devpts",
    "devtmpfs",
    "efivarfs",
    "fusectl",
    "hugetlbfs",
    "mqueue",
    "nsfs",
    "proc",
    "pstore",
    "rpc_pipefs",
    "securityfs",
    "sysfs",
    "tracefs",
];

/// Filesystems that may be across a network, or behind a FUSE daemon: a stat
/// of one can hang for as long as the server or the daemon does, and the
/// dock asks about the trash from the compositor. Their cans are not looked
/// for, the way gio leaves network mounts out of its trash. `fuse.*` (sshfs,
/// rclone, gvfsd-fuse…) is matched by prefix; `fuseblk`, a local disk such
/// as an NTFS stick through ntfs-3g, is not FUSE-over-network and stays in.
const NETWORK_FILESYSTEMS: &[&str] = &[
    "9p",
    "afs",
    "ceph",
    "cifs",
    "coda",
    "davfs",
    "fuse",
    "glusterfs",
    "lustre",
    "ncpfs",
    "nfs",
    "nfs4",
    "ocfs2",
    "smb3",
    "smbfs",
    "sshfs",
];

/// Whether a filesystem of type `fstype` is never searched for cans.
fn is_skipped(fstype: &str) -> bool {
    PSEUDO_FILESYSTEMS.contains(&fstype)
        || NETWORK_FILESYSTEMS.contains(&fstype)
        || fstype.starts_with("fuse.")
}

/// The mount points in `/proc/self/mountinfo`, less the filesystems
/// [`is_skipped`] leaves out.
fn mount_points() -> Vec<PathBuf> {
    std::fs::read_to_string("/proc/self/mountinfo")
        .map(|text| parse_mountinfo(&text))
        .unwrap_or_default()
}

fn parse_mountinfo(text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter_map(|line| {
            let (before, after) = line.split_once(" - ")?;
            let fstype = after.split(' ').next()?;
            if is_skipped(fstype) {
                return None;
            }
            before.split(' ').nth(4).map(unescape_mount)
        })
        .collect()
}

/// Undo mountinfo's octal escapes (`\040` for a space and so on).
fn unescape_mount(field: &str) -> PathBuf {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 4 <= bytes.len() {
            let digits = &bytes[i + 1..i + 4];
            if digits.iter().all(|d| (b'0'..=b'7').contains(d)) {
                let value = digits
                    .iter()
                    .fold(0u32, |acc, d| acc * 8 + u32::from(d - b'0'));
                if let Ok(value) = u8::try_from(value) {
                    out.push(value);
                    i += 4;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    PathBuf::from(OsString::from_vec(out))
}

/// Local time as `YYYY-MM-DDTHH:MM:SS`, the format a `.trashinfo`'s
/// `DeletionDate` key requires.
fn deletion_date() -> String {
    // SAFETY: `tm` is plain data and `localtime_r` fills every field read back.
    unsafe {
        let mut when: libc::time_t = 0;
        libc::time(&mut when);
        let mut broken: libc::tm = std::mem::zeroed();
        libc::localtime_r(&when, &mut broken);
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            broken.tm_year + 1900,
            broken.tm_mon + 1,
            broken.tm_mday,
            broken.tm_hour,
            broken.tm_min,
            broken.tm_sec,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::os::unix::fs::PermissionsExt;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("otto-kit-trash-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Tmp(root)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn a_trashed_file_lands_in_the_can_with_a_sidecar() {
        let root = Tmp::new("basic");
        let source = root.0.join("a b.txt");
        std::fs::write(&source, "x").unwrap();
        let trash = root.0.join("Trash");

        let (to, info) = trash_into(&source, &trash).unwrap();

        assert!(!source.exists(), "the original is gone");
        assert_eq!(to, trash.join("files/a b.txt"));
        let sidecar = std::fs::read_to_string(&info).unwrap();
        assert!(sidecar.contains(&format!("Path={}", crate::uri::encode_path(&source))));

        // A second item under the same name is numbered, not overwritten.
        std::fs::write(&source, "y").unwrap();
        let (again, _) = trash_into(&source, &trash).unwrap();
        assert_eq!(again, trash.join("files/a b 2.txt"));
    }

    /// The spec's reason for `O_EXCL`: many trashes of one name at once all
    /// land, each under its own name, none on top of another.
    #[test]
    fn concurrent_trashes_of_one_name_all_land_apart() {
        let root = Tmp::new("race");
        let trash = root.0.join("Trash");
        const THREADS: usize = 24;
        let sources: Vec<PathBuf> = (0..THREADS)
            .map(|i| {
                let dir = root.0.join(format!("from-{i}"));
                std::fs::create_dir_all(&dir).unwrap();
                let file = dir.join("same.txt");
                std::fs::write(&file, i.to_string()).unwrap();
                file
            })
            .collect();

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(THREADS));
        let handles: Vec<_> = sources
            .iter()
            .cloned()
            .map(|source| {
                let trash = trash.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    trash_into(&source, &trash).map(|landed| (source, landed))
                })
            })
            .collect();
        let landed: Vec<_> = handles
            .into_iter()
            .map(|h| h.join().unwrap().expect("every trash succeeds"))
            .collect();

        let names: HashSet<_> = landed.iter().map(|(_, (to, _))| to.clone()).collect();
        assert_eq!(names.len(), THREADS, "every item has its own name");
        for (source, (to, info)) in &landed {
            // Each item is the one its sidecar says it is.
            let i: usize = std::fs::read_to_string(to).unwrap().parse().unwrap();
            assert_eq!(source, &sources[i]);
            let can = Can::trusted(&trash);
            assert_eq!(can.origin(to.file_name().unwrap()).as_ref(), Ok(source));
            assert!(info.exists());
        }
        let in_can = std::fs::read_dir(trash.join("files")).unwrap().count();
        assert_eq!(in_can, THREADS);
    }

    /// An item already in `files/` without a sidecar keeps its name.
    #[test]
    fn an_orphan_item_is_never_overwritten() {
        let root = Tmp::new("orphan");
        let trash = root.0.join("Trash");
        std::fs::create_dir_all(trash.join("files")).unwrap();
        std::fs::write(trash.join("files/x.txt"), "orphan").unwrap();
        let source = root.0.join("x.txt");
        std::fs::write(&source, "new").unwrap();

        let (to, _) = trash_into(&source, &trash).unwrap();

        assert_eq!(to, trash.join("files/x 2.txt"));
        assert_eq!(
            std::fs::read_to_string(trash.join("files/x.txt")).unwrap(),
            "orphan"
        );
        assert!(!trash.join("info/x.txt.trashinfo").exists());
    }

    /// A failed move takes its reservation back.
    #[test]
    fn a_failed_move_leaves_no_sidecar() {
        let root = Tmp::new("fail");
        let trash = root.0.join("Trash");
        let missing = root.0.join("never-there.txt");

        assert!(trash_into(&missing, &trash).is_err());

        let sidecars = std::fs::read_dir(trash.join("info")).unwrap().count();
        assert_eq!(sidecars, 0);
    }

    #[test]
    fn the_topdir_is_the_highest_ancestor_on_the_same_device() {
        let devices: HashMap<&Path, u64> = [
            (Path::new("/"), 1),
            (Path::new("/media"), 1),
            (Path::new("/media/usb"), 2),
            (Path::new("/media/usb/photos"), 2),
            (Path::new("/media/usb/photos/2024"), 2),
            (Path::new("/home"), 3),
            (Path::new("/home/u"), 3),
        ]
        .into_iter()
        .collect();
        let device = |p: &Path| devices.get(p).copied();

        assert_eq!(
            topdir_of(Path::new("/media/usb/photos/2024"), device),
            Path::new("/media/usb")
        );
        assert_eq!(
            topdir_of(Path::new("/media/usb"), device),
            Path::new("/media/usb")
        );
        assert_eq!(topdir_of(Path::new("/home/u"), device), Path::new("/home"));
        assert_eq!(topdir_of(Path::new("/media"), device), Path::new("/"));
    }

    #[test]
    fn a_topdir_without_a_shared_trash_gets_a_private_one() {
        let me = uid();
        let top = Tmp::new("topdir-own");

        let can = topdir_can(&top.0, me).unwrap();

        assert_eq!(can.dir(), top.0.join(format!(".Trash-{me}")));
        let mode = std::fs::metadata(can.dir()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        assert_eq!(can.base(), Some(top.0.as_path()));
        assert_eq!(existing_topdir_cans(&top.0, me), vec![can]);
    }

    #[test]
    fn a_sticky_shared_trash_is_used_per_user() {
        let me = uid();
        let top = Tmp::new("topdir-shared");
        let shared = top.0.join(".Trash");
        std::fs::create_dir(&shared).unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o1777)).unwrap();

        let can = topdir_can(&top.0, me).unwrap();

        assert_eq!(can.dir(), shared.join(me.to_string()));
        assert_eq!(can.base(), Some(top.0.as_path()), "relative to topdir");
        assert!(!top.0.join(format!(".Trash-{me}")).exists());
    }

    #[test]
    fn a_shared_trash_failing_the_checks_is_passed_over() {
        let me = uid();
        // Not sticky.
        let top = Tmp::new("topdir-loose");
        std::fs::create_dir(top.0.join(".Trash")).unwrap();
        std::fs::set_permissions(top.0.join(".Trash"), std::fs::Permissions::from_mode(0o777))
            .unwrap();
        assert_eq!(
            topdir_can(&top.0, me).unwrap().dir(),
            top.0.join(format!(".Trash-{me}"))
        );
        assert!(!top.0.join(format!(".Trash/{me}")).exists());

        // A symlink, even to a sticky directory.
        let top = Tmp::new("topdir-link");
        let elsewhere = top.0.join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::fs::set_permissions(&elsewhere, std::fs::Permissions::from_mode(0o1777)).unwrap();
        std::os::unix::fs::symlink(&elsewhere, top.0.join(".Trash")).unwrap();
        assert_eq!(
            topdir_can(&top.0, me).unwrap().dir(),
            top.0.join(format!(".Trash-{me}"))
        );
        assert!(existing_topdir_cans(&top.0, me)
            .iter()
            .all(|can| !can.dir().starts_with(top.0.join(".Trash/"))));
    }

    /// A topdir can records `Path=` relative to the topdir, and reads it
    /// back to the absolute origin.
    #[test]
    fn a_topdir_can_records_a_relative_path() {
        let me = uid();
        let top = Tmp::new("topdir-relative");
        let dir = top.0.join("photos");
        std::fs::create_dir(&dir).unwrap();
        let source = dir.join("cat.png");
        std::fs::write(&source, "meow").unwrap();
        let can = topdir_can(&top.0, me).unwrap();

        let (to, info) = place(&source, &source, &can, Some(&top.0)).unwrap();

        let body = std::fs::read_to_string(info).unwrap();
        assert!(body.contains("\nPath=photos/cat.png\n"), "{body}");
        let found = Can::of_item(&to).unwrap();
        assert_eq!(found, can);
        assert_eq!(found.origin(to.file_name().unwrap()), Ok(source));
    }

    #[test]
    fn a_trashed_directory_is_listed_in_directorysizes() {
        let root = Tmp::new("dirsizes");
        let trash = root.0.join("Trash");
        let folder = root.0.join("a folder");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("inside"), vec![7u8; 10_000]).unwrap();

        let (to, info) = trash_into(&folder, &trash).unwrap();

        let sizes = std::fs::read_to_string(trash.join("directorysizes")).unwrap();
        let mut fields = sizes.lines().next().unwrap().splitn(3, ' ');
        let size: u64 = fields.next().unwrap().parse().unwrap();
        let mtime: i64 = fields.next().unwrap().parse().unwrap();
        assert_eq!(fields.next(), Some("a%20folder"));
        assert!(size >= 10_000, "counts the blocks under it: {size}");
        assert_eq!(mtime, std::fs::metadata(info).unwrap().mtime());

        // A file is not listed; forgetting the folder empties the cache.
        let file = root.0.join("plain");
        std::fs::write(&file, "x").unwrap();
        trash_into(&file, &trash).unwrap();
        Can::trusted(&trash).forget_directory_size(to.file_name().unwrap());
        assert_eq!(
            std::fs::read_to_string(trash.join("directorysizes")).unwrap(),
            ""
        );
    }

    #[test]
    fn a_sidecar_without_a_path_is_skipped_rather_than_guessed() {
        let can = Can::trusted("/x/Trash");
        assert_eq!(
            can.origin_in("[Trash Info]\nDeletionDate=x\n"),
            Err(BadOrigin::Unknown)
        );
        assert_eq!(
            can.origin_in("[Trash Info]\nPath=/tmp/a%20b\nDeletionDate=x\n"),
            Ok(PathBuf::from("/tmp/a b"))
        );
        assert_eq!(
            can.origin_in("[Trash Info]\nPath=a%20b\n"),
            Ok(PathBuf::from("/x/a b")),
            "relative to the directory the can is in"
        );
    }

    /// A stick's sidecar cannot send Put Back anywhere but onto the stick.
    #[test]
    fn a_topdir_sidecar_may_only_point_under_its_topdir() {
        let can = Can::topdir(Path::new("/media/usb"), false, 1000);
        let origin = |path: &str| can.origin_in(&format!("[Trash Info]\nPath={path}\n"));

        assert_eq!(
            origin("photos/cat.png"),
            Ok(PathBuf::from("/media/usb/photos/cat.png"))
        );
        assert_eq!(origin("/home/u/.bashrc"), Err(BadOrigin::Outside));
        assert_eq!(origin("../../home/u/.bashrc"), Err(BadOrigin::Outside));
        assert_eq!(origin("photos/../../etc"), Err(BadOrigin::Outside));
        assert_eq!(origin("./photos"), Err(BadOrigin::Outside));
        assert_eq!(origin(""), Err(BadOrigin::Outside));
    }

    #[test]
    fn mountinfo_yields_real_mount_points() {
        let text = "\
22 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw
23 22 0:21 / /proc rw,nosuid shared:12 - proc proc rw
60 22 8:17 / /run/media/u/MY\\040STICK rw,nosuid shared:40 - vfat /dev/sdb1 rw
61 22 0:50 / /mnt/nas rw,relatime shared:41 - nfs4 nas:/export rw
62 22 0:51 / /mnt/cloud rw,relatime shared:42 - fuse.rclone remote: rw
63 22 0:52 / /mnt/samba rw,relatime shared:43 - cifs //host/share rw
64 22 8:33 / /run/media/u/NTFS rw,relatime shared:44 - fuseblk /dev/sdc1 rw
";
        assert_eq!(
            parse_mountinfo(text),
            vec![
                PathBuf::from("/"),
                PathBuf::from("/run/media/u/MY STICK"),
                PathBuf::from("/run/media/u/NTFS"),
            ],
            "no pseudo, network or FUSE-daemon filesystems"
        );
    }

    #[test]
    fn candidate_names_number_before_the_extension() {
        let names: Vec<_> = candidate_names(OsStr::new("photo.png")).take(3).collect();
        assert_eq!(names[0], "photo.png");
        assert_eq!(names[1], "photo 2.png");
        assert_eq!(names[2], "photo 3.png");
        let hidden: Vec<_> = candidate_names(OsStr::new(".rc")).take(2).collect();
        assert_eq!(hidden[1], ".rc 2");
    }

    /// Someone else's uid, for a can "another user" made: the tests cannot
    /// chown, so the can is made by this user and checked as that one's.
    fn somebody_else() -> u32 {
        uid().wrapping_add(1)
    }

    /// A private topdir can under `top`, made by hand the way an attacker or
    /// another implementation would, for user `uid`.
    fn hand_made_can(top: &Path, uid: u32) -> PathBuf {
        let can = top.join(format!(".Trash-{uid}"));
        std::fs::create_dir(&can).unwrap();
        std::fs::set_permissions(&can, std::fs::Permissions::from_mode(0o700)).unwrap();
        can
    }

    /// The attack the checks exist for: `.Trash-$uid/files` planted as a
    /// symlink to somebody's home. The can is not listed, not trashed into,
    /// and deleting "from" it touches nothing.
    #[test]
    fn a_can_whose_files_is_a_symlink_is_never_used() {
        let me = uid();
        let top = Tmp::new("planted-files");
        let victim = top.0.join("victim");
        std::fs::create_dir(&victim).unwrap();
        std::fs::write(victim.join("thesis.txt"), "years of work").unwrap();
        let can = hand_made_can(&top.0, me);
        std::fs::create_dir(can.join("info")).unwrap();
        std::os::unix::fs::symlink(&victim, can.join("files")).unwrap();

        assert!(existing_topdir_cans(&top.0, me).is_empty(), "not listed");
        assert_eq!(topdir_can(&top.0, me), None, "not trashed into");
        let item = can.join("files/thesis.txt");
        assert_eq!(Can::of_item(&item), None);
        assert!(delete_forever_as(&item, me).is_err());
        assert_eq!(
            std::fs::read_to_string(victim.join("thesis.txt")).unwrap(),
            "years of work"
        );

        // `info/` and `directorysizes` the same.
        std::fs::remove_file(can.join("files")).unwrap();
        std::fs::create_dir(can.join("files")).unwrap();
        std::fs::remove_dir(can.join("info")).unwrap();
        std::os::unix::fs::symlink(&victim, can.join("info")).unwrap();
        assert!(existing_topdir_cans(&top.0, me).is_empty());
        std::fs::remove_file(can.join("info")).unwrap();
        std::os::unix::fs::symlink(victim.join("thesis.txt"), can.join("directorysizes")).unwrap();
        assert!(existing_topdir_cans(&top.0, me).is_empty());
        std::fs::remove_file(can.join("directorysizes")).unwrap();
        assert_eq!(
            existing_topdir_cans(&top.0, me).len(),
            1,
            "clean, it is used"
        );
    }

    /// A can another user made under this user's name (on `/tmp`, say), or
    /// one others can write to, is not this user's can.
    #[test]
    fn a_can_that_is_not_private_is_never_used() {
        let other = somebody_else();
        let top = Tmp::new("foreign-can");
        let can = hand_made_can(&top.0, other);
        std::fs::create_dir(can.join("files")).unwrap();
        std::fs::write(can.join("files/x"), "x").unwrap();

        // Owned by "us", checked as `other`: a foreign owner.
        assert!(existing_topdir_cans(&top.0, other).is_empty());
        assert_eq!(topdir_can(&top.0, other), None, "falls back to home");
        assert!(delete_forever_as(&can.join("files/x"), other).is_err());
        assert!(can.join("files/x").exists());

        // The right owner, but group-writable.
        let me = uid();
        let top = Tmp::new("loose-can");
        let can = hand_made_can(&top.0, me);
        std::fs::set_permissions(&can, std::fs::Permissions::from_mode(0o770)).unwrap();
        assert!(existing_topdir_cans(&top.0, me).is_empty());
        assert_eq!(topdir_can(&top.0, me), None);
    }

    /// Another user pre-creating `.Trash/$uid` in a shared `.Trash` does not
    /// get this user's files: that can is passed over.
    #[test]
    fn a_shared_can_made_by_someone_else_is_passed_over() {
        let other = somebody_else();
        let top = Tmp::new("foreign-shared");
        let shared = top.0.join(".Trash");
        std::fs::create_dir(&shared).unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o1777)).unwrap();
        std::fs::create_dir(shared.join(other.to_string())).unwrap();

        // Neither `.Trash/$uid` nor the `.Trash-$uid` made next is `other`'s.
        assert_eq!(topdir_can(&top.0, other), None);
        assert!(existing_topdir_cans(&top.0, other).is_empty());
    }

    /// Deleting forever walks down from the checked `files/` and never
    /// follows a symlink inside the item.
    #[test]
    fn delete_forever_stays_inside_the_can() {
        let me = uid();
        let top = Tmp::new("forever");
        let outside = top.0.join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("keep"), "keep").unwrap();
        let folder = top.0.join("folder");
        std::fs::create_dir_all(folder.join("deep")).unwrap();
        std::fs::write(folder.join("deep/a"), "a").unwrap();
        std::os::unix::fs::symlink(&outside, folder.join("link")).unwrap();
        let can = topdir_can(&top.0, me).unwrap();
        let (to, info) = place(&folder, &folder, &can, Some(&top.0)).unwrap();

        // Something inside a trashed folder, then the folder itself.
        delete_forever_as(&to.join("deep/a"), me).unwrap();
        assert!(!to.join("deep/a").exists());
        assert!(info.exists(), "the folder's sidecar stays");
        delete_forever_as(&to, me).unwrap();

        assert!(!to.exists());
        assert!(!info.exists(), "the sidecar went with it");
        assert_eq!(
            std::fs::read_to_string(outside.join("keep")).unwrap(),
            "keep"
        );
        assert!(
            delete_forever_as(&top.0.join("folder"), me).is_err(),
            "not in a can"
        );
    }

    /// The temporary `directorysizes` is created, never opened: a symlink
    /// planted under its predictable name is not written through.
    #[test]
    fn a_planted_directorysizes_temp_is_not_followed() {
        let root = Tmp::new("planted-temp");
        let trash = root.0.join("Trash");
        std::fs::create_dir_all(&trash).unwrap();
        let victim = root.0.join("victim");
        std::fs::write(&victim, "precious").unwrap();
        let next = DIRECTORYSIZES_SERIAL.load(Ordering::Relaxed);
        for serial in next..next + 64 {
            std::os::unix::fs::symlink(
                &victim,
                trash.join(format!(".directorysizes.{}.{serial}", std::process::id())),
            )
            .unwrap();
        }
        let folder = root.0.join("folder");
        std::fs::create_dir(&folder).unwrap();

        trash_into(&folder, &trash).unwrap();

        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious");
        let sizes = std::fs::symlink_metadata(trash.join("directorysizes"));
        assert!(sizes.map_or(true, |meta| meta.is_file()));
    }
}
