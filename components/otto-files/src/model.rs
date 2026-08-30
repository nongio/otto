//! The directory model: entries, places, sorting.
//!
//! Every filesystem call in here runs on a worker thread — see [`Directory`].
//! The UI thread only ever reads a finished [`Snapshot`]. See
//! `specs/file-browser.md` under *Async I/O*.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use otto_kit::components::scroll::ScrollView;
use otto_kit::filetype::{self, Kind};
use skia_safe::Rect;

/// One entry in a directory.
///
/// `size` and `modified` are `None` until the metadata pass fills them in.
/// The initial listing uses only what `readdir` returns, because 10,000 files
/// is 10,000 `stat` calls before anything can be drawn.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub hidden: bool,
    pub kind: Kind,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
}

impl Entry {
    /// The Kind column's text.
    pub fn kind_label(&self) -> &'static str {
        if self.is_dir {
            otto_kit::t!("files-kind-folder")
        } else {
            self.kind.label()
        }
    }

    /// Icon names to try, most specific first, so a sparse theme still resolves.
    pub fn icon_chain(&self) -> Vec<String> {
        if self.is_dir {
            return vec!["folder".to_string(), "inode-directory".to_string()];
        }
        match filetype::mime_for_name(&self.name) {
            Some(mime) => filetype::icon_names(mime),
            None => vec![self.kind.generic_icon().to_string()],
        }
    }
}

// ---------------------------------------------------------------------------
// Sorting
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Kind,
    Modified,
}

impl SortKey {
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => otto_kit::t!("files-column-name"),
            SortKey::Size => otto_kit::t!("files-column-size"),
            SortKey::Kind => otto_kit::t!("files-column-kind"),
            SortKey::Modified => otto_kit::t!("files-column-date-modified"),
        }
    }
}

/// Compare two names the way a person reads them: digit runs compare as
/// numbers, so `file2` precedes `file10`, and case is ignored.
///
/// No locale collation — a known limitation, taken deliberately rather than
/// depending on a collation crate. See `specs/file-picker.md` under *Sorting*.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();

    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ac), Some(bc)) => {
                if ac.is_ascii_digit() && bc.is_ascii_digit() {
                    // Take the whole digit run from each side and compare it as
                    // a number, ignoring leading zeros.
                    let an = take_digits(&mut ai);
                    let bn = take_digits(&mut bi);
                    let atrim = an.trim_start_matches('0');
                    let btrim = bn.trim_start_matches('0');
                    let ord = atrim
                        .len()
                        .cmp(&btrim.len())
                        .then_with(|| atrim.cmp(btrim))
                        // Equal values: more leading zeros sorts first, so the
                        // ordering stays total rather than arbitrary.
                        .then_with(|| an.len().cmp(&bn.len()));
                    if ord != Ordering::Equal {
                        return ord;
                    }
                } else {
                    let al = ac.to_ascii_lowercase();
                    let bl = bc.to_ascii_lowercase();
                    if al != bl {
                        return al.cmp(&bl);
                    }
                    ai.next();
                    bi.next();
                }
            }
        }
    }
}

fn take_digits(it: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut s = String::new();
    while let Some(c) = it.peek().copied() {
        if c.is_ascii_digit() {
            s.push(c);
            it.next();
        } else {
            break;
        }
    }
    s
}

// ---------------------------------------------------------------------------
// Reading, off the UI thread
// ---------------------------------------------------------------------------

/// A finished read of one directory.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub path: PathBuf,
    pub entries: Vec<Entry>,
    /// Set when the directory could not be read at all — shown in place of the
    /// listing, rather than as an empty directory.
    pub error: Option<String>,
}

/// One directory in the path stack: its contents, what is selected in it, and
/// where it is scrolled.
///
/// The stack is what Miller columns render side by side and what the list view
/// renders the last element of. Navigation pushes and pops it in both views, so
/// switching between them preserves where the user is.
pub struct Column {
    pub path: PathBuf,
    pub snapshot: Snapshot,
    pub loader: Directory,
    /// The selection, held as entry **names** rather than indices.
    ///
    /// Indices into the filtered, sorted list are only meaningful until
    /// something re-sorts, re-filters or reloads it — and all three happen
    /// while a selection is live. Names survive every one of those, which is
    /// what makes a multi-selection stable when a file appears in the
    /// directory underneath it.
    pub selection: std::collections::BTreeSet<String>,
    /// Where the keyboard is, as an index into the current visible list.
    pub cursor: Option<usize>,
    /// Where a range selection extends from.
    pub anchor: Option<usize>,
    /// This pane's scrolling, momentum, overscroll and scrollbar — the shared
    /// otto-kit scroll view, the same one the settings app drives. Its
    /// viewport and content height are re-set every frame by the view, since
    /// both change with the window size and the listing.
    pub scroll: ScrollView,
    /// Bumped whenever [`Self::snapshot`] is replaced, so a cached order can
    /// tell whether it was computed from the listing that is there now.
    pub epoch: u64,
    /// The filtered, sorted order of `snapshot`, recomputed only when
    /// something it depends on moves. See [`SortCache`].
    pub sorted: std::cell::RefCell<SortCache>,
    /// This directory's inotify watch. Dropped with the column, which is what
    /// keeps the watch set equal to what is on screen.
    watch: crate::watch::DirWatch,
    /// Set when a snapshot landed because the *directory* changed rather than
    /// because the user navigated. The cursor is an index, so it has to be
    /// re-derived after one of these; the selection is by name and does not.
    pub refreshed: bool,
    /// Set when the directory itself was deleted or moved away. The pane
    /// showing it has to go somewhere that still exists.
    pub gone: bool,
    /// An in-place re-read is in flight, so the snapshot it delivers is the
    /// one that sets `refreshed`.
    reload_pending: bool,
}

impl Column {
    pub fn new(path: PathBuf) -> Self {
        let mut loader = Directory::new();
        loader.load(&path);
        let watch = crate::watch::DirWatch::new(&path);
        Self {
            path,
            snapshot: Snapshot::default(),
            loader,
            selection: std::collections::BTreeSet::new(),
            cursor: None,
            anchor: None,
            scroll: ScrollView::new(Rect::new_empty()),
            epoch: 0,
            sorted: std::cell::RefCell::new(SortCache::default()),
            watch,
            refreshed: false,
            gone: false,
            reload_pending: false,
        }
    }

    pub fn loading(&self) -> bool {
        self.loader.loading
    }

    /// A first read, with nothing to show until it lands.
    ///
    /// An in-place re-read is not one: the listing already on screen is still
    /// very nearly right, so a delete or a paste keeps it up rather than
    /// blinking the whole pane through a "Loading" placeholder and back.
    pub fn awaiting_first_listing(&self) -> bool {
        self.loader.loading && self.epoch == 0
    }

