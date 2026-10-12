//! The one line a document's edits wait in: every edit, undo, redo and save
//! of a PDF open for editing runs here, one at a time, in the order it was
//! asked for, on a thread of the document's own.
//!
//! The queue keeps what the window needs to step back and to save: the file
//! as it was last opened or saved, in memory, and the edits made on it
//! since, oldest first. Undo opens those bytes again and makes every edit
//! but the last, so it never depends on what the file on disk has become.
//! An edit is kept only once it has been made; one that fails is taken out
//! again, the document put back to what the kept edits say.
//!
//! An edit names fields and annotations by their place in the page's list,
//! read from the snapshot the window had when it was made (its [`Base`]).
//! A delete or an undo renumbers a page, so when one has run since that
//! snapshot, each name is followed to the same field or annotation in the
//! document as it is when the edit runs; one that can't be followed is
//! refused rather than made on something else.

// Rust guideline compliant 2026-02-21

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender, SyncSender};
use std::sync::{Arc, Mutex, Weak};

use otto_kit::prelude::AppContext;
use otto_pdf::{AnnotId, Document, Edit, Snapshot};

use crate::viewer::{Stamp, Viewer};

/// What an edit was made against: the document as the window last heard of
/// it.
#[derive(Debug, Clone, PartialEq)]
pub struct Base {
    /// How many times the document had been renumbered (see [`Outcome`]).
    pub numbering: u64,
    pub snapshot: Arc<Snapshot>,
}

/// Work on the document.
#[derive(Debug, Clone, PartialEq)]
pub enum Job {
    /// Make edits, in memory, named against `base`.
    Apply { edits: Vec<Edit>, base: Base },
    /// Take back the last edit kept.
    Undo,
    /// Make the last edit taken back again.
    Redo,
    /// Write the document over its file; over a file changed on disk since
    /// it was opened or saved only when `overwrite`.
    Save { overwrite: bool },
    /// Write the document to another file, and carry on with that one.
    SaveAs(PathBuf),
}

/// Which kind of job an [`Outcome`] answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Apply,
    Undo,
    Redo,
    Save,
    SaveAs,
}

impl Kind {
    /// Whether the job changes the document rather than writes it.
    pub fn changes(self) -> bool {
        matches!(self, Self::Apply | Self::Undo | Self::Redo)
    }
}

impl Job {
    pub fn kind(&self) -> Kind {
        match self {
            Self::Apply { .. } => Kind::Apply,
            Self::Undo => Kind::Undo,
            Self::Redo => Kind::Redo,
            Self::Save { .. } => Kind::Save,
            Self::SaveAs(_) => Kind::SaveAs,
        }
    }
}

/// Why a job wasn't done.
#[derive(Debug, Clone, PartialEq)]
pub enum Failure {
    /// The engine refused, or the file couldn't be written.
    Engine(otto_pdf::Error),
    /// What the edit names was changed or taken away before it ran.
    Stale,
    /// The file changed on disk since it was opened or last saved, and a
    /// save would replace that change.
    ChangedOnDisk,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Engine(err) => write!(f, "{err}"),
            Self::Stale => write!(
                f,
                "the document changed before the edit was made; read it again and retry"
            ),
            Self::ChangedOnDisk => write!(f, "the file changed on disk since it was opened"),
        }
    }
}

impl std::error::Error for Failure {}

/// A job done, or not: what the document and its history are after it.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub kind: Kind,
    pub result: Result<(), Failure>,
    /// The document as it really is now, whatever the result.
    pub snapshot: Arc<Snapshot>,
    /// Counts the times the document's pages may have been renumbered: a
    /// delete, an undo, an edit taken out again. An id read before the last
    /// one may name something else now.
    pub numbering: u64,
    /// The pages that may look different, 0-based, to be drawn again.
    pub pages: Vec<usize>,
    /// How many edits Undo and Redo have to take back or make again.
    pub undo: usize,
    pub redo: usize,
    /// Whether the document holds changes its file doesn't.
    pub unsaved: bool,
    /// The file the document saves over, and its stamp as last opened or
    /// saved.
    pub path: PathBuf,
    pub stamp: Option<Stamp>,
}

