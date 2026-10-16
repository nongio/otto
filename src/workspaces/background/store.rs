//! Decoded wallpapers, shared by every workspace that shows them.
//!
//! A wallpaper is opened once per file and rendered once per screen size
//! into a *frame*: an image exactly the size of the output, drawn 1:1. Every
//! workspace on outputs of that size shares the frame, so ten workspaces cost
//! one decode and one frame rather than ten of each.
//!
//! On the compositor's thread a file is only stat'ed and its first few
//! kilobytes checked ([`wallpaper::probe`]), so an unreadable path or a file
//! that is not an image is reported to whoever set it straight away. Reading
//! the whole file, decoding and resampling are not cheap — an 8K photo takes
//! a few hundred milliseconds — so they happen on a worker thread, and the
//! frame comes back through the event loop; until it arrives a workspace
//! keeps whatever it was showing.

use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Arc, RwLock};
use std::time::SystemTime;

use layers::skia;
use otto_kit::wallpaper::{self, FrameStyle, Source};

/// A wallpaper file as it is on disk now. A file rewritten in place gets a
/// new key, so it is decoded again rather than served from the old pixels.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SourceKey {
    path: String,
    len: u64,
    modified: Option<SystemTime>,
}

impl SourceKey {
    fn stat(path: &str) -> Result<Self, String> {
        let meta = std::fs::metadata(path).map_err(|err| err.to_string())?;
        Ok(Self {
            path: path.to_string(),
            len: meta.len(),
            modified: meta.modified().ok(),
        })
    }
}

/// One frame: a source rendered for one screen size and style.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct FrameKey {
    source: SourceKey,
    target: (i32, i32),
    style: StyleKey,
}

/// A [`FrameStyle`], by value, as something that hashes: its floats by bit
/// pattern. Two styles that would render the same frame differently never
/// share a key; two that only differ by the sign of a zero merely render it
/// twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct StyleKey {
    mode: wallpaper::Mode,
    numbers: [u32; 16],
}

impl From<&FrameStyle> for StyleKey {
    fn from(style: &FrameStyle) -> Self {
        let layout = style.layout.sanitised();
        // Settings a mode ignores are left out, so changing them does not
        // render the same frame again.
        let position = if layout.mode.uses_position() {
            layout.position
        } else {
            (0.5, 0.5)
        };
        let zoom = if layout.mode.uses_zoom() {
            layout.zoom
        } else {
            1.0
        };
        let scale = if layout.mode.is_natural_size() {
            style.output_scale
        } else {
            1.0
        };
        let span = match (layout.mode, style.span) {
            (wallpaper::Mode::Span, Some(span)) => [
                span.desktop.left,
                span.desktop.top,
                span.desktop.right,
                span.desktop.bottom,
                span.screen.left,
                span.screen.top,
                span.screen.right,
                span.screen.bottom,
            ],
            _ => [0.0; 8],
        };
        Self {
            mode: layout.mode,
            numbers: [
                position.0.to_bits(),
                position.1.to_bits(),
                zoom.to_bits(),
                scale.to_bits(),
                style.fill.r.to_bits(),
                style.fill.g.to_bits(),
                style.fill.b.to_bits(),
                // Frames are opaque whatever the fill's alpha.
                0,
                span[0].to_bits(),
                span[1].to_bits(),
                span[2].to_bits(),
                span[3].to_bits(),
                span[4].to_bits(),
                span[5].to_bits(),
                span[6].to_bits(),
                span[7].to_bits(),
            ],
        }
    }
}

/// A finished frame, on its way back from the worker.
pub struct FrameDone {
    key: FrameKey,
    /// `None` when the image could not be decoded.
    frame: Option<skia::Image>,
    /// Nobody wanted the frame any more by the time the worker got to it.
    cancelled: bool,
}

/// What [`WallpaperStore::lookup`] has for a request.
pub(crate) enum Lookup {
    Ready(skia::Image),
    /// Being rendered; keep showing what is there.
    Pending,
    /// The file cannot be shown, and why.
    Failed(String),
}

struct Job {
    key: FrameKey,
    style: FrameStyle,
}

/// Where finished frames go: the event loop in the compositor, a plain
/// channel in tests.
pub(crate) type FrameSink = Box<dyn Fn(FrameDone) + Send>;