    /// Re-read this directory, keeping everything the user positioned: the
    /// selection (held by name), the scroll offset and the scroll metrics.
    /// Only the listing is replaced.
    pub fn reload(&mut self) {
        self.loader.load(&self.path);
        self.reload_pending = true;
    }

    /// Take a finished read, if one has arrived, and start a fresh one when
    /// the directory has changed underneath. Returns whether anything changed,
    /// so the caller knows to repaint.
    pub fn poll(&mut self) -> bool {
        match self.watch.take() {
            // A re-read in place: `snapshot` is replaced, and nothing the user
            // positioned — selection, scroll offset — is touched.
            Some(crate::watch::Change::Modified) => {
                self.reload();
            }
            Some(crate::watch::Change::Gone) => self.gone = true,
            None => {}
        }
        match self.loader.poll() {
            Some(snapshot) => {
                self.snapshot = snapshot;
                self.refreshed = std::mem::take(&mut self.reload_pending);
                // The only place the listing is ever replaced, so the only
                // place the sorted order can go stale under it.
                self.epoch = self.epoch.wrapping_add(1);
                true
            }
            None => false,
        }
    }
}

/// The filtered, sorted order of a column's listing, remembered between
/// frames.
///
/// Sorting is not a per-frame cost that anyone can afford: a directory of
/// twenty-five thousand entries takes ~20 ms to filter and sort, and the
/// browser asks for the order eight times a frame (once per column while
/// building the frame, twice more for the subtitle, once per column again
/// for the scroll metrics) *plus* once per column on every wheel event. Any
/// of those alone would blow a 120 Hz budget on a big directory.
///
/// `key` is everything the order depends on: which listing it was computed
/// from ([`Column::epoch`]), the three settings that decide the order, and —
/// in the picker — which of the request's filters is in force.
/// The entries themselves are not copied — `order` holds indices into
/// [`Column::snapshot`], which is what `epoch` guards.
#[derive(Default)]
pub struct SortCache {
    pub key: Option<(u64, SortKey, bool, bool, usize)>,
    pub order: Vec<usize>,
}

/// Reads directories on worker threads and hands finished snapshots back.
///
/// The UI thread calls [`Directory::load`] and then [`Directory::poll`]; it
/// never blocks, so a stalled network mount costs a spinner rather than a
/// frame.
pub struct Directory {
    rx: Option<Receiver<Snapshot>>,
    /// Bumped on every load. A snapshot arriving with a stale generation is
    /// dropped — that is how navigating away cancels a slow read, without
    /// needing to interrupt the worker.
    generation: Arc<Mutex<u64>>,
    current: u64,
    pub loading: bool,
}

impl Default for Directory {
    fn default() -> Self {
        Self::new()
    }
}

impl Directory {
    pub fn new() -> Self {
        Self {
            rx: None,
            generation: Arc::new(Mutex::new(0)),
            current: 0,
            loading: false,
        }
    }

    /// Start reading `path`. Any read already in flight is abandoned.
    pub fn load(&mut self, path: &Path) {
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.loading = true;

        let generation = {
            let mut g = self.generation.lock().unwrap();
            *g += 1;
            *g
        };
        self.current = generation;

        let path = path.to_path_buf();
        let gen_handle = Arc::clone(&self.generation);
        std::thread::spawn(move || {
            let snapshot = read_directory(&path);
            // Only deliver if this is still the read the UI is waiting for.
            if *gen_handle.lock().unwrap() == generation {
                let _ = tx.send(snapshot);
                // Wake the UI thread. Without this the snapshot waits for the
                // next input: a window with nothing moving commits no frames,
                // so there is no frame callback to notice it landed.
                otto_kit::prelude::AppContext::request_wakeup();
            }
        });
    }

    /// A finished snapshot, if one has arrived. Never blocks.
    pub fn poll(&mut self) -> Option<Snapshot> {
        let rx = self.rx.as_ref()?;
        match rx.try_recv() {
            Ok(snapshot) => {
                self.loading = false;
                Some(snapshot)
            }
            Err(_) => None,
        }
    }
}

/// Read one directory. Runs on a worker thread, never on the UI thread.
///
/// This first version stats every entry as it reads, which is what the spec
/// forbids on the fast path for very large directories. The streaming
/// name-first pass is the next change; the snapshot shape is already what it
/// will deliver.
fn read_directory(path: &Path) -> Snapshot {
    let read = match std::fs::read_dir(path) {
        Ok(read) => read,
        Err(err) => {
            return Snapshot {
                path: path.to_path_buf(),
                entries: Vec::new(),
                error: Some(describe_error(&err)),
            }
        }
    };

    let mut entries = Vec::new();
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();

        // `file_type` comes from readdir's d_type where the filesystem
        // supplies it, so it is usually free.
        let file_type = entry.file_type().ok();
        let is_symlink = file_type.is_some_and(|t| t.is_symlink());
        // A symlink's target decides whether it behaves as a directory.
        let is_dir = match file_type {
            Some(t) if t.is_dir() => true,
            Some(t) if t.is_symlink() => path.is_dir(),
            _ => false,
        };

        let meta = entry.metadata().ok();
        let kind = if is_dir {
            Kind::Folder
        } else {
            filetype::kind_for_name(&name)
        };

        entries.push(Entry {
            hidden: name.starts_with('.') || name.ends_with('~'),
            kind,
            size: meta.as_ref().map(|m| m.len()),
            modified: meta.as_ref().and_then(|m| m.modified().ok()),
            name,
            path,
            is_dir,
            is_symlink,
        });
    }

    Snapshot {
        path: path.to_path_buf(),
        entries,
        error: None,
    }
}

