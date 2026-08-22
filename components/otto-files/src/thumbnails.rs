//! Thumbnails for the entries on screen: what has been found, what is being
//! looked for, and what turned out not to exist.
//!
//! The browser draws every file with its type's icon. A thumbnail replaces
//! that icon for the files where the picture *is* the identity — a photo, a
//! page, a frame of video — and the whole point is that it must not cost
//! anything to get one. Two rules keep it that way:
//!
//! * **Only what is visible is asked for.** A folder of ten thousand pictures
//!   costs the same as a folder of thirty, because the caller feeds this store
//!   the range the viewport is actually showing and nothing else.
//! * **Only a few at a time.** Every lookup that misses the shared cache ends
//!   in a sandboxed decode, which is a process; without a ceiling, scrolling
//!   through a photo library would fork one per file. [`MAX_IN_FLIGHT`] is
//!   that ceiling.
//!
//! The store holds no thread of its own and starts no work. [`Store::wanted`]
//! says what is worth fetching and the host — which owns the runtime — fetches
//! it and reports back through [`Store::finish`]. That keeps this module
//! testable without a compositor, a worker pool or a filesystem.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use skia_safe as skia;

use crate::thumbcache;

/// How many decodes may be outstanding at once.
///
/// Small on purpose. Each one is a sandboxed process, and the work is only
/// ever for what is on screen right now — a scroll that outruns the decoder
/// should leave stale requests behind rather than pile more on.
pub const MAX_IN_FLIGHT: usize = 4;

/// How many thumbnails to keep. Past this the oldest are dropped.
///
/// A grid cell's worth of pixels is around a quarter of a megabyte, so this is
/// a memory ceiling in the tens of megabytes — enough to scroll a large folder
/// and come back without re-fetching, far short of holding a photo library
/// resident.
pub const CAPACITY: usize = 512;

/// Where one file's thumbnail has got to.
enum State {
    /// Somebody is looking for it.
    Pending,
    /// Found, decoded, ready to draw.
    Ready(skia::Image),
    /// Looked for and not available — no cached thumbnail, and either no way
    /// to make one or an attempt that failed. Remembered so it is not looked
    /// for again on every frame.
    Absent,
}

struct Slot {
    state: State,
    /// The modification time this was fetched against. A file rewritten under
    /// us invalidates its thumbnail, and comparing on read is what notices.
    modified: Option<SystemTime>,
    /// Insertion order, for eviction.
    stamp: u64,
}

/// One file to fetch a thumbnail for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    /// The size to look for in the shared cache, and to decode to if nothing
    /// is there.
    pub size: thumbcache::Size,
    /// Whether this application can produce the thumbnail itself when the
    /// shared cache has none. False for the kinds Otto has no decoder for —
    /// the lookup is still worth doing, because another application may have
    /// left one behind, but a miss is final.
    pub may_generate: bool,
}

/// What a finished job found.
pub enum Found {
    Thumbnail(skia::Image),
    Nothing,
}

/// The thumbnails this window knows about.
#[derive(Default)]
pub struct Store {
    slots: HashMap<PathBuf, Slot>,
    in_flight: usize,
    clock: u64,
    /// Bumped whenever something lands that changes what a pane would draw.
    ///
    /// The panes are cached pictures, replayed until their content key moves;
    /// a thumbnail arriving changes the picture without changing anything else
    /// the key is made of, so the key folds this in and a landing thumbnail
    /// invalidates exactly the panes that might show it.
    epoch: u64,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    /// The counter the pane content keys mix in. See [`Store::epoch`].
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The thumbnail to draw for `path`, if one is ready and still current.
    ///
    /// Takes the entry's modification time because a thumbnail outlives the
    /// file it was made from: a picture edited in place keeps its path, and
    /// drawing the old thumbnail would be showing the user something that is
    /// no longer there.
    pub fn image(&self, path: &Path, modified: Option<SystemTime>) -> Option<&skia::Image> {
        let slot = self.slots.get(path)?;
        if slot.modified != modified {
            return None;
        }
        match &slot.state {
            State::Ready(image) => Some(image),
            _ => None,
        }
    }

