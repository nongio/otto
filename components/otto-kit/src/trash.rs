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
//! The spec is <https://specifications.freedesktop.org/trash/latest/>.

use std::ffi::{OsStr, OsString};
use std::fs::{DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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
}

impl Can {
    /// The can at `dir`, whether or not it exists yet.
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The home trash.
    pub fn home() -> Option<Self> {
        home_trash_dir().map(Self::at)
    }

    /// The can an item in the trash belongs to: `item` is `<can>/files/<name>`.
    pub fn of_item(item: &Path) -> Option<Self> {
        let files = item.parent()?;
        if files.file_name()? != "files" {
            return None;
        }
        Some(Self::at(files.parent()?))
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
        let mut file = name.to_os_string();
        file.push(".trashinfo");
        self.info_dir().join(file)
    }

    /// The directory a relative `Path=` is relative to: the one the can
    /// resides in, which for `$topdir/.Trash/$uid` is `$topdir`, not
    /// `$topdir/.Trash`.
    pub fn base(&self) -> Option<&Path> {
        let parent = self.dir.parent()?;
        if parent.file_name() == Some(OsStr::new(".Trash")) {
            parent.parent()
        } else {
            Some(parent)
        }
    }

    /// Where the item called `name` came from, from its sidecar.
    pub fn origin(&self, name: &OsStr) -> Option<PathBuf> {
        let body = std::fs::read_to_string(self.sidecar(name)).ok()?;
        self.origin_in(&body)
    }

    /// The origin a sidecar's body records, resolved against [`Self::base`]
    /// when it is relative.
    pub fn origin_in(&self, body: &str) -> Option<PathBuf> {
        let recorded = path_key(body)?;
        if recorded.is_absolute() {
            Some(recorded)
        } else {
            Some(self.base()?.join(recorded))
        }
    }

    /// Drop the `directorysizes` entry for `name`, once it has left the can.
    /// The cache is advisory, so a failure is not reported.
    pub fn forget_directory_size(&self, name: &OsStr) {
        self.update_directory_sizes(name, None);
    }

    /// Record the size of the trashed directory `name` in `directorysizes`.
    ///
    /// The size is what `du -B1` would say. Walking a huge tree on the
    /// trashing thread would cost far more than the rename did, so past
    /// [`SIZE_WALK_LIMIT`] entries nothing is written: a missing entry is what
    /// any reader of the cache already copes with, by computing it.
    fn record_directory_size(&self, name: &OsStr) {
        let Ok(info) = std::fs::metadata(self.sidecar(name)) else {
            return;
        };
        let mut budget = SIZE_WALK_LIMIT;
        let Some(size) = disk_usage(&self.files_dir().join(name), &mut budget) else {
            return;
        };
        self.update_directory_sizes(name, Some((size, info.mtime())));
    }

    /// Rewrite `directorysizes` without `name`, plus `entry` for it when
    /// given: to a temporary file, renamed into place, as the spec requires.
    fn update_directory_sizes(&self, name: &OsStr, entry: Option<(u64, i64)>) {
        let path = self.dir.join("directorysizes");
        let existing = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound && entry.is_none() => return,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
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

        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let temp = self.dir.join(format!(
            ".directorysizes.{}.{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let written = std::fs::write(&temp, out).and_then(|()| std::fs::rename(&temp, &path));
        if written.is_err() {
            std::fs::remove_file(&temp).ok();
        }
    }
}

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
/// there.
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
/// As [`trash_into`].
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
///
/// # Errors
///
/// The directory creation, sidecar creation or move that failed, as text.
pub fn trash_into(source: &Path, trash: &Path) -> Result<(PathBuf, PathBuf), String> {
    place(source, source, &Can::at(trash), None)
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
    let files_dir = can.files_dir();
    let info_dir = can.info_dir();
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&files_dir)
        .and_then(|()| {
            DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&info_dir)
        })
        .map_err(|e| e.to_string())?;

