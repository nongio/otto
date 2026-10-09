//! A query pinned to a place and a time, ready to ask.
//!
//! [`Plan::new`] resolves what a [`Query`] leaves open: which folders `in:`
//! and the default scope mean, and which instants `modified:today` means.
//! The plan then answers in two ways that agree with each other:
//! [`Plan::sparql`] asks the index, and [`Plan::admits`] checks a file the
//! caller found some other way, such as a picture whose words Otto read.
//!
//! ## SPARQL shape
//!
//! Every triple pattern comes first and every `FILTER` after, under
//! `SELECT DISTINCT`. LocalSearch plans the other orders badly: a size
//! pattern written after a filter took a minute on a home directory where
//! this shape takes a tenth of a second.
//!
//! Anything the user typed reaches the index as a bound parameter (`~name`),
//! never spliced into the query text, so there is no escaping to get wrong.
//! Values Otto computes itself (dates, byte counts) are written inline.

// Rust guideline compliant 2026-02-21

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::dates::{self, DAY};
use crate::query::{Civil, Kind, Query, SizeBound, Sort, TermKind, When};

/// Where a search looks when the query says nothing about it, and what
/// relative paths in `in:` are relative to.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    /// Folders searched, each with everything under it, when the query has no
    /// `in:`. Files uses home for Everywhere and the open folder for This
    /// Folder.
    pub roots: Vec<PathBuf>,
    /// What `~` means in `in:`.
    pub home: Option<PathBuf>,
    /// What a relative `in:` path is relative to.
    pub cwd: Option<PathBuf>,
}

/// The instant a query is read at, and the local time zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    /// Seconds since the epoch.
    pub now: i64,
    /// Seconds east of UTC, for where "today" starts.
    pub utc_offset: i64,
}

impl Clock {
    /// The system clock and time zone, now.
    pub fn now() -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
        Self {
            now,
            utc_offset: dates::local_offset(now),
        }
    }

    /// Local midnight at the start of today, as seconds since the epoch.
    fn midnight(self) -> i64 {
        (self.now + self.utc_offset).div_euclid(DAY) * DAY - self.utc_offset
    }

    /// When a calendar period starts and ends, local time.
    fn period(self, civil: Civil) -> (i64, i64) {
        let Civil { year, month, day } = civil;
        let start = dates::days_from_civil(year, month.unwrap_or(1), day.unwrap_or(1));
        let end = match (month, day) {
            (Some(_), Some(_)) => start + 1,
            (Some(m), None) => start + i64::from(dates::days_in_month(year, m)),
            _ => dates::days_from_civil(year + 1, 1, 1),
        };
        (start * DAY - self.utc_offset, end * DAY - self.utc_offset)
    }
}

/// A time range in seconds since the epoch; `from` inclusive, `to` exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Range {
    from: Option<i64>,
    to: Option<i64>,
}

impl Range {
    fn resolve(when: When, clock: Clock) -> Self {
        let span = |from, to| Range { from, to };
        let seconds = |s: u64| i64::try_from(s).unwrap_or(i64::MAX);
        match when {
            When::Today => span(Some(clock.midnight()), None),
            When::Yesterday => span(Some(clock.midnight() - DAY), Some(clock.midnight())),
            When::Within(s) => span(Some(clock.now.saturating_sub(seconds(s))), None),
            When::OlderThan(s) => span(None, Some(clock.now.saturating_sub(seconds(s)))),
            When::During(c) => {
                let (from, to) = clock.period(c);
                span(Some(from), Some(to))
            }
            When::Before(c) => span(None, Some(clock.period(c).0)),
            When::After(c) => span(Some(clock.period(c).1), None),
        }
    }

    fn contains(self, t: i64) -> bool {
        self.from.is_none_or(|from| t >= from) && self.to.is_none_or(|to| t < to)
    }
}

/// One condition of a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Filter {
    /// Lowercase; matched in order, not necessarily adjacent.
    Word(String),
    /// Lowercase; matched as written.
    Phrase(String),
    Text(String),
    Kinds(Vec<Kind>),
    Modified(Range),
    Size(SizeBound),
}

/// A query resolved against a [`Scope`] and a [`Clock`].
#[derive(Debug, Clone)]
pub struct Plan {
    roots: Vec<PathBuf>,
    excluded: Vec<PathBuf>,
    /// Each condition, and whether it is negated.
    filters: Vec<(bool, Filter)>,
    sort: Sort,
}

