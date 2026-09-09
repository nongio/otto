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
//! Everything here goes to LocalSearch (TinySPARQL) over D-Bus. There is no
//! second implementation to fall back to, and that is deliberate: a search of
//! our own that reads every directory under home takes seconds where the index
//! takes a fraction of one, and it answers a *different* question — matching
//! names by subsequence where the index matches by substring — so which one
//! ran decided what you found. One source is slower to be unavailable and
//! never quietly disagrees with itself.
//!
//! So the indexer not running is a real state the window has to show, not an
//! internal detail to paper over: [`Batch::available`] carries it, and the
//! pane says the indexer is not running rather than showing an empty listing.
//! "No file matches" and "nothing is able to look" are different answers and
//! must not look alike.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use crate::model::{entry_for_path, Entry};

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
    /// The text to match names against. `None` is Recent: everything under
    /// the roots, ranked by when it was written rather than by a query.
    pub query: Option<String>,
    /// Where to look. Directories; each is searched with everything below it.
    pub roots: Vec<PathBuf>,
    /// Skip directories themselves. Recent is about files you saved, and a
    /// folder's mtime changes every time anything inside it does, so folders
    /// would otherwise crowd out the files the listing is for.
    pub files_only: bool,
    /// How long to wait before starting.
    ///
    /// Zero everywhere now: a search runs when Return is pressed rather than
    /// on every keystroke, so there is no burst of half-written queries to
    /// wait out, and any delay here is latency between asking and being
    /// answered. Kept because the seam is worth having if a search ever fires
    /// itself again.
    pub debounce: Duration,
}

impl Request {
    /// The Recent listing: what was written most recently across the user's
    /// own folders.
    pub fn recent() -> Self {
        Self {
            query: None,
            roots: recent_roots(),
            files_only: true,
            debounce: Duration::ZERO,
        }
    }

