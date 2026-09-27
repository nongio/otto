//! Finding files that are not in the folder you are looking at.
//!
//! Two features share this module, because they are the same question asked
//! twice: **Recent** is a search with no query. Same traversal, same roots,
//! same cap — the only difference is what decides which results survive it
//! (how recently a file was written, rather than how well its name matches).
//!
//! Everything here runs on a worker thread and streams back. A search over a
//! home directory cannot block the UI thread until it finishes, and it must
//! not make the person wait for the *whole* answer before showing them any of
//! it. [`Search`] is [`crate::model::Directory`] in the same shape — start,
//! then poll on the UI thread, with a generation counter so navigating away
//! abandons a read in flight rather than having to interrupt it.
//!
//! ## One source: the desktop's index
//!
//! Everything here goes to LocalSearch (TinySPARQL) over D-Bus, through
//! `otto-search`, which owns the query language and the SPARQL. There is no
//! second implementation to fall back to, and that is deliberate: a search of
//! our own that reads every directory under home takes seconds where the index
//! takes a fraction of one. One source is slower to be unavailable and never
//! quietly disagrees with itself.
//!
//! The crate also owns what happens to the index's answer: paging past files
//! it remembers but the disk no longer has, statting and rechecking each row,
//! ranking and capping ([`otto_search::find`]). What stays here is what
//! belongs to a window: the worker thread, the generation counter, turning
//! results into [`Entry`]s, and merging in the pictures whose words Otto has
//! read.
//!
//! So the indexer not running is a real state the window has to show, not an
//! internal detail to paper over: [`Batch::available`] carries it, and the
//! pane says the indexer is not running rather than showing an empty listing.
//! "No file matches" and "nothing is able to look" are different answers and
//! must not look alike.
//!
//! An indexer that is running but still working through the disk answers too,
//! just not completely. [`IndexWatch`] asks it how far along it is while a
//! search is open, so the window can say results may be incomplete.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use otto_search::index::{State, Status};
use otto_search::{Clock, Found, Plan, Scope};

use crate::model::Entry;

/// The path a pane of search results carries.
///
/// Distinct from [`crate::recent::SENTINEL`], though both name listings with
/// no directory behind them and neither is a path anything will ever open.
/// They have to differ because the sidebar lights the place whose path matches
/// the pane's: sharing one sentinel lit *Recent* the moment anyone typed a
/// query, and the sidebar said you had navigated somewhere you had not.
pub const SENTINEL: &str = "/dev/null/otto-search";

/// How many results a search keeps. Everything past this is dropped by rank,
/// so what survives is the best matches rather than whichever the traversal
/// happened to reach first.
///
/// A grid nobody will scroll to the end of does not need to be complete; it
/// needs to have the answer near the top. Five hundred tiles is already more
/// than anyone reads, and the cap is what keeps the sort — which the view
/// redoes whenever the listing is replaced — off the frame budget.
pub const LIMIT: usize = 500;

/// What to look for, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The query, in the language of `specs/search-language.md`.
    pub query: String,
    /// Where to look when the query has no `in:`. Directories; each is
    /// searched with everything below it.
    pub roots: Vec<PathBuf>,
    /// What a relative `in:` is relative to: the folder the search started in.
    pub cwd: Option<PathBuf>,
    /// How long to wait before starting.
    ///
    /// Zero everywhere now: a search runs when Return is pressed rather than
    /// on every keystroke, so there is no burst of half-written queries to
    /// wait out, and any delay here is latency between asking and being
    /// answered. Kept because the seam is worth having if a search ever fires
    /// itself again.
    pub debounce: Duration,
}

/// Recent, as a query: files but not folders, newest first.
///
/// Folders are left out because a folder's time changes every time anything
/// inside it does, and they would otherwise crowd out the files the listing
/// is for.
const RECENT_QUERY: &str = "-kind:folder sort:modified";

impl Request {
    /// The Recent listing: what was written most recently across the user's
    /// own folders.
    pub fn recent() -> Self {
        Self {
            query: RECENT_QUERY.to_string(),
            roots: recent_roots(),
            cwd: None,
            debounce: Duration::ZERO,
        }
    }

    /// A search of everything, for the Everywhere scope.
    pub fn everywhere(query: String) -> Self {
        Self {
            query,
            roots: crate::model::home_dir().into_iter().collect(),
            cwd: None,
            debounce: Duration::ZERO,
        }
    }