/// A SPARQL query and the values bound into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sparql {
    pub text: String,
    /// `(name, value)` for each `~name` in the text.
    pub bindings: Vec<(String, String)>,
    /// Whether the second column holds a text snippet.
    pub snippets: bool,
}

/// What is known about a file found without asking the index, for
/// [`Plan::admits`].
#[derive(Debug, Clone, Copy)]
pub struct Facts<'a> {
    pub path: &'a Path,
    pub is_dir: bool,
    pub size: Option<u64>,
    /// Seconds since the epoch.
    pub modified: Option<i64>,
    pub mime: Option<&'a str>,
}

/// Where snippet highlights start and end. Control characters, so no text
/// in a document can be mistaken for them; see [`crate::Snippet`].
pub(crate) const MARK_OPEN: char = '\u{2}';
pub(crate) const MARK_CLOSE: char = '\u{3}';

/// Words of context either side of a match in a snippet. Enough to read the
/// hit in a line under a file name; more would wrap.
const SNIPPET_WORDS: u32 = 10;

/// Name endings that make a file an archive, for the index, which has no
/// type for them; see [`kind_sparql`].
const ARCHIVE_EXTENSIONS: &[&str] = &[
    "zip", "tar", "gz", "tgz", "xz", "txz", "bz2", "tbz2", "zst", "7z", "rar",
];

/// MIME types of archives, for [`Plan::admits`], where a caller knows one.
const ARCHIVE_TYPES: &[&str] = &[
    "application/zip",
    "application/x-tar",
    "application/gzip",
    "application/x-compressed-tar",
    "application/x-xz",
    "application/x-xz-compressed-tar",
    "application/x-bzip2",
    "application/x-bzip2-compressed-tar",
    "application/zstd",
    "application/x-zstd-compressed-tar",
    "application/x-7z-compressed",
    "application/vnd.rar",
    "application/x-rar",
];

impl Plan {
    /// Resolve `query` against `scope` and `clock`.
    ///
    /// `None` when there is nowhere to look: no `in:` and no default roots.
    pub fn new(query: &Query, scope: &Scope, clock: Clock) -> Option<Self> {
        let mut roots = Vec::new();
        let mut excluded = Vec::new();
        let mut filters = Vec::new();
        for term in &query.terms {
            let filter = match &term.kind {
                TermKind::Word(word) => Filter::Word(word.to_lowercase()),
                TermKind::Phrase(phrase) => Filter::Phrase(phrase.to_lowercase()),
                TermKind::Text(text) => Filter::Text(text.clone()),
                TermKind::Kind(kinds) => Filter::Kinds(kinds.clone()),
                TermKind::Modified(when) => Filter::Modified(Range::resolve(*when, clock)),
                TermKind::Size(bound) => Filter::Size(*bound),
                TermKind::In(path) => {
                    let Some(path) = resolve_path(path, scope) else {
                        continue;
                    };
                    if term.negated {
                        excluded.push(path);
                    } else {
                        roots.push(path);
                    }
                    continue;
                }
                TermKind::Sort(_) => continue,
            };
            filters.push((term.negated, filter));
        }
        if roots.is_empty() {
            roots.clone_from(&scope.roots);
        }
        if roots.is_empty() {
            return None;
        }
        Some(Self {
            roots,
            excluded,
            filters,
            sort: query.sort(),
        })
    }

    /// The folders searched.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// The order asked for.
    pub fn sort(&self) -> Sort {
        self.sort
    }

    /// The words and phrases the contents must contain.
    pub fn text_needles(&self) -> impl Iterator<Item = &str> {
        self.filters.iter().filter_map(|(negated, f)| match f {
            Filter::Text(text) if !negated => Some(text.as_str()),
            _ => None,
        })
    }

    /// Whether any word or phrase scores names, so that [`Plan::rank`] orders
    /// results rather than scoring them all zero.
    pub fn ranks_names(&self) -> bool {
        self.filters
            .iter()
            .any(|(negated, f)| !negated && matches!(f, Filter::Word(_) | Filter::Phrase(_)))
    }