/// What the queue does its work on: the PDFium document a window edits, or
/// a stand-in in the tests.
pub trait Target {
    /// The document as it is now.
    fn snapshot(&self) -> Snapshot;
    /// Make `edits` on the document as it is.
    fn apply(&mut self, edits: &[Edit]) -> Result<Snapshot, otto_pdf::Error>;
    /// Open `bytes` afresh as the file at `path` and make `history` on it,
    /// batch by batch; put that in place of the document only when all of
    /// it was made, leaving the document as it was otherwise.
    fn rebuild(
        &mut self,
        path: &Path,
        bytes: &[u8],
        history: &[Vec<Edit>],
    ) -> Result<Snapshot, otto_pdf::Error>;
    /// Write the document to `path`, and answer the bytes written.
    fn write(&mut self, path: &Path) -> Result<Vec<u8>, otto_pdf::Error>;
    /// The stamp of the file at `path` now.
    fn stamp(&self, path: &Path) -> Option<Stamp>;
}

/// The history the queue keeps for one document.
#[derive(Debug)]
pub struct Ledger {
    /// The file the document saves over.
    path: PathBuf,
    /// The file's bytes as last opened or saved: where Undo starts from.
    base: Vec<u8>,
    /// The file's stamp as last opened or saved.
    stamp: Option<Stamp>,
    /// The edits made on `base`, oldest first, each named against the
    /// document as the ones before it left it.
    history: Vec<Vec<Edit>>,
    /// Edits taken back, the next to make again last.
    redo: Vec<Vec<Edit>>,
    numbering: u64,
    /// Whether the document holds changes the history doesn't, after an
    /// edit failed halfway and couldn't be taken out again.
    diverged: bool,
}

impl Ledger {
    /// The history of a document just opened from `base`, the bytes of the
    /// file at `path`, whose stamp was `stamp` when they were read.
    pub fn new(path: PathBuf, base: Vec<u8>, stamp: Option<Stamp>) -> Self {
        Self {
            path,
            base,
            stamp,
            history: Vec::new(),
            redo: Vec::new(),
            numbering: 0,
            diverged: false,
        }
    }

    /// Do `job` on `target`, and say how that went.
    pub fn run(&mut self, target: &mut impl Target, job: Job) -> Outcome {
        let kind = job.kind();
        let before = target.snapshot();
        let (result, pages) = match job {
            Job::Apply { edits, base } => match self.resolve(edits, &base, &before) {
                Some(edits) => {
                    let made = self.make(target, &before, edits.clone());
                    if made.0.is_ok() {
                        self.history.push(edits);
                        self.redo.clear();
                    }
                    made
                }
                None => (Err(Failure::Stale), Vec::new()),
            },
            Job::Undo => self.undo(target),
            Job::Redo => match self.redo.pop() {
                Some(edits) => {
                    let made = self.make(target, &before, edits.clone());
                    if made.0.is_ok() {
                        self.history.push(edits);
                    } else {
                        // What is left to redo was made on top of this one.
                        self.redo.clear();
                    }
                    made
                }
                None => (Ok(()), Vec::new()),
            },
            Job::Save { overwrite } => {
                let path = self.path.clone();
                if !overwrite && target.stamp(&path) != self.stamp {
                    (Err(Failure::ChangedOnDisk), Vec::new())
                } else {
                    (self.write(target, path), Vec::new())
                }
            }
            Job::SaveAs(path) => (self.write(target, path), Vec::new()),
        };
        Outcome {
            kind,
            result,
            snapshot: Arc::new(target.snapshot()),
            numbering: self.numbering,
            pages,
            undo: self.history.len(),
            redo: self.redo.len(),
            unsaved: !self.history.is_empty() || self.diverged,
            path: self.path.clone(),
            stamp: self.stamp,
        }
    }

