//! Text recognition in pictures: whether it is on, and the background pass
//! over the folder on screen.
//!
//! Recognition on preview happens in `present.rs` beside the decode. This
//! module holds what is shared: the setting, and the pass that recognises
//! the pictures a pane is showing while the window is idle, so Find can
//! answer for a picture nobody has opened yet. It rides on the thumbnail
//! scheduler's notion of "visible": the same rows, the same pane, one
//! sandboxed worker at a time, and never while a preview decode is in
//! flight.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use skia_safe::Rect;

use super::*;

/// The `[peek]` section, read once per process.
fn config() -> &'static crate::places_config::PeekConfig {
    static CONFIG: OnceLock<crate::places_config::PeekConfig> = OnceLock::new();
    CONFIG.get_or_init(crate::places_config::peek)
}

/// Whether pictures are recognised at all. On unless `files.toml` says
/// `[peek] recognise_text = false`; words already remembered are shown
/// and searched either way, since they cost nothing.
pub(super) fn enabled() -> bool {
    config().recognise_text
}

/// The recogniser command line: the configured one, else tesseract.
pub(super) fn command() -> &'static str {
    let configured = config().recogniser.trim();
    if configured.is_empty() {
        otto_peek::ocr::DEFAULT_COMMAND
    } else {
        configured
    }
}

/// Whether the recogniser is installed.
pub(super) fn available() -> bool {
    otto_peek::ocr::available(command())
}

/// Drop the entries whose pictures nobody has touched in months.
///
/// Once per run, the first time the background pass finds nothing left to
/// read: the cache is Otto's own and grows with every folder visited, and
/// the moment the pass runs dry is the moment nothing is waiting on the
/// disk. On its own thread — it reads every entry in the cache, and the
/// caller is the frame loop.
fn prune_once() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static PRUNED: AtomicBool = AtomicBool::new(false);
    if PRUNED.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(|| crate::ocrcache::prune(crate::ocrcache::KEEP_FOR));
}

/// The size the background pass recognises at: a preview-sized decode, so a
/// remembered entry lines up with what a later preview draws without losing
/// small print to a thumbnail-sized one.
fn pass_panel() -> Rect {
    Rect::from_wh(800.0, 600.0)
}

/// Read the words out of pictures now, without a window.
///
/// The same work the background pass does, asked for directly: a folder can
/// be made searchable — and Get Info given an answer — before anybody opens
/// it. Directories are read one level deep; anything that is not a picture is
/// passed over. Returns whether every picture named was read.
pub fn recognise_paths(paths: &[PathBuf]) -> bool {
    if !enabled() {
        eprintln!("text recognition is off in files.toml");
        return false;
    }
    if !available() {
        eprintln!("no recogniser installed: {}", command());
        return false;
    }
    let mut all = true;
    for path in pictures_under(paths) {
        let started = std::time::Instant::now();
        // Recognised at twice the panel's size, the way a HiDPI session does
        // it: small print survives it, and a remembered entry carries the
        // size it was found at, so a panel of any size can scale the words to
        // what it is showing.
        match peek::recognise(
            &path,
            pass_panel(),
            2.0,
            1,
            command(),
            peek::Priority::Interactive,
        ) {
            Some(words) => println!(
                "{}: {} {} in {:.1}s",
                path.display(),
                words.len(),
                if words.len() == 1 { "word" } else { "words" },
                started.elapsed().as_secs_f32()
            ),
            None => {
                all = false;
                println!("{}: could not be read", path.display());
            }
        }
    }
    all
}

/// The pictures among `paths`, plus the pictures directly inside any
/// directory named. Sorted, so a run over a folder reports in the order the
/// browser would list it.
fn pictures_under(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in paths {
        if path.is_dir() {
            let Ok(dir) = std::fs::read_dir(path) else {
                eprintln!("{}: cannot be read", path.display());
                continue;
            };
            let mut found: Vec<PathBuf> = dir
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| is_picture(path))
                .collect();
            found.sort();
            out.append(&mut found);
        } else if is_picture(path) {
            out.push(path.clone());
        } else {
            eprintln!("{}: not a picture", path.display());
        }
    }
    out
}

pub(super) fn is_picture(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| crate::command::is_picture_name(&name.to_string_lossy()))
}

/// A picture to recognise, and how much of a hurry it is in.
pub(super) struct Job {
    pub path: PathBuf,
    pub priority: peek::Priority,
}