    /// A search of everything, for the Everywhere scope.
    pub fn everywhere(query: String) -> Self {
        Self {
            query: Some(query),
            roots: crate::model::home_dir().into_iter().collect(),
            files_only: false,
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
            query: Some(query),
            roots: vec![dir],
            files_only: false,
            debounce: Duration::ZERO,
        }
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

/// Why the index could not answer. Carried so the log can say, and so the
/// pane can tell "nothing matched" from "nothing was able to look".
#[derive(Debug)]
struct Unavailable(String);

/// The worker body.
fn run(request: Request, sink: Sink) {
    if !request.debounce.is_zero() {
        std::thread::sleep(request.debounce);
        if !sink.alive() {
            return;
        }
    }

    if let Err(Unavailable(why)) = ask_index(&request, &sink) {
        tracing::info!("search: the index could not answer ({why})");
        // Empty *and* unavailable, which the pane shows as the indexer not
        // running. Nothing was sent before the failure — every path that can
        // fail does so before the first `send` — so this cannot be appended
        // to half an answer.
        sink.send(Vec::new(), true, false);
    }
}

// ---------------------------------------------------------------------------
// Ranking
// ---------------------------------------------------------------------------

/// A result and the number that decides whether it survives the cap.
///
/// For a query that is the name score; for Recent it is the modification time.
/// Higher is better in both, so one comparison serves both.
struct Ranked {
    rank: i64,
    entry: Entry,
}

/// Rank `entry` for `request`, or `None` if it does not belong in the results.
fn rank(entry: &Entry, request: &Request) -> Option<i64> {
    if request.files_only && entry.is_dir {
        return None;
    }
    match &request.query {
        Some(query) => otto_kit::matching::score(&entry.name, query).map(i64::from),
        // Recent: newest first. A file whose time could not be read sorts
        // last rather than being dropped — it is still a file that is there.
        None => Some(entry.modified.and_then(epoch_secs).unwrap_or(i64::MIN)),
    }
}

fn epoch_secs(t: SystemTime) -> Option<i64> {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

/// The running result set: everything found so far, trimmed to the best
/// [`LIMIT`] whenever it has grown enough to be worth the sort.
#[derive(Default)]
struct Best(Vec<Ranked>);

impl Best {
    fn push(&mut self, ranked: Ranked) {
        self.0.push(ranked);
        // Trimmed at twice the cap rather than at the cap, so a walk through a
        // directory of a million files sorts once per thousand results instead
        // of once per result.
        if self.0.len() > LIMIT * 2 {
            self.trim();
        }
    }

    fn trim(&mut self) {
        self.0.sort_by_key(|ranked| std::cmp::Reverse(ranked.rank));
        self.0.truncate(LIMIT);
    }

    /// The result set as it stands, best first.
    fn entries(&mut self) -> Vec<Entry> {
        self.trim();
        self.0.iter().map(|r| r.entry.clone()).collect()
    }
}

// ---------------------------------------------------------------------------
// LocalSearch
// ---------------------------------------------------------------------------

// LocalSearch (TinySPARQL), the desktop's file index, reached over D-Bus.
//
// It is asked only for **paths**. Everything else — size, modification time,
// kind, whether it is a folder — is read from the filesystem here, by the same
// code that builds an ordinary directory listing. That is not duplicated work:
// an index is always a little behind the disk, and a listing built from what
// it remembers would name files that have been deleted and give the sizes they
// used to have. Statting every row is what makes a result an ordinary [`Entry`]
// that the grid, the thumbnailer and Quick View can all treat like any other.

const LOCALSEARCH_NAME: &str = "org.freedesktop.LocalSearch3";
const ENDPOINT_PATH: &str = "/org/freedesktop/Tracker3/Endpoint";
const ENDPOINT_IFACE: &str = "org.freedesktop.Tracker3.Endpoint";

/// Ask the index, and hand what it says to `sink`.
///
/// `Err` means the indexer could not be reached at all, and nothing has been
/// sent. An empty `Ok` is the opposite: the index looked and there was
/// nothing, which is an answer and is shown as one.
fn ask_index(request: &Request, sink: &Sink) -> Result<(), Unavailable> {
    let sparql = sparql_for(request).ok_or_else(|| Unavailable("nothing to ask".into()))?;
    let urls = query(&sparql)?;

    let mut best = Best::default();
    for url in urls {
        if !sink.alive() {
            return Ok(());
        }
        let Some(path) = path_from_file_url(&url) else {
            continue;
        };
        let Some(entry) = entry_for_path(&path) else {
            // Indexed but no longer on disk. Dropped silently: the index
            // catching up is not something to tell anyone about. It is why
            // the rows are statted at all rather than trusted.
            continue;
        };
        if let Some(rank) = rank(&entry, request) {
            best.push(Ranked { rank, entry });
        }
    }

    // Sent even when empty, and marked available: the index answered, and
    // "nothing matched" is what it said.
    sink.send(best.entries(), true, true);
    Ok(())
}

/// The SPARQL for a request, or `None` when there is nothing to ask about.
///
/// Names are matched by **substring**, which is narrower than the subsequence
/// match [`otto_kit::matching::score`] does on the results. A query the index
/// answers can therefore miss a file the walk would have found — `otfl` finds
/// `otto-files.rs` on a machine with no indexer and not on one with. Widening
/// it would mean asking the index for every file under the roots and scoring
/// them here, which is the walk with extra steps.
fn sparql_for(request: &Request) -> Option<String> {
    let mut wheres = vec![
        "?f a nfo:FileDataObject".to_string(),
        "?f nfo:fileName ?n".to_string(),
        "?f nfo:fileLastModified ?m".to_string(),
    ];

    if request.files_only {
        // Folders are `nfo:FileDataObject` too, and they dominate a
        // newest-first answer — a folder's time changes every time anything
        // inside it does. Excluded by mime type rather than by asking whether
        // the resource is an `nfo:Folder`: the two are equivalent, and the
        // type test made the same query take seventeen seconds instead of one.
        wheres.push("?f nie:interpretedAs/nie:mimeType ?mt".to_string());
        wheres.push("FILTER(?mt != \"inode/directory\")".to_string());
    }

    if request.roots.is_empty() {
        return None;
    }
    let scope = request
        .roots
        .iter()
        .map(|root| format!("STRSTARTS(STR(?f), {})", sparql_string(&file_url(root))))
        .collect::<Vec<_>>()
        .join(" || ");
    wheres.push(format!("FILTER({scope})"));

    if let Some(query) = &request.query {
        let needle = query.to_lowercase();
        wheres.push(format!(
            "FILTER(CONTAINS(fn:lower-case(?n), {}))",
            sparql_string(&needle)
        ));
    }

    // Ordered newest-first in both cases, because that is what the cap has to
    // cut against: for Recent it *is* the ranking, and for a query it is the
    // least arbitrary way to choose which matches to bring back when there are
    // more of them than anyone will read.
    Some(format!(
        "SELECT DISTINCT ?f WHERE {{ {} }} ORDER BY DESC(?m) LIMIT {}",
        wheres.join(" . "),
        LIMIT * 2
    ))
}

/// `text` as a SPARQL string literal.
///
/// The query is built from something the user typed, so this is the boundary
/// that keeps a quotation mark in a filename from being a query of its own.
/// Control characters are dropped rather than escaped — no filename anyone
/// means to search for has one, and there is no reason to find out what the
/// parser does with a raw newline.
fn sparql_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A path as a `file:` URL, percent-encoding what has to be encoded.
fn file_url(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The path a `file:` URL names, or `None` if it names something else.
fn path_from_file_url(url: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let rest = url.strip_prefix("file://")?;
    // Only this host's own files. `file://otherhost/...` is not ours to open.
    let rest = rest.strip_prefix('/').map(|r| format!("/{r}"))?;

    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

/// Run one SPARQL SELECT and return the first column of every row.
///
/// `Query` hands its rows back down a pipe rather than in the reply, so the
/// read end is drained on a thread of its own: the daemon writes the whole
/// result set before the method returns, and a result set larger than a pipe
/// buffer would otherwise deadlock the two ends against each other.
fn query(sparql: &str) -> Result<Vec<String>, Unavailable> {
    let (read, write) = pipe().map_err(|e| Unavailable(format!("pipe: {e}")))?;

    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut file = std::fs::File::from(read);
        let mut buffer = Vec::new();
        let _ = file.read_to_end(&mut buffer);
        buffer
    });

    // A runtime of this thread's own rather than the application's. The search
    // worker is a plain thread and has no handle to the one `main` is running,
    // and a current-thread runtime for one round trip costs less than plumbing
    // one through would.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Unavailable(format!("runtime: {e}")))?;

    let called = runtime.block_on(async {
        let connection = zbus::Connection::session().await?;
        let arguments: HashMap<String, zbus::zvariant::Value<'_>> = HashMap::new();
        connection
            .call_method(
                Some(LOCALSEARCH_NAME),
                ENDPOINT_PATH,
                Some(ENDPOINT_IFACE),
                "Query",
                &(sparql, zbus::zvariant::Fd::Owned(write), arguments),
            )
            .await
            .map(|_| ())
    });

    let buffer = reader.join().unwrap_or_default();
    called.map_err(|e| Unavailable(format!("{e}")))?;
    Ok(cursor_rows(&buffer))
}

/// A pipe, both ends owned, with the write end kept out of a child's hands.
fn pipe() -> std::io::Result<(std::os::fd::OwnedFd, std::os::fd::OwnedFd)> {
    use std::os::fd::FromRawFd;
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` is two ints, which is what `pipe2` writes, and each is
    // wrapped in an `OwnedFd` exactly once so neither is closed twice.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    unsafe {
        Ok((
            std::os::fd::OwnedFd::from_raw_fd(fds[0]),
            std::os::fd::OwnedFd::from_raw_fd(fds[1]),
        ))
    }
}

/// The first column of every row in a TinySPARQL cursor stream.
///
/// The wire format is not documented anywhere, but it is simple and it is what
/// `Query` gives you — there is no JSON or Turtle route out of a `SELECT`.
/// Each row is, in the machine's own byte order:
///
/// ```text
/// u32                 number of columns
/// u32 × columns       value type of each column
/// u32 × columns       end offset of each column's text within the blob
/// bytes               the blob: one NUL-terminated string per column
/// ```
///
/// Offsets are the index of each string's terminating NUL, so the blob is one
/// byte longer than the last of them. A stream that does not parse yields the
/// rows read so far rather than an error: a partial answer from an index that
/// is only an accelerator is not worth failing a search over.
fn cursor_rows(buffer: &[u8]) -> Vec<String> {
    let mut rows = Vec::new();
    let mut at = 0usize;

    let u32_at = |buffer: &[u8], at: usize| -> Option<usize> {
        let bytes: [u8; 4] = buffer.get(at..at + 4)?.try_into().ok()?;
        Some(u32::from_ne_bytes(bytes) as usize)
    };

    loop {
        let Some(columns) = u32_at(buffer, at) else {
            return rows;
        };
        at += 4;
        if columns == 0 {
            return rows;
        }
        // The types are skipped: every column asked for here is a string or a
        // resource, and both arrive as text in the blob.
        at += 4 * columns;
        let Some(first_end) = u32_at(buffer, at) else {
            return rows;
        };
        let Some(last_end) = u32_at(buffer, at + 4 * (columns - 1)) else {
            return rows;
        };
        at += 4 * columns;
        let Some(blob) = buffer.get(at..at + last_end + 1) else {
            return rows;
        };
        at += last_end + 1;
        match blob.get(..first_end).map(String::from_utf8_lossy) {
            Some(text) => rows.push(text.into_owned()),
            None => return rows,
        }
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
        let sparql = sparql_for(&Request::folder("report".into(), dir.clone()))
            .expect("a folder scope is answerable");

        assert!(
            sparql.contains(&format!("STRSTARTS(STR(?f), \"{}", file_url(&dir))),
            "confined to the folder: {sparql}"
        );
        assert!(
            sparql.contains("report"),
            "and still asks the query: {sparql}"
        );

        // Everywhere asks the same question of the whole of home, so the two
        // are comparable rather than merely both called search.
        let everywhere = sparql_for(&Request::everywhere("report".into()));
        assert!(everywhere.is_some_and(|q| q.contains("report")));
    }

    /// Recent is the same query with nothing to match on: newest first, and
    /// without the folders, whose time changes whenever anything inside them
    /// does and which would otherwise fill the listing.
    #[test]
    fn recent_asks_the_index_for_files_newest_first() {
        let sparql = sparql_for(&Request::recent()).expect("Recent is answerable");
        assert!(sparql.contains("ORDER BY DESC(?m)"), "{sparql}");
        assert!(sparql.contains("inode/directory"), "{sparql}");
    }

    /// The state that has to be visible rather than papered over. With nothing
    /// able to answer, the batch comes back empty *and* marked unavailable, so
    /// the pane can say the indexer is off instead of "nothing found" — which
    /// would send someone looking for a file that is sitting on the disk.
    #[test]
    fn an_index_that_cannot_answer_says_so_rather_than_reporting_an_empty_result() {
        // No roots, so the query cannot even be built: the failure happens
        // before the bus is touched, which makes this independent of whether a
        // daemon happens to be running on the machine running the tests.
        let batch = run_to_batch(Request {
            query: Some("anything".into()),
            roots: Vec::new(),
            files_only: false,
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

    /// Against the real daemon, which is the only thing that can say whether
    /// the SPARQL is *accepted* as well as well-formed. Ignored by default:
    /// it needs a running indexer, and what it finds depends on the machine.
    ///
    ///     cargo test -p otto-files --lib live_index -- --ignored --nocapture
    #[test]
    #[ignore = "needs a running LocalSearch indexer"]
    fn live_index_answers_both_scopes() {
        let home = crate::model::home_dir().expect("a home directory");

        for (what, request) in [
            ("recent", Request::recent()),
            ("everywhere", Request::everywhere("rs".into())),
        ] {
            let sparql = sparql_for(&request).expect("answerable");
            let rows = query(&sparql).unwrap_or_else(|e| panic!("{what}: {e:?}"));
            println!("{what}: {} rows", rows.len());
            assert!(!rows.is_empty(), "{what} returned nothing");
        }

        // The scope has to actually narrow, not merely parse: a folder filter
        // the index ignored would make This Folder a slower Everywhere, and
        // nothing on screen would say so.
        let dir = home.join("Documents");
        if !dir.is_dir() {
            println!("no Documents to scope to; skipping the narrowing check");
            return;
        }
        let sparql = sparql_for(&Request::folder("a".into(), dir.clone())).expect("answerable");
        let rows = query(&sparql).expect("the folder scope is accepted");
        println!("folder: {} rows under {}", rows.len(), dir.display());
        let prefix = file_url(&dir);
        assert!(
            rows.iter().all(|url| url.starts_with(&prefix)),
            "a row escaped the folder scope"
        );
    }

    #[test]
    fn a_quotation_mark_in_a_query_cannot_close_the_sparql_string() {
        let escaped = sparql_string("say \"hi\" \\ now");
        assert_eq!(escaped, "\"say \\\"hi\\\" \\\\ now\"");
        assert_eq!(sparql_string("a\nb"), "\"ab\"");
    }

    #[test]
    fn file_urls_round_trip_through_the_characters_that_need_encoding() {
        let path = PathBuf::from("/home/u/Documents/a b&c%d — é.txt");
        assert_eq!(path_from_file_url(&file_url(&path)), Some(path));
    }

    #[test]
    fn a_cursor_row_is_read_back_out_of_its_wire_format() {
        // One row, two columns: the shape `Query` actually writes.
        let url = "file:///tmp/x.txt";
        let mtime = "2026-01-01T00:00:00Z";
        let mut blob = Vec::new();
        blob.extend_from_slice(url.as_bytes());
        blob.push(0);
        blob.extend_from_slice(mtime.as_bytes());
        blob.push(0);

        let mut wire = Vec::new();
        wire.extend_from_slice(&2u32.to_ne_bytes());
        wire.extend_from_slice(&1u32.to_ne_bytes());
        wire.extend_from_slice(&5u32.to_ne_bytes());
        wire.extend_from_slice(&(url.len() as u32).to_ne_bytes());
        wire.extend_from_slice(&((url.len() + 1 + mtime.len()) as u32).to_ne_bytes());
        wire.extend_from_slice(&blob);

        assert_eq!(cursor_rows(&wire), vec![url.to_string()]);
    }

    #[test]
    fn a_truncated_cursor_yields_what_it_had_rather_than_panicking() {
        assert!(cursor_rows(&[1, 0, 0]).is_empty());
        assert!(cursor_rows(&2u32.to_ne_bytes()).is_empty());
    }
}