    /// Whether the query asks anything of a file's contents.
    pub fn searches_text(&self) -> bool {
        self.filters
            .iter()
            .any(|(_, f)| matches!(f, Filter::Text(_)))
    }

    /// How well `name` matches the words and phrases, or `None` if it does
    /// not. Higher is better; zero when there is nothing to match.
    ///
    /// Each word is scored on its own and the scores added, so `tax invoice`
    /// finds `invoice-tax.pdf`: the words are independent conditions, not one
    /// string that has to appear in order.
    pub fn rank(&self, name: &str) -> Option<i64> {
        let mut total = 0i64;
        for (negated, filter) in &self.filters {
            let needle = match filter {
                Filter::Word(w) => w,
                Filter::Phrase(p) => p,
                _ => continue,
            };
            let lower = name.to_lowercase();
            let hit = match filter {
                Filter::Phrase(_) => lower.contains(needle.as_str()).then_some(0),
                _ => crate::matching::score(name, needle),
            };
            match (negated, hit) {
                (false, Some(score)) => total += i64::from(score),
                (false, None) => return None,
                (true, _) if lower.contains(needle.as_str()) => return None,
                (true, _) => {}
            }
        }
        Some(total)
    }

    /// Whether a file found some other way meets every condition except
    /// `text:`, which only the finder can know.
    ///
    /// A kind the MIME type cannot settle (`document`, `app`) is judged on the
    /// type alone, which is narrower than the index's view of it.
    pub fn admits(&self, facts: &Facts<'_>) -> bool {
        self.admits_with(facts, true)
    }

    /// [`Plan::admits`] for a file whose words, rather than its name, matched
    /// the query's words: the words and phrases are taken as met.
    pub fn admits_ignoring_names(&self, facts: &Facts<'_>) -> bool {
        self.admits_with(facts, false)
    }

    /// The words and phrases a name has to contain.
    pub fn name_needles(&self) -> impl Iterator<Item = &str> {
        self.filters.iter().filter_map(|(negated, f)| match f {
            Filter::Word(text) | Filter::Phrase(text) if !negated => Some(text.as_str()),
            _ => None,
        })
    }

    fn admits_with(&self, facts: &Facts<'_>, names: bool) -> bool {
        let under = |root: &PathBuf| facts.path.starts_with(root) && facts.path != root;
        if !self.roots.iter().any(under) || self.excluded.iter().any(under) {
            return false;
        }
        let name = facts
            .path
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        if names && self.rank(&name).is_none() {
            return false;
        }
        self.filters.iter().all(|(negated, filter)| {
            let holds = match filter {
                Filter::Word(_) | Filter::Phrase(_) | Filter::Text(_) => return true,
                Filter::Kinds(kinds) => kinds.iter().any(|&k| kind_admits(k, facts, &name)),
                Filter::Modified(range) => facts.modified.is_some_and(|t| range.contains(t)),
                Filter::Size(bound) => facts.size.is_some_and(|s| match *bound {
                    SizeBound::AtLeast(b) => s >= b,
                    SizeBound::Below(b) => s < b,
                }),
            };
            holds != *negated
        })
    }

    /// Whether a row the index returned still meets the conditions the disk
    /// answers better than the index does.
    ///
    /// The index lags the disk, so a file's time and size are checked again
    /// against `facts`, which the caller reads from the filesystem. A plain
    /// `-kind:folder` is left out of the SPARQL entirely and decided here: it
    /// costs the index a join on every row, and a stat answers it for free.
    pub fn recheck(&self, facts: &Facts<'_>) -> bool {
        self.filters.iter().all(|(negated, filter)| {
            let holds = match filter {
                Filter::Kinds(kinds) if kinds == &[Kind::Folder] => facts.is_dir,
                Filter::Modified(range) => facts.modified.is_none_or(|t| range.contains(t)),
                Filter::Size(bound) => facts.size.is_none_or(|s| match *bound {
                    SizeBound::AtLeast(b) => s >= b,
                    SizeBound::Below(b) => s < b,
                }),
                _ => return true,
            };
            holds != *negated
        })
    }

