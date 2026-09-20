//! Words recognised in pictures, remembered between runs.
//!
//! One entry per file under `$XDG_CACHE_HOME/otto/ocr/`, keyed the way the
//! thumbnail cache keys its entries — see [`crate::thumbcache::key_for`] — and
//! valid while the file's modification time matches. The entry holds text and
//! boxes, not pixels, and nothing but Otto reads it, so it lives under Otto's
//! own directory rather than the freedesktop one.
//!
//! Boxes are in the coordinates of the decode they were recognised on, whose
//! size the entry records; a consumer drawing a different decode scales them
//! with [`scale_words`].
//!
//! The format is lines of tab-separated text: a header, a blank line, then
//! one word per line with its text last. Word text never contains a tab or a
//! newline, because the recogniser tokenises on both.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use otto_kit::preview::Word;

use crate::thumbcache;

const FORMAT: &str = "otto-ocr 1";

/// Entries whose file has not been touched for this long are pruned.
pub const KEEP_FOR: Duration = Duration::from_secs(90 * 24 * 60 * 60);

/// What was recognised in one picture.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub mtime: u64,
    pub languages: String,
    /// The recogniser command line the words came out of. An entry made by a
    /// different engine still selects and searches, but the background pass
    /// reads the picture again, so changing the setting improves what is
    /// remembered without throwing any of it away.
    pub recogniser: String,
    pub width: u32,
    pub height: u32,
    pub words: Vec<Word>,
}

/// What is known about the words in one picture.
///
/// Not something the cache stores — it is assembled from an entry, or from
/// the fact that a recogniser is running on the file right now — but this is
/// where the answer comes from, so it is named here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// A recogniser is on this picture now.
    Reading,
    /// This many words are remembered for the file as it stands.
    Words(usize),
    /// It has been read, and there was nothing in it to read.
    Empty,
    /// Nothing has read it yet.
    Unread,
}

#[cfg(test)]
thread_local! {
    /// Where this test's cache lives. Per thread rather than per process:
    /// `cargo test` runs the tests of one binary side by side, and a cache
    /// root in the environment would be read by whichever of them happened
    /// to be looking at the time.
    static TEST_ROOT: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Point this thread's cache at `dir` for the rest of the test.
#[cfg(test)]
fn use_root(dir: &Path) {
    TEST_ROOT.with(|root| *root.borrow_mut() = Some(dir.to_path_buf()));
}

/// `$XDG_CACHE_HOME/otto/ocr`, or `OTTO_OCR_CACHE` when a caller points it
/// elsewhere. `None` when there is no home to put it under.
fn root() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(dir) = TEST_ROOT.with(|root| root.borrow().clone()) {
        return Some(dir);
    }
    if let Some(dir) = std::env::var_os("OTTO_OCR_CACHE") {
        return Some(PathBuf::from(dir));
    }
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("otto").join("ocr"))
}

fn file_name(path: &Path, page: u32) -> String {
    let key = thumbcache::key_for(path);
    if page <= 1 {
        format!("{key}.tsv")
    } else {
        format!("{key}.p{page}.tsv")
    }
}

fn entry_path(path: &Path, page: u32) -> Option<PathBuf> {
    Some(root()?.join(file_name(path, page)))
}

fn seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The recognised words for `path` at `page`, when an entry exists and was
/// made from the file as it is now.
pub fn lookup(path: &Path, page: u32, modified: SystemTime) -> Option<Entry> {
    let text = fs::read_to_string(entry_path(path, page)?).ok()?;
    let (entry, _) = parse(&text)?;
    (entry.mtime == seconds(modified)).then_some(entry)
}

/// Whether an entry exists for `path` at `page` for the file as it is now.
/// Cheaper than [`lookup`] only in what it returns; it still reads the file.
pub fn has(path: &Path, page: u32, modified: SystemTime) -> bool {
    lookup(path, page, modified).is_some()
}

/// Whether what is remembered for `path` was made by the recogniser and the
/// languages in use now.
///
/// An entry that is merely old is still good to select and to search — this
/// is what tells the background pass to read the picture again anyway, so
/// that installing a language pack or naming a better engine improves the
/// cache over time instead of all at once.
pub fn is_current(
    path: &Path,
    page: u32,
    modified: SystemTime,
    languages: &str,
    recogniser: &str,
) -> bool {
    lookup(path, page, modified)
        .is_some_and(|entry| entry.languages == languages && entry.recogniser == recogniser)
}

