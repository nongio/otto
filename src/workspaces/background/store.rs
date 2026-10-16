//! Decoded wallpapers, shared by every workspace that shows them.
//!
//! A wallpaper is opened once per file and rendered once per screen size
//! into a *frame*: an image exactly the size of the output, drawn 1:1. Every
//! workspace on outputs of that size shares the frame, so ten workspaces cost
//! one decode and one frame rather than ten of each.
//!
//! Opening is synchronous and cheap — the file's bytes and header — so an
//! unreadable path is reported to whoever set it straight away. Decoding and
//! resampling are not: an 8K photo takes a few hundred milliseconds, which
//! the compositor cannot spend on its own thread. Frames are rendered on a
//! worker thread and handed back through the event loop; until one arrives
//! a workspace keeps whatever it was showing.

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
    source: Source,
}

/// Where finished frames go: the event loop in the compositor, a plain
/// channel in tests.
pub(crate) type FrameSink = Box<dyn Fn(FrameDone) + Send>;

pub(crate) struct WallpaperStore {
    sources: HashMap<SourceKey, Source>,
    frames: HashMap<FrameKey, skia::Image>,
    failed_sources: HashMap<SourceKey, String>,
    failed_frames: HashSet<FrameKey>,
    in_flight: HashSet<FrameKey>,
    /// The frames asked for in the current pass. Shared with the worker so
    /// it can skip a job nobody wants any more — a wallpaper picked and then
    /// replaced before it finished decoding.
    wanted: Arc<RwLock<HashSet<FrameKey>>>,
    /// Path to key, within one pass, so a path is stat'ed once per pass
    /// however many workspaces show it.
    stat_cache: HashMap<String, Result<SourceKey, String>>,
    sink: Option<FrameSink>,
    jobs: Option<mpsc::Sender<Job>>,
}

impl WallpaperStore {
    pub(crate) fn new(sink: FrameSink) -> Self {
        Self {
            sources: HashMap::new(),
            frames: HashMap::new(),
            failed_sources: HashMap::new(),
            failed_frames: HashSet::new(),
            in_flight: HashSet::new(),
            wanted: Arc::new(RwLock::new(HashSet::new())),
            stat_cache: HashMap::new(),
            sink: Some(sink),
            jobs: None,
        }
    }

    /// Start a pass over every workspace. Whatever the pass does not ask for
    /// is dropped at [`Self::end_pass`].
    pub(crate) fn begin_pass(&mut self) {
        self.stat_cache.clear();
        if let Ok(mut wanted) = self.wanted.write() {
            wanted.clear();
        }
    }

    /// The frame showing `path` in `style` on a `target`-sized screen.
    pub(crate) fn lookup(&mut self, path: &str, style: &FrameStyle, target: (i32, i32)) -> Lookup {
        let source_key = match self
            .stat_cache
            .entry(path.to_string())
            .or_insert_with(|| SourceKey::stat(path))
        {
            Ok(key) => key.clone(),
            Err(err) => return Lookup::Failed(err.clone()),
        };
        if let Some(err) = self.failed_sources.get(&source_key) {
            return Lookup::Failed(err.clone());
        }
        let key = FrameKey {
            source: source_key.clone(),
            target,
            style: StyleKey::from(style),
        };
        if let Ok(mut wanted) = self.wanted.write() {
            wanted.insert(key.clone());
        }
        if let Some(frame) = self.frames.get(&key) {
            return Lookup::Ready(frame.clone());
        }
        if self.failed_frames.contains(&key) {
            return Lookup::Failed("the image could not be decoded".to_string());
        }
        if self.in_flight.contains(&key) {
            return Lookup::Pending;
        }

        let source = match self.sources.get(&source_key) {
            Some(source) => source.clone(),
            None => match wallpaper::open(path) {
                Ok(source) => {
                    self.sources.insert(source_key.clone(), source.clone());
                    source
                }
                Err(err) => {
                    let err = err.to_string();
                    self.failed_sources.insert(source_key, err.clone());
                    return Lookup::Failed(err);
                }
            },
        };
        self.submit(Job {
            key: key.clone(),
            style: *style,
            source,
        });
        self.in_flight.insert(key);
        Lookup::Pending
    }

    /// Drop every frame and source the pass did not ask for.
    pub(crate) fn end_pass(&mut self) {
        let Ok(wanted) = self.wanted.read() else {
            return;
        };
        self.frames.retain(|key, _| wanted.contains(key));
        self.failed_frames.retain(|key| wanted.contains(key));
        let sources: HashSet<&SourceKey> = wanted.iter().map(|key| &key.source).collect();
        self.sources.retain(|key, _| sources.contains(key));
    }

    /// Take a finished frame. Returns whether the workspaces need another
    /// pass to pick it up.
    pub(crate) fn finish(&mut self, done: FrameDone) -> bool {
        self.in_flight.remove(&done.key);
        let wanted = self
            .wanted
            .read()
            .map(|wanted| wanted.contains(&done.key))
            .unwrap_or(false);
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

    /// Whether a frame this pass asked for is still being rendered.
    pub(crate) fn pending(&self) -> bool {
        self.wanted
            .read()
            .map(|wanted| self.in_flight.iter().any(|key| wanted.contains(key)))
            .unwrap_or(false)
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
            let frame = render(&job, &mut None);
            if let Some(sink) = &self.sink {
                sink(FrameDone {
                    key: job.key,
                    frame,
                    cancelled: false,
                });
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
                // The last raster decoded, so the frames of one file for
                // several screen sizes share one decode.
                let mut decoded: Option<(SourceKey, skia::Image)> = None;
                while let Ok(job) = queue.recv() {
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
                    let frame = render(&job, &mut decoded);
                    sink(FrameDone {
                        key: job.key,
                        frame,
                        cancelled: false,
                    });
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

/// Render one job, reusing `decoded` when it holds the same file.
fn render(job: &Job, decoded: &mut Option<(SourceKey, skia::Image)>) -> Option<skia::Image> {
    let source = match &job.source {
        Source::Raster(lazy) => {
            let raster = match decoded {
                Some((key, raster)) if *key == job.key.source => raster.clone(),
                _ => {
                    let raster =
                        lazy.make_raster_image(None, skia::image::CachingHint::Disallow)?;
                    *decoded = Some((job.key.source.clone(), raster.clone()));
                    raster
                }
            };
            Source::Raster(raster)
        }
        svg => svg.clone(),
    };
    wallpaper::render_frame(&source, &job.style, job.key.target)
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
        assert_eq!(store.sources.len(), 1);
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