/// A message worth showing a user, rather than a debug rendering.
fn describe_error(err: &std::io::Error) -> String {
    match err.kind() {
        std::io::ErrorKind::PermissionDenied => otto_kit::t_owned!("files-folder-denied"),
        std::io::ErrorKind::NotFound => otto_kit::t_owned!("files-folder-gone"),
        _ => otto_kit::t_owned!("files-folder-open-failed", error = err.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Places
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Place {
    pub label: String,
    pub path: PathBuf,
    pub icon: &'static str,
}

/// The sidebar's places: home, then the XDG user directories that actually
/// exist. A directory that is not there is not listed — an empty row leading
/// nowhere is worse than its absence.
pub fn places() -> Vec<Place> {
    let mut places = Vec::new();
    let Some(home) = home_dir() else {
        return places;
    };

    places.push(Place {
        // Not a directory name — the folder on disk is called whatever the
        // user's login is — so this one is always translated.
        label: otto_kit::t_owned!("files-home"),
        path: home.clone(),
        icon: "user-home",
    });

    // `user-dirs.dirs` is `XDG_DESKTOP_DIR="$HOME/Desktop"` per line. Parsed
    // here rather than by a crate: it is five lines of shell-ish assignment.
    let configured = user_dirs(&home);

    const WANTED: &[(&str, &str, &str, &str)] = &[
        (
            "XDG_DESKTOP_DIR",
            "Desktop",
            "user-desktop",
            "files-desktop",
        ),
        (
            "XDG_DOCUMENTS_DIR",
            "Documents",
            "folder-documents",
            "files-documents",
        ),
        (
            "XDG_DOWNLOAD_DIR",
            "Downloads",
            "folder-download",
            "files-downloads",
        ),
        ("XDG_MUSIC_DIR", "Music", "folder-music", "files-music"),
        (
            "XDG_PICTURES_DIR",
            "Pictures",
            "folder-pictures",
            "files-pictures",
        ),
        ("XDG_VIDEOS_DIR", "Videos", "folder-videos", "files-videos"),
    ];

    for (key, fallback_name, icon, message) in WANTED {
        let path = configured
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| home.join(fallback_name));
        if path.is_dir() {
            let on_disk = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| (*fallback_name).to_string());
            // These are real folders, and the sidebar should agree with what
            // the terminal and every other application show. A system set up
            // in another language already named them in that language, so the
            // name on disk is both truthful and localised, and it wins.
            //
            // The catalogue is only consulted when the folder still carries
            // its English default — an account created before the language
            // was chosen, or one made by hand — where translating the shortcut
            // is a kindness rather than a lie.
            let label = if on_disk == *fallback_name {
                otto_kit::t_owned!(message)
            } else {
                on_disk
            };
            places.push(Place { label, path, icon });
        }
    }

    places
}

fn user_dirs(home: &Path) -> Vec<(String, PathBuf)> {
    let config = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));

    let Ok(text) = std::fs::read_to_string(config.join("user-dirs.dirs")) else {
        return Vec::new();
    };

    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            let value = value.trim().trim_matches('"');
            let expanded = match value.strip_prefix("$HOME/") {
                Some(rest) => home.join(rest),
                None if value == "$HOME" => home.to_path_buf(),
                None => PathBuf::from(value),
            };
            Some((key.trim().to_string(), expanded))
        })
        .collect()
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

/// Human-readable size, the way a file manager writes it.
pub fn format_size(bytes: u64) -> String {
    // Under a kilobyte the count is exact and needs a plural rule — one byte,
    // two bytes, and whatever the local grammar does with 2 and 5.
    if bytes < 1000 {
        return otto_kit::t_owned!("files-size-bytes", count = bytes as f64);
    }

    const UNITS: &[&str] = &[
        "files-size-kb",
        "files-size-mb",
        "files-size-gb",
        "files-size-tb",
    ];
    // Divided once up front: anything reaching here is at least a kilobyte,
    // and UNITS starts at KB rather than at bytes, so the counter and the unit
    // it names stay in step.
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    // One decimal below ten, none above: the extra digit stops a 1 GB file and
    // a 9 GB file from looking the same, and is noise once the number is wide.
    let rendered = if value < 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.0}")
    };
    otto_kit::t_owned!(UNITS[unit], value = rendered)
}

/// Date, as a listing shows it. Deliberately plain: no locale formatting, and
/// no relative "yesterday" — both need more than the standard library gives.
pub fn format_time(time: SystemTime) -> String {
    let Ok(elapsed) = time.duration_since(SystemTime::UNIX_EPOCH) else {
        return String::new();
    };
    let secs = elapsed.as_secs() as i64;
    let days = secs.div_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let time_of_day = secs.rem_euclid(86_400);
    let (hour, minute) = (time_of_day / 3600, (time_of_day % 3600) / 60);
    // Assembled from parts rather than formatted from a pattern, because the
    // month names have to be translated too — and the order of the parts is
    // itself a local convention, which is why the carrier string owns it.
    const MONTHS: [&str; 12] = [
        "files-month-jan",
        "files-month-feb",
        "files-month-mar",
        "files-month-apr",
        "files-month-may",
        "files-month-jun",
        "files-month-jul",
        "files-month-aug",
        "files-month-sep",
        "files-month-oct",
        "files-month-nov",
        "files-month-dec",
    ];
    let month = otto_kit::t!(MONTHS[(month as usize - 1).min(11)]);
    otto_kit::t_owned!(
        "files-date-modified",
        day = day.to_string(),
        month = month,
        year = year.to_string(),
        time = format!("{hour:02}:{minute:02}")
    )
}