/// Remember what was recognised. Written whole to a temporary file and
/// renamed into place, so a reader never sees half an entry.
pub fn store(
    path: &Path,
    page: u32,
    modified: SystemTime,
    languages: &str,
    recogniser: &str,
    size: (u32, u32),
    words: &[Word],
) -> std::io::Result<()> {
    let Some(dir) = root() else {
        return Err(std::io::Error::other("no cache directory"));
    };
    fs::create_dir_all(&dir)?;
    set_mode(&dir, 0o700);
    let entry = Entry {
        mtime: seconds(modified),
        languages: languages.to_string(),
        recogniser: recogniser.to_string(),
        width: size.0,
        height: size.1,
        words: words.to_vec(),
    };
    let text = serialise(&thumbcache::uri_for(path), &entry);
    let temp = dir.join(format!(".{}.{}", file_name(path, page), std::process::id()));
    {
        let mut file = fs::File::create(&temp)?;
        set_mode(&temp, 0o600);
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    let result = fs::rename(&temp, dir.join(file_name(path, page)));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
}

/// Files whose recognised text contains `query`, ignoring case, that still
/// exist. Entries whose file has been replaced since are skipped: the words
/// in them are of a picture that is gone.
pub fn matches(query: &str) -> Vec<PathBuf> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let Some(dir) = root() else {
        return Vec::new();
    };
    let Ok(listing) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for item in listing.flatten() {
        let name = item.file_name();
        let name = name.to_string_lossy();
        if !name.ends_with(".tsv") || name.starts_with('.') {
            continue;
        }
        let Ok(text) = fs::read_to_string(item.path()) else {
            continue;
        };
        let Some((entry, uri)) = parse(&text) else {
            continue;
        };
        if !contains_words(&entry.words, &needle) {
            continue;
        }
        let Some(path) = otto_peek::uri::uri_to_path(&uri) else {
            continue;
        };
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        let current = meta.modified().map(seconds).unwrap_or(0);
        if current != entry.mtime {
            continue;
        }
        if !found.contains(&path) {
            found.push(path);
        }
    }
    found
}

/// Whether the words, read as one text, contain `needle` (already lowercase).
fn contains_words(words: &[Word], needle: &str) -> bool {
    let mut text = String::with_capacity(words.len() * 6);
    for word in words {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&word.text.to_lowercase());
    }
    text.contains(needle)
}

/// Drop entries whose picture was last modified longer ago than `older_than`.
pub fn prune(older_than: Duration) {
    let Some(dir) = root() else {
        return;
    };
    let Ok(listing) = fs::read_dir(&dir) else {
        return;
    };
    let cutoff = seconds(SystemTime::now()).saturating_sub(older_than.as_secs());
    for item in listing.flatten() {
        // Entries only. The dotted names are the temporary files [`store`]
        // renames into place, and one of them may be half-written by another
        // process this moment; removing it would lose that entry.
        let name = item.file_name();
        let name = name.to_string_lossy();
        if !name.ends_with(".tsv") || name.starts_with('.') {
            continue;
        }
        let Ok(text) = fs::read_to_string(item.path()) else {
            continue;
        };
        match parse(&text) {
            Some((entry, _)) if entry.mtime < cutoff => {
                let _ = fs::remove_file(item.path());
            }
            Some(_) => {}
            None => {
                let _ = fs::remove_file(item.path());
            }
        }
    }
}

/// Words recognised on a `from`-sized decode, re-expressed for a `to`-sized
/// one of the same picture.
pub fn scale_words(words: &[Word], from: (u32, u32), to: (u32, u32)) -> Vec<Word> {
    if from == to || from.0 == 0 || from.1 == 0 {
        return words.to_vec();
    }
    let sx = to.0 as f32 / from.0 as f32;
    let sy = to.1 as f32 / from.1 as f32;
    words
        .iter()
        .map(|word| Word {
            text: word.text.clone(),
            left: (word.left as f32 * sx).round() as u32,
            top: (word.top as f32 * sy).round() as u32,
            width: ((word.width as f32 * sx).round() as u32).max(1),
            height: ((word.height as f32 * sy).round() as u32).max(1),
            confidence: word.confidence,
            block: word.block,
            paragraph: word.paragraph,
            line: word.line,
        })
        .collect()
}

fn serialise(uri: &str, entry: &Entry) -> String {
    let mut out = String::new();
    out.push_str(FORMAT);
    out.push('\n');
    out.push_str(&format!("uri\t{uri}\n"));
    out.push_str(&format!("mtime\t{}\n", entry.mtime));
    out.push_str(&format!("languages\t{}\n", entry.languages));
    out.push_str(&format!("recogniser\t{}\n", entry.recogniser));
    out.push_str(&format!("size\t{}\t{}\n", entry.width, entry.height));
    out.push('\n');
    for word in &entry.words {
        let text: String = word
            .text
            .chars()
            .filter(|c| *c != '\t' && *c != '\n' && *c != '\r')
            .collect();
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            word.left,
            word.top,
            word.width,
            word.height,
            word.confidence,
            word.block,
            word.paragraph,
            word.line,
            text
        ));
    }
    out
}