impl Browser {
    /// One picture on screen that has no remembered words yet, if the window
    /// is idle enough to spend a worker on it: no preview decode or
    /// recognition in flight, no thumbnails outstanding, and no pass already
    /// running.
    ///
    /// Files are checked once per window and then left alone whatever the
    /// answer, so a folder of unreadable pictures costs one look each rather
    /// than one worker per frame.
    pub(super) fn sync_recognition(&mut self) -> Option<Job> {
        if !enabled() || !self.ocr_reading.is_empty() || !available() {
            return None;
        }
        // Asked for by name, which outranks the pass's own idea of a good
        // moment: somebody is waiting for this one, and it is read again
        // whatever is already remembered about it.
        if let Some(path) = self.ocr_queue.pop_front() {
            self.ocr_seen.insert(path.clone());
            self.begin_reading(path.clone());
            return Some(Job {
                path,
                priority: peek::Priority::Interactive,
            });
        }
        if self.peek_pending || self.peek_recognising || self.thumbs.is_busy() {
            return None;
        }
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let entries = self.visible(depth);
        let candidate = entries.iter().find(|entry| {
            entry.modified.is_some()
                && !self.ocr_seen.contains(&entry.path)
                && is_picture(&entry.path)
        });
        let Some(candidate) = candidate else {
            prune_once();
            return None;
        };
        let path = candidate.path.clone();
        let modified = candidate.modified?;
        self.ocr_seen.insert(path.clone());
        // An entry made by another engine, or before a language pack was
        // installed, still shows and still searches — but it is read again,
        // which is how the cache catches up with what is installed now.
        if crate::ocrcache::is_current(&path, 1, modified, &otto_peek::ocr::languages(), command())
        {
            return None;
        }
        self.begin_reading(path.clone());
        Some(Job {
            path,
            priority: peek::Priority::Background,
        })
    }

    /// Read the pictures the command palette's *Run text recognition* was
    /// aimed at — the selection, or the entry under the cursor — again.
    ///
    /// Queued rather than run: one recogniser at a time is the rule, and the
    /// pass already knows how to hand them out one by one. Refused when
    /// nothing in the target is a picture, so the palette says why rather
    /// than appearing to do nothing.
    pub(super) fn recognise_selection(&mut self) -> Result<(), String> {
        let targets = self.recognition_targets();
        if targets.is_empty() {
            return Err(otto_kit::t_owned!("files-recognise-no-pictures"));
        }
        for path in targets {
            if !self.ocr_queue.contains(&path) {
                self.ocr_queue.push_back(path);
            }
        }
        self.dirty = true;
        Ok(())
    }

    /// The pictures a command would act on: the selection, or the entry under
    /// the cursor when nothing is selected, the way every other command reads
    /// the window.
    fn recognition_targets(&self) -> Vec<PathBuf> {
        let selected: Vec<PathBuf> = self
            .selected_entries()
            .into_iter()
            .map(|entry| entry.path)
            .collect();
        let targets = if selected.is_empty() {
            self.selected_entry()
                .map(|entry| entry.path)
                .into_iter()
                .collect()
        } else {
            selected
        };
        targets
            .into_iter()
            .filter(|path| is_picture(path))
            .collect()
    }

    /// A recogniser has started on `path`.
    ///
    /// Both the panel's own recognition and the background pass report here,
    /// so that anything asking what is happening to a picture — Get Info, the
    /// pass deciding whether the window is idle — has one place to ask.
    pub(super) fn begin_reading(&mut self, path: PathBuf) {
        self.ocr_reading.insert(path.clone());
        self.refresh_text_status(&path);
    }

    /// A recogniser has finished with `path`, whatever it found.
    pub(super) fn end_reading(&mut self, path: &Path) {
        self.ocr_reading.remove(path);
        self.refresh_text_status(path);
    }

    /// Whether a recogniser is on this picture right now.
    pub(super) fn reading(&self, path: &Path) -> bool {
        self.ocr_reading.contains(path)
    }

    /// What the panels should say about the words in a file.
    ///
    /// `None` for anything that is not a picture, and for a picture nothing
    /// will ever read: with no recogniser installed, "not read yet" is a
    /// promise that is not coming. Reads the cache, so it is asked when the
    /// answer can have changed rather than while drawing.
    pub(super) fn text_status(&self, path: &Path) -> Option<crate::ocrcache::Status> {
        use crate::ocrcache::Status;
        if !is_picture(path) {
            return None;
        }
        if self.reading(path) {
            return Some(Status::Reading);
        }
        let modified = std::fs::metadata(path).ok()?.modified().ok()?;
        match crate::ocrcache::lookup(path, 1, modified) {
            Some(entry) if entry.words.is_empty() => Some(Status::Empty),
            Some(entry) => Some(Status::Words(entry.words.len())),
            None => (enabled() && available()).then_some(Status::Unread),
        }
    }