/// Days since the Unix epoch to a civil date. Howard Hinnant's algorithm —
/// exact, branch-light, and shorter than taking on a date crate.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order_reads_numbers_as_numbers() {
        assert_eq!(natural_cmp("file2", "file10"), std::cmp::Ordering::Less);
        assert_eq!(natural_cmp("file10", "file2"), std::cmp::Ordering::Greater);
        assert_eq!(natural_cmp("a", "a"), std::cmp::Ordering::Equal);
    }

    #[test]
    fn natural_order_ignores_case() {
        assert_eq!(natural_cmp("Apple", "apple"), std::cmp::Ordering::Equal);
        assert_eq!(natural_cmp("apple", "Banana"), std::cmp::Ordering::Less);
    }

    #[test]
    fn natural_order_is_a_total_order() {
        // Leading zeros must not make two different names compare equal, or
        // the sort becomes unstable in a way the user sees as flicker.
        assert_ne!(natural_cmp("file007", "file7"), std::cmp::Ordering::Equal);
    }

    /// Compared against the catalogue rather than against English prose.
    ///
    /// What this guards is the threshold each size crosses and how many
    /// decimals survive it — 1.5 KB rather than 1.5 kB, 15 KB rather than
    /// 15.0. The words around the number are the catalogue's business, and
    /// spelling them out here would fail the test on a developer whose own
    /// session is not English, which is not a bug in `format_size`.
    #[test]
    fn sizes_read_the_way_a_file_manager_writes_them() {
        assert_eq!(
            format_size(0),
            otto_kit::t_owned!("files-size-bytes", count = 0.0)
        );
        assert_eq!(
            format_size(999),
            otto_kit::t_owned!("files-size-bytes", count = 999.0)
        );
        assert_eq!(
            format_size(1_500),
            otto_kit::t_owned!("files-size-kb", value = "1.5")
        );
        assert_eq!(
            format_size(15_000),
            otto_kit::t_owned!("files-size-kb", value = "15")
        );
        assert_eq!(
            format_size(2_000_000),
            otto_kit::t_owned!("files-size-mb", value = "2.0")
        );
    }

    #[test]
    fn epoch_formats_correctly() {
        // A fixed point, so the date arithmetic is pinned rather than trusted.
        //
        // Asserted part by part rather than as one string: the order of day,
        // month and year is now the locale's business — en-GB puts the day
        // first, en-US the month — and pinning one ordering here would make
        // this test fail on a correctly translated desktop. What it is
        // actually guarding is `civil_from_days`, and that shows up in the
        // parts whatever order they are printed in.
        //
        // The month comes from the catalogue for the same reason: its name is
        // translated too, so "Jan" holds only in English.
        for (secs, day, month, year, time) in [
            (0u64, "1", otto_kit::t!("files-month-jan"), "1970", "00:00"),
            (
                1_700_000_000,
                "14",
                otto_kit::t!("files-month-nov"),
                "2023",
                "22:13",
            ),
        ] {
            let at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs);
            let rendered = format_time(at);
            for part in [day, month, year, time] {
                assert!(rendered.contains(part), "{rendered:?} is missing {part:?}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Get Info
// ---------------------------------------------------------------------------

/// Everything the Get Info panel shows about one entry.
///
/// Read on a worker like everything else — `stat`, and the passwd/group lookups
/// behind it, can block on a networked account database just as a slow mount
/// can block a listing.
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub mime: String,
    pub kind: Kind,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub created: Option<SystemTime>,
    /// Unix permission bits, the low 12 of `st_mode` (setuid/setgid/sticky
    /// included, so toggling a bit cannot silently clear them).
    pub mode: u32,
    pub owner: String,
    pub group: String,
    pub link_target: Option<PathBuf>,
    /// Set when the panel could not read the file at all.
    pub error: Option<String>,
}

impl FileInfo {
    /// `rwxr-xr-x`, the way `ls -l` writes it.
    pub fn mode_string(&self) -> String {
        let mut s = String::with_capacity(9);
        for who in 0..3 {
            // Owner bits are the top triad, so shift down by 6, 3, 0.
            let shift = 6 - who * 3;
            let bits = (self.mode >> shift) & 0o7;
            s.push(if bits & 0b100 != 0 { 'r' } else { '-' });
            s.push(if bits & 0b010 != 0 { 'w' } else { '-' });
            s.push(if bits & 0b001 != 0 { 'x' } else { '-' });
        }
        s
    }

    /// The octal a user would type at `chmod`.
    pub fn mode_octal(&self) -> String {
        format!("{:03o}", self.mode & 0o777)
    }

    /// Is the bit for `who` (0 owner, 1 group, 2 other) and `what`
    /// (0 read, 1 write, 2 execute) set?
    pub fn permission(&self, who: usize, what: usize) -> bool {
        self.mode & permission_bit(who, what) != 0
    }
}

/// The single `st_mode` bit for one cell of the permissions grid.
pub fn permission_bit(who: usize, what: usize) -> u32 {
    let shift = 6 - who.min(2) * 3;
    let bit = match what.min(2) {
        0 => 0b100,
        1 => 0b010,
        _ => 0b001,
    };
    bit << shift
}

/// Read everything the info panel needs. Runs on a worker.
pub fn read_info(path: &Path) -> FileInfo {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());

    // `symlink_metadata`, not `metadata`: the panel describes *this* entry, and
    // the permissions it offers to change are this entry's own.
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) => {
            return FileInfo {
                name,
                path: path.to_path_buf(),
                is_dir: false,
                mime: String::new(),
                kind: Kind::Other,
                size: 0,
                modified: None,
                accessed: None,
                created: None,
                mode: 0,
                owner: String::new(),
                group: String::new(),
                link_target: None,
                error: Some(describe_error(&err)),
            }
        }
    };

    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::PermissionsExt;

    let is_dir = meta.is_dir();
    let mime = if is_dir {
        "inode/directory".to_string()
    } else {
        filetype::mime_for_name(&name)
            .unwrap_or("application/octet-stream")
            .to_string()
    };

    FileInfo {
        kind: if is_dir {
            Kind::Folder
        } else {
            filetype::kind_of(&mime)
        },
        size: meta.len(),
        modified: meta.modified().ok(),
        accessed: meta.accessed().ok(),
        created: meta.created().ok(),
        // Low 12 bits: the 9 rwx plus setuid/setgid/sticky.
        mode: meta.permissions().mode() & 0o7777,
        owner: user_name(meta.uid()).unwrap_or_else(|| meta.uid().to_string()),
        group: group_name(meta.gid()).unwrap_or_else(|| meta.gid().to_string()),
        link_target: std::fs::read_link(path).ok(),
        name,
        path: path.to_path_buf(),
        is_dir,
        mime,
        error: None,
    }
}

/// Apply a new permission mode. Runs on a worker; returns the reason on failure
/// so the panel can say why rather than silently reverting.
pub fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(|err| {
        match err.kind() {
            std::io::ErrorKind::PermissionDenied => {
                "Only the file's owner can change its permissions.".to_string()
            }
            _ => format!("Could not change permissions: {err}"),
        }
    })
}

/// Resolve a uid through the system account database.
///
/// `getpwuid_r` rather than parsing `/etc/passwd`, because the passwd file is
/// not the whole story on a machine with networked or systemd-homed accounts —
/// it would show a bare number for exactly the users who are not local.
fn user_name(uid: u32) -> Option<String> {
    // SAFETY: `getpwuid_r` writes into the buffers we hand it and reports the
    // result through `result`, which is null when there is no such user. No
    // pointer we pass outlives this call.
    unsafe {
        let mut pwd: libc::passwd = std::mem::zeroed();
        let mut buf = vec![0 as libc::c_char; 2048];
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        if libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result) != 0
            || result.is_null()
        {
            return None;
        }
        cstr_to_string(pwd.pw_name)
    }
}

fn group_name(gid: u32) -> Option<String> {
    // SAFETY: as `user_name`.
    unsafe {
        let mut grp: libc::group = std::mem::zeroed();
        let mut buf = vec![0 as libc::c_char; 2048];
        let mut result: *mut libc::group = std::ptr::null_mut();
        if libc::getgrgid_r(gid, &mut grp, buf.as_mut_ptr(), buf.len(), &mut result) != 0
            || result.is_null()
        {
            return None;
        }
        cstr_to_string(grp.gr_name)
    }
}

/// SAFETY: `ptr` must be a NUL-terminated C string owned by the caller's buffer.
unsafe fn cstr_to_string(ptr: *const libc::c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    Some(std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned())
}

#[cfg(test)]
mod info_tests {
    use super::*;

    fn info_with_mode(mode: u32) -> FileInfo {
        FileInfo {
            name: "f".into(),
            path: PathBuf::from("f"),
            is_dir: false,
            mime: String::new(),
            kind: Kind::Other,
            size: 0,
            modified: None,
            accessed: None,
            created: None,
            mode,
            owner: String::new(),
            group: String::new(),
            link_target: None,
            error: None,
        }
    }

