//! Running a [`Plan`] to a ranked, capped list of files that are really there.
//!
//! One policy for every caller: the index is paged past the files it
//! remembers but the disk no longer has, each row is statted and checked
//! again against the plan, and what survives is ranked by the plan's order
//! and cut to a limit. Files streams the pages into a window; the
//! `otto-search` command prints the last one.
//!
//! ```no_run
//! use otto_search::{find, parse, Clock, Plan, Scope};
//!
//! let scope = Scope { roots: vec!["/home/u".into()], ..Scope::default() };
//! let plan = Plan::new(&parse("invoice kind:pdf"), &scope, Clock::now()).unwrap();
//! for found in find::find(&plan, 20, &mut ())?.best() {
//!     println!("{}", found.path.display());
//! }
//! # Ok::<(), otto_search::Unavailable>(())
//! ```

// Rust guideline compliant 2026-02-21

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::index::{self, Snippet, Unavailable};
use crate::plan::{Facts, Plan};
use crate::query::Sort;

/// A file the index named, as the disk describes it now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    /// The last component of `path`, lossily decoded.
    pub name: String,
    /// Whether it is a folder, following a symlink to one.
    pub is_dir: bool,
    pub is_symlink: bool,
    /// Bytes; `None` for a broken symlink.
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    /// For a `text:` query, the passage that matched.
    pub snippet: Option<Snippet>,
}

impl Found {
    /// Read what the disk says about `path`, or `None` if nothing is there.
    ///
    /// The link itself is looked at first: a broken symlink still exists and
    /// still belongs in a listing, and following it alone would say it does
    /// not. Size, time and folder-ness are the target's.
    pub fn stat(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_string_lossy().into_owned();
        let link = std::fs::symlink_metadata(path).ok()?;
        let is_symlink = link.file_type().is_symlink();
        let meta = if is_symlink {
            std::fs::metadata(path).ok()
        } else {
            Some(link)
        };
        Some(Self {
            name,
            path: path.to_path_buf(),
            is_dir: meta.as_ref().is_some_and(std::fs::Metadata::is_dir),
            is_symlink,
            size: meta.as_ref().map(std::fs::Metadata::len),
            modified: meta.as_ref().and_then(|m| m.modified().ok()),
            snippet: None,
        })
    }

    /// Seconds since the epoch this was last written, if known.
    pub fn modified_secs(&self) -> Option<i64> {
        self.modified.and_then(epoch_secs)
    }

    /// What [`Plan::recheck`] and [`Plan::admits`] need, with the MIME type
    /// the caller knows, if any.
    pub fn facts<'a>(&'a self, mime: Option<&'a str>) -> Facts<'a> {
        Facts {
            path: &self.path,
            is_dir: self.is_dir,
            size: self.size,
            modified: self.modified_secs(),
            mime,
        }
    }

    /// This file as one line of JSON, the `otto-search --json` format:
    /// `path`, `name`, `kind` (`file` or `folder`), `modified` (RFC 3339 in
    /// local time, or `null`), `size` (bytes, or `null` for a folder) and
    /// `snippet` (or `null`).
    pub fn to_json(&self) -> String {
        let mut out = String::from("{\"path\":");
        json_string(&mut out, &self.path.to_string_lossy());
        out.push_str(",\"name\":");
        json_string(&mut out, &self.name);
        out.push_str(",\"kind\":");
        out.push_str(if self.is_dir {
            "\"folder\""
        } else {
            "\"file\""
        });
        out.push_str(",\"modified\":");
        match self.modified_secs() {
            Some(secs) => json_string(&mut out, &crate::dates::rfc3339_local(secs)),
            None => out.push_str("null"),
        }
        out.push_str(",\"size\":");
        match self.size.filter(|_| !self.is_dir) {
            Some(size) => out.push_str(&size.to_string()),
            None => out.push_str("null"),
        }
        out.push_str(",\"snippet\":");
        match &self.snippet {
            Some(snippet) => json_string(&mut out, &snippet.text),
            None => out.push_str("null"),
        }
        out.push('}');
        out
    }
}