    let path_value = match relative_to.and_then(|base| recorded.strip_prefix(base).ok()) {
        Some(relative) if !relative.as_os_str().is_empty() => relative,
        _ => recorded,
    };
    let info = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        crate::uri::encode_path(path_value),
        deletion_date(),
    );

    for candidate in candidate_names(name) {
        let sidecar = can.sidecar(&candidate);
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&sidecar)
        {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err.to_string()),
        };
        let target = files_dir.join(&candidate);
        // An item without a sidecar (left by a crash, or another program)
        // still owns its name: never move on top of it.
        if std::fs::symlink_metadata(&target).is_ok() {
            drop(file);
            std::fs::remove_file(&sidecar).ok();
            continue;
        }
        if let Err(err) = file.write_all(info.as_bytes()) {
            drop(file);
            std::fs::remove_file(&sidecar).ok();
            return Err(err.to_string());
        }
        drop(file);
        if let Err(err) = move_entry(source, &target) {
            std::fs::remove_file(&sidecar).ok();
            return Err(err);
        }
        if std::fs::symlink_metadata(&target).is_ok_and(|m| m.is_dir()) {
            can.record_directory_size(&candidate);
        }
        return Ok((target, sidecar));
    }
    Err(format!("no free name in {}", files_dir.display()))
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
/// when neither could be made.
fn topdir_can(topdir: &Path, uid: u32) -> Option<Can> {
    let shared = topdir.join(".Trash");
    if std::fs::symlink_metadata(&shared).is_ok() {
        if shared_trash_passes_checks(&shared) {
            let own = shared.join(uid.to_string());
            if ensure_private_dir(&own) {
                return Some(Can::at(own));
            }
        } else {
            tracing::warn!(
                "trash: {} is not a sticky, real directory; not using it",
                shared.display()
            );
        }
    }
    let own = topdir.join(format!(".Trash-{uid}"));
    ensure_private_dir(&own).then(|| Can::at(own))
}

/// The topdir cans already on the filesystem whose top is `topdir`.
fn existing_topdir_cans(topdir: &Path, uid: u32) -> Vec<Can> {
    let mut found = Vec::new();
    let shared = topdir.join(".Trash");
    if shared_trash_passes_checks(&shared) {
        let own = shared.join(uid.to_string());
        if is_real_dir(&own) {
            found.push(Can::at(own));
        }
    }
    let own = topdir.join(format!(".Trash-{uid}"));
    if is_real_dir(&own) {
        found.push(Can::at(own));
    }
    found
}

/// `$topdir/.Trash` may only be used when it is a directory, not a symlink,
/// with the sticky bit set.
fn shared_trash_passes_checks(shared: &Path) -> bool {
    std::fs::symlink_metadata(shared)
        .is_ok_and(|meta| meta.is_dir() && meta.permissions().mode() & 0o1000 != 0)
}

fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
}

/// `path` as a directory only its owner can enter, made if missing.
fn ensure_private_dir(path: &Path) -> bool {
    match DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => true,
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => is_real_dir(path),
        Err(_) => false,
    }
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
    // SAFETY: getuid cannot fail.
    unsafe { libc::getuid() }
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