    /// `edits` named against the document as it is, or `None` when what
    /// one names can't be found in it any more.
    fn resolve(&self, edits: Vec<Edit>, base: &Base, now: &Snapshot) -> Option<Vec<Edit>> {
        if base.numbering == self.numbering {
            return Some(edits);
        }
        let old = &base.snapshot;
        let field = |id| follow_field(old, now, id);
        let annotation = |id| follow_annotation(old, now, id);
        edits
            .into_iter()
            .map(|edit| {
                Some(match edit {
                    Edit::SetText { field: id, text } => Edit::SetText {
                        field: field(id)?,
                        text,
                    },
                    Edit::SetChecked { field: id, checked } => Edit::SetChecked {
                        field: field(id)?,
                        checked,
                    },
                    Edit::Choose {
                        field: id,
                        index,
                        add,
                    } => Edit::Choose {
                        field: field(id)?,
                        index,
                        add,
                    },
                    Edit::Add { page, annotation } => Edit::Add { page, annotation },
                    Edit::Move {
                        annotation: id,
                        dx,
                        dy,
                    } => Edit::Move {
                        annotation: annotation(id)?,
                        dx,
                        dy,
                    },
                    Edit::SetContents {
                        annotation: id,
                        text,
                    } => Edit::SetContents {
                        annotation: annotation(id)?,
                        text,
                    },
                    Edit::SetColor {
                        annotation: id,
                        color,
                    } => Edit::SetColor {
                        annotation: annotation(id)?,
                        color,
                    },
                    Edit::Delete { annotation: id } => Edit::Delete {
                        annotation: annotation(id)?,
                    },
                })
            })
            .collect()
    }

    /// Make `edits` on the document as it is, `before`. On a failure the
    /// edits made before the failing one are taken out again.
    fn make(
        &mut self,
        target: &mut impl Target,
        before: &Snapshot,
        edits: Vec<Edit>,
    ) -> (Result<(), Failure>, Vec<usize>) {
        let pages = pages_of(before, &edits);
        match target.apply(&edits) {
            Ok(_) => {
                if edits.iter().any(|edit| matches!(edit, Edit::Delete { .. })) {
                    self.numbering += 1;
                }
                (Ok(()), pages)
            }
            Err(err) => {
                if target.snapshot() != *before {
                    self.restore(target);
                }
                (Err(Failure::Engine(err)), pages)
            }
        }
    }

    /// Put the document back to the saved bytes with the history made on
    /// them.
    fn restore(&mut self, target: &mut impl Target) {
        self.numbering += 1;
        match target.rebuild(&self.path, &self.base, &self.history) {
            Ok(_) => self.diverged = false,
            Err(err) => {
                tracing::warn!(path = %self.path.display(), %err, "the document couldn't be put back after a failed edit");
                self.diverged = true;
            }
        }
    }

    fn undo(&mut self, target: &mut impl Target) -> (Result<(), Failure>, Vec<usize>) {
        let Some(last) = self.history.pop() else {
            return (Ok(()), Vec::new());
        };
        match target.rebuild(&self.path, &self.base, &self.history) {
            // The edit taken back was named against the document the edits
            // before it leave, which is what the rebuild made.
            Ok(made) => {
                self.numbering += 1;
                self.diverged = false;
                let pages = pages_of(&made, &last);
                self.redo.push(last);
                (Ok(()), pages)
            }
            Err(err) => {
                self.history.push(last);
                (Err(Failure::Engine(err)), Vec::new())
            }
        }
    }

    /// Write the document to `path` and carry on with that file, its
    /// changes all saved.
    fn write(&mut self, target: &mut impl Target, path: PathBuf) -> Result<(), Failure> {
        let bytes = target.write(&path).map_err(Failure::Engine)?;
        self.stamp = target.stamp(&path);
        self.path = path;
        self.base = bytes;
        self.history.clear();
        self.redo.clear();
        self.diverged = false;
        Ok(())
    }
}