    /// A search of one directory and everything under it, for the This Folder
    /// scope.
    ///
    /// The same query as Everywhere with a narrower root, rather than a
    /// different mechanism: the scope pills are one question asked of two
    /// haystacks, and a folder scope that matched by different rules from the
    /// one beside it would make switching between them unreadable.
    pub fn folder(query: String, dir: PathBuf) -> Self {
        Self {
            query,
            roots: vec![dir.clone()],
            cwd: Some(dir),
            debounce: Duration::ZERO,
        }
    }

    /// The query resolved against this request's scope, now. `None` when
    /// there is nowhere to look.
    fn plan(&self) -> Option<Plan> {
        let scope = Scope {
            roots: self.roots.clone(),
            home: crate::model::home_dir(),
            cwd: self.cwd.clone(),
        };
        Plan::new(&otto_search::parse(&self.query), &scope, Clock::now())
    }
}

/// Recent's roots: the XDG user directories that exist.
///
/// Not the whole of home. Recent answers "where did that go" about work, and
/// a home directory's recently-written files are overwhelmingly caches, dot
/// directories and build output — true, useless, and enough of them to bury
/// the document the person is actually looking for.
fn recent_roots() -> Vec<PathBuf> {
    let roots: Vec<PathBuf> = crate::model::places()
        .into_iter()
        .filter(|place| !place.recent && Some(&place.path) != crate::model::home_dir().as_ref())
        .map(|place| place.path)
        .collect();
    if roots.is_empty() {
        return crate::model::home_dir().into_iter().collect();
    }
    roots
}

/// A result set as it stands.
///
/// Batches **replace** rather than append: each one is the best results found
/// so far, already ranked and capped. Appending would mean the UI holding a
/// growing list it has to re-rank itself, and a cap that could only ever
/// truncate whatever arrived first.
#[derive(Debug, Clone)]
pub struct Batch {
    pub entries: Vec<Entry>,
    /// No more batches are coming for this request.
    pub done: bool,
    /// Whether the index answered at all.
    ///
    /// `false` means the file indexer is not running — not that it looked and
    /// found nothing. The window has to tell those apart: an empty listing
    /// under a query is an answer, and showing one when nothing was able to
    /// look is a lie the person cannot see through.
    pub available: bool,
}

/// A search in flight, polled from the UI thread.
///
/// The same contract as [`crate::model::Directory`]: [`start`](Self::start)
/// never blocks, [`poll`](Self::poll) never blocks, and a request that has
/// been superseded delivers nothing.
pub struct Search {
    rx: Option<Receiver<Batch>>,
    /// Bumped on every start. A batch tagged with a stale generation is never
    /// sent — that is how a new query cancels the old one without needing to
    /// interrupt a thread mid-`readdir`.
    generation: Arc<Mutex<u64>>,
    pub running: bool,
}

impl Default for Search {
    fn default() -> Self {
        Self::idle()
    }
}

impl Drop for Search {
    /// Bump the generation on the way out, so a walk started by a pane that
    /// has since been closed stops at its next directory instead of reading
    /// the rest of a home directory for nobody.
    fn drop(&mut self) {
        if let Ok(mut generation) = self.generation.lock() {
            *generation += 1;
        }
    }
}

impl Search {
    /// A handle with nothing running. Every column has one; most never use it.
    pub fn idle() -> Self {
        Self {
            rx: None,
            generation: Arc::new(Mutex::new(0)),
            running: false,
        }
    }

    /// Start `request`. Anything already in flight is abandoned.
    pub fn start(&mut self, request: Request) {
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.running = true;

        let generation = {
            let mut g = self.generation.lock().unwrap();
            *g += 1;
            *g
        };
        let sink = Sink {
            tx,
            generation: Arc::clone(&self.generation),
            mine: generation,
        };
        std::thread::spawn(move || run(request, sink));
    }

    /// The newest result set, if one has arrived. Never blocks.
    ///
    /// Every queued batch is drained and only the last is returned: they
    /// replace one another, so handing the caller the older ones would only
    /// make it sort and draw listings that are already superseded.
    pub fn poll(&mut self) -> Option<Batch> {
        let rx = self.rx.as_ref()?;
        let mut latest = None;
        while let Ok(batch) = rx.try_recv() {
            latest = Some(batch);
        }
        if latest.as_ref().is_some_and(|b| b.done) {
            self.running = false;
        }
        latest
    }
}