    /// The SPARQL that asks the index for this plan, at most `limit` rows.
    ///
    /// Rows are candidates: pass each through [`Plan::recheck`] with what the
    /// disk says about it. They are file URLs, then a snippet when
    /// [`Plan::searches_text`], and they come back roughly best first so that a cap cuts the right end: names
    /// that contain each word outright before those that only contain its
    /// letters in order, then newest first.
    pub fn sparql(&self, limit: usize) -> Sparql {
        self.sparql_page(0, limit)
    }

    /// The rows of [`Plan::sparql`] from `offset` on, for a caller that needs
    /// more after dropping rows the index remembers but the disk no longer
    /// has.
    pub fn sparql_page(&self, offset: usize, limit: usize) -> Sparql {
        let mut q = Builder::default();
        let needs_size = self.sort == Sort::Size || self.has(|f| matches!(f, Filter::Size(_)));
        let needs_mime = self
            .filters
            .iter()
            .any(|(negated, f)| matches!(f, Filter::Kinds(_)) && !left_to_recheck(*negated, f));
        let text: Vec<&str> = self.text_needles().collect();

        // Triple patterns first; see the module docs.
        q.pattern("?f a nfo:FileDataObject ; nfo:fileName ?n ; nfo:fileLastModified ?m");
        if needs_size {
            q.pattern("?f nfo:fileSize ?s");
        }
        if !text.is_empty() {
            let fts = text
                .iter()
                .map(|t| fts_phrase(t))
                .collect::<Vec<_>>()
                .join(" ");
            let name = q.bind("text", fts);
            q.pattern(&format!("?f nie:interpretedAs ?c . ?c fts:match {name}"));
        }
        // Optional, because the index records a type only for files it has an
        // extractor for: an archive or a source file has none, and requiring
        // one would drop them from every query that mentions a kind.
        if needs_mime {
            q.pattern("OPTIONAL { ?f nie:interpretedAs ?c . ?c nie:mimeType ?mt }");
        }

        let under: Vec<String> = self
            .roots
            .iter()
            .map(|root| {
                let name = q.bind("root", folder_url(root));
                format!("STRSTARTS(STR(?f), {name})")
            })
            .collect();
        q.filter(&under.join(" || "));
        for path in &self.excluded {
            let name = q.bind("out", folder_url(path));
            q.filter(&format!("!STRSTARTS(STR(?f), {name})"));
        }

        let mut order = Vec::new();
        for (negated, filter) in &self.filters {
            if left_to_recheck(*negated, filter) {
                continue;
            }
            let expr = match filter {
                Filter::Word(word) if !negated => {
                    let pattern = q.bind("word", subsequence_pattern(word));
                    let whole = q.bind("whole", word.clone());
                    order.push(format!("DESC(CONTAINS(fn:lower-case(?n), {whole}))"));
                    format!("REGEX(?n, {pattern}, \"i\")")
                }
                Filter::Word(text) | Filter::Phrase(text) => {
                    let name = q.bind("name", text.clone());
                    format!("CONTAINS(fn:lower-case(?n), {name})")
                }
                Filter::Text(text) if *negated => {
                    let name = q.bind("text", fts_phrase(text));
                    q.filter(&format!(
                        "NOT EXISTS {{ ?f nie:interpretedAs ?x . ?x fts:match {name} }}"
                    ));
                    continue;
                }
                Filter::Text(_) => continue,
                Filter::Kinds(kinds) => {
                    let any: Vec<String> = kinds.iter().map(|&k| kind_sparql(k)).collect();
                    format!("({})", any.join(" || "))
                }
                Filter::Modified(range) => {
                    let mut parts = Vec::new();
                    if let Some(from) = range.from {
                        parts.push(format!("?m >= {}", datetime(from)));
                    }
                    if let Some(to) = range.to {
                        parts.push(format!("?m < {}", datetime(to)));
                    }
                    format!("({})", parts.join(" && "))
                }
                Filter::Size(SizeBound::AtLeast(b)) => format!("?s >= {b}"),
                Filter::Size(SizeBound::Below(b)) => format!("?s < {b}"),
            };
            if *negated {
                q.filter(&format!("!({expr})"));
            } else {
                q.filter(&expr);
            }
        }

        match self.sort {
            Sort::Size => order = vec!["DESC(?s)".to_owned()],
            Sort::Modified => order.clear(),
            Sort::Relevance | Sort::Name => {}
        }
        order.push("DESC(?m)".to_owned());

        let columns = if text.is_empty() {
            "?f".to_owned()
        } else {
            format!(
                "?f (fts:snippet(?c, \"\\u{:04X}\", \"\\u{:04X}\", \"…\", {SNIPPET_WORDS}) AS ?snippet)",
                u32::from(MARK_OPEN),
                u32::from(MARK_CLOSE)
            )
        };
        Sparql {
            text: format!(
                "SELECT DISTINCT {columns} WHERE {{ {} }} ORDER BY {} LIMIT {limit}{}",
                q.body(),
                order.join(" "),
                if offset > 0 {
                    format!(" OFFSET {offset}")
                } else {
                    String::new()
                }
            ),
            bindings: q.bindings,
            snippets: !text.is_empty(),
        }
    }