    /// Re-read what the panels say about words, for whichever of them is
    /// showing the file that just changed: Get Info, whose window has a
    /// buffer of its own and must be told to redraw, and the preview column.
    fn refresh_text_status(&mut self, path: &Path) {
        if self.info.as_ref().is_some_and(|info| info.path == path) {
            self.info_text = self.text_status(path);
            self.info_dirty = true;
        }
        if self.preview.as_ref().is_some_and(|pane| pane.path == path) {
            let status = self.text_status(path);
            if let Some(pane) = self.preview.as_mut() {
                pane.text = status;
            }
            self.dirty = true;
        }
    }
}

impl FilesApp {
    /// Recognise one picture off the UI thread, for Find.
    pub(super) fn start_recognition(&self, job: Job) {
        let state = Arc::clone(&self.state);
        let scale = AppContext::scale_factor().max(1) as f32;
        tokio::task::spawn_blocking(move || {
            let _ = peek::recognise(&job.path, pass_panel(), scale, 1, command(), job.priority);
            state.lock().unwrap().end_reading(&job.path);
            // Another picture may be waiting its turn; the frame loop hands
            // it out.
            AppContext::request_wakeup();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("otto-files-ocr-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// While a recogniser is on the file Get Info is showing, the panel says
    /// the words are coming — not that there are none, which is what an empty
    /// row would have been read as.
    #[test]
    fn get_info_says_a_picture_is_being_read() {
        let dir = scratch("status");
        let path = dir.join("shot.png");
        std::fs::write(&path, b"a picture by its name").unwrap();

        let mut browser = Browser::new(dir.clone());
        browser.info = Some(crate::model::read_info(&path));

        browser.begin_reading(path.clone());
        assert_eq!(browser.info_text, Some(crate::ocrcache::Status::Reading));
        assert!(browser.info_dirty, "the panel has its own window to redraw");

        browser.end_reading(&path);
        assert_ne!(
            browser.info_text,
            Some(crate::ocrcache::Status::Reading),
            "the recogniser has finished"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A recogniser working on one picture says nothing about another.
    #[test]
    fn the_panel_only_hears_about_its_own_file() {
        let dir = scratch("other");
        let shown = dir.join("shown.png");
        let other = dir.join("other.png");
        std::fs::write(&shown, b"one").unwrap();
        std::fs::write(&other, b"two").unwrap();

        let mut browser = Browser::new(dir.clone());
        browser.info = Some(crate::model::read_info(&shown));
        browser.info_dirty = false;

        browser.begin_reading(other);
        assert_eq!(browser.info_text, None);
        assert!(!browser.info_dirty);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The preview column's caption follows the same recogniser, so a picture
    /// selected while the pass is on it says so under its dates.
    #[test]
    fn the_preview_caption_follows_the_recogniser() {
        let dir = scratch("caption");
        let path = dir.join("shot.png");
        std::fs::write(&path, b"a picture by its name").unwrap();

        let mut browser = Browser::new(dir.clone());
        browser.preview = Some(PreviewPaneState {
            path: path.clone(),
            generation: 1,
            pending: false,
            decoded: None,
            video: None,
            text: None,
        });

        browser.begin_reading(path.clone());
        let pane = browser.preview.as_ref().unwrap();
        assert_eq!(pane.text, Some(crate::ocrcache::Status::Reading));
        assert!(browser.dirty);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A picture asked for by name goes ahead of the pass's own scan, is not
    /// stopped by an answer already in the cache, and runs at the priority of
    /// work somebody is waiting for.
    #[test]
    fn an_asked_for_picture_jumps_the_queue() {
        if !available() {
            return; // Nothing to hand a job to on this machine.
        }
        let dir = scratch("queue");
        let path = dir.join("shot.png");
        std::fs::write(&path, b"a picture by its name").unwrap();

        let mut browser = Browser::new(dir.clone());
        browser.ocr_queue.push_back(path.clone());

        let job = browser.sync_recognition().expect("the queue is handed out");
        assert_eq!(job.path, path);
        assert_eq!(job.priority, peek::Priority::Interactive);
        assert!(browser.reading(&path), "the panels say it is being read");

        // One at a time: nothing else goes out while it is running.
        assert!(browser.sync_recognition().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Asked for a folder, the command line reads the pictures in it and
    /// leaves everything else alone.
    #[test]
    fn a_folder_offers_up_its_pictures() {
        let dir = scratch("folder");
        for name in ["a.png", "b.jpg", "notes.txt", "clip.mp4"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let found = pictures_under(std::slice::from_ref(&dir));
        let names: Vec<_> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.png", "b.jpg"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