    /// Which of `visible` are worth fetching, in the order given, up to the
    /// in-flight ceiling.
    ///
    /// Marks what it returns as pending, so calling it again before those jobs
    /// finish does not hand out the same work twice. Everything already
    /// known — ready, pending, or established to be absent — is skipped, as is
    /// anything this application has previously recorded a failure for.
    pub fn wanted(
        &mut self,
        visible: impl IntoIterator<Item = Request>,
        size: thumbcache::Size,
    ) -> Vec<Job> {
        let mut jobs = Vec::new();
        for request in visible {
            if self.in_flight + jobs.len() >= MAX_IN_FLIGHT {
                break;
            }
            if !self.needs_fetch(&request) {
                continue;
            }
            // A file this application already failed on stays failed until it
            // changes: the marker in the shared cache is exactly the record of
            // "do not try this again", and honouring it is what keeps a folder
            // of unreadable files from re-forking a worker per visit.
            if thumbcache::is_known_failure(&request.path, request.modified) {
                self.insert(request.path, request.modified, State::Absent);
                continue;
            }
            self.insert(request.path.clone(), request.modified, State::Pending);
            jobs.push(Job {
                path: request.path,
                modified: request.modified,
                size,
                may_generate: request.may_generate,
            });
        }
        self.in_flight += jobs.len();
        jobs
    }

    /// Whether this file is worth a fetch: not already known, and not already
    /// being fetched.
    fn needs_fetch(&self, request: &Request) -> bool {
        match self.slots.get(&request.path) {
            // Known against a different version of the file — the picture has
            // changed and the old answer, whatever it was, is void.
            Some(slot) if slot.modified != request.modified => true,
            Some(_) => false,
            None => true,
        }
    }

    /// Record what a job found. Wakes the panes that might draw it.
    pub fn finish(&mut self, path: PathBuf, modified: Option<SystemTime>, found: Found) {
        self.in_flight = self.in_flight.saturating_sub(1);
        let state = match found {
            Found::Thumbnail(image) => State::Ready(image),
            Found::Nothing => State::Absent,
        };
        // Only a picture landing changes what is drawn. A miss changes only
        // what will be asked for again, and repainting for it would be a frame
        // that renders the same pixels.
        let repaint = matches!(state, State::Ready(_));
        self.insert(path, modified, state);
        if repaint {
            self.epoch = self.epoch.wrapping_add(1);
        }
    }

    /// Whether anything is outstanding — the host's cue to keep the frame loop
    /// alive so results are painted as they land.
    pub fn is_busy(&self) -> bool {
        self.in_flight > 0
    }

    fn insert(&mut self, path: PathBuf, modified: Option<SystemTime>, state: State) {
        self.clock = self.clock.wrapping_add(1);
        let stamp = self.clock;
        self.slots.insert(
            path,
            Slot {
                state,
                modified,
                stamp,
            },
        );
        self.evict();
    }

    /// Drop the oldest entries once over capacity.
    ///
    /// Oldest by insertion, not by use: tracking use would mean a mutable
    /// borrow on every draw, and the access pattern here is a scroll through a
    /// folder, where insertion order and recency are near enough the same
    /// thing.
    fn evict(&mut self) {
        while self.slots.len() > CAPACITY {
            let Some(oldest) = self
                .slots
                .iter()
                // A pending slot is somebody's outstanding job; evicting it
                // would let the same work be started again while the first is
                // still running.
                .filter(|(_, slot)| !matches!(slot.state, State::Pending))
                .min_by_key(|(_, slot)| slot.stamp)
                .map(|(path, _)| path.clone())
            else {
                return;
            };
            self.slots.remove(&oldest);
        }
    }