    fn has(&self, test: impl Fn(&Filter) -> bool) -> bool {
        self.filters.iter().any(|(_, f)| test(f))
    }
}

/// Whether a condition is left out of the SPARQL for [`Plan::recheck`].
fn left_to_recheck(negated: bool, filter: &Filter) -> bool {
    negated && matches!(filter, Filter::Kinds(kinds) if kinds == &[Kind::Folder])
}

/// Assembles a query body and its bindings.
#[derive(Debug, Default)]
struct Builder {
    patterns: Vec<String>,
    filters: Vec<String>,
    bindings: Vec<(String, String)>,
}

impl Builder {
    fn pattern(&mut self, pattern: &str) {
        self.patterns.push(pattern.to_owned());
    }

    fn filter(&mut self, expr: &str) {
        self.filters.push(format!("FILTER({expr})"));
    }

    /// Bind `value` under a fresh name starting with `stem`; returns `~name`.
    fn bind(&mut self, stem: &str, value: String) -> String {
        let name = format!("{stem}{}", self.bindings.len());
        let reference = format!("~{name}");
        self.bindings.push((name, value));
        reference
    }

    fn body(&self) -> String {
        let mut body = self.patterns.join(" . ");
        for filter in &self.filters {
            let _ = write!(body, " {filter}");
        }
        body
    }
}

/// Resolve an `in:` path: `~` against home, relative against the current
/// folder. `None` if it needs one of those and it is not known.
fn resolve_path(typed: &str, scope: &Scope) -> Option<PathBuf> {
    let path = if typed == "~" {
        scope.home.clone()?
    } else if let Some(rest) = typed.strip_prefix("~/") {
        scope.home.as_ref()?.join(rest)
    } else if typed.starts_with('/') {
        PathBuf::from(typed)
    } else {
        scope.cwd.as_ref()?.join(typed)
    };
    // Components, not text: `in:Documents/` and `in:Documents` are one folder.
    Some(path.components().collect())
}

/// A folder as the prefix of the `file:` URLs under it. The trailing slash is
/// what stops `~/Doc` from matching `~/Documents`.
fn folder_url(path: &Path) -> String {
    let mut url = crate::index::file_url(path);
    if !url.ends_with('/') {
        url.push('/');
    }
    url
}

/// A regular expression matching names that contain `word`'s characters in
/// order, as [`crate::matching::score`] does.
fn subsequence_pattern(word: &str) -> String {
    let mut pattern = String::new();
    for (i, c) in word.chars().enumerate() {
        if i > 0 {
            pattern.push_str(".*");
        }
        if "\\.^$|?*+()[]{}".contains(c) {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern
}

/// `text` as one quoted FTS5 string, so what someone typed is matched as
/// words and never read as FTS syntax (`NEAR`, `*`, column filters).
fn fts_phrase(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\""))
}

fn datetime(epoch: i64) -> String {
    format!("\"{}\"^^xsd:dateTime", dates::xsd_datetime(epoch))
}