/// How long the worker keeps a decoded image once it has nothing to do: long
/// enough for the frames of every output after a change, short enough that a
/// full-size decode is not held for the rest of the session.
const KEEP_DECODED: std::time::Duration = std::time::Duration::from_secs(10);

pub(crate) struct WallpaperStore {
    frames: HashMap<FrameKey, skia::Image>,
    failed_sources: HashMap<SourceKey, String>,
    failed_frames: HashSet<FrameKey>,
    in_flight: HashSet<FrameKey>,
    /// The frames the last finished pass asked for. Shared with the worker so
    /// it can skip a job nobody wants any more — a wallpaper picked and then
    /// replaced before it finished decoding.
    wanted: Arc<RwLock<HashSet<FrameKey>>>,
    /// The frames the pass in progress has asked for so far. Swapped into
    /// `wanted` at [`Self::end_pass`], so the worker never sees a half-built
    /// set and cancels a job that is still wanted.
    next_wanted: HashSet<FrameKey>,
    /// Path to key, within one pass, so a path is stat'ed and probed once per
    /// pass however many workspaces show it.
    stat_cache: HashMap<String, Result<SourceKey, String>>,
    sink: Option<FrameSink>,
    jobs: Option<mpsc::Sender<Job>>,
}

impl WallpaperStore {
    pub(crate) fn new(sink: FrameSink) -> Self {
        Self {
            frames: HashMap::new(),
            failed_sources: HashMap::new(),
            failed_frames: HashSet::new(),
            in_flight: HashSet::new(),
            wanted: Arc::new(RwLock::new(HashSet::new())),
            next_wanted: HashSet::new(),
            stat_cache: HashMap::new(),
            sink: Some(sink),
            jobs: None,
        }
    }

    /// Start a pass over every workspace. Whatever the pass does not ask for
    /// is dropped at [`Self::end_pass`].
    pub(crate) fn begin_pass(&mut self) {
        self.stat_cache.clear();
        self.next_wanted.clear();
    }

    /// The frame showing `path` in `style` on a `target`-sized screen.
    pub(crate) fn lookup(&mut self, path: &str, style: &FrameStyle, target: (i32, i32)) -> Lookup {
        let failed_sources = &self.failed_sources;
        let source_key = match self.stat_cache.entry(path.to_string()).or_insert_with(|| {
            let key = SourceKey::stat(path)?;
            // A file that already failed is not probed again until the
            // failures are forgotten.
            if let Some(err) = failed_sources.get(&key) {
                return Err(err.clone());
            }
            wallpaper::probe(path).map_err(|err| err.to_string())?;
            Ok(key)
        }) {
            Ok(key) => key.clone(),
            Err(err) => return Lookup::Failed(err.clone()),
        };
        let key = FrameKey {
            source: source_key,
            target,
            style: StyleKey::from(style),
        };
        self.next_wanted.insert(key.clone());
        if let Some(frame) = self.frames.get(&key) {
            return Lookup::Ready(frame.clone());
        }
        if self.failed_frames.contains(&key) {
            return Lookup::Failed("the image could not be decoded".to_string());
        }
        if !self.in_flight.contains(&key) {
            self.in_flight.insert(key.clone());
            self.submit(Job { key, style: *style });
        }
        Lookup::Pending
    }

    /// Drop every frame the pass did not ask for, and let the worker know
    /// what is wanted now.
    pub(crate) fn end_pass(&mut self) {
        let wanted = std::mem::take(&mut self.next_wanted);
        self.frames.retain(|key, _| wanted.contains(key));
        self.failed_frames.retain(|key| wanted.contains(key));
        *self
            .wanted
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = wanted;
    }

    /// Take a finished frame. Returns whether the workspaces need another
    /// pass to pick it up.
    pub(crate) fn finish(&mut self, done: FrameDone) -> bool {
        self.in_flight.remove(&done.key);
        let wanted = self
            .wanted
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(&done.key);
        if done.cancelled {
            // Still wanted after all — asked for again since the worker
            // skipped it: the next pass submits it afresh.
            return wanted;
        }
        if !wanted {
            return false;
        }
        match done.frame {
            Some(frame) => {
                self.frames.insert(done.key, frame);
            }
            None => {
                tracing::warn!("wallpaper: cannot decode {}", done.key.source.path);
                self.failed_sources.insert(
                    done.key.source.clone(),
                    "the image could not be decoded".into(),
                );
                self.failed_frames.insert(done.key);
            }
        }
        true
    }