    /// Forget everything. For a refresh, where the directory's contents may
    /// have changed under every path at once.
    pub fn clear(&mut self) {
        self.slots.clear();
        // In-flight jobs are deliberately still counted: they are still
        // running, and their results will be dropped on arrival by the mtime
        // check. Zeroing the count here would let the ceiling be exceeded.
        self.epoch = self.epoch.wrapping_add(1);
    }
}

/// Carry out one job: look in the shared cache, and failing that decode the
/// file ourselves if it is a kind we can decode.
///
/// **Blocks.** It reads files and may spawn a sandboxed worker, so it belongs
/// on a background thread and never on the UI thread.
///
/// Nothing is written back to the shared cache — not the thumbnail, and not a
/// failure marker. Publishing there is a promise to every other file manager
/// on the system about the bytes and their size, and this half only consumes.
/// A miss is remembered for the lifetime of the window instead, by the
/// [`State::Absent`] the caller records.
pub fn fetch(job: &Job) -> Found {
    if let Some(image) = thumbcache::lookup(&job.path, job.modified, job.size) {
        return Found::Thumbnail(image);
    }
    if !job.may_generate {
        return Found::Nothing;
    }

    // The same sandboxed decoder Quick View uses, asked for a thumbnail-sized
    // picture rather than a panel-sized one. Untrusted bytes are parsed in the
    // worker, never here.
    let request = otto_quickview::decode::Request {
        width: job.size.pixels(),
        height: job.size.pixels(),
        name: job
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        ..Default::default()
    };
    match otto_quickview::decode_path(&job.path, &request) {
        otto_kit::preview::Preview::Pixels { pixels, .. } => match pixels.to_image() {
            Some(image) => Found::Thumbnail(image),
            None => Found::Nothing,
        },
        // Everything else a previewer can return — a text listing, an archive's
        // contents, an unavailable file — is not a picture, and standing it in
        // for one would put a wall of identical grey cards in the grid where
        // the type icons say something useful.
        _ => Found::Nothing,
    }
}

