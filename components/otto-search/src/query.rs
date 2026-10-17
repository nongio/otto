//! The query language: text in, structured query out.
//!
//! Everything that accepts a search accepts this syntax: the Files strip, the
//! launcher, the command line, D-Bus and the agents. The grammar is specified
//! in `specs/search-language.md`; this module is the one implementation.
//!
//! Parsing never fails. A token that does not parse as what it claims to be
//! stays in the query as a plain word and carries an [`Issue`], so the caller
//! can hint at it. Nothing typed is dropped silently.
//!
//! Every term keeps the byte range it came from, so a caller that shows terms
//! as chips can turn them back into exactly the text that was typed.

// Rust guideline compliant 2026-02-21

use std::fmt;
use std::ops::Range;

/// A parsed query: its terms, in the order they were typed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Query {
    pub terms: Vec<Term>,
}

/// One token of a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Term {
    /// Where the token sits in the source text, in bytes.
    pub span: Range<usize>,
    /// Written with a leading `-`: the term excludes rather than requires.
    pub negated: bool,
    pub kind: TermKind,
    /// Why the token was not read as what it looked like, if it was not.
    /// Such a term is always a [`TermKind::Word`] holding the raw token.
    pub issue: Option<Issue>,
}

/// What a term asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermKind {
    /// The name contains these characters in order: `otfl` finds
    /// `otto-files.rs`.
    Word(String),
    /// The name contains this exact text: `"tax return"`.
    Phrase(String),
    /// The contents contain this word or phrase: `text:ricevuta`.
    Text(String),
    /// The file is one of these kinds: `kind:image,video`.
    Kind(Vec<Kind>),
    /// The file is under this folder, as typed: `in:~/Documents`. Resolved
    /// against home and the current folder when the query is compiled.
    In(String),
    /// When the file was last written: `modified:<7d`.
    Modified(When),
    /// How large the file is: `size:>100M`.
    Size(SizeBound),
    /// How to order the results: `sort:size`.
    Sort(Sort),
}

/// A kind of file, as `kind:` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Image,
    Video,
    Audio,
    Document,
    Pdf,
    Text,
    Folder,
    App,
    Archive,
}

impl Kind {
    /// Every kind, in the order a picker lists them.
    pub const ALL: [Kind; 9] = [
        Kind::Document,
        Kind::Pdf,
        Kind::Image,
        Kind::Video,
        Kind::Audio,
        Kind::Text,
        Kind::Archive,
        Kind::App,
        Kind::Folder,
    ];

    /// The canonical name, as a query writes it.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Image => "image",
            Kind::Video => "video",
            Kind::Audio => "audio",
            Kind::Document => "document",
            Kind::Pdf => "pdf",
            Kind::Text => "text",
            Kind::Folder => "folder",
            Kind::App => "app",
            Kind::Archive => "archive",
        }
    }

    fn from_name(name: &str) -> Option<Kind> {
        Some(match name {
            "image" | "images" | "picture" | "pictures" | "photo" | "photos" => Kind::Image,
            "video" | "videos" | "movie" | "movies" => Kind::Video,
            "audio" | "music" | "sound" | "song" | "songs" => Kind::Audio,
            "document" | "documents" | "doc" | "docs" => Kind::Document,
            "pdf" => Kind::Pdf,
            "text" => Kind::Text,
            "folder" | "folders" | "dir" | "directory" => Kind::Folder,
            "app" | "apps" | "application" | "applications" => Kind::App,
            "archive" | "archives" | "zip" => Kind::Archive,
            _ => return None,
        })
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A span of time for `modified:`, before it is pinned to a clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// Since midnight, local time.
    Today,
    /// The whole of the day before today, local time.
    Yesterday,
    /// Within the last this many seconds: `<7d`, `week`.
    Within(u64),
    /// Longer ago than this many seconds: `>1y`.
    OlderThan(u64),
    /// During a calendar year, month or day, local time: `2025-03`.
    During(Civil),
    /// Before a calendar period starts: `<2025`.
    Before(Civil),
    /// After a calendar period ends: `>2025-03`.
    After(Civil),
}

/// A calendar year, month or day. `month` and `day` narrow it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Civil {
    pub year: i32,
    pub month: Option<u32>,
    pub day: Option<u32>,
}