/// The field `id` names in `old`, found again in `now`: the widget with
/// the same name on the same page, in the same place.
pub fn follow_field(old: &Snapshot, now: &Snapshot, id: AnnotId) -> Option<AnnotId> {
    let field = old.field(id)?;
    only(now.fields.iter().filter(|other| {
        other.id.page == field.id.page && other.name == field.name && other.rect == field.rect
    }))
    .map(|other| other.id)
}

/// The annotation `id` names in `old`, found again in `now`: the one on the
/// same page the same in every way but its place in the list.
pub fn follow_annotation(old: &Snapshot, now: &Snapshot, id: AnnotId) -> Option<AnnotId> {
    let annotation = old.annotation(id)?;
    only(now.annotations.iter().filter(|other| {
        other.id.page == annotation.id.page
            && other.rect == annotation.rect
            && other.kind == annotation.kind
            && other.color == annotation.color
            && other.author == annotation.author
            && other.editable == annotation.editable
    }))
    .map(|other| other.id)
}

/// The one item `items` holds, if it holds exactly one.
fn only<T>(mut items: impl Iterator<Item = T>) -> Option<T> {
    let first = items.next()?;
    items.next().is_none().then_some(first)
}

/// The pages a set of edits touches, given the document they apply to.
pub fn pages_of(snapshot: &Snapshot, edits: &[Edit]) -> Vec<usize> {
    let mut pages: Vec<usize> = Vec::new();
    let mut push = |page: usize| {
        if !pages.contains(&page) {
            pages.push(page);
        }
    };
    for edit in edits {
        match edit {
            Edit::SetText { field, .. }
            | Edit::SetChecked { field, .. }
            | Edit::Choose { field, .. } => {
                // A field shown in several places, or a radio group, changes
                // on every page it is on.
                let name = snapshot.field(*field).map(|field| field.name.clone());
                push(field.page);
                for other in &snapshot.fields {
                    if Some(&other.name) == name.as_ref() {
                        push(other.id.page);
                    }
                }
            }
            Edit::Add { page, .. } => push(*page),
            Edit::Move { annotation, .. }
            | Edit::SetContents { annotation, .. }
            | Edit::SetColor { annotation, .. }
            | Edit::Delete { annotation } => push(annotation.page),
        }
    }
    pages
}

/// The PDFium document a window edits, as the queue works on it. The lock
/// is held only for each call, so pages are drawn in between.
struct Pdf<'a>(&'a Mutex<Document>);

impl Target for Pdf<'_> {
    fn snapshot(&self) -> Snapshot {
        self.0.lock().unwrap().snapshot()
    }

    fn apply(&mut self, edits: &[Edit]) -> Result<Snapshot, otto_pdf::Error> {
        self.0.lock().unwrap().apply(edits)
    }

    fn rebuild(
        &mut self,
        path: &Path,
        bytes: &[u8],
        history: &[Vec<Edit>],
    ) -> Result<Snapshot, otto_pdf::Error> {
        let mut fresh = Document::open_bytes(path.to_path_buf(), bytes.to_vec(), None)?;
        for edits in history {
            fresh.apply(edits)?;
        }
        let snapshot = fresh.snapshot();
        *self.0.lock().unwrap() = fresh;
        Ok(snapshot)
    }

    fn write(&mut self, path: &Path) -> Result<Vec<u8>, otto_pdf::Error> {
        self.0.lock().unwrap().save_to(path)
    }

    fn stamp(&self, path: &Path) -> Option<Stamp> {
        Stamp::of(path)
    }
}

/// A job, and who waits for how it went.
struct Request {
    job: Job,
    reply: Option<SyncSender<Outcome>>,
}

/// The queue of one open document: the sending end of its thread.
#[derive(Debug, Clone)]
pub struct Queue {
    sender: Sender<Request>,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request").field("job", &self.job).finish()
    }
}