    /// Forget every failure, so the next pass tries those files again — for
    /// when the user sets a path explicitly, and should hear about it afresh.
    pub(crate) fn forget_failures(&mut self) {
        self.failed_sources.clear();
        self.failed_frames.clear();
    }

    /// Whether a frame the last pass asked for is still being rendered.
    pub(crate) fn pending(&self) -> bool {
        let wanted = self
            .wanted
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.in_flight.iter().any(|key| wanted.contains(key))
    }

    fn submit(&mut self, job: Job) {
        if self.jobs.is_none() {
            self.jobs = self.spawn_worker();
        }
        let failed = match &self.jobs {
            Some(jobs) => jobs.send(job).err().map(|err| err.0),
            None => Some(job),
        };
        if let Some(job) = failed {
            // No worker: render here rather than not at all.
            tracing::warn!("wallpaper: no worker thread, rendering on the compositor thread");
            let done = run(job, &mut Decoded::default());
            if let Some(sink) = &self.sink {
                sink(done);
            }
        }
    }

    fn spawn_worker(&mut self) -> Option<mpsc::Sender<Job>> {
        let sink = self.sink.take()?;
        let wanted = self.wanted.clone();
        let (jobs, queue) = mpsc::channel::<Job>();
        let spawned = std::thread::Builder::new()
            .name("otto-wallpaper".into())
            .spawn(move || {
                // Below the compositor: a wallpaper can wait a frame, the
                // desktop cannot. The resample's own threads inherit it.
                // SAFETY: `setpriority` on the calling thread (`0` is this
                // thread under Linux's per-thread nice), no pointers.
                unsafe {
                    libc::setpriority(libc::PRIO_PROCESS, 0, 10);
                }
                let mut decoded = Decoded::default();
                loop {
                    let job = match queue.recv_timeout(KEEP_DECODED) {
                        Ok(job) => job,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            decoded = Decoded::default();
                            continue;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    let still_wanted = wanted
                        .read()
                        .map(|wanted| wanted.contains(&job.key))
                        .unwrap_or(true);
                    if !still_wanted {
                        sink(FrameDone {
                            key: job.key,
                            frame: None,
                            cancelled: true,
                        });
                        continue;
                    }
                    sink(run(job, &mut decoded));
                }
            });
        match spawned {
            Ok(_) => Some(jobs),
            Err(err) => {
                tracing::warn!("wallpaper: cannot start the worker thread: {err}");
                None
            }
        }
    }
}

/// The last file the worker opened and decoded, so the frames of one file
/// for several screens share one read and one decode.
#[derive(Default)]
struct Decoded {
    key: Option<SourceKey>,
    source: Option<Source>,
}

/// Render one job. A panic in a decoder or the resampler comes back as a
/// failed frame rather than taking the worker down with the job still in
/// flight.
fn run(job: Job, decoded: &mut Decoded) -> FrameDone {
    let frame = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render(&job, decoded)))
        .unwrap_or_else(|_| {
            tracing::warn!("wallpaper: rendering {} panicked", job.key.source.path);
            *decoded = Decoded::default();
            None
        });
    FrameDone {
        key: job.key,
        frame,
        cancelled: false,
    }
}

