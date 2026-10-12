//! The thread that owns PDFium, and [`Document`], the handle every other
//! thread uses.
//!
//! PDFium keeps global state and must not be entered from two threads at
//! once, so one thread, started when the first document opens, owns the
//! library and every open document. A [`Document`] sends it work as
//! closures and waits for the answer.

// Rust guideline compliant 2026-02-21

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::OnceLock;

use crate::engine::Doc;
use crate::{library, Edit, Error, Result, Snapshot};

/// What the worker thread holds: every open document, by id.
#[derive(Default)]
struct Engine {
    docs: HashMap<u64, Doc>,
    next_id: u64,
}

impl Engine {
    fn doc(&mut self, id: u64) -> Result<&mut Doc> {
        self.docs
            .get_mut(&id)
            .ok_or_else(|| Error::Failed("the document is no longer open".into()))
    }
}

type Job = Box<dyn FnOnce(&mut Engine) + Send>;

static WORKER: OnceLock<Sender<Job>> = OnceLock::new();

fn worker() -> Result<Sender<Job>> {
    if let Some(sender) = WORKER.get() {
        return Ok(sender.clone());
    }
    let (sender, receiver) = mpsc::channel::<Job>();
    let spawned = std::thread::Builder::new()
        .name("otto-pdf".into())
        .spawn(move || {
            let mut engine = Engine::default();
            for job in receiver {
                job(&mut engine);
            }
        });
    if let Err(error) = spawned {
        return Err(Error::Failed(format!(
            "couldn't start the PDF thread: {error}"
        )));
    }
    // A second thread racing here loses its worker, which exits as its
    // sender drops; both callers then share the winner.
    Ok(WORKER.get_or_init(|| sender).clone())
}

fn stopped() -> Error {
    Error::Failed("the PDF thread stopped".into())
}

/// Runs `work` on the worker thread and waits for its answer.
fn call<T: Send + 'static>(
    worker: &Sender<Job>,
    work: impl FnOnce(&mut Engine) -> T + Send + 'static,
) -> Result<T> {
    let (reply, answer) = mpsc::sync_channel(1);
    let job: Job = Box::new(move |engine| {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(engine)));
        if let Ok(value) = outcome {
            let _ = reply.send(value);
        }
    });
    worker.send(job).map_err(|_| stopped())?;
    answer.recv().map_err(|_| stopped())
}

/// An open PDF, its form filled and its annotations changed in memory until
/// it is saved.
///
/// A cheap handle: the document itself lives on the PDFium thread, and
/// every call waits for that thread. Dropping it closes the document there,
/// unsaved changes and all.
#[derive(Debug)]
pub struct Document {
    id: u64,
    path: PathBuf,
    snapshot: Snapshot,
    worker: Sender<Job>,
}

impl Document {
    /// Opens the PDF at `path`, with its password if it has one.
    ///
    /// # Errors
    ///
    /// [`Error::Unavailable`] without PDFium, [`Error::Password`] when the
    /// file needs a password and `password` is missing or wrong, and
    /// [`Error::Unreadable`] when the file can't be read or isn't a PDF.
    pub fn open(path: PathBuf, password: Option<String>) -> Result<Self> {
        library::load()?;
        let bytes = fs::read(&path)
            .map_err(|error| Error::Unreadable(format!("{}: {error}", path.display())))?;
        Self::open_bytes(path, bytes, password)
    }

    /// Opens the PDF in `bytes` as the file at `path`: what it is saved
    /// over, though nothing is read from there. For a document read once and
    /// opened again from memory, whatever the file has become since.
    ///
    /// # Errors
    ///
    /// As for [`open`](Self::open).
    pub fn open_bytes(path: PathBuf, bytes: Vec<u8>, password: Option<String>) -> Result<Self> {
        let ffi = library::load()?;
        let worker = worker()?;
        let (id, snapshot) = call(&worker, move |engine| {
            let doc = Doc::open(ffi, bytes, password)?;
            let snapshot = doc.snapshot();
            let id = engine.next_id;
            engine.next_id += 1;
            engine.docs.insert(id, doc);
            Ok::<_, Error>((id, snapshot))
        })??;
        Ok(Self {
            id,
            path,
            snapshot,
            worker,
        })
    }