/// A size bound for `size:`, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeBound {
    /// At least this large: `>100M`, or a bare `100M`.
    AtLeast(u64),
    /// Smaller than this: `<1K`.
    Below(u64),
}

/// How results are ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    /// Best name match first; newest first when there is nothing to match.
    #[default]
    Relevance,
    /// Newest first.
    Modified,
    /// Largest first.
    Size,
    /// A to Z.
    Name,
}

impl Sort {
    fn from_name(name: &str) -> Option<Sort> {
        Some(match name {
            "relevance" | "best" => Sort::Relevance,
            "modified" | "date" | "time" | "recent" | "newest" => Sort::Modified,
            "size" | "largest" => Sort::Size,
            "name" => Sort::Name,
            _ => return None,
        })
    }
}

/// Why a token was kept as a plain word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Issue {
    /// `foo:bar` where `foo` is not a key the language has.
    UnknownKey(String),
    /// A known key with nothing after the colon, as while typing `kind:`.
    MissingValue(String),
    /// A known key whose value does not parse: `kind:blob`, `size:big`.
    BadValue { key: String, value: String },
    /// A leading `-` on something that cannot be excluded, such as `sort:`.
    CannotNegate(String),
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Issue::UnknownKey(key) => write!(f, "no filter is called {key}:"),
            Issue::MissingValue(key) => write!(f, "{key}: needs a value"),
            Issue::BadValue { key, value } => write!(f, "{key}: does not understand {value}"),
            Issue::CannotNegate(key) => write!(f, "{key}: cannot be excluded"),
        }
    }
}

impl std::error::Error for Issue {}