/// One visible entry, as the store needs to see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub may_generate: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(name: &str) -> Request {
        Request {
            path: PathBuf::from(format!("/tmp/{name}")),
            modified: Some(SystemTime::UNIX_EPOCH),
            may_generate: true,
        }
    }

    /// A 1×1 image, standing in for a decoded thumbnail.
    fn image() -> skia::Image {
        let info = skia::ImageInfo::new(
            (1, 1),
            skia::ColorType::RGBA8888,
            skia::AlphaType::Premul,
            None,
        );
        skia::images::raster_from_data(&info, skia::Data::new_copy(&[0, 0, 0, 255]), 4).unwrap()
    }

    #[test]
    fn asks_for_what_it_does_not_have() {
        let mut store = Store::new();
        let jobs = store.wanted([request("a"), request("b")], thumbcache::Size::Normal);
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].path, PathBuf::from("/tmp/a"));
    }

    /// The same entries coming round on the next frame must not be fetched
    /// again while their first fetch is still running.
    #[test]
    fn does_not_ask_twice_for_a_pending_file() {
        let mut store = Store::new();
        let first = store.wanted([request("a")], thumbcache::Size::Normal);
        assert_eq!(first.len(), 1);
        let second = store.wanted([request("a")], thumbcache::Size::Normal);
        assert!(second.is_empty());
    }

    #[test]
    fn does_not_ask_again_for_a_known_absence() {
        let mut store = Store::new();
        store.wanted([request("a")], thumbcache::Size::Normal);
        store.finish(
            PathBuf::from("/tmp/a"),
            Some(SystemTime::UNIX_EPOCH),
            Found::Nothing,
        );
        assert!(store
            .wanted([request("a")], thumbcache::Size::Normal)
            .is_empty());
    }

    #[test]
    fn stops_at_the_in_flight_ceiling() {
        let mut store = Store::new();
        let many: Vec<Request> = (0..20).map(|n| request(&format!("f{n}"))).collect();
        let jobs = store.wanted(many.clone(), thumbcache::Size::Normal);
        assert_eq!(jobs.len(), MAX_IN_FLIGHT);

        // Nothing more until something finishes.
        assert!(store
            .wanted(many.clone(), thumbcache::Size::Normal)
            .is_empty());

        store.finish(jobs[0].path.clone(), jobs[0].modified, Found::Nothing);
        assert_eq!(store.wanted(many, thumbcache::Size::Normal).len(), 1);
    }

    #[test]
    fn serves_a_finished_thumbnail() {
        let mut store = Store::new();
        let mtime = Some(SystemTime::UNIX_EPOCH);
        store.wanted([request("a")], thumbcache::Size::Normal);
        store.finish(PathBuf::from("/tmp/a"), mtime, Found::Thumbnail(image()));
        assert!(store.image(Path::new("/tmp/a"), mtime).is_some());
    }

    /// The file changed after its thumbnail was made: the old picture is not
    /// of this file any more and must not be drawn, and the file is worth
    /// fetching again.
    #[test]
    fn a_rewritten_file_invalidates_its_thumbnail() {
        let mut store = Store::new();
        let old = Some(SystemTime::UNIX_EPOCH);
        let new = Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(60));
        store.wanted([request("a")], thumbcache::Size::Normal);
        store.finish(PathBuf::from("/tmp/a"), old, Found::Thumbnail(image()));

        assert!(store.image(Path::new("/tmp/a"), new).is_none());
        let refetch = Request {
            path: PathBuf::from("/tmp/a"),
            modified: new,
            may_generate: true,
        };
        assert_eq!(store.wanted([refetch], thumbcache::Size::Normal).len(), 1);
    }

    /// A picture landing must move the key the panes are cached on; a miss
    /// must not, or every unthumbnailable file in a folder would cost a
    /// repaint.
    #[test]
    fn only_a_picture_moves_the_epoch() {
        let mut store = Store::new();
        let mtime = Some(SystemTime::UNIX_EPOCH);
        store.wanted([request("a"), request("b")], thumbcache::Size::Normal);

        let before = store.epoch();
        store.finish(PathBuf::from("/tmp/a"), mtime, Found::Nothing);
        assert_eq!(store.epoch(), before);

        store.finish(PathBuf::from("/tmp/b"), mtime, Found::Thumbnail(image()));
        assert_ne!(store.epoch(), before);
    }

    #[test]
    fn evicts_the_oldest_past_capacity() {
        let mut store = Store::new();
        let mtime = Some(SystemTime::UNIX_EPOCH);
        for n in 0..CAPACITY + 10 {
            store.insert(
                PathBuf::from(format!("/tmp/f{n}")),
                mtime,
                State::Ready(image()),
            );
        }
        assert_eq!(store.slots.len(), CAPACITY);
        // The first inserted are the first gone; the last inserted survive.
        assert!(store.image(Path::new("/tmp/f0"), mtime).is_none());
        assert!(store
            .image(Path::new(&format!("/tmp/f{}", CAPACITY + 9)), mtime)
            .is_some());
    }

    /// Eviction must never take a slot somebody is still working on, or the
    /// same file would be fetched twice over.
    #[test]
    fn never_evicts_a_pending_slot() {
        let mut store = Store::new();
        let mtime = Some(SystemTime::UNIX_EPOCH);
        store.insert(PathBuf::from("/tmp/pending"), mtime, State::Pending);
        for n in 0..CAPACITY + 10 {
            store.insert(
                PathBuf::from(format!("/tmp/f{n}")),
                mtime,
                State::Ready(image()),
            );
        }
        assert!(store.slots.contains_key(Path::new("/tmp/pending")));
    }

    #[test]
    fn is_busy_while_work_is_outstanding() {
        let mut store = Store::new();
        assert!(!store.is_busy());
        let jobs = store.wanted([request("a")], thumbcache::Size::Normal);
        assert!(store.is_busy());
        store.finish(jobs[0].path.clone(), jobs[0].modified, Found::Nothing);
        assert!(!store.is_busy());
    }
}