    #[test]
    fn mode_renders_the_way_ls_does() {
        assert_eq!(info_with_mode(0o755).mode_string(), "rwxr-xr-x");
        assert_eq!(info_with_mode(0o644).mode_string(), "rw-r--r--");
        assert_eq!(info_with_mode(0o000).mode_string(), "---------");
        assert_eq!(info_with_mode(0o777).mode_string(), "rwxrwxrwx");
        assert_eq!(info_with_mode(0o644).mode_octal(), "644");
    }

    #[test]
    fn permission_bits_address_the_right_cell() {
        let info = info_with_mode(0o640);
        // owner rw-, group r--, other ---
        assert!(info.permission(0, 0) && info.permission(0, 1) && !info.permission(0, 2));
        assert!(info.permission(1, 0) && !info.permission(1, 1));
        assert!(!info.permission(2, 0));
    }

    #[test]
    fn toggling_a_bit_leaves_the_others_alone() {
        // The grid edits one bit at a time; setuid and the rest must survive.
        let mode = 0o4755;
        let toggled = mode ^ permission_bit(2, 1);
        assert_eq!(toggled, 0o4757);
        assert_eq!(toggled & 0o7000, 0o4000, "setuid preserved");
    }

    #[test]
    fn the_current_user_resolves() {
        // If this returns None the panel would show a bare uid for everyone.
        let uid = unsafe { libc::getuid() };
        assert!(user_name(uid).is_some());
    }
}

#[cfg(test)]
mod chmod_tests {
    use super::*;

