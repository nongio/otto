//! LocalSearch (TinySPARQL), the desktop's file index, reached over D-Bus.
//!
//! It is asked for **paths** and, for full-text queries, snippets. Anything
//! else a caller shows (size, modification time, kind) it should read from
//! the filesystem: the index is always a little behind the disk, and a row it
//! remembers can name a file that has since been deleted or rewritten.

// Rust guideline compliant 2026-02-21

use std::collections::HashMap;
use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::plan::{Sparql, MARK_CLOSE, MARK_OPEN};

const LOCALSEARCH_NAME: &str = "org.freedesktop.LocalSearch3";
const ENDPOINT_PATH: &str = "/org/freedesktop/Tracker3/Endpoint";
const ENDPOINT_IFACE: &str = "org.freedesktop.Tracker3.Endpoint";

/// One file the index returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub path: PathBuf,
    /// For a full-text query, the passage that matched.
    pub snippet: Option<Snippet>,
}

/// A passage of a file's text with the matched words marked.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snippet {
    pub text: String,
    /// Byte ranges of `text` that matched.
    pub marks: Vec<Range<usize>>,
}

impl Snippet {
    /// Read a snippet the index wrote with [`MARK_OPEN`] and [`MARK_CLOSE`]
    /// around each match.
    fn parse(raw: &str) -> Self {
        let mut snippet = Snippet::default();
        let mut open = None;
        for c in raw.chars() {
            match c {
                MARK_OPEN => open = Some(snippet.text.len()),
                MARK_CLOSE => {
                    if let Some(start) = open.take() {
                        snippet.marks.push(start..snippet.text.len());
                    }
                }
                // Documents carry line breaks and tabs mid-passage; a snippet
                // is shown as one line.
                c if c.is_whitespace() => {
                    if !snippet.text.ends_with(' ') {
                        snippet.text.push(' ');
                    }
                }
                c => snippet.text.push(c),
            }
        }
        snippet
    }
}

impl fmt::Display for Snippet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// The index could not answer: it is not running, or refused the query.
///
/// Distinct from an empty answer. "Nothing matched" and "nothing was able to
/// look" must not look alike to the person searching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unavailable(String);

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the file index could not answer: {}", self.0)
    }
}

impl std::error::Error for Unavailable {}

/// Run `sparql` against LocalSearch.
///
/// Blocks for one D-Bus round trip; call it off any UI thread. Rows whose URL
/// is not a local `file:` URL are skipped.
///
/// # Errors
///
/// [`Unavailable`] when the index is not running or rejects the query.
pub fn search(sparql: &Sparql) -> Result<Vec<Hit>, Unavailable> {
    let rows = query(&sparql.text, &sparql.bindings)?;
    Ok(rows
        .into_iter()
        .filter_map(|mut row| {
            let snippet =
                (sparql.snippets && row.len() > 1).then(|| Snippet::parse(&row.swap_remove(1)));
            let path = path_from_file_url(row.first()?)?;
            Some(Hit { path, snippet })
        })
        .collect())
}

/// How much the index holds, on the volumes mounted now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    /// Files, folders excluded.
    pub files: u64,
    pub folders: u64,
}

/// Count the files and folders the index holds.
///
/// One round trip, counted by the index itself, so it stays quick on an index
/// of hundreds of thousands of files. Blocks; call it off any UI thread.
///
/// Asking starts LocalSearch if it is installed but not running, since its
/// endpoint is D-Bus activatable. A caller that must not start it checks that
/// the name has an owner first.
///
/// # Errors
///
/// [`Unavailable`] when the index is not running or rejects the query.
pub fn counts() -> Result<Counts, Unavailable> {
    // The two counts `localsearch status` prints (`count-files.rq` and
    // `count-folders.rq` in LocalSearch), in one SELECT: only what is on a
    // mounted volume, and a folder is not also counted as a file.
    const COUNTS: &str = "SELECT ?files ?folders WHERE { \
        { SELECT (COUNT(?file) AS ?files) WHERE { GRAPH tracker:FileSystem { \
            ?file a nfo:FileDataObject ; nie:dataSource/tracker:available true . \
            FILTER (! EXISTS { ?file nie:interpretedAs/rdf:type nfo:Folder }) } } } \
        { SELECT (COUNT(?folder) AS ?folders) WHERE { GRAPH tracker:FileSystem { \
            ?folder a nfo:Folder ; \
                nie:isStoredAs/nie:dataSource/tracker:available true . } } } }";
    let rows = query(COUNTS, &[])?;
    let row = rows
        .first()
        .ok_or_else(|| Unavailable("no answer to a count".into()))?;
    let number = |column: usize| -> Result<u64, Unavailable> {
        row.get(column)
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| Unavailable(format!("not a count: {row:?}")))
    };
    Ok(Counts {
        files: number(0)?,
        folders: number(1)?,
    })
}