fn render(job: &Job, decoded: &mut Decoded) -> Option<skia::Image> {
    if decoded.key.as_ref() != Some(&job.key.source) {
        *decoded = Decoded::default();
        let source = match wallpaper::open(&job.key.source.path).ok()? {
            Source::Raster(lazy) => {
                Source::Raster(lazy.make_raster_image(None, skia::image::CachingHint::Disallow)?)
            }
            svg => svg,
        };
        decoded.key = Some(job.key.source.clone());
        decoded.source = Some(source);
    }
    wallpaper::render_frame(decoded.source.as_ref()?, &job.style, job.key.target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn write_png(name: &str, w: i32, h: i32, colour: skia::Color) -> String {
        let mut surface = skia::surfaces::raster_n32_premul((w, h)).unwrap();
        surface.canvas().clear(colour);
        let png = surface
            .image_snapshot()
            .encode(None, skia::EncodedImageFormat::PNG, 100)
            .unwrap();
        let dir = std::env::temp_dir().join(format!("otto-wallpaper-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, png.as_bytes()).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn store() -> (WallpaperStore, mpsc::Receiver<FrameDone>) {
        let (tx, rx) = mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        let store = WallpaperStore::new(Box::new(move |done| {
            let _ = tx.lock().unwrap().send(done);
        }));
        (store, rx)
    }

    /// Run the worker's results back into the store until nothing is
    /// pending, as the event loop does.
    fn drain(store: &mut WallpaperStore, rx: &mpsc::Receiver<FrameDone>) {
        while store.pending() {
            let done = rx.recv_timeout(Duration::from_secs(30)).expect("a frame");
            store.finish(done);
        }
    }

    /// A frame is rendered once and then served to every workspace that
    /// asks for the same file at the same size.
    #[test]
    fn one_frame_serves_every_workspace_of_a_size() {
        let (mut store, rx) = store();
        let path = write_png("shared.png", 64, 36, skia::Color::RED);
        store.begin_pass();
        assert!(matches!(
            store.lookup(&path, &FrameStyle::default(), (32, 32)),
            Lookup::Pending
        ));
        assert!(matches!(
            store.lookup(&path, &FrameStyle::default(), (32, 32)),
            Lookup::Pending
        ));
        store.end_pass();
        drain(&mut store, &rx);

        store.begin_pass();
        let (Lookup::Ready(a), Lookup::Ready(b)) = (
            store.lookup(&path, &FrameStyle::default(), (32, 32)),
            store.lookup(&path, &FrameStyle::default(), (32, 32)),
        ) else {
            panic!("the frame is ready");
        };
        store.end_pass();
        assert_eq!(a.unique_id(), b.unique_id());
        assert_eq!((a.width(), a.height()), (32, 32));
        assert!(rx.try_recv().is_err(), "rendered once");
    }

    /// A frame nobody asks for any more is dropped, and so is its source.
    #[test]
    fn unused_frames_are_dropped() {
        let (mut store, rx) = store();
        let first = write_png("first.png", 16, 16, skia::Color::RED);
        let second = write_png("second.png", 16, 16, skia::Color::BLUE);
        store.begin_pass();
        store.lookup(&first, &FrameStyle::default(), (8, 8));
        store.end_pass();
        drain(&mut store, &rx);
        assert_eq!(store.frames.len(), 1);

        store.begin_pass();
        store.lookup(&second, &FrameStyle::default(), (8, 8));
        store.end_pass();
        assert!(store.frames.is_empty());
    }

    /// An unreadable path fails at once, without a trip to the worker.
    #[test]
    fn an_unreadable_path_fails_straight_away() {
        let (mut store, rx) = store();
        store.begin_pass();
        assert!(matches!(
            store.lookup("/nonexistent/otto.png", &FrameStyle::default(), (8, 8)),
            Lookup::Failed(_)
        ));
        store.end_pass();
        assert!(!store.pending());
        assert!(rx.try_recv().is_err());
    }

    /// A file rewritten in place is decoded again, not served stale.
    #[test]
    fn a_rewritten_file_is_decoded_again() {
        let (mut store, rx) = store();
        let path = write_png("rewritten.png", 16, 16, skia::Color::RED);
        store.begin_pass();
        store.lookup(&path, &FrameStyle::default(), (8, 8));
        store.end_pass();
        drain(&mut store, &rx);

        // A different size, so the length and the key change even where the
        // filesystem's timestamps are too coarse to.
        let rewritten = write_png("rewritten.png", 20, 20, skia::Color::BLUE);
        assert_eq!(rewritten, path);
        store.begin_pass();
        assert!(matches!(
            store.lookup(&path, &FrameStyle::default(), (8, 8)),
            Lookup::Pending
        ));
        store.end_pass();
    }

    /// A frame finished after its file was replaced is discarded, not shown.
    #[test]
    fn a_frame_nobody_wants_any_more_is_discarded() {
        let (mut store, rx) = store();
        let old = write_png("old.png", 16, 16, skia::Color::RED);
        let new = write_png("new.png", 16, 16, skia::Color::BLUE);
        store.begin_pass();
        store.lookup(&old, &FrameStyle::default(), (8, 8));
        store.end_pass();
        store.begin_pass();
        store.lookup(&new, &FrameStyle::default(), (8, 8));
        store.end_pass();
        drain(&mut store, &rx);
        // Whatever the worker did with the old job, only the new frame is
        // kept.
        while let Ok(done) = rx.recv_timeout(Duration::from_millis(200)) {
            store.finish(done);
        }
        assert_eq!(store.frames.len(), 1);
        assert!(store.frames.keys().all(|key| key.source.path == new));
    }
}