/// The SPARQL condition for a kind, over the name `?n`, the extracted
/// resource `?c` and its MIME type `?mt`.
///
/// `?c` and `?mt` are unbound for a file the index has no extractor for, so
/// every test on them is guarded with `BOUND`: the condition is then plainly
/// false rather than an error, and negating it keeps the file.
fn kind_sparql(kind: Kind) -> String {
    let mime = |test: &str| format!("(BOUND(?mt) && {test})");
    let graph = |graph: &str| {
        format!(
            "(BOUND(?c) && EXISTS {{ GRAPH tracker:{graph} {{ ?c a nie:InformationElement }} }})"
        )
    };
    match kind {
        Kind::Image => mime("STRSTARTS(?mt, \"image/\")"),
        Kind::Video => mime("STRSTARTS(?mt, \"video/\")"),
        Kind::Audio => mime("STRSTARTS(?mt, \"audio/\")"),
        Kind::Text => mime("STRSTARTS(?mt, \"text/\")"),
        Kind::Pdf => mime("?mt = \"application/pdf\""),
        // Folders always carry a type, extractor or not.
        Kind::Folder => mime("?mt = \"inode/directory\""),
        // The index sorts what it extracts into content graphs; these two
        // kinds are exactly its Documents and Software graphs.
        Kind::Document => graph("Documents"),
        Kind::App => graph("Software"),
        // No extractor reads archives, so the index has no type for them:
        // the name is all there is to go on.
        Kind::Archive => format!(
            "REGEX(?n, \"\\\\.({})$\", \"i\")",
            ARCHIVE_EXTENSIONS.join("|")
        ),
    }
}