/// What the file indexer is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// LocalSearch is not installed: nothing on the bus can start it.
    Missing,
    /// Installed but not running. Searching would start it.
    Stopped,
    /// Running and caught up with the disk, as far as it knows.
    Idle,
    /// Crawling or reading files; results may be missing what it has not
    /// reached yet.
    Indexing,
    /// Paused, by an application or because the battery or disk is low.
    Paused,
}

impl State {
    /// The word used for this state in `otto-search --json`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Stopped => "stopped",
            Self::Idle => "idle",
            Self::Indexing => "indexing",
            Self::Paused => "paused",
        }
    }
}

/// The indexer's state and how far along it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Status {
    pub state: State,
    /// From 0 to 1; 1 when idle.
    pub progress: f64,
    /// The indexer's own estimate of the time left, when it has one.
    pub remaining: Option<std::time::Duration>,
}

impl Status {
    /// Whether a search now may miss files: the indexer is still at work, or
    /// stopped partway.
    pub fn is_behind(&self) -> bool {
        matches!(self.state, State::Indexing | State::Paused)
    }

    /// The status as one JSON object, the trailing line of `otto-search --json`.
    pub fn to_json(&self) -> String {
        let remaining = self
            .remaining
            .map_or_else(|| "null".to_owned(), |d| d.as_secs().to_string());
        format!(
            "{{\"index\":{{\"state\":\"{}\",\"progress\":{:.2},\"remaining_seconds\":{remaining}}}}}",
            self.state.as_str(),
            self.progress,
        )
    }
}

const MINER_PATH: &str = "/org/freedesktop/Tracker3/Miner/Files";
const MINER_IFACE: &str = "org.freedesktop.Tracker3.Miner";

/// Ask LocalSearch what it is doing, without starting it.
///
/// A few D-Bus round trips; blocks, so call it off any UI thread. Any failure
/// to reach the bus reads as [`State::Missing`]: either way there is no index
/// to search.
pub fn status() -> Status {
    let absent = |state| Status {
        state,
        progress: 0.0,
        remaining: None,
    };
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return absent(State::Missing);
    };
    runtime.block_on(async {
        let Ok(connection) = zbus::Connection::session().await else {
            return absent(State::Missing);
        };
        let Ok(bus) = zbus::fdo::DBusProxy::new(&connection).await else {
            return absent(State::Missing);
        };
        let name = zbus::names::BusName::try_from(LOCALSEARCH_NAME).expect("a valid bus name");
        if !bus.name_has_owner(name).await.unwrap_or(false) {
            let activatable = bus
                .list_activatable_names()
                .await
                .unwrap_or_default()
                .iter()
                .any(|n| n.as_str() == LOCALSEARCH_NAME);
            return absent(if activatable {
                State::Stopped
            } else {
                State::Missing
            });
        }
        let call = |method: &'static str| {
            let connection = connection.clone();
            async move {
                connection
                    .call_method(
                        Some(LOCALSEARCH_NAME),
                        MINER_PATH,
                        Some(MINER_IFACE),
                        method,
                        &(),
                    )
                    .await
            }
        };
        let text: String = match call("GetStatus").await {
            Ok(reply) => reply.body().deserialize().unwrap_or_default(),
            Err(_) => return absent(State::Missing),
        };
        let progress: f64 = match call("GetProgress").await {
            Ok(reply) => reply.body().deserialize().unwrap_or(0.0),
            Err(_) => 0.0,
        };
        let seconds: i32 = match call("GetRemainingTime").await {
            Ok(reply) => reply.body().deserialize().unwrap_or(-1),
            Err(_) => -1,
        };
        Status {
            state: state_from(&text),
            progress: progress.clamp(0.0, 1.0),
            remaining: u64::try_from(seconds)
                .ok()
                .filter(|&s| s > 0)
                .map(std::time::Duration::from_secs),
        }
    })
}