impl Query {
    /// Whether nothing was typed that asks for anything.
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// The order the query asks for: the last `sort:` wins.
    pub fn sort(&self) -> Sort {
        self.terms
            .iter()
            .rev()
            .find_map(|term| match term.kind {
                TermKind::Sort(sort) => Some(sort),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// The words and phrases a name has to match, for ranking.
    pub fn name_needles(&self) -> impl Iterator<Item = &str> {
        self.terms
            .iter()
            .filter(|t| !t.negated)
            .filter_map(|t| match &t.kind {
                TermKind::Word(word) => Some(word.as_str()),
                TermKind::Phrase(phrase) => Some(phrase.as_str()),
                _ => None,
            })
    }

    /// The words and phrases the contents have to contain.
    pub fn text_needles(&self) -> impl Iterator<Item = &str> {
        self.terms
            .iter()
            .filter(|t| !t.negated)
            .filter_map(|t| match &t.kind {
                TermKind::Text(text) => Some(text.as_str()),
                _ => None,
            })
    }

    /// The problems found while parsing, with the span each belongs to.
    pub fn issues(&self) -> impl Iterator<Item = (&Range<usize>, &Issue)> {
        self.terms
            .iter()
            .filter_map(|t| t.issue.as_ref().map(|issue| (&t.span, issue)))
    }
}

/// Parse `text` into a query. Never fails; see the module docs.
pub fn parse(text: &str) -> Query {
    let terms = tokens(text).map(|span| term(text, span)).collect();
    Query { terms }
}

/// The byte ranges of the whitespace-separated tokens in `text`. A double
/// quote runs to the next double quote, spaces and all; one left open runs to
/// the end, so a phrase being typed is one token.
fn tokens(text: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    let mut chars = text.char_indices().peekable();
    std::iter::from_fn(move || {
        while chars.next_if(|(_, c)| c.is_whitespace()).is_some() {}
        let (start, _) = *chars.peek()?;
        let mut quoted = false;
        let mut end = text.len();
        while let Some(&(at, c)) = chars.peek() {
            if c.is_whitespace() && !quoted {
                end = at;
                break;
            }
            if c == '"' {
                quoted = !quoted;
            }
            chars.next();
        }
        Some(start..end)
    })
}

/// Read one token.
fn term(text: &str, span: Range<usize>) -> Term {
    let raw = &text[span.clone()];
    let (negated, body) = match raw.strip_prefix('-') {
        Some(rest) if !rest.is_empty() => (true, rest),
        _ => (false, raw),
    };

    let (kind, issue) = match split_key(body) {
        Some((key, value)) => keyed(&key, value),
        None if body.starts_with('"') => (TermKind::Phrase(unquote(body).to_owned()), None),
        None => (TermKind::Word(body.to_owned()), None),
    };

    // A token that did not parse is a word, and a word keeps the whole token
    // it was typed as, minus only the negation.
    let (kind, issue) = match (kind, issue) {
        (TermKind::Sort(_), None) if negated => (
            TermKind::Word(body.to_owned()),
            Some(Issue::CannotNegate("sort".to_owned())),
        ),
        (_, Some(issue)) => (TermKind::Word(body.to_owned()), Some(issue)),
        (kind, None) => (kind, None),
    };

    Term {
        span,
        negated,
        kind,
        issue,
    }
}

/// Split `key:value`, or `None` if the token is not shaped like a filter.
///
/// Only letters before the colon make a key. `12:30`, `C:\`, `http://…` and a
/// name with a colon later in it are words, and do not earn a hint.
fn split_key(token: &str) -> Option<(String, &str)> {
    let (key, value) = token.split_once(':')?;
    if key.is_empty() || !key.chars().all(char::is_alphabetic) || value.starts_with("//") {
        return None;
    }
    Some((key.to_lowercase(), value))
}

/// Read the value of a known key, or say why it cannot be read.
fn keyed(key: &str, value: &str) -> (TermKind, Option<Issue>) {
    let value = unquote(value);
    let known = matches!(key, "text" | "kind" | "in" | "modified" | "size" | "sort");
    if !known {
        return (
            TermKind::Word(String::new()),
            Some(Issue::UnknownKey(key.to_owned())),
        );
    }
    if value.is_empty() {
        return (
            TermKind::Word(String::new()),
            Some(Issue::MissingValue(key.to_owned())),
        );
    }
    let bad = || {
        Some(Issue::BadValue {
            key: key.to_owned(),
            value: value.to_owned(),
        })
    };
    let lower = value.to_lowercase();
    let kind = match key {
        "text" => Some(TermKind::Text(value.to_owned())),
        "in" => Some(TermKind::In(value.to_owned())),
        "kind" => lower
            .split(',')
            .filter(|name| !name.is_empty())
            .map(Kind::from_name)
            .collect::<Option<Vec<_>>>()
            .filter(|kinds| !kinds.is_empty())
            .map(TermKind::Kind),
        "modified" => when(&lower).map(TermKind::Modified),
        "size" => size(&lower).map(TermKind::Size),
        "sort" => Sort::from_name(&lower).map(TermKind::Sort),
        _ => None,
    };
    match kind {
        Some(kind) => (kind, None),
        None => (TermKind::Word(String::new()), bad()),
    }
}

/// The text inside double quotes, or `text` itself if it is not quoted. An
/// unclosed quote is read to the end.
fn unquote(text: &str) -> &str {
    match text.strip_prefix('"') {
        Some(rest) => rest.strip_suffix('"').unwrap_or(rest),
        None => text,
    }
}

const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;
/// Relative months and years are fixed lengths, not calendar ones: `<1m`
/// means "about the last month", and pinning it to calendar arithmetic would
/// make it mean a different number of days depending on the date.
const MONTH: u64 = 30 * DAY;
const YEAR: u64 = 365 * DAY;

/// Read a `modified:` value.
fn when(value: &str) -> Option<When> {
    match value {
        "today" => return Some(When::Today),
        "yesterday" => return Some(When::Yesterday),
        "week" => return Some(When::Within(7 * DAY)),
        "month" => return Some(When::Within(MONTH)),
        "year" => return Some(When::Within(YEAR)),
        _ => {}
    }
    let (op, rest) = match value.as_bytes().first()? {
        b'<' => (Some('<'), &value[1..]),
        b'>' => (Some('>'), &value[1..]),
        _ => (None, value),
    };
    if let Some(civil) = civil(rest) {
        return Some(match op {
            Some('<') => When::Before(civil),
            Some('>') => When::After(civil),
            _ => When::During(civil),
        });
    }
    let seconds = duration(rest)?;
    match op {
        Some('>') => Some(When::OlderThan(seconds)),
        // A bare `7d` reads as "in the last seven days": nobody means
        // "exactly seven days ago to the second".
        _ => Some(When::Within(seconds)),
    }
}

/// Read `2025`, `2025-03` or `2025-03-14`.
fn civil(value: &str) -> Option<Civil> {
    let mut parts = value.split('-');
    let year = parts.next()?;
    if year.len() != 4 {
        return None;
    }
    let year: i32 = year.parse().ok()?;
    let month = parts.next().map(str::parse::<u32>).transpose().ok()?;
    let day = parts.next().map(str::parse::<u32>).transpose().ok()?;
    if parts.next().is_some() {
        return None;
    }
    if month.is_some_and(|m| !(1..=12).contains(&m)) {
        return None;
    }
    if let (Some(m), Some(d)) = (month, day) {
        if d == 0 || d > crate::dates::days_in_month(year, m) {
            return None;
        }
    }
    Some(Civil { year, month, day })
}

/// Read `7d`, `3h`, `2w`, `6m`, `1y` as seconds.
fn duration(value: &str) -> Option<u64> {
    let unit_at = value.find(|c: char| !c.is_ascii_digit())?;
    let count: u64 = value[..unit_at].parse().ok()?;
    let unit = match &value[unit_at..] {
        "h" => HOUR,
        "d" => DAY,
        "w" => 7 * DAY,
        "m" => MONTH,
        "y" => YEAR,
        _ => return None,
    };
    count.checked_mul(unit)
}

/// Read a `size:` value.
///
/// Units are powers of 1000, matching how Files shows sizes: a file listed as
/// "1 MB" holds at least a million bytes and is found by `size:>1M`. The
/// binary units (`KiB`, `MiB`, …) count in powers of 1024.
fn size(value: &str) -> Option<SizeBound> {
    let (below, rest) = match value.as_bytes().first()? {
        b'<' => (true, &value[1..]),
        b'>' => (false, &value[1..]),
        _ => (false, value),
    };
    let number_end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(rest.len());
    let number: f64 = rest[..number_end].parse().ok()?;
    let unit = &rest[number_end..];
    let (unit, base) = match unit.strip_suffix("ib") {
        Some(unit) => (unit, 1024u64),
        None => (unit.strip_suffix('b').unwrap_or(unit), 1000),
    };
    let power = match unit {
        "" if base == 1000 => 0,
        "k" => 1,
        "m" => 2,
        "g" => 3,
        "t" => 4,
        _ => return None,
    };
    let scale = base.pow(power);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "a size typed by hand is far below where f64 loses whole bytes"
    )]
    let bytes = (number * scale as f64).round() as u64;
    Some(if below {
        SizeBound::Below(bytes)
    } else {
        SizeBound::AtLeast(bytes)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<TermKind> {
        parse(text).terms.into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn plain_words_are_words() {
        assert_eq!(
            kinds("invoice  tax"),
            vec![
                TermKind::Word("invoice".into()),
                TermKind::Word("tax".into())
            ]
        );
    }

    #[test]
    fn a_quoted_phrase_is_one_term_with_its_spaces() {
        assert_eq!(
            kinds("\"tax return\" 2024"),
            vec![
                TermKind::Phrase("tax return".into()),
                TermKind::Word("2024".into()),
            ]
        );
    }

    #[test]
    fn an_unclosed_quote_runs_to_the_end() {
        assert_eq!(kinds("\"tax ret"), vec![TermKind::Phrase("tax ret".into())]);
    }

    #[test]
    fn filters_read_their_values() {
        assert_eq!(
            kinds("kind:image,VIDEO text:\"tax return\" in:~/Documents sort:size"),
            vec![
                TermKind::Kind(vec![Kind::Image, Kind::Video]),
                TermKind::Text("tax return".into()),
                TermKind::In("~/Documents".into()),
                TermKind::Sort(Sort::Size),
            ]
        );
    }

    #[test]
    fn a_leading_dash_excludes() {
        let query = parse("-kind:image -draft");
        assert!(query.terms.iter().all(|t| t.negated));
        assert_eq!(query.terms[1].kind, TermKind::Word("draft".into()));
    }

    #[test]
    fn a_lone_dash_is_a_word() {
        let query = parse("-");
        assert!(!query.terms[0].negated);
        assert_eq!(query.terms[0].kind, TermKind::Word("-".into()));
    }

    #[test]
    fn spans_give_back_the_typed_text() {
        let text = "  invoice -kind:pdf  \"a b\" ";
        let query = parse(text);
        let pieces: Vec<&str> = query.terms.iter().map(|t| &text[t.span.clone()]).collect();
        assert_eq!(pieces, vec!["invoice", "-kind:pdf", "\"a b\""]);
    }

    #[test]
    fn an_unknown_key_stays_as_text_with_a_hint() {
        let query = parse("foo:bar");
        assert_eq!(query.terms[0].kind, TermKind::Word("foo:bar".into()));
        assert_eq!(query.terms[0].issue, Some(Issue::UnknownKey("foo".into())));
    }

    #[test]
    fn a_bad_value_stays_as_text_with_a_hint() {
        let query = parse("kind:blob size:big kind:");
        assert!(query
            .terms
            .iter()
            .all(|t| matches!(t.kind, TermKind::Word(_))));
        assert!(matches!(query.terms[0].issue, Some(Issue::BadValue { .. })));
        assert!(matches!(query.terms[1].issue, Some(Issue::BadValue { .. })));
        assert_eq!(
            query.terms[2].issue,
            Some(Issue::MissingValue("kind".into()))
        );
    }

    #[test]
    fn colons_that_are_not_filters_earn_no_hint() {
        for text in ["12:30", "http://example.com", "a1:b"] {
            let query = parse(text);
            assert_eq!(query.terms[0].kind, TermKind::Word(text.into()), "{text}");
            assert_eq!(query.terms[0].issue, None, "{text}");
        }
    }

    #[test]
    fn sort_cannot_be_negated() {
        let query = parse("-sort:size");
        assert_eq!(
            query.terms[0].issue,
            Some(Issue::CannotNegate("sort".into()))
        );
        assert_eq!(query.sort(), Sort::Relevance);
    }

    #[test]
    fn modified_reads_keywords_durations_and_dates() {
        let cases = [
            ("today", When::Today),
            ("yesterday", When::Yesterday),
            ("week", When::Within(7 * DAY)),
            ("<7d", When::Within(7 * DAY)),
            ("3h", When::Within(3 * HOUR)),
            (">1y", When::OlderThan(YEAR)),
            (
                "2025",
                When::During(Civil {
                    year: 2025,
                    month: None,
                    day: None,
                }),
            ),
            (
                "<2025-03",
                When::Before(Civil {
                    year: 2025,
                    month: Some(3),
                    day: None,
                }),
            ),
            (
                ">2025-03-14",
                When::After(Civil {
                    year: 2025,
                    month: Some(3),
                    day: Some(14),
                }),
            ),
        ];
        for (value, expected) in cases {
            assert_eq!(when(value), Some(expected), "{value}");
        }
        for value in ["2025-13", "2025-02-30", "soon", "7x", "25"] {
            assert_eq!(when(value), None, "{value}");
        }
    }

    #[test]
    fn size_counts_in_powers_of_1000_like_files() {
        assert_eq!(size(">100m"), Some(SizeBound::AtLeast(100_000_000)));
        assert_eq!(size("<1k"), Some(SizeBound::Below(1000)));
        assert_eq!(size("1.5gb"), Some(SizeBound::AtLeast(1_500_000_000)));
        assert_eq!(size("10"), Some(SizeBound::AtLeast(10)));
        assert_eq!(size("10b"), Some(SizeBound::AtLeast(10)));
        assert_eq!(size("big"), None);
        assert_eq!(size("1ib"), None);
    }

    #[test]
    fn binary_units_count_in_powers_of_1024() {
        assert_eq!(size("2mib"), Some(SizeBound::AtLeast(2 << 20)));
        assert_eq!(size("<1kib"), Some(SizeBound::Below(1024)));
    }

    #[test]
    fn the_last_sort_wins() {
        assert_eq!(parse("sort:size a sort:name").sort(), Sort::Name);
        assert_eq!(parse("a").sort(), Sort::Relevance);
    }

    #[test]
    fn needles_skip_exclusions() {
        let query = parse("a -b \"c d\" text:e -text:f");
        assert_eq!(query.name_needles().collect::<Vec<_>>(), vec!["a", "c d"]);
        assert_eq!(query.text_needles().collect::<Vec<_>>(), vec!["e"]);
    }
}