    /// The toggle path end to end: read a real file's mode, flip one bit
    /// through `set_mode`, and confirm the re-read agrees. This is what the
    /// info sheet's checkboxes do on every click.
    #[test]
    fn toggling_group_write_applies_and_reads_back() {
        let dir = std::env::temp_dir().join(format!("otto-files-chmod-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("target.txt");
        std::fs::write(&path, b"x").unwrap();
        set_mode(&path, 0o644).unwrap();

        let before = read_info(&path);
        assert_eq!(before.mode_octal(), "644");
        assert!(!before.permission(1, 1), "group write starts clear");

        let next = before.mode ^ permission_bit(1, 1);
        set_mode(&path, next).unwrap();

        let after = read_info(&path);
        assert_eq!(after.mode_octal(), "664");
        assert!(after.permission(1, 1), "group write is now set");
        // Everything else must be untouched.
        assert!(after.permission(0, 0) && after.permission(0, 1));
        assert!(!after.permission(2, 1));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// setuid and friends live above the nine rwx bits and must survive a
    /// toggle — the grid edits one bit, not the whole mode.
    #[test]
    fn special_bits_survive_a_toggle() {
        let dir = std::env::temp_dir().join(format!("otto-files-suid-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("target.txt");
        std::fs::write(&path, b"x").unwrap();
        set_mode(&path, 0o2755).unwrap();

        let before = read_info(&path);
        assert_eq!(before.mode & 0o7000, 0o2000, "setgid is set to begin with");

        set_mode(&path, before.mode ^ permission_bit(2, 1)).unwrap();
        let after = read_info(&path);
        assert_eq!(after.mode & 0o7000, 0o2000, "setgid survived");
        assert_eq!(after.mode_octal(), "757");

        std::fs::remove_dir_all(&dir).ok();
    }
}

// ---------------------------------------------------------------------------
// Clipboard and file operations
// ---------------------------------------------------------------------------

/// What a cut or copy put on the clipboard.
///
/// **Internal to this application.** otto-kit has no `wl_data_device` support,
/// so nothing here reaches the system clipboard and copying a file in the
/// browser cannot be pasted into another application. Cross-application copy
/// needs data-device support in the toolkit first — the same gap that blocks
/// drag and drop. See `specs/file-browser.md`.
#[derive(Debug, Clone, Default)]
pub struct Clipboard {
    pub paths: Vec<PathBuf>,
    /// A cut is not applied until the paste: the source stays put until then,
    /// so an abandoned cut loses nothing.
    pub cut: bool,
}

impl Clipboard {
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

/// How to resolve a destination that already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnConflict {
    /// Give the new file a numbered name; never destroys anything.
    KeepBoth,
    Replace,
    Skip,
}

/// One concrete thing an operation did to the filesystem, in enough detail to
/// put it back.
///
/// Recorded per item rather than per operation because an operation is not
/// all-or-nothing: a paste of ten files can move eight, skip one and fail on
/// one, and only the eight that happened may be undone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Something that existed at `from` now lives at `to` — a move, a rename,
    /// a drag between directories. Undone by moving it back.
    Moved { from: PathBuf, to: PathBuf },
    /// Something now exists at `path` that did not before — a copy, a new
    /// folder. Undone by taking it away again.
    Created { path: PathBuf },
    /// Trashed: the item sits at `to` inside the trash can with its
    /// `.trashinfo` sidecar at `info`, and came from `from`. Undone by putting
    /// it back and dropping the sidecar, which is a restore.
    Trashed {
        from: PathBuf,
        to: PathBuf,
        info: PathBuf,
    },
}

/// The outcome of a paste.
#[derive(Debug, Clone, Default)]
pub struct OpResult {
    /// Everything this operation actually did, in the order it did it. The
    /// undo stack is built from this; see [`undo`].
    pub changes: Vec<Change>,
    pub moved: usize,
    pub copied: usize,
    pub skipped: usize,
    /// Items moved to Trash — kept apart from `moved` since it reads as a
    /// different sentence in [`Self::summary`].
    pub trashed: usize,
    /// One message per file that failed. A failure stops that file, not the
    /// whole operation.
    pub errors: Vec<String>,
}

impl OpResult {
    pub fn summary(&self) -> String {
        if !self.errors.is_empty() {
            return self.errors[0].clone();
        }
        if self.trashed > 0 {
            return format!(
                "Moved {} item{} to Trash",
                self.trashed,
                plural(self.trashed)
            );
        }
        match (self.moved, self.copied, self.skipped) {
            (0, 0, 0) => String::new(),
            (m, 0, _) if m > 0 => format!("Moved {m} item{}", plural(m)),
            (0, c, _) if c > 0 => format!("Copied {c} item{}", plural(c)),
            (m, c, _) => format!("Moved {m}, copied {c}"),
        }
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// Paste `clipboard` into `dest`. Runs on a worker thread.
pub fn paste(clipboard: &Clipboard, dest: &Path, on_conflict: OnConflict) -> OpResult {
    let mut result = OpResult::default();

    for source in &clipboard.paths {
        let Some(name) = source.file_name() else {
            continue;
        };

        // Pasting a directory into itself or its own descendant would recurse
        // forever. Checked before anything is written.
        if source.is_dir() && dest.starts_with(source) {
            result.errors.push(format!(
                "Cannot paste “{}” into itself.",
                name.to_string_lossy()
            ));
            continue;
        }

        let mut target = dest.join(name);
        if target == *source {
            // Copying into the same directory: always keep both, or the file
            // would be asked to replace itself.
            target = unique_name(dest, name);
        } else if target.exists() {
            match on_conflict {
                OnConflict::Skip => {
                    result.skipped += 1;
                    continue;
                }
                OnConflict::KeepBoth => target = unique_name(dest, name),
                OnConflict::Replace => {}
            }
        }

        // Whether the destination was already there decides how an undo
        // takes the copy back: replacing an existing file overwrote it, and
        // removing our copy would not bring the old one back — so that one is
        // not recorded as undoable at all.
        let replaced = !clipboard.cut && target.exists();
        let outcome = if clipboard.cut {
            move_entry(source, &target).map(|_| {
                result.moved += 1;
                result.changes.push(Change::Moved {
                    from: source.clone(),
                    to: target.clone(),
                });
            })
        } else {
            copy_entry(source, &target).map(|_| {
                result.copied += 1;
                if !replaced {
                    result.changes.push(Change::Created {
                        path: target.clone(),
                    });
                }
            })
        };
        if let Err(err) = outcome {
            result
                .errors
                .push(format!("“{}”: {err}", name.to_string_lossy()));
        }
    }

    result
}

/// Put back everything in `changes`, most recent first.
///
/// Reverse order matters: a paste that both created a file and moved another
/// out of its way has to be unwound in the order it was wound.
///
/// Undoing is itself an operation that can fail — the destination may be gone,
/// or something may have taken the name back — and a failure stops that one
/// item, not the rest, the same way [`paste`] does. The counts in the result
/// describe the *undo*, so a summary reads as what just happened.
///
/// Nothing here deletes a file outright. Taking back a copy trashes it, so an
/// undo is never the thing that loses data; only a directory this app created
/// and that is still empty is removed, since there is nothing in it to lose.
pub fn undo(changes: &[Change]) -> OpResult {
    let mut result = OpResult::default();

    for change in changes.iter().rev() {
        match change {
            Change::Moved { from, to } => {
                if from.exists() {
                    result.errors.push(format!(
                        "Can\u{2019}t put \u{201c}{}\u{201d} back \u{2014} something is there now.",
                        name_of(from)
                    ));
                    continue;
                }
                match move_entry(to, from) {
                    Ok(()) => result.moved += 1,
                    Err(err) => result
                        .errors
                        .push(format!("\u{201c}{}\u{201d}: {err}", name_of(to))),
                }
            }
            Change::Created { path } => {
                if !path.exists() {
                    continue;
                }
                let empty_dir = path.is_dir()
                    && std::fs::read_dir(path)
                        .map(|mut d| d.next().is_none())
                        .unwrap_or(false);
                if empty_dir {
                    match std::fs::remove_dir(path) {
                        Ok(()) => result.trashed += 1,
                        Err(err) => result
                            .errors
                            .push(format!("\u{201c}{}\u{201d}: {err}", name_of(path))),
                    }
                    continue;
                }
                let trashed = move_to_trash(std::slice::from_ref(path));
                result.trashed += trashed.trashed;
                result.errors.extend(trashed.errors);
            }
            Change::Trashed { from, to, info } => {
                if from.exists() {
                    result.errors.push(format!(
                        "Can\u{2019}t restore \u{201c}{}\u{201d} \u{2014} something is there now.",
                        name_of(from)
                    ));
                    continue;
                }
                match move_entry(to, from) {
                    Ok(()) => {
                        // The sidecar describes an item that is no longer in
                        // the trash; leaving it would show a phantom there.
                        std::fs::remove_file(info).ok();
                        result.moved += 1;
                    }
                    Err(err) => result
                        .errors
                        .push(format!("\u{201c}{}\u{201d}: {err}", name_of(to))),
                }
            }
        }
    }

    result
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// `name`, `name 2`, `name 3`… — the first that does not exist in `dir`.
///
/// The suffix goes before the extension, so `photo.png` becomes `photo 2.png`
/// rather than `photo.png 2`.
fn unique_name(dir: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let name = name.to_string_lossy();
    let (stem, ext) = match name.rsplit_once('.') {
        // A leading dot is the whole name of a hidden file, not an extension.
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name.as_ref(), String::new()),
    };

    for n in 2..10_000 {
        let candidate = dir.join(format!("{stem} {n}{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem} {}{ext}", std::process::id()))
}

/// Move one entry, falling back to copy-then-delete across filesystems.
///
/// The source is unlinked only after the destination is fully written — that
/// ordering is the whole guarantee that a failed move cannot lose data.
fn move_entry(source: &Path, target: &Path) -> Result<(), String> {
    match std::fs::rename(source, target) {
        Ok(()) => Ok(()),
        // EXDEV: a rename cannot cross filesystems, so do it the long way.
        Err(err) if err.raw_os_error() == Some(libc::EXDEV) => {
            copy_entry(source, target)?;
            remove_entry(source)
        }
        Err(err) => Err(err.to_string()),
    }
}

/// Copy a file or a directory tree.
fn copy_entry(source: &Path, target: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(source).map_err(|e| e.to_string())?;

    if meta.is_symlink() {
        // Copy the link itself, not what it points at — following it could
        // duplicate a whole tree the user did not ask for.
        let link = std::fs::read_link(source).map_err(|e| e.to_string())?;
        return std::os::unix::fs::symlink(link, target).map_err(|e| e.to_string());
    }

    if meta.is_dir() {
        std::fs::create_dir_all(target).map_err(|e| e.to_string())?;
        for entry in std::fs::read_dir(source)
            .map_err(|e| e.to_string())?
            .flatten()
        {
            copy_entry(&entry.path(), &target.join(entry.file_name()))?;
        }
        return Ok(());
    }

    // Write to a temporary name in the destination directory and rename into
    // place, so an interrupted copy never leaves a truncated file wearing the
    // real name.
    let parent = target.parent().unwrap_or(Path::new("."));
    let temp = parent.join(format!(
        ".{}.otto-files-{}",
        target
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default(),
        std::process::id()
    ));

    let copy = std::fs::copy(source, &temp).map_err(|e| e.to_string());
    if let Err(err) = copy {
        std::fs::remove_file(&temp).ok();
        return Err(err);
    }
    if let Err(err) = std::fs::rename(&temp, target) {
        std::fs::remove_file(&temp).ok();
        return Err(err.to_string());
    }
    Ok(())
}

fn remove_entry(path: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.is_dir() && !meta.is_symlink() {
        std::fs::remove_dir_all(path).map_err(|e| e.to_string())
    } else {
        std::fs::remove_file(path).map_err(|e| e.to_string())
    }
}

/// `dir.join(name)`, or the next free numbered variant if that is taken.
///
/// Unlike [`unique_name`] — which always numbers, because every caller of it
/// already knows the plain name collides — this checks first, so a fresh
/// "untitled folder" doesn't open as "untitled folder 2" for no reason.
fn first_free_name(dir: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let plain = dir.join(name);
    if plain.exists() {
        unique_name(dir, name)
    } else {
        plain
    }
}

/// Create a new, empty directory in `dest`, named "untitled folder" or a
/// numbered variant if that name is already taken.
pub fn create_folder(dest: &Path) -> Result<PathBuf, String> {
    let target = first_free_name(dest, std::ffi::OsStr::new("untitled folder"));
    std::fs::create_dir(&target).map_err(|e| e.to_string())?;
    Ok(target)
}

// ---------------------------------------------------------------------------
// Trash
// ---------------------------------------------------------------------------

/// Move `paths` to the freedesktop trash can, each with a `.trashinfo`
/// sidecar recording where it came from — so a spec-compliant file manager
/// (this one, eventually) can restore it. Runs on the calling thread; see
/// [`paste`] for why that is acceptable here.
pub fn move_to_trash(paths: &[PathBuf]) -> OpResult {
    let mut result = OpResult::default();
    let Some(trash) = trash_dir() else {
        result
            .errors
            .push("No home directory to trash into.".to_string());
        return result;
    };
    let files_dir = trash.join("files");
    let info_dir = trash.join("info");
    if let Err(err) =
        std::fs::create_dir_all(&files_dir).and_then(|_| std::fs::create_dir_all(&info_dir))
    {
        result
            .errors
            .push(format!("Couldn\u{2019}t prepare Trash: {err}"));
        return result;
    }

    for source in paths {
        let Some(name) = source.file_name() else {
            continue;
        };
        match trash_one(source, name, &files_dir, &info_dir) {
            Ok((to, info)) => {
                result.trashed += 1;
                result.changes.push(Change::Trashed {
                    from: source.clone(),
                    to,
                    info,
                });
            }
            Err(err) => result
                .errors
                .push(format!("\u{201c}{}\u{201d}: {err}", name.to_string_lossy())),
        }
    }
    result
}

/// Returns where the item landed in the trash and where its sidecar went, so
/// the caller can record a restorable [`Change::Trashed`].
fn trash_one(
    source: &Path,
    name: &std::ffi::OsStr,
    files_dir: &Path,
    info_dir: &Path,
) -> Result<(PathBuf, PathBuf), String> {
    let target = first_free_name(files_dir, name);
    let trashed_name = target.file_name().unwrap().to_string_lossy().to_string();
    let info_path = info_dir.join(format!("{trashed_name}.trashinfo"));

    move_entry(source, &target)?;

    let info = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        percent_encode_path(source),
        deletion_date(),
    );
    std::fs::write(&info_path, info).map_err(|e| e.to_string())?;
    Ok((target, info_path))
}

/// A trash can under a temp directory, for tests.
///
/// Redirecting `XDG_DATA_HOME` is the only way to keep [`move_to_trash`] out
/// of the developer's real Trash, and the environment belongs to the whole
/// process — so it is set exactly once, to one value, and never unset. A test
/// that set it and cleared it around itself would decide, by timing alone,
/// where a test running beside it put its files.
#[cfg(test)]
pub(crate) fn test_data_home() -> &'static Path {
    use std::sync::OnceLock;
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let path =
            std::env::temp_dir().join(format!("otto-files-test-data-{}", std::process::id()));
        std::fs::create_dir_all(&path).expect("test data home");
        std::env::set_var("XDG_DATA_HOME", &path);
        path
    })
}

/// `$XDG_DATA_HOME/Trash`, falling back to `~/.local/share/Trash` — the
/// "home trashcan" the freedesktop Trash spec describes.
fn trash_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir).join("Trash"));
    }
    home_dir().map(|h| h.join(".local/share/Trash"))
}