/// The worker's end of a [`Search`].
struct Sink {
    tx: Sender<Batch>,
    generation: Arc<Mutex<u64>>,
    mine: u64,
}

impl Sink {
    /// Is anyone still waiting for this? Checked often enough that an
    /// abandoned walk stops within a directory or two of being superseded.
    fn alive(&self) -> bool {
        *self.generation.lock().unwrap() == self.mine
    }

    /// Hand over a result set. Returns whether to keep going.
    fn send(&self, entries: Vec<Entry>, done: bool, available: bool) -> bool {
        if !self.alive() {
            return false;
        }
        if self
            .tx
            .send(Batch {
                entries,
                done,
                available,
            })
            .is_err()
        {
            return false;
        }
        // Wake the UI thread. Without this the batch waits for the next input:
        // a window with nothing moving commits no frames, so there is no frame
        // callback to notice it landed. Same reason as `Directory::load`.
        otto_kit::prelude::AppContext::request_wakeup();
        true
    }
}

/// The worker body.
fn run(request: Request, mut sink: Sink) {
    if !request.debounce.is_zero() {
        std::thread::sleep(request.debounce);
        if !sink.alive() {
            return;
        }
    }

    let Some(plan) = request.plan() else {
        tracing::info!("search: nowhere to look");
        sink.send(Vec::new(), true, false);
        return;
    };
    // The paging, statting, rechecking and ranking are `otto-search`'s, so
    // the command line and this window give the same answer; `sink` streams
    // each page but the last as it lands.
    let (mut results, available) = match otto_search::find(&plan, LIMIT, &mut sink) {
        Ok(results) => (results, true),
        Err(why) => {
            tracing::info!("search: {why}");
            // Empty *and* unavailable, which the pane shows as the indexer
            // not running. Nothing was sent before the failure, so this cannot
            // be appended to half an answer. Pictures whose text is remembered
            // still answer `text:`: a word read off a screenshot is found
            // either way.
            (otto_search::Results::new(&plan, LIMIT), false)
        }
    };
    if !sink.alive() {
        return;
    }
    ask_pictures(&plan, &mut results);
    // Sent even when empty: when the index answered, "nothing matched" is
    // what it said.
    sink.send(entries(&mut results), true, available);
}

/// Pictures whose remembered words match the query, and that meet the rest
/// of it.
///
/// Matched against the `text:` terms when there are any. Otherwise the plain
/// words stand in, so that typing a word seen in a screenshot finds the
/// screenshot, as `specs/peek-ocr.md` promises; the name then need not match.
///
/// Read off Otto's own cache of what the recogniser found in pictures the
/// person has looked at (see `ocrcache`), so this answers only for those. A
/// hit that the index also returned is the same path, and
/// [`otto_search::Results::best`] keeps one row per path.
fn ask_pictures(plan: &Plan, results: &mut otto_search::Results) {
    let by_text = plan.searches_text();
    let needles: Vec<&str> = if by_text {
        plan.text_needles().collect()
    } else {
        plan.name_needles().collect()
    };
    let Some((first, rest)) = needles.split_first() else {
        return;
    };
    let mut paths = crate::ocrcache::matches(first);
    for needle in rest {
        let also = crate::ocrcache::matches(needle);
        paths.retain(|path| also.contains(path));
    }
    for path in paths {
        let Some(found) = Found::stat(&path) else {
            continue;
        };
        let facts = found.facts(otto_kit::filetype::mime_for_name(&found.name));
        let admitted = if by_text {
            plan.admits(&facts)
        } else {
            plan.admits_ignoring_names(&facts)
        };
        if !admitted {
            continue;
        }
        // Found by its words rather than its name, it ranks with the best
        // name matches: the words are a whole-word hit.
        let score = plan.rank(&found.name).unwrap_or(WORDS_SCORE);
        results.push(found, score);
    }
}

/// The name score given to a picture found by its words.
const WORDS_SCORE: i64 = i32::MAX as i64;

/// The result set as the pane shows it.
fn entries(results: &mut otto_search::Results) -> Vec<Entry> {
    results.best().into_iter().map(Entry::from).collect()
}

impl otto_search::Progress for Sink {
    fn alive(&self) -> bool {
        Sink::alive(self)
    }

    /// More is coming; show what there is meanwhile.
    fn page(&mut self, results: &mut otto_search::Results) -> bool {
        self.send(entries(results), false, true)
    }
}