/// Append `text` to `out` as a JSON string literal.
fn json_string(out: &mut String, text: &str) {
    use std::fmt::Write as _;
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn epoch_secs(t: SystemTime) -> Option<i64> {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
}

/// The running result set: everything found so far, trimmed to the best
/// `limit` whenever it has grown enough to be worth the sort.
///
/// Callers that find files some other way (Files, from the words it has read
/// in pictures) [`push`](Self::push) them in beside the index's, and one path
/// found both ways is one result.
#[derive(Debug, Clone)]
pub struct Results {
    limit: usize,
    sort: Sort,
    ranks_names: bool,
    /// Each file and the number that decides whether it survives the cap.
    /// Higher is better, whatever the order asked for.
    ranked: Vec<(i64, Found)>,
}

impl Results {
    /// An empty set for `plan`, keeping at most `limit` files.
    pub fn new(plan: &Plan, limit: usize) -> Self {
        Self {
            limit,
            sort: plan.sort(),
            ranks_names: plan.ranks_names(),
            ranked: Vec::new(),
        }
    }

    /// Add `found`, whose name scored `score` under [`Plan::rank`].
    pub fn push(&mut self, found: Found, score: i64) {
        let key = self.order_key(&found, score);
        self.ranked.push((key, found));
        // Trimmed at twice the cap rather than at the cap, so a long answer
        // sorts once per `limit` results instead of once per result.
        if self.ranked.len() > self.limit.saturating_mul(2) {
            self.trim();
        }
    }

    /// The number the cap keeps the highest of.
    ///
    /// Name score, modification time or size, depending on the order asked
    /// for. An A-to-Z order ranks like relevance: the cap keeps the best
    /// matches, and [`Self::best`] then puts them in name order.
    fn order_key(&self, found: &Found, score: i64) -> i64 {
        // A file whose time could not be read sorts last rather than being
        // dropped: it is still a file that is there.
        let newest = found.modified_secs().unwrap_or(i64::MIN);
        match self.sort {
            Sort::Modified => newest,
            Sort::Size => found
                .size
                .map_or(i64::MIN, |s| i64::try_from(s).unwrap_or(i64::MAX)),
            // Nothing to match on is Recent's question: newest first.
            Sort::Relevance | Sort::Name if !self.ranks_names => newest,
            Sort::Relevance | Sort::Name => score,
        }
    }

    /// Sort best first, keep the best row for each path, and cut to the
    /// limit. Duplicates go before the cut, so a file found twice cannot
    /// crowd another out of the last place.
    fn trim(&mut self) {
        // Stable, so equal keys keep the index's own order, which is roughly
        // best first already.
        self.ranked.sort_by_key(|(key, _)| std::cmp::Reverse(*key));
        let mut seen = HashSet::new();
        self.ranked
            .retain(|(_, found)| seen.insert(found.path.clone()));
        self.ranked.truncate(self.limit);
    }

    /// The result set as it stands, one per path: best first, or A to Z for
    /// `sort:name`.
    pub fn best(&mut self) -> Vec<Found> {
        self.trim();
        let mut best: Vec<Found> = self.ranked.iter().map(|(_, found)| found.clone()).collect();
        if self.sort == Sort::Name {
            best.sort_by_cached_key(|found| found.name.to_lowercase());
        }
        best
    }
}

/// How a caller follows a [`find`] in progress. Both methods default to
/// "carry on", so `&mut ()` suits a caller that only wants the end result.
pub trait Progress {
    /// Whether anyone still wants the answer. Asked between rows, so a
    /// search that has been superseded stops within one stat.
    fn alive(&self) -> bool {
        true
    }

    /// The results so far, after a page that is to be followed by another.
    /// Returns whether to carry on.
    fn page(&mut self, _results: &mut Results) -> bool {
        true
    }
}

impl Progress for () {}

/// Rows asked of the index first, as a multiple of the limit.
const PAGE_FACTOR: usize = 2;

/// The least a first page asks for.
///
/// A small limit (the command line's twenty) would otherwise rank from a
/// handful of candidates. A round trip costs the index's sort of every
/// match, not the rows it returns, so asking for more costs little.
const MIN_PAGE: usize = 200;

/// How many pages to ask for before settling for what was found.
///
/// The index can remember files the disk no longer has: a folder deleted
/// while the indexer was not watching stays in it, sometimes thousands of
/// rows. Those rows are dropped here, and a page that was mostly ghosts is
/// followed by a bigger one, so they cannot starve a listing. Each page grows
/// fourfold for the same reason as [`MIN_PAGE`]: two big asks are cheaper
/// than five small ones.
const MAX_PAGES: usize = 3;

/// Run `plan` against the index to at most `limit` files that exist now.
///
/// Rows are statted rather than trusted, since the index lags the disk: one
/// whose file is gone is dropped, and each survivor is passed through
/// [`Plan::recheck`] with the disk's time and size and ranked with
/// [`Plan::rank`]. `progress` sees the results after every page but the
/// last, and can stop the search between rows.
///
/// A search stopped by `progress` returns what it had.
///
/// # Errors
///
/// [`Unavailable`] when the index could not answer the first page. A later
/// page that fails ends the search with what the earlier ones found.
pub fn find(
    plan: &Plan,
    limit: usize,
    progress: &mut impl Progress,
) -> Result<Results, Unavailable> {
    let mut results = Results::new(plan, limit);
    let mut found = 0;
    let (mut offset, mut size) = (0, limit.saturating_mul(PAGE_FACTOR).max(MIN_PAGE));
    for page in 0..MAX_PAGES {
        let hits = match index::search(&plan.sparql_page(offset, size)) {
            Ok(hits) => hits,
            Err(why) if page == 0 => return Err(why),
            Err(why) => {
                tracing::info!("search: page {page}: {why}");
                break;
            }
        };
        let full = hits.len() == size;
        offset += size;
        size = size.saturating_mul(4);
        let mut gone = 0;
        for hit in hits {
            if !progress.alive() {
                return Ok(results);
            }
            // Indexed but no longer on disk. Dropped silently: the index
            // catching up is not something to tell anyone about.
            let Some(mut file) = Found::stat(&hit.path) else {
                gone += 1;
                continue;
            };
            if !plan.recheck(&file.facts(None)) {
                continue;
            }
            if let Some(score) = plan.rank(&file.name) {
                file.snippet = hit.snippet;
                found += 1;
                results.push(file, score);
            }
        }
        if gone > 0 {
            tracing::debug!("search: page {page} named {gone} files that are gone");
        }
        if !full || found >= limit {
            break;
        }
        if !progress.page(&mut results) {
            break;
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse, Clock, Scope};

    fn plan(text: &str) -> Plan {
        let scope = Scope {
            roots: vec![PathBuf::from("/home/u")],
            ..Scope::default()
        };
        Plan::new(&parse(text), &scope, Clock::now()).expect("answerable")
    }

    fn file(name: &str, secs: u64, size: u64) -> Found {
        Found {
            path: PathBuf::from("/home/u").join(name),
            name: name.to_owned(),
            is_dir: false,
            is_symlink: false,
            size: Some(size),
            modified: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs)),
            snippet: None,
        }
    }

    fn names(results: &mut Results) -> Vec<String> {
        results.best().into_iter().map(|f| f.name).collect()
    }

    #[test]
    fn relevance_keeps_the_best_names_and_one_row_per_path() {
        let mut results = Results::new(&plan("tax"), 2);
        results.push(file("a", 30, 1), 5);
        results.push(file("b", 20, 1), 50);
        results.push(file("c", 10, 1), 20);
        results.push(file("b", 20, 1), 49);
        assert_eq!(names(&mut results), ["b", "c"]);
    }

    #[test]
    fn nothing_to_match_is_newest_first() {
        let mut results = Results::new(&plan("-kind:folder"), 10);
        results.push(file("old", 1, 1), 0);
        results.push(file("new", 9, 1), 0);
        assert_eq!(names(&mut results), ["new", "old"]);
    }

    #[test]
    fn size_and_name_orders() {
        let mut results = Results::new(&plan("sort:size"), 10);
        results.push(file("small", 1, 1), 0);
        results.push(file("big", 1, 100), 0);
        assert_eq!(names(&mut results), ["big", "small"]);

        // The cap keeps the best matches; the survivors come A to Z.
        let mut results = Results::new(&plan("e sort:name"), 2);
        results.push(file("Zebra", 1, 1), 90);
        results.push(file("apple", 1, 1), 80);
        results.push(file("eel", 1, 1), 1);
        assert_eq!(names(&mut results), ["apple", "Zebra"]);
    }

    #[test]
    fn a_broken_symlink_is_still_found() {
        let dir = std::env::temp_dir().join(format!("otto-search-find-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let link = dir.join("dangling");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(dir.join("nowhere"), &link).unwrap();

        let found = Found::stat(&link).expect("the link exists");
        assert!(found.is_symlink && !found.is_dir);
        assert_eq!(found.size, None);
        assert_eq!(Found::stat(&dir.join("nowhere")), None);
        assert!(Found::stat(&dir).expect("a folder").is_dir);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn json_lines_escape_what_they_must() {
        let mut found = file("a \"b\"\n.txt", 0, 12);
        found.snippet = Some(Snippet {
            text: "tab\there".into(),
            marks: Vec::new(),
        });
        let json = found.to_json();
        assert!(json.starts_with("{\"path\":\"/home/u/a \\\"b\\\"\\n.txt\""));
        assert!(json.contains("\"kind\":\"file\""));
        assert!(json.contains("\"size\":12"));
        assert!(json.ends_with("\"snippet\":\"tab\\there\"}"));

        let mut folder = file("dir", 0, 4096);
        folder.is_dir = true;
        folder.modified = None;
        let json = folder.to_json();
        assert!(json.contains("\"kind\":\"folder\",\"modified\":null,\"size\":null"));
    }
}