/// Percent-encode a path the way a `.trashinfo`'s `Path=` key requires:
/// everything but the unreserved characters and the `/` separator.
fn percent_encode_path(path: &Path) -> String {
    let mut out = String::new();
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Local time as `YYYY-MM-DDTHH:MM:SS`, the format a `.trashinfo`'s
/// `DeletionDate` key requires.
fn deletion_date() -> String {
    // Safe: `tm` is POD and `localtime_r` fills every field it reads back.
    unsafe {
        let mut when: libc::time_t = 0;
        libc::time(&mut when);
        let mut broken: libc::tm = std::mem::zeroed();
        libc::localtime_r(&when, &mut broken);
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            broken.tm_year + 1900,
            broken.tm_mon + 1,
            broken.tm_mday,
            broken.tm_hour,
            broken.tm_min,
            broken.tm_sec,
        )
    }
}

#[cfg(test)]
mod paste_tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "otto-files-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            std::fs::remove_dir_all(&dir).ok();
            std::fs::create_dir_all(&dir).unwrap();
            Tmp(dir)
        }
        fn file(&self, name: &str, body: &str) -> PathBuf {
            let p = self.0.join(name);
            std::fs::write(&p, body).unwrap();
            p
        }
        fn dir(&self, name: &str) -> PathBuf {
            let p = self.0.join(name);
            std::fs::create_dir_all(&p).unwrap();
            p
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn copy_leaves_the_source_alone() {
        let t = Tmp::new("copy");
        let src = t.file("a.txt", "hello");
        let dest = t.dir("dest");

        let clip = Clipboard {
            paths: vec![src.clone()],
            cut: false,
        };
        let result = paste(&clip, &dest, OnConflict::KeepBoth);

        assert_eq!(result.copied, 1);
        assert!(result.errors.is_empty());
        assert!(src.exists(), "copy must not remove the source");
        assert_eq!(
            std::fs::read_to_string(dest.join("a.txt")).unwrap(),
            "hello"
        );
    }

    #[test]
    fn cut_removes_the_source_only_after_the_destination_exists() {
        let t = Tmp::new("cut");
        let src = t.file("a.txt", "hello");
        let dest = t.dir("dest");

        let clip = Clipboard {
            paths: vec![src.clone()],
            cut: true,
        };
        let result = paste(&clip, &dest, OnConflict::KeepBoth);

        assert_eq!(result.moved, 1);
        assert!(!src.exists(), "cut removes the source");
        assert_eq!(
            std::fs::read_to_string(dest.join("a.txt")).unwrap(),
            "hello"
        );
    }

    #[test]
    fn keep_both_never_destroys_the_existing_file() {
        let t = Tmp::new("keepboth");
        let src = t.file("a.txt", "new");
        let dest = t.dir("dest");
        std::fs::write(dest.join("a.txt"), "existing").unwrap();

        let clip = Clipboard {
            paths: vec![src],
            cut: false,
        };
        let result = paste(&clip, &dest, OnConflict::KeepBoth);

        assert_eq!(result.copied, 1);
        assert_eq!(
            std::fs::read_to_string(dest.join("a.txt")).unwrap(),
            "existing",
            "the file that was there must survive"
        );
        assert_eq!(
            std::fs::read_to_string(dest.join("a 2.txt")).unwrap(),
            "new"
        );
    }

    #[test]
    fn the_numbered_suffix_goes_before_the_extension() {
        let t = Tmp::new("suffix");
        let dest = t.dir("dest");
        std::fs::write(dest.join("photo.png"), "x").unwrap();
        let src = t.file("photo.png", "y");

        paste(
            &Clipboard {
                paths: vec![src],
                cut: false,
            },
            &dest,
            OnConflict::KeepBoth,
        );
        assert!(dest.join("photo 2.png").exists(), "expected `photo 2.png`");
    }

    #[test]
    fn a_dotfile_is_not_treated_as_all_extension() {
        let t = Tmp::new("dotfile");
        let dest = t.dir("dest");
        std::fs::write(dest.join(".bashrc"), "x").unwrap();
        let src = t.file(".bashrc", "y");

        paste(
            &Clipboard {
                paths: vec![src],
                cut: false,
            },
            &dest,
            OnConflict::KeepBoth,
        );
        assert!(
            dest.join(".bashrc 2").exists(),
            "a leading dot is the name, not an extension"
        );
    }

    #[test]
    fn skip_leaves_both_sides_untouched() {
        let t = Tmp::new("skip");
        let src = t.file("a.txt", "new");
        let dest = t.dir("dest");
        std::fs::write(dest.join("a.txt"), "existing").unwrap();

        let result = paste(
            &Clipboard {
                paths: vec![src.clone()],
                cut: true,
            },
            &dest,
            OnConflict::Skip,
        );

        assert_eq!(result.skipped, 1);
        assert_eq!(result.moved, 0);
        assert!(src.exists(), "a skipped cut must not remove the source");
        assert_eq!(
            std::fs::read_to_string(dest.join("a.txt")).unwrap(),
            "existing"
        );
    }

    #[test]
    fn pasting_into_the_same_directory_duplicates_rather_than_self_replacing() {
        let t = Tmp::new("same");
        let src = t.file("a.txt", "body");

        let result = paste(
            &Clipboard {
                paths: vec![src.clone()],
                cut: false,
            },
            &t.0,
            OnConflict::Replace,
        );

        assert!(result.errors.is_empty());
        assert!(src.exists(), "the original must survive");
        assert!(t.0.join("a 2.txt").exists());
    }

    #[test]
    fn a_directory_cannot_be_pasted_into_itself() {
        let t = Tmp::new("recurse");
        let outer = t.dir("outer");
        let inner = outer.join("inner");
        std::fs::create_dir_all(&inner).unwrap();

        let result = paste(
            &Clipboard {
                paths: vec![outer.clone()],
                cut: false,
            },
            &inner,
            OnConflict::KeepBoth,
        );

        assert_eq!(result.copied, 0);
        assert_eq!(result.errors.len(), 1, "refused with a reason");
        assert!(result.errors[0].contains("itself"));
    }

    #[test]
    fn directories_copy_with_their_contents() {
        let t = Tmp::new("tree");
        let tree = t.dir("tree");
        std::fs::create_dir_all(tree.join("sub")).unwrap();
        std::fs::write(tree.join("sub/deep.txt"), "deep").unwrap();
        let dest = t.dir("dest");

        let result = paste(
            &Clipboard {
                paths: vec![tree],
                cut: false,
            },
            &dest,
            OnConflict::KeepBoth,
        );

        assert_eq!(result.copied, 1);
        assert_eq!(
            std::fs::read_to_string(dest.join("tree/sub/deep.txt")).unwrap(),
            "deep"
        );
    }

    #[test]
    fn a_symlink_is_copied_as_a_link_not_as_its_target() {
        let t = Tmp::new("symlink");
        let target = t.file("target.txt", "body");
        let link = t.0.join("link.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let dest = t.dir("dest");

        paste(
            &Clipboard {
                paths: vec![link],
                cut: false,
            },
            &dest,
            OnConflict::KeepBoth,
        );

        let copied = dest.join("link.txt");
        let meta = std::fs::symlink_metadata(&copied).unwrap();
        assert!(
            meta.is_symlink(),
            "following the link would duplicate its target"
        );
    }

    #[test]
    fn one_failure_does_not_abandon_the_rest() {
        let t = Tmp::new("partial");
        let good = t.file("good.txt", "g");
        let missing = t.0.join("does-not-exist.txt");
        let dest = t.dir("dest");

        let result = paste(
            &Clipboard {
                paths: vec![missing, good],
                cut: false,
            },
            &dest,
            OnConflict::KeepBoth,
        );

        assert_eq!(result.copied, 1, "the good file still went across");
        assert_eq!(result.errors.len(), 1, "and the bad one was reported");
        assert!(dest.join("good.txt").exists());
    }

    #[test]
    fn no_temporary_file_is_left_behind() {
        let t = Tmp::new("temp");
        let src = t.file("a.txt", "body");
        let dest = t.dir("dest");

        paste(
            &Clipboard {
                paths: vec![src],
                cut: false,
            },
            &dest,
            OnConflict::KeepBoth,
        );

        let leftovers: Vec<_> = std::fs::read_dir(&dest)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("otto-files-"))
            .collect();
        assert!(leftovers.is_empty(), "stray temp files: {leftovers:?}");
    }

    #[test]
    fn create_folder_names_the_first_one_plainly() {
        let t = Tmp::new("newfolder");
        let made = create_folder(&t.0).unwrap();
        assert_eq!(made.file_name().unwrap(), "untitled folder");
        assert!(made.is_dir());
    }

    #[test]
    fn create_folder_numbers_past_a_collision() {
        let t = Tmp::new("newfolder-collide");
        t.dir("untitled folder");
        let made = create_folder(&t.0).unwrap();
        assert_eq!(made.file_name().unwrap(), "untitled folder 2");
    }

    #[test]
    fn trash_moves_the_file_and_writes_a_sidecar() {
        let home = test_data_home();
        let t = Tmp::new("trash");
        let victim = t.file("gone.txt", "bye");

        let result = move_to_trash(std::slice::from_ref(&victim));

        assert_eq!(result.trashed, 1, "{:?}", result.errors);
        assert!(!victim.exists());
        // The can is shared with every other test in this binary, so the name
        // may have been numbered out of a collision; the change says which.
        let Some(Change::Trashed { to, info, .. }) = result.changes.first() else {
            panic!("no trashed change recorded: {:?}", result.changes);
        };
        assert!(to.starts_with(home.join("Trash/files")), "{to:?}");
        let trashed = to;
        assert!(trashed.exists());
        let contents = std::fs::read_to_string(info).unwrap();
        assert!(contents.starts_with("[Trash Info]\n"));
        assert!(contents.contains("DeletionDate="));
    }
}