/// How long the indexer is left alone between checks while it is behind.
///
/// Often enough that the notice goes within a few seconds of the indexer
/// finishing; each check is a handful of D-Bus round trips on a worker, so
/// this costs nothing the person would notice.
const INDEX_RECHECK: Duration = Duration::from_secs(3);

/// What the file indexer is doing, asked on a worker and polled from the UI
/// thread, for as long as a search is open.
///
/// Asked alongside every search, then again every [`INDEX_RECHECK`] while the
/// indexer is behind, so a notice that results may be incomplete goes away
/// once it catches up. Once it is caught up, or the watch is stopped, the
/// worker ends.
pub struct IndexWatch {
    rx: Option<Receiver<Status>>,
    /// Cleared to tell the current worker nobody is listening any more.
    alive: Arc<AtomicBool>,
    status: Option<Status>,
}

impl Default for IndexWatch {
    fn default() -> Self {
        Self {
            rx: None,
            alive: Arc::new(AtomicBool::new(false)),
            status: None,
        }
    }
}

impl Drop for IndexWatch {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
    }
}

impl IndexWatch {
    /// Ask again now, and keep asking while the indexer is behind. A worker
    /// already running is abandoned. The last answer stays until the new one
    /// lands, so a notice on screen does not blink off and back on.
    pub fn start(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
        let alive = Arc::new(AtomicBool::new(true));
        self.alive = Arc::clone(&alive);
        let (tx, rx) = channel();
        self.rx = Some(rx);
        std::thread::spawn(move || watch_index(&tx, &alive));
    }

    /// Stop asking and forget the answer: nothing is searching any more.
    pub fn stop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
        self.rx = None;
        self.status = None;
    }

    /// Take the newest answer, if one has arrived. Returns whether it
    /// changed. Never blocks.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = self.rx.as_ref() else {
            return false;
        };
        let mut changed = false;
        while let Ok(status) = rx.try_recv() {
            changed |= self.status != Some(status);
            self.status = Some(status);
        }
        changed
    }

    /// A watch that has already heard `status`, with no worker behind it.
    #[cfg(test)]
    pub(crate) fn answered(status: Status) -> Self {
        let mut watch = Self::default();
        watch.status = Some(status);
        watch
    }

    /// The last answer, if one has arrived since the watch started.
    pub fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }
}

/// The watch's worker body.
fn watch_index(tx: &Sender<Status>, alive: &AtomicBool) {
    loop {
        let status = otto_search::index::status();
        if !alive.load(Ordering::Relaxed) || tx.send(status).is_err() {
            return;
        }
        otto_kit::prelude::AppContext::request_wakeup();
        if !keeps_watching(&status) {
            return;
        }
        std::thread::sleep(INDEX_RECHECK);
        if !alive.load(Ordering::Relaxed) {
            return;
        }
    }
}

/// Whether the indexer is worth asking again later. Only while it is behind:
/// idle stays idle until something changes on disk, and a missing or stopped
/// indexer is already reported by the search itself.
fn keeps_watching(status: &Status) -> bool {
    status.is_behind()
}