/// An entry and the URI it was made for, or `None` for anything that is not
/// an entry in this format.
fn parse(text: &str) -> Option<(Entry, String)> {
    let mut lines = text.lines();
    if lines.next()? != FORMAT {
        return None;
    }
    let mut uri = None;
    let mut entry = Entry {
        mtime: 0,
        languages: String::new(),
        recogniser: String::new(),
        width: 0,
        height: 0,
        words: Vec::new(),
    };
    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
        let mut fields = line.split('\t');
        match fields.next()? {
            "uri" => uri = Some(fields.next()?.to_string()),
            "mtime" => entry.mtime = fields.next()?.parse().ok()?,
            "languages" => entry.languages = fields.next().unwrap_or("").to_string(),
            // Absent in entries written before the engine was recorded; they
            // read as having come from an unknown one, so the pass replaces
            // them once and every entry afterwards says what made it.
            "recogniser" => entry.recogniser = fields.next().unwrap_or("").to_string(),
            "size" => {
                entry.width = fields.next()?.parse().ok()?;
                entry.height = fields.next()?.parse().ok()?;
            }
            _ => {}
        }
    }
    for line in lines {
        let fields: Vec<&str> = line.splitn(9, '\t').collect();
        if fields.len() != 9 {
            continue;
        }
        let number = |index: usize| fields[index].parse::<u32>().ok();
        let word = Word {
            left: number(0)?,
            top: number(1)?,
            width: number(2)?,
            height: number(3)?,
            confidence: number(4)?.min(100) as u8,
            block: number(5)?,
            paragraph: number(6)?,
            line: number(7)?,
            text: fields[8].to_string(),
        };
        entry.words.push(word);
    }
    Some((entry, uri?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, left: u32) -> Word {
        Word {
            text: text.into(),
            left,
            top: 10,
            width: 30,
            height: 12,
            confidence: 90,
            block: 1,
            paragraph: 1,
            line: 1,
        }
    }

    /// A cache of this thread's own, so tests running side by side in one
    /// binary cannot read each other's entries.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "otto-ocrcache-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        use_root(&dir);
        dir
    }

    #[test]
    fn an_entry_round_trips_and_is_found_by_its_words() {
        let dir = scratch("roundtrip");
        let picture = dir.join("sign.png");
        fs::write(&picture, b"not really a png").unwrap();
        let modified = fs::metadata(&picture).unwrap().modified().unwrap();

        let words = vec![word("Hello", 21), word("Otto", 82)];
        store(
            &picture,
            1,
            modified,
            "eng",
            "tesseract",
            (400, 120),
            &words,
        )
        .unwrap();

        let entry = lookup(&picture, 1, modified).expect("stored");
        assert_eq!(entry.words, words);
        assert_eq!((entry.width, entry.height), (400, 120));
        assert_eq!(entry.languages, "eng");
        assert_eq!(entry.recogniser, "tesseract");

        assert_eq!(matches("otto"), vec![picture.clone()]);
        assert_eq!(matches("hello otto"), vec![picture.clone()]);
        assert!(matches("goodbye").is_empty());

        // A different mtime is a different picture.
        assert!(lookup(&picture, 1, modified + Duration::from_secs(5)).is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    /// Installing a language pack, or naming a better engine, does not
    /// invalidate what is remembered — but the pass is told to read the
    /// picture again, which is how the cache catches up.
    #[test]
    fn a_different_engine_or_language_is_still_read_but_not_current() {
        let dir = scratch("current");
        let picture = dir.join("sign.png");
        fs::write(&picture, b"not really a png").unwrap();
        let modified = fs::metadata(&picture).unwrap().modified().unwrap();
        store(
            &picture,
            1,
            modified,
            "eng",
            "tesseract",
            (400, 120),
            &[word("Hello", 21)],
        )
        .unwrap();

        assert!(is_current(&picture, 1, modified, "eng", "tesseract"));
        assert!(!is_current(&picture, 1, modified, "ita+eng", "tesseract"));
        assert!(!is_current(&picture, 1, modified, "eng", "ocrmypdf"));
        // Still good to select and to search in the meantime.
        assert!(has(&picture, 1, modified));
        assert_eq!(matches("hello"), vec![picture]);

        let _ = fs::remove_dir_all(&dir);
    }

    /// Pruning drops what is stale and leaves alone the half-written
    /// temporary file another process is renaming into place.
    #[test]
    fn pruning_spares_a_temporary_file() {
        let dir = scratch("prune");
        let picture = dir.join("sign.png");
        fs::write(&picture, b"not really a png").unwrap();
        // A picture last touched in 1970: older than any cutoff.
        let ancient = UNIX_EPOCH + Duration::from_secs(1_000);
        store(
            &picture,
            1,
            ancient,
            "eng",
            "tesseract",
            (400, 120),
            &[word("Hello", 21)],
        )
        .unwrap();
        let temp = dir.join(".deadbeef.tsv.1234");
        fs::write(&temp, b"half an entry").unwrap();

        prune(KEEP_FOR);
        assert!(!has(&picture, 1, ancient), "the entry was pruned");
        assert!(temp.exists(), "another process is still writing this");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn words_scale_with_the_decode() {
        let scaled = scale_words(&[word("a", 100)], (400, 120), (200, 60));
        assert_eq!(
            (
                scaled[0].left,
                scaled[0].top,
                scaled[0].width,
                scaled[0].height
            ),
            (50, 5, 15, 6)
        );
    }

    #[test]
    fn garbage_is_not_an_entry() {
        assert!(parse("").is_none());
        assert!(parse("otto-ocr 1\nmtime\tx\n").is_none());
    }
}