/// The state behind one of the miner's status strings.
///
/// They are free text ("Idle", "Paused", "Crawling single directory …",
/// "Processing…"), so only the two fixed ones are named and anything else
/// is work in progress.
fn state_from(text: &str) -> State {
    match text {
        "Idle" => State::Idle,
        "Paused" => State::Paused,
        _ => State::Indexing,
    }
}

/// Ask LocalSearch to look at `path` again now, and at everything under it
/// when it is a folder.
///
/// For a file moved or written since the index last noticed. Blocks for one
/// round trip.
///
/// # Errors
///
/// [`Unavailable`] when the indexer's control service does not answer.
pub fn reindex(path: &Path) -> Result<(), Unavailable> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Unavailable(format!("runtime: {e}")))?;
    runtime
        .block_on(async {
            let connection = zbus::Connection::session().await?;
            connection
                .call_method(
                    Some("org.freedesktop.LocalSearch3.Control"),
                    "/org/freedesktop/Tracker3/Miner/Files/Index",
                    Some("org.freedesktop.Tracker3.Miner.Files.Index"),
                    "IndexLocation",
                    &(file_url(path), Vec::<&str>::new(), Vec::<&str>::new()),
                )
                .await
                .map(|_| ())
        })
        .map_err(|e: zbus::Error| Unavailable(e.to_string()))
}