impl Queue {
    /// Start the thread that runs `doc`'s jobs, keeping `ledger`. Each
    /// outcome goes to the window's viewer, which then draws again the
    /// pages it touched. The thread ends once the queue is dropped and the
    /// jobs sent before are done.
    ///
    /// # Errors
    ///
    /// When the thread can't be started.
    pub fn start(
        doc: Arc<Mutex<Document>>,
        mut ledger: Ledger,
        viewer: Weak<Mutex<Viewer>>,
    ) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::channel::<Request>();
        std::thread::Builder::new()
            .name("otto-preview-edits".into())
            .spawn(move || {
                for request in receiver {
                    let outcome = ledger.run(&mut Pdf(&doc), request.job);
                    let viewer = viewer.upgrade();
                    if let Some(viewer) = &viewer {
                        viewer.lock().unwrap().finish_batch(&doc, &outcome);
                        AppContext::request_wakeup();
                    }
                    if let Some(reply) = request.reply {
                        let _ = reply.send(outcome.clone());
                    }
                    if let Some(viewer) = &viewer {
                        crate::editing::redraw(viewer, &doc, &outcome.pages);
                    }
                }
            })?;
        Ok(Self { sender })
    }

    /// Put `job` at the end of the line; `reply`, when given, hears how it
    /// went once it has run. Returns whether the thread is still there to
    /// take it.
    pub fn send(&self, job: Job, reply: Option<SyncSender<Outcome>>) -> bool {
        self.sender.send(Request { job, reply }).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use otto_pdf::{AnnotKind, Annotation, Field, FieldKind, NewAnnotation, Page, Rect, Rgb};

    use super::*;

    /// A stand-in document: pages of annotations, each a colour at a
    /// place. Bytes "written" are remembered, to be opened again.
    #[derive(Debug, Clone, Default, PartialEq)]
    struct Fake {
        pages: Vec<Vec<(f32, Rgb)>>,
    }

    #[derive(Default)]
    struct FakeTarget {
        doc: Fake,
        files: HashMap<Vec<u8>, Fake>,
        written: Vec<PathBuf>,
        disk: Option<Stamp>,
        writes: usize,
    }

    /// A colour the stand-in refuses, for an edit that fails.
    const REFUSED: Rgb = Rgb(-1.0, 0.0, 0.0);

    impl Fake {
        fn snapshot(&self) -> Snapshot {
            let mut annotations = Vec::new();
            for (page, list) in self.pages.iter().enumerate() {
                for (index, (x, color)) in list.iter().enumerate() {
                    annotations.push(Annotation {
                        id: AnnotId { page, index },
                        rect: Rect::new(*x, 0.0, *x + 1.0, 1.0),
                        kind: AnnotKind::Note {
                            text: String::new(),
                        },
                        color: Some(*color),
                        author: None,
                        editable: true,
                    });
                }
            }
            Snapshot {
                pages: vec![
                    Page {
                        width: 100.0,
                        height: 100.0
                    };
                    self.pages.len()
                ],
                fields: Vec::new(),
                annotations,
                signed: false,
            }
        }

        fn apply(&mut self, edits: &[Edit]) -> Result<(), otto_pdf::Error> {
            let mut doomed = Vec::new();
            for edit in edits {
                match edit {
                    Edit::Add {
                        page,
                        annotation: NewAnnotation::Note { at, color, .. },
                    } => {
                        if *color == REFUSED {
                            return Err(otto_pdf::Error::Failed("refused".into()));
                        }
                        self.pages[*page].push((at.0, *color));
                    }
                    Edit::SetColor { annotation, color } => {
                        let entry = self.pages[annotation.page]
                            .get_mut(annotation.index)
                            .ok_or_else(|| otto_pdf::Error::Invalid("no such".into()))?;
                        entry.1 = *color;
                    }
                    Edit::Delete { annotation } => doomed.push(*annotation),
                    other => panic!("the stand-in doesn't do {other:?}"),
                }
            }
            doomed.sort_by(|a, b| b.cmp(a));
            for id in doomed {
                self.pages[id.page].remove(id.index);
            }
            Ok(())
        }

        fn bytes(&self) -> Vec<u8> {
            format!("{self:?}").into_bytes()
        }
    }

    impl Target for FakeTarget {
        fn snapshot(&self) -> Snapshot {
            self.doc.snapshot()
        }

        fn apply(&mut self, edits: &[Edit]) -> Result<Snapshot, otto_pdf::Error> {
            self.doc.apply(edits)?;
            Ok(self.doc.snapshot())
        }

        fn rebuild(
            &mut self,
            _path: &Path,
            bytes: &[u8],
            history: &[Vec<Edit>],
        ) -> Result<Snapshot, otto_pdf::Error> {
            let mut fresh = self
                .files
                .get(bytes)
                .cloned()
                .ok_or_else(|| otto_pdf::Error::Unreadable("unknown bytes".into()))?;
            for edits in history {
                fresh.apply(edits)?;
            }
            self.doc = fresh;
            Ok(self.doc.snapshot())
        }

        fn write(&mut self, path: &Path) -> Result<Vec<u8>, otto_pdf::Error> {
            let bytes = self.doc.bytes();
            self.files.insert(bytes.clone(), self.doc.clone());
            self.written.push(path.to_path_buf());
            self.writes += 1;
            Ok(bytes)
        }

        fn stamp(&self, _path: &Path) -> Option<Stamp> {
            self.disk
        }
    }

    /// Two different stamps, of two real files.
    fn stamps() -> (tempfile::TempDir, Stamp, Stamp) {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"bb").unwrap();
        let (a, b) = (Stamp::of(&a).unwrap(), Stamp::of(&b).unwrap());
        (dir, a, b)
    }

    /// A document of one page with three notes, x = 0, 10, 20, and its
    /// ledger, with the file on disk stamped `stamp`.
    fn three_notes(stamp: Stamp) -> (FakeTarget, Ledger) {
        let gray = Rgb(0.5, 0.5, 0.5);
        let doc = Fake {
            pages: vec![vec![(0.0, gray), (10.0, gray), (20.0, gray)]],
        };
        let bytes = doc.bytes();
        let mut target = FakeTarget {
            doc: doc.clone(),
            disk: Some(stamp),
            ..FakeTarget::default()
        };
        target.files.insert(bytes.clone(), doc);
        let ledger = Ledger::new(PathBuf::from("/doc.pdf"), bytes, Some(stamp));
        (target, ledger)
    }

    fn base(ledger: &Ledger, target: &FakeTarget) -> Base {
        Base {
            numbering: ledger.numbering,
            snapshot: Arc::new(target.snapshot()),
        }
    }

    /// Make `edits`, named against the document as it is.
    fn apply(ledger: &mut Ledger, target: &mut FakeTarget, edits: Vec<Edit>) -> Outcome {
        let job = Job::Apply {
            edits,
            base: base(ledger, target),
        };
        ledger.run(target, job)
    }

    fn note(x: f32, color: Rgb) -> Edit {
        Edit::Add {
            page: 0,
            annotation: NewAnnotation::Note {
                at: (x, 0.0),
                text: String::new(),
                color,
            },
        }
    }

    fn colors(target: &FakeTarget) -> Vec<(f32, Rgb)> {
        target.doc.pages[0].clone()
    }

    const RED: Rgb = Rgb(1.0, 0.0, 0.0);
    const BLUE: Rgb = Rgb(0.0, 0.0, 1.0);

    #[test]
    fn an_undo_then_a_redo_leave_the_edit_made_and_kept() {
        let (_dir, stamp, _) = stamps();
        let (mut target, mut ledger) = three_notes(stamp);
        let first = Job::Apply {
            edits: vec![note(30.0, RED)],
            base: base(&ledger, &target),
        };
        assert_eq!(ledger.run(&mut target, first).result, Ok(()));
        let undone = ledger.run(&mut target, Job::Undo);
        assert_eq!(undone.result, Ok(()));
        assert_eq!((undone.undo, undone.redo), (0, 1));
        assert!(!undone.unsaved, "back to the file as saved");
        assert_eq!(colors(&target).len(), 3);
        let redone = ledger.run(&mut target, Job::Redo);
        assert_eq!(redone.result, Ok(()));
        assert_eq!((redone.undo, redone.redo), (1, 0));
        assert!(redone.unsaved);
        assert_eq!(colors(&target)[3], (30.0, RED));
        // A second undo steps back from the redone edit, not past it.
        ledger.run(&mut target, Job::Undo);
        assert_eq!(colors(&target).len(), 3);
    }

    #[test]
    fn a_failed_edit_is_taken_out_and_not_kept() {
        let (_dir, stamp, _) = stamps();
        let (mut target, mut ledger) = three_notes(stamp);
        let before = target.snapshot();
        let outcome = apply(
            &mut ledger,
            &mut target,
            // The first is made before the second fails.
            vec![note(30.0, RED), note(40.0, REFUSED)],
        );
        assert!(matches!(outcome.result, Err(Failure::Engine(_))));
        assert_eq!(*outcome.snapshot, before, "the half-made edit is undone");
        assert_eq!(outcome.undo, 0);
        assert!(!outcome.unsaved);
        // An undo now has nothing to take back.
        ledger.run(&mut target, Job::Undo);
        assert_eq!(target.snapshot(), before);
    }

    #[test]
    fn an_edit_made_before_a_delete_ran_lands_on_what_it_named() {
        let (_dir, stamp, _) = stamps();
        let (mut target, mut ledger) = three_notes(stamp);
        // Both made against the same snapshot: delete the first note, then
        // recolour the third, which the delete moves down to index 1.
        let base = base(&ledger, &target);
        let delete = Job::Apply {
            edits: vec![Edit::Delete {
                annotation: AnnotId { page: 0, index: 0 },
            }],
            base: base.clone(),
        };
        let recolour = Job::Apply {
            edits: vec![Edit::SetColor {
                annotation: AnnotId { page: 0, index: 2 },
                color: BLUE,
            }],
            base: base.clone(),
        };
        assert_eq!(ledger.run(&mut target, delete).result, Ok(()));
        assert_eq!(ledger.run(&mut target, recolour).result, Ok(()));
        assert_eq!(colors(&target)[1], (20.0, BLUE));
        assert_eq!(colors(&target)[0].1, Rgb(0.5, 0.5, 0.5));
        // What was kept replays to the same document.
        let now = target.snapshot();
        ledger.run(&mut target, Job::Undo);
        ledger.run(&mut target, Job::Redo);
        assert_eq!(target.snapshot(), now);
        // An edit on the note the delete took is refused, not made on
        // whatever took its place.
        let gone = Job::Apply {
            edits: vec![Edit::SetColor {
                annotation: AnnotId { page: 0, index: 0 },
                color: RED,
            }],
            base,
        };
        let outcome = ledger.run(&mut target, gone);
        assert_eq!(outcome.result, Err(Failure::Stale));
        assert!(!colors(&target).iter().any(|(_, color)| *color == RED));
    }

    #[test]
    fn a_save_writes_what_came_before_it_and_starts_the_history_again() {
        let (_dir, stamp, _) = stamps();
        let (mut target, mut ledger) = three_notes(stamp);
        apply(&mut ledger, &mut target, vec![note(30.0, RED)]);
        let saved = ledger.run(&mut target, Job::Save { overwrite: false });
        assert_eq!(saved.result, Ok(()));
        assert!(!saved.unsaved);
        assert_eq!((saved.undo, saved.redo), (0, 0));
        assert_eq!(target.written, vec![PathBuf::from("/doc.pdf")]);
        // Undo after the save starts from what was saved.
        ledger.run(&mut target, Job::Undo);
        assert_eq!(colors(&target).len(), 4);
    }

    #[test]
    fn a_save_over_a_file_changed_on_disk_asks_first() {
        let (_dir, stamp, other) = stamps();
        let (mut target, mut ledger) = three_notes(stamp);
        apply(&mut ledger, &mut target, vec![note(30.0, RED)]);
        target.disk = Some(other);
        let refused = ledger.run(&mut target, Job::Save { overwrite: false });
        assert_eq!(refused.result, Err(Failure::ChangedOnDisk));
        assert!(refused.unsaved);
        assert_eq!(target.writes, 0);
        // Undo still works, from the bytes kept in memory.
        let undone = ledger.run(&mut target, Job::Undo);
        assert_eq!(undone.result, Ok(()));
        ledger.run(&mut target, Job::Redo);
        let saved = ledger.run(&mut target, Job::Save { overwrite: true });
        assert_eq!(saved.result, Ok(()));
        assert_eq!(saved.stamp, Some(other));
        // The file as just saved is the one later saves compare against.
        assert_eq!(
            ledger
                .run(&mut target, Job::Save { overwrite: false })
                .result,
            Ok(())
        );
    }

    #[test]
    fn a_save_as_carries_on_with_the_new_file() {
        let (_dir, stamp, _) = stamps();
        let (mut target, mut ledger) = three_notes(stamp);
        apply(&mut ledger, &mut target, vec![note(30.0, RED)]);
        let copy = PathBuf::from("/copy.pdf");
        let outcome = ledger.run(&mut target, Job::SaveAs(copy.clone()));
        assert_eq!(outcome.result, Ok(()));
        assert_eq!(outcome.path, copy);
        assert!(!outcome.unsaved);
        ledger.run(&mut target, Job::Save { overwrite: false });
        assert_eq!(target.written, vec![copy.clone(), copy]);
    }

    #[test]
    fn a_field_is_followed_by_name_and_place() {
        let field = |index, name: &str| Field {
            id: AnnotId { page: 0, index },
            name: name.into(),
            tooltip: None,
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            kind: FieldKind::Checkbox { checked: false },
            read_only: false,
            required: false,
            font_size: 0.0,
        };
        let old = Snapshot {
            fields: vec![field(3, "agree")],
            ..Snapshot::default()
        };
        let mut moved = field(2, "agree");
        moved.kind = FieldKind::Checkbox { checked: true };
        let now = Snapshot {
            fields: vec![moved],
            ..Snapshot::default()
        };
        let id = AnnotId { page: 0, index: 3 };
        assert_eq!(
            follow_field(&old, &now, id),
            Some(AnnotId { page: 0, index: 2 })
        );
        assert_eq!(follow_field(&old, &Snapshot::default(), id), None);
    }

    fn snapshot() -> Snapshot {
        let field = |page, index, name: &str| Field {
            id: AnnotId { page, index },
            name: name.to_owned(),
            tooltip: None,
            rect: Rect::new(10.0, 10.0, 60.0, 22.0),
            kind: FieldKind::Radio {
                checked: false,
                export: "a".into(),
            },
            read_only: false,
            required: false,
            font_size: 0.0,
        };
        Snapshot {
            pages: vec![
                Page {
                    width: 600.0,
                    height: 800.0,
                };
                3
            ],
            fields: vec![
                field(0, 0, "group"),
                field(2, 1, "group"),
                field(1, 0, "other"),
            ],
            annotations: Vec::new(),
            signed: false,
        }
    }

    #[test]
    fn a_field_edit_redraws_every_page_its_name_is_on() {
        let pages = pages_of(
            &snapshot(),
            &[Edit::SetChecked {
                field: AnnotId { page: 0, index: 0 },
                checked: true,
            }],
        );
        assert_eq!(pages, vec![0, 2]);
    }

    #[test]
    fn an_annotation_edit_redraws_its_page_once() {
        let id = AnnotId { page: 1, index: 3 };
        let pages = pages_of(
            &snapshot(),
            &[
                Edit::Move {
                    annotation: id,
                    dx: 1.0,
                    dy: 0.0,
                },
                Edit::SetColor {
                    annotation: id,
                    color: RED,
                },
            ],
        );
        assert_eq!(pages, vec![1]);
    }
}