/// What the window says about the indexer being behind, if it is.
pub fn indexing_notice(status: &Status) -> Option<String> {
    match status.state {
        State::Indexing => Some(otto_kit::t_owned!(
            "files-search-indexing",
            percent = (status.progress * 100.0).round() as i64
        )),
        State::Paused => Some(otto_kit::t_owned!("files-search-indexing-paused")),
        State::Missing | State::Stopped | State::Idle => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run a request the way the worker does, and report the one batch that
    /// comes back. No daemon is needed for the cases below: each one either
    /// never reaches the bus or is answered before it would.
    fn run_to_batch(request: Request) -> Batch {
        let (tx, rx) = channel();
        let sink = Sink {
            tx,
            generation: Arc::new(Mutex::new(1)),
            mine: 1,
        };
        run(request, sink);
        rx.into_iter().last().expect("a result set arrived")
    }

    /// The two scope pills differ in how much of the disk the index may answer
    /// from, and in nothing else: same query, narrower root. A folder scope
    /// that only matched the rows already on screen would miss everything in
    /// the subfolders, which is most of what "this folder" means.
    #[test]
    fn a_folder_scoped_search_asks_only_about_that_folder() {
        let dir = PathBuf::from("/home/u/Documents");
        let plan = Request::folder("report".into(), dir.clone())
            .plan()
            .expect("a folder scope is answerable");
        assert_eq!(plan.roots(), std::slice::from_ref(&dir));

        // A relative `in:` is relative to the folder the search started in.
        let plan = Request::folder("report in:notes".into(), dir.clone())
            .plan()
            .expect("answerable");
        assert_eq!(plan.roots(), [dir.join("notes")]);
    }

    /// Recent is the same machinery with nothing to match on: newest first,
    /// and without the folders.
    #[test]
    fn recent_is_files_newest_first() {
        let request = Request {
            roots: vec![PathBuf::from("/home/u")],
            ..Request::recent()
        };
        let plan = request.plan().expect("Recent is answerable");
        assert_eq!(plan.sort(), otto_search::Sort::Modified);
        assert!(plan.sparql(10).text.ends_with("ORDER BY DESC(?m) LIMIT 10"));
    }

    /// The state that has to be visible rather than papered over. With nothing
    /// able to answer, the batch comes back empty *and* marked unavailable, so
    /// the pane can say the indexer is off instead of "nothing found", which
    /// would send someone looking for a file that is sitting on the disk.
    #[test]
    fn an_index_that_cannot_answer_says_so_rather_than_reporting_an_empty_result() {
        // No roots, so the query cannot even be built: the failure happens
        // before the bus is touched, which makes this independent of whether a
        // daemon happens to be running on the machine running the tests.
        let batch = run_to_batch(Request {
            query: "anything".into(),
            roots: Vec::new(),
            cwd: None,
            debounce: Duration::ZERO,
        });

        assert!(batch.done);
        assert!(batch.entries.is_empty());
        assert!(
            !batch.available,
            "an empty listing and an unanswerable one must not look alike"
        );
    }

    /// A search nobody is waiting for any more delivers nothing, which is how
    /// the next keystroke cancels the last one without interrupting a thread.
    #[test]
    fn a_superseded_search_stops_sending() {
        let (tx, rx) = channel();
        let generation = Arc::new(Mutex::new(1));
        let sink = Sink {
            tx,
            generation: Arc::clone(&generation),
            mine: 1,
        };
        // Someone typed another character before this one got anywhere.
        *generation.lock().unwrap() = 2;
        assert!(!sink.send(Vec::new(), true, true));
        drop(sink);
        assert!(rx.into_iter().next().is_none());
    }

    fn status(state: State, progress: f64) -> Status {
        Status {
            state,
            progress,
            remaining: None,
        }
    }

    /// Only an indexer that is behind is asked again: once it is idle the
    /// notice is gone and nothing needs watching, and a missing or stopped
    /// one is what the search itself reports.
    #[test]
    fn the_indexer_is_watched_only_while_it_is_behind() {
        assert!(keeps_watching(&status(State::Indexing, 0.4)));
        assert!(keeps_watching(&status(State::Paused, 0.4)));
        assert!(!keeps_watching(&status(State::Idle, 1.0)));
        assert!(!keeps_watching(&status(State::Stopped, 0.0)));
        assert!(!keeps_watching(&status(State::Missing, 0.0)));
    }

    /// The notice names how far along the indexer is while it works, says
    /// when it is paused, and is absent otherwise.
    #[test]
    fn the_indexing_notice_follows_the_indexer() {
        let working = indexing_notice(&status(State::Indexing, 0.617)).expect("a notice");
        assert!(working.contains("62"), "{working}");
        let paused = indexing_notice(&status(State::Paused, 0.617)).expect("a notice");
        assert_ne!(working, paused);
        assert!(!paused.contains("62"), "{paused}");
        for state in [State::Idle, State::Stopped, State::Missing] {
            assert_eq!(indexing_notice(&status(state, 1.0)), None);
        }
    }

    /// Against the real daemon, end to end through the worker. Ignored by
    /// default: it needs a running indexer, and what it finds depends on the
    /// machine. `otto-search`'s own `live_index` test covers every filter.
    ///
    ///     cargo test -p otto-files --lib live_index -- --ignored --nocapture
    #[test]
    #[ignore = "needs a running LocalSearch indexer"]
    fn live_index_answers_both_scopes() {
        for (what, request) in [
            ("recent", Request::recent()),
            ("everywhere", Request::everywhere("rs".into())),
        ] {
            let started = std::time::Instant::now();
            let batch = run_to_batch(request);
            println!("{what}: {:.2}s", started.elapsed().as_secs_f64());
            println!("{what}: {} entries", batch.entries.len());
            assert!(batch.available, "{what}: the index did not answer");
            assert!(!batch.entries.is_empty(), "{what} returned nothing");
        }
    }
}