/// Run one SPARQL SELECT and return every row, each column as text.
///
/// `Query` hands its rows back down a pipe rather than in the reply, so the
/// read end is drained on a thread of its own: the daemon writes the whole
/// result set before the method returns, and a result set larger than a pipe
/// buffer would otherwise deadlock the two ends against each other.
fn query(sparql: &str, bindings: &[(String, String)]) -> Result<Vec<Vec<String>>, Unavailable> {
    let (read, write) = pipe().map_err(|e| Unavailable(format!("pipe: {e}")))?;

    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut file = std::fs::File::from(read);
        let mut buffer = Vec::new();
        let _ = file.read_to_end(&mut buffer);
        buffer
    });

    // A runtime of this thread's own rather than the caller's. Callers are
    // plain worker threads, and a current-thread runtime for one round trip
    // costs less than plumbing one through would.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Unavailable(format!("runtime: {e}")))?;

    let called = runtime.block_on(async {
        let connection = zbus::Connection::session().await?;
        let arguments: HashMap<&str, zbus::zvariant::Value<'_>> = bindings
            .iter()
            .map(|(name, value)| (name.as_str(), zbus::zvariant::Value::from(value.as_str())))
            .collect();
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
    called.map_err(|e| Unavailable(e.to_string()))?;
    Ok(cursor_rows(&buffer))
}

/// A pipe, both ends owned, with the write end kept out of a child's hands.
fn pipe() -> std::io::Result<(std::os::fd::OwnedFd, std::os::fd::OwnedFd)> {
    use std::os::fd::FromRawFd;
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` is two ints, which is what `pipe2` writes.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `pipe2` succeeded, so both are open descriptors this process
    // owns, and each is wrapped exactly once so neither is closed twice.
    unsafe {
        Ok((
            std::os::fd::OwnedFd::from_raw_fd(fds[0]),
            std::os::fd::OwnedFd::from_raw_fd(fds[1]),
        ))
    }
}

/// Every row in a TinySPARQL cursor stream.
///
/// The wire format is not documented anywhere, but it is simple and it is what
/// `Query` gives you; there is no JSON or Turtle route out of a `SELECT`.
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
/// rows read so far rather than an error: a partial answer is still an answer.
fn cursor_rows(buffer: &[u8]) -> Vec<Vec<String>> {
    let u32_at = |at: usize| -> Option<usize> {
        let bytes: [u8; 4] = buffer.get(at..at + 4)?.try_into().ok()?;
        usize::try_from(u32::from_ne_bytes(bytes)).ok()
    };

    let mut rows = Vec::new();
    let mut at = 0usize;
    while let Some(columns) = u32_at(at).filter(|&c| c > 0) {
        at += 4;
        // The types are skipped: every column is read as text, which is how
        // the blob carries all of them.
        at += 4 * columns;
        let Some(ends) = (0..columns)
            .map(|i| u32_at(at + 4 * i))
            .collect::<Option<Vec<_>>>()
        else {
            break;
        };
        at += 4 * columns;
        let last = *ends.last().unwrap_or(&0);
        let Some(blob) = buffer.get(at..at + last + 1) else {
            break;
        };
        at += last + 1;
        let mut start = 0;
        let mut row = Vec::with_capacity(columns);
        for end in ends {
            let Some(text) = blob.get(start..end) else {
                return rows;
            };
            row.push(String::from_utf8_lossy(text).into_owned());
            start = end + 1;
        }
        rows.push(row);
    }
    rows
}

/// A path as a `file:` URL, percent-encoding what has to be encoded.
pub fn file_url(path: &Path) -> String {
    use std::fmt::Write as _;
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(char::from(byte));
            }
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

/// The path a `file:` URL names, or `None` if it names something else.
pub fn path_from_file_url(url: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    // Only this host's own files. `file://otherhost/...` is not ours to open.
    let rest = url.strip_prefix("file:///")?;

    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() + 1);
    out.push(b'/');
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let byte = bytes
                .get(i + 1..i + 3)
                .and_then(|hex| std::str::from_utf8(hex).ok())
                .and_then(|hex| u8::from_str_radix(hex, 16).ok());
            if let Some(byte) = byte {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(rows: &[&[&str]]) -> Vec<u8> {
        let mut wire = Vec::new();
        for row in rows {
            let mut blob = Vec::new();
            let mut ends = Vec::new();
            for column in *row {
                blob.extend_from_slice(column.as_bytes());
                ends.push(u32::try_from(blob.len()).unwrap());
                blob.push(0);
            }
            wire.extend_from_slice(&u32::try_from(row.len()).unwrap().to_ne_bytes());
            for _ in *row {
                wire.extend_from_slice(&1u32.to_ne_bytes());
            }
            for end in ends {
                wire.extend_from_slice(&end.to_ne_bytes());
            }
            wire.extend_from_slice(&blob);
        }
        wire
    }

    #[test]
    fn cursor_rows_come_back_with_every_column() {
        let rows = cursor_rows(&wire(&[&["file:///a", "x"], &["file:///b", ""]]));
        assert_eq!(rows, vec![vec!["file:///a", "x"], vec!["file:///b", ""]]);
    }

    #[test]
    fn a_truncated_cursor_yields_what_it_had() {
        let mut bytes = wire(&[&["file:///a"], &["file:///b"]]);
        bytes.truncate(bytes.len() - 3);
        assert_eq!(cursor_rows(&bytes), vec![vec!["file:///a"]]);
        assert!(cursor_rows(&[1, 0, 0]).is_empty());
    }

    #[test]
    fn file_urls_round_trip_through_the_characters_that_need_encoding() {
        let path = PathBuf::from("/home/u/Documents/a b&c%d — é.txt");
        assert_eq!(path_from_file_url(&file_url(&path)), Some(path));
        assert_eq!(path_from_file_url("file://host/x"), None);
        assert_eq!(
            path_from_file_url("file:///a%zz"),
            Some(PathBuf::from("/a%zz"))
        );
    }

    #[test]
    fn miner_status_text_maps_to_a_state() {
        assert_eq!(state_from("Idle"), State::Idle);
        assert_eq!(state_from("Paused"), State::Paused);
        assert_eq!(state_from("Crawling single directory 'x'"), State::Indexing);
        assert_eq!(state_from("Initializing"), State::Indexing);
    }

    #[test]
    fn status_json_is_one_object() {
        let status = Status {
            state: State::Indexing,
            progress: 0.5,
            remaining: Some(std::time::Duration::from_secs(90)),
        };
        assert_eq!(
            status.to_json(),
            r#"{"index":{"state":"indexing","progress":0.50,"remaining_seconds":90}}"#
        );
        assert!(status.is_behind());
    }

    #[test]
    fn snippets_become_one_line_with_marked_ranges() {
        let snippet = Snippet::parse("…the \u{2}tax\u{3}\nreturn \u{2}2024\u{3}…");
        assert_eq!(snippet.text, "…the tax return 2024…");
        let marked: Vec<&str> = snippet
            .marks
            .iter()
            .map(|r| &snippet.text[r.clone()])
            .collect();
        assert_eq!(marked, vec!["tax", "2024"]);
    }
}