    /// The file this document was opened from, and is saved over.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What the document is now, as read after it opened or after the last
    /// [`apply`](Self::apply).
    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.clone()
    }

    /// Makes `edits` in memory and answers what the document is then.
    ///
    /// Every id names an entry of the current [`snapshot`](Self::snapshot).
    /// Adds, moves and changes run in order and deletions run last, so no
    /// edit renumbers what a later one in the same call names. Fields and
    /// annotations come back renumbered for the next call.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] for an id that names nothing editable or an edit
    /// that doesn't fit what it names, [`Error::Failed`] when PDFium
    /// refuses; the message starts with which edit (`edit 2: …`). The edits
    /// before it stay made, and the snapshot shows them.
    pub fn apply(&mut self, edits: &[Edit]) -> Result<Snapshot> {
        let id = self.id;
        let edits = edits.to_vec();
        let (outcome, snapshot) = call(&self.worker, move |engine| {
            let doc = engine.doc(id)?;
            let (_, outcome) = doc.apply(&edits);
            Ok::<_, Error>((outcome, doc.snapshot()))
        })??;
        self.snapshot = snapshot;
        outcome.map(|()| self.snapshot.clone())
    }

    /// Draws page `page` (0-based) as the document is now, unsaved edits
    /// included: `width` pixels wide, RGBA, opaque. Returns the height with
    /// the pixels.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] for a page the document doesn't have,
    /// [`Error::Failed`] when PDFium can't draw it.
    pub fn render(&self, page: usize, width: u32) -> Result<(u32, Vec<u8>)> {
        let id = self.id;
        call(&self.worker, move |engine| {
            engine.doc(id)?.render(page, width)
        })?
    }

    /// Saves over the file it was opened from.
    ///
    /// A signed document is saved by appending the changes, so its
    /// signatures still cover the bytes they signed; any other is written
    /// afresh. A locked document stays locked. The new file goes beside the
    /// old one first and replaces it whole, with the old one's permissions.
    /// Returns the bytes written: the file as it is now.
    ///
    /// # Errors
    ///
    /// [`Error::Failed`] when PDFium can't write the document, and
    /// [`Error::Io`] when the file can't be.
    pub fn save(&mut self) -> Result<Vec<u8>> {
        let path = self.path.clone();
        self.save_with(&path, self.snapshot.signed)
    }

    /// Writes a copy to `path`, saved the way [`save`](Self::save) saves,
    /// and returns the bytes written. The document stays the one at
    /// [`path`](Self::path).
    ///
    /// # Errors
    ///
    /// As for [`save`](Self::save).
    pub fn save_to(&mut self, path: &Path) -> Result<Vec<u8>> {
        self.save_with(path, self.snapshot.signed)
    }

    /// Writes to `path`, appending the changes when `incremental`.
    pub(crate) fn save_with(&mut self, path: &Path, incremental: bool) -> Result<Vec<u8>> {
        let id = self.id;
        let bytes = call(&self.worker, move |engine| {
            engine.doc(id)?.save(incremental)
        })??;
        write_replacing(path, &bytes)?;
        Ok(bytes)
    }
}

impl Drop for Document {
    fn drop(&mut self) {
        let id = self.id;
        let close: Job = Box::new(move |engine| {
            engine.docs.remove(&id);
        });
        let _ = self.worker.send(close);
    }
}

/// How many times a temporary name is drawn before giving up, should each
/// one already be taken.
const TEMP_TRIES: u32 = 16;

/// A name for the file written beside `name` before it replaces it: hidden,
/// and with a suffix nobody can guess ahead of time, drawn from the clock,
/// the process, a counter and the standard library's per-process random
/// hash keys.
fn temp_name(name: &std::ffi::OsStr) -> String {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    hasher.write_u128(now.as_nanos());
    hasher.write_u32(std::process::id());
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    format!(
        ".{}.otto-pdf-{:016x}",
        name.to_string_lossy(),
        hasher.finish()
    )
}

/// Writes `bytes` to a new file beside `path`, then renames it over `path`,
/// so a reader sees the old file or the new one and never half of one.
///
/// The new file is made fresh, never opened through a link or over a file
/// already there, with the old one's permissions from the start (or the
/// usual ones, under the umask, for a file that isn't there yet), and takes
/// the old one's owner where allowed.
fn write_replacing(path: &Path, bytes: &[u8]) -> Result<()> {
    let io = |error: std::io::Error| Error::Io(format!("{}: {error}", path.display()));
    // Saving through a link replaces what it points at, not the link.
    let target = match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => fs::canonicalize(path).map_err(io)?,
        _ => path.to_path_buf(),
    };
    let name = target
        .file_name()
        .ok_or_else(|| Error::Io(format!("{} names no file", path.display())))?;
    let dir = match target.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let old = fs::metadata(&target).ok();
    let mode = old
        .as_ref()
        .map_or(0o666, |old| old.permissions().mode() & 0o7777);
    let (temp, mut file) = (|| {
        let mut tries = 0;
        loop {
            let temp = dir.join(temp_name(name));
            let opened = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .mode(mode)
                .open(&temp);
            match opened {
                Ok(file) => return Ok((temp, file)),
                Err(error)
                    if error.kind() == std::io::ErrorKind::AlreadyExists
                        && tries + 1 < TEMP_TRIES =>
                {
                    tries += 1;
                }
                Err(error) => return Err(error),
            }
        }
    })()
    .map_err(io)?;
    let written = (|| {
        if let Some(old) = &old {
            use std::os::unix::fs::MetadataExt;
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
    if let Err(error) = written {
        let _ = fs::remove_file(&temp);
        return Err(io(error));
    }
    // The rename itself survives a crash once the directory is synced.
    if let Ok(dir) = fs::File::open(&dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}