/// The mount points in `/proc/self/mountinfo`, less [`PSEUDO_FILESYSTEMS`].
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
            if PSEUDO_FILESYSTEMS.contains(&fstype) {
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
            let can = Can::at(&trash);
            assert_eq!(can.origin(to.file_name().unwrap()).as_ref(), Some(source));
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
        let top = Tmp::new("topdir-own");

        let can = topdir_can(&top.0, 4242).unwrap();

        assert_eq!(can.dir(), top.0.join(".Trash-4242"));
        let mode = std::fs::metadata(can.dir()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        assert_eq!(can.base(), Some(top.0.as_path()));
        assert_eq!(existing_topdir_cans(&top.0, 4242), vec![can]);
    }

    #[test]
    fn a_sticky_shared_trash_is_used_per_user() {
        let top = Tmp::new("topdir-shared");
        let shared = top.0.join(".Trash");
        std::fs::create_dir(&shared).unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o1777)).unwrap();

        let can = topdir_can(&top.0, 4242).unwrap();

        assert_eq!(can.dir(), shared.join("4242"));
        assert_eq!(can.base(), Some(top.0.as_path()), "relative to topdir");
        assert!(!top.0.join(".Trash-4242").exists());
    }

    #[test]
    fn a_shared_trash_failing_the_checks_is_passed_over() {
        // Not sticky.
        let top = Tmp::new("topdir-loose");
        std::fs::create_dir(top.0.join(".Trash")).unwrap();
        std::fs::set_permissions(top.0.join(".Trash"), std::fs::Permissions::from_mode(0o777))
            .unwrap();
        assert_eq!(
            topdir_can(&top.0, 4242).unwrap().dir(),
            top.0.join(".Trash-4242")
        );
        assert!(!top.0.join(".Trash/4242").exists());

        // A symlink, even to a sticky directory.
        let top = Tmp::new("topdir-link");
        let elsewhere = top.0.join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::fs::set_permissions(&elsewhere, std::fs::Permissions::from_mode(0o1777)).unwrap();
        std::os::unix::fs::symlink(&elsewhere, top.0.join(".Trash")).unwrap();
        assert_eq!(
            topdir_can(&top.0, 4242).unwrap().dir(),
            top.0.join(".Trash-4242")
        );
        assert!(existing_topdir_cans(&top.0, 4242)
            .iter()
            .all(|can| !can.dir().starts_with(top.0.join(".Trash/"))));
    }

    /// A topdir can records `Path=` relative to the topdir, and reads it
    /// back to the absolute origin.
    #[test]
    fn a_topdir_can_records_a_relative_path() {
        let top = Tmp::new("topdir-relative");
        let dir = top.0.join("photos");
        std::fs::create_dir(&dir).unwrap();
        let source = dir.join("cat.png");
        std::fs::write(&source, "meow").unwrap();
        let can = topdir_can(&top.0, 4242).unwrap();

        let (to, info) = place(&source, &source, &can, Some(&top.0)).unwrap();

        let body = std::fs::read_to_string(info).unwrap();
        assert!(body.contains("\nPath=photos/cat.png\n"), "{body}");
        let found = Can::of_item(&to).unwrap();
        assert_eq!(found, can);
        assert_eq!(found.origin(to.file_name().unwrap()), Some(source));
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
        let can = Can::of_item(&to).unwrap();
        can.forget_directory_size(to.file_name().unwrap());
        assert_eq!(
            std::fs::read_to_string(trash.join("directorysizes")).unwrap(),
            ""
        );
    }

    #[test]
    fn a_sidecar_without_a_path_is_skipped_rather_than_guessed() {
        let can = Can::at("/x/Trash");
        assert_eq!(can.origin_in("[Trash Info]\nDeletionDate=x\n"), None);
        assert_eq!(
            can.origin_in("[Trash Info]\nPath=/tmp/a%20b\nDeletionDate=x\n"),
            Some(PathBuf::from("/tmp/a b"))
        );
        assert_eq!(
            can.origin_in("[Trash Info]\nPath=a%20b\n"),
            Some(PathBuf::from("/x/a b")),
            "relative to the directory the can is in"
        );
    }

    #[test]
    fn mountinfo_yields_real_mount_points() {
        let text = "\
22 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw
23 22 0:21 / /proc rw,nosuid shared:12 - proc proc rw
60 22 8:17 / /run/media/u/MY\\040STICK rw,nosuid shared:40 - vfat /dev/sdb1 rw
";
        assert_eq!(
            parse_mountinfo(text),
            vec![PathBuf::from("/"), PathBuf::from("/run/media/u/MY STICK")]
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
}