/// Whether a file is of `kind`, judged locally the way the index judges it.
fn kind_admits(kind: Kind, facts: &Facts<'_>, name: &str) -> bool {
    let (is_dir, mime) = (facts.is_dir, facts.mime.unwrap_or(""));
    match kind {
        Kind::Folder => is_dir,
        _ if is_dir => false,
        Kind::Image => mime.starts_with("image/"),
        Kind::Video => mime.starts_with("video/"),
        Kind::Audio => mime.starts_with("audio/"),
        Kind::Text => mime.starts_with("text/"),
        Kind::Pdf => mime == "application/pdf",
        Kind::Archive => {
            ARCHIVE_TYPES.contains(&mime)
                || name.rsplit_once('.').is_some_and(|(_, ext)| {
                    ARCHIVE_EXTENSIONS
                        .iter()
                        .any(|a| a.eq_ignore_ascii_case(ext))
                })
        }
        Kind::App => mime == "application/x-desktop",
        Kind::Document => {
            mime == "application/pdf"
                || mime == "application/epub+zip"
                || mime == "application/msword"
                || mime.starts_with("application/vnd.oasis.opendocument.")
                || mime.starts_with("application/vnd.openxmlformats-officedocument.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::parse;

    /// 2026-03-14 12:00:00 in a zone one hour east of UTC.
    const CLOCK: Clock = Clock {
        now: 1_773_489_600,
        utc_offset: 3600,
    };

    fn scope() -> Scope {
        Scope {
            roots: vec![PathBuf::from("/home/u")],
            home: Some(PathBuf::from("/home/u")),
            cwd: Some(PathBuf::from("/home/u/Documents")),
        }
    }

    fn plan(text: &str) -> Plan {
        Plan::new(&parse(text), &scope(), CLOCK).expect("answerable")
    }

    fn binding<'a>(sparql: &'a Sparql, stem: &str) -> Vec<&'a str> {
        sparql
            .bindings
            .iter()
            .filter(|(name, _)| name.trim_end_matches(|c: char| c.is_ascii_digit()) == stem)
            .map(|(_, value)| value.as_str())
            .collect()
    }

    /// The whole shape once, as a golden string: triples before filters,
    /// user text only through bindings.
    #[test]
    fn a_plain_word_compiles_to_a_bound_subsequence_match() {
        let sparql = plan("otfl").sparql(10);
        assert_eq!(
            sparql.text,
            "SELECT DISTINCT ?f WHERE { \
             ?f a nfo:FileDataObject ; nfo:fileName ?n ; nfo:fileLastModified ?m \
             FILTER(STRSTARTS(STR(?f), ~root0)) \
             FILTER(REGEX(?n, ~word1, \"i\")) } \
             ORDER BY DESC(CONTAINS(fn:lower-case(?n), ~whole2)) DESC(?m) LIMIT 10"
        );
        assert_eq!(binding(&sparql, "root"), vec!["file:///home/u/"]);
        assert_eq!(binding(&sparql, "word"), vec!["o.*t.*f.*l"]);
        assert_eq!(binding(&sparql, "whole"), vec!["otfl"]);
    }

    #[test]
    fn typed_text_never_reaches_the_query_text() {
        let nasty = "\"}) } DROP";
        let sparql = plan(&format!("\"{nasty}\" text:{nasty} in:/tmp/{nasty}")).sparql(10);
        assert!(!sparql.text.contains("DROP"), "{}", sparql.text);
    }

    #[test]
    fn regex_characters_in_a_word_are_literal() {
        assert_eq!(subsequence_pattern("a.b"), "a.*\\..*b");
        assert_eq!(subsequence_pattern("c++"), "c.*\\+.*\\+");
    }

    #[test]
    fn in_replaces_the_default_scope_and_resolves_paths() {
        let plan = plan("in:~/Pictures in:notes -in:/home/u/Pictures/old");
        assert_eq!(
            plan.roots(),
            [
                PathBuf::from("/home/u/Pictures"),
                PathBuf::from("/home/u/Documents/notes"),
            ]
        );
        let sparql = plan.sparql(10);
        assert_eq!(
            binding(&sparql, "out"),
            vec!["file:///home/u/Pictures/old/"]
        );
        assert!(sparql.text.contains("!STRSTARTS(STR(?f), ~out"));
    }

    #[test]
    fn nowhere_to_look_is_no_plan() {
        let empty = Scope::default();
        assert!(Plan::new(&parse("a"), &empty, CLOCK).is_none());
        assert!(Plan::new(&parse("a in:/tmp"), &empty, CLOCK).is_some());
    }

    #[test]
    fn kinds_ask_for_an_optional_mime_type_and_are_ored() {
        let sparql = plan("kind:image,pdf").sparql(10);
        assert!(sparql
            .text
            .contains("OPTIONAL { ?f nie:interpretedAs ?c . ?c nie:mimeType ?mt }"));
        assert!(sparql.text.contains(
            "((BOUND(?mt) && STRSTARTS(?mt, \"image/\")) || (BOUND(?mt) && ?mt = \"application/pdf\"))"
        ));
        // Negated, a file with no type is kept: it is not a folder.
        let sparql = plan("-kind:folder,image").sparql(10);
        assert!(sparql
            .text
            .contains("!(((BOUND(?mt) && ?mt = \"inode/directory\")"));
    }

    #[test]
    fn document_and_app_kinds_use_the_content_graphs() {
        assert!(plan("kind:document")
            .sparql(1)
            .text
            .contains("GRAPH tracker:Documents"));
        assert!(plan("kind:app")
            .sparql(1)
            .text
            .contains("GRAPH tracker:Software"));
    }

    #[test]
    fn archives_are_known_by_name() {
        let sparql = plan("kind:archive").sparql(1);
        assert!(
            sparql.text.contains("REGEX(?n, \"\\\\.(zip|tar|gz|"),
            "{}",
            sparql.text
        );
        let plan = plan("kind:archive");
        let facts = |path| Facts {
            path: Path::new(path),
            is_dir: false,
            size: None,
            modified: None,
            mime: None,
        };
        assert!(plan.admits(&facts("/home/u/a.TAR.GZ")));
        assert!(!plan.admits(&facts("/home/u/a.txt")));
    }

    #[test]
    fn text_asks_full_text_with_a_snippet() {
        let sparql = plan("text:\"tax return\" text:2024").sparql(10);
        assert!(sparql.snippets);
        assert!(sparql.text.contains("?c fts:match ~text"));
        assert!(sparql.text.contains("fts:snippet(?c"));
        assert_eq!(binding(&sparql, "text"), vec!["\"tax return\" \"2024\""]);
    }

    #[test]
    fn a_quote_inside_text_stays_inside_the_fts_string() {
        assert_eq!(fts_phrase("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn negated_text_excludes_by_its_own_match() {
        let sparql = plan("report -text:draft").sparql(10);
        assert!(!sparql.snippets);
        assert!(sparql
            .text
            .contains("NOT EXISTS { ?f nie:interpretedAs ?x . ?x fts:match ~text"));
    }

    #[test]
    fn dates_resolve_against_local_midnight() {
        // Local midnight on 2026-03-14 is 23:00 UTC the day before.
        let sparql = plan("modified:today").sparql(10);
        assert!(
            sparql
                .text
                .contains("?m >= \"2026-03-13T23:00:00Z\"^^xsd:dateTime"),
            "{}",
            sparql.text
        );

        let sparql = plan("modified:2025-02").sparql(10);
        assert!(sparql.text.contains(
            "(?m >= \"2025-01-31T23:00:00Z\"^^xsd:dateTime && ?m < \"2025-02-28T23:00:00Z\"^^xsd:dateTime)"
        ));

        let sparql = plan("modified:>1d").sparql(10);
        assert!(sparql
            .text
            .contains("(?m < \"2026-03-13T12:00:00Z\"^^xsd:dateTime)"));
    }

    #[test]
    fn size_filters_and_sorts_on_the_file_size() {
        let sparql = plan("size:>1M sort:size").sparql(10);
        assert!(sparql.text.contains("?f nfo:fileSize ?s"));
        assert!(sparql.text.contains("FILTER(?s >= 1000000)"));
        assert!(sparql.text.ends_with("ORDER BY DESC(?s) DESC(?m) LIMIT 10"));
    }

    #[test]
    fn later_pages_carry_an_offset() {
        assert!(plan("a").sparql_page(0, 10).text.ends_with("LIMIT 10"));
        assert!(plan("a")
            .sparql_page(20, 10)
            .text
            .ends_with("LIMIT 10 OFFSET 20"));
    }

    #[test]
    fn recent_is_newest_first_and_leaves_folders_to_the_stat() {
        let plan = plan("-kind:folder sort:modified");
        let sparql = plan.sparql(10);
        assert!(sparql.text.ends_with("ORDER BY DESC(?m) LIMIT 10"));
        assert!(!sparql.text.contains("?mt"), "{}", sparql.text);
        let facts = |is_dir| Facts {
            path: Path::new("/home/u/x"),
            is_dir,
            size: None,
            modified: None,
            mime: None,
        };
        assert!(plan.recheck(&facts(false)));
        assert!(!plan.recheck(&facts(true)));
    }

    #[test]
    fn recheck_trusts_the_disk_over_the_index_for_time_and_size() {
        let plan = plan("modified:today size:>1K");
        let facts = |modified, size| Facts {
            path: Path::new("/home/u/x"),
            is_dir: false,
            size: Some(size),
            modified: Some(modified),
            mime: None,
        };
        assert!(plan.recheck(&facts(CLOCK.now, 4096)));
        assert!(!plan.recheck(&facts(CLOCK.now - 2 * DAY, 4096)));
        assert!(!plan.recheck(&facts(CLOCK.now, 10)));
    }

    #[test]
    fn words_rank_independently_of_their_order() {
        let plan = plan("tax invoice");
        assert!(plan.rank("invoice-tax.pdf").is_some());
        assert!(plan.rank("invoice.pdf").is_none());
        let plan = super::tests::plan("report -draft");
        assert!(plan.rank("report.pdf").is_some());
        assert!(plan.rank("report-draft.pdf").is_none());
    }

    #[test]
    fn admits_checks_what_it_can_know_locally() {
        let plan = plan("kind:image modified:today -in:~/Pictures/old");
        let today = CLOCK.now - 60;
        let picture = |path, modified| Facts {
            path: Path::new(path),
            is_dir: false,
            size: Some(10),
            modified: Some(modified),
            mime: Some("image/png"),
        };
        assert!(plan.admits(&picture("/home/u/Pictures/a.png", today)));
        assert!(!plan.admits(&picture("/home/u/Pictures/old/a.png", today)));
        assert!(!plan.admits(&picture("/home/u/Pictures/a.png", today - 2 * DAY)));
        assert!(!plan.admits(&picture("/elsewhere/a.png", today)));
        let mut text_file = picture("/home/u/a.txt", today);
        text_file.mime = Some("text/plain");
        assert!(!plan.admits(&text_file));
    }

    #[test]
    fn admitting_by_words_skips_only_the_name() {
        let plan = plan("receipt kind:image");
        let facts = |mime| Facts {
            path: Path::new("/home/u/Pictures/IMG_0042.png"),
            is_dir: false,
            size: None,
            modified: None,
            mime: Some(mime),
        };
        assert!(!plan.admits(&facts("image/png")));
        assert!(plan.admits_ignoring_names(&facts("image/png")));
        assert!(!plan.admits_ignoring_names(&facts("text/plain")));
        assert_eq!(plan.name_needles().collect::<Vec<_>>(), vec!["receipt"]);
    }
}
