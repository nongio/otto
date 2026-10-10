//! The Search pane.
//!
//! Shows what the file index is doing and edits what it looks at. What it
//! looks at is Otto's configuration (`[search]`), bound to `org.otto.Settings`
//! like any other row; the compositor pushes it on to LocalSearch. What it is
//! doing is LocalSearch's own runtime state, asked of it directly, as are
//! Start and Re-index.
//!
//! Asking blocks for a D-Bus round trip, so it runs on a thread of its own.
//! While the pane is on screen, that thread asks the indexer how it is doing
//! every couple of seconds, keeps the answer in [`snapshot`], and wakes the
//! main loop when it changes; the rows are built from the last answer and
//! never wait for one.

// Rust guideline compliant 2026-02-21

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use otto_search::{index, State, Status};

use crate::file_picker;
use crate::model::{group, untitled, Control, Pane, Row};
use crate::settings_client::{self, Value};

/// The user unit that runs the indexer, which Start starts.
const UNIT: &str = "localsearch-3.service";

/// How often the status row is refreshed while the pane is shown.
///
/// Often enough that the percentage moves while someone watches it, rarely
/// enough that a pane left open costs nothing: each refresh is a few D-Bus
/// calls to the indexer and one count query.
const POLL: Duration = Duration::from_secs(2);

/// The settings the pane edits.
const FOLDERS_ID: &str = "search.folders";
const SKIP_REPOS_ID: &str = "search.skip_code_repositories";
const REMOVABLE_ID: &str = "search.index_removable_drives";

/// The prefix of a folder row's identifier; the rest is the entry itself.
const FOLDER_ROW: &str = "search.folder:";

/// What became of the last Re-index press.
#[derive(Clone, Debug, Default, PartialEq)]
enum Reindex {
    #[default]
    NotAsked,
    Asked,
    Failed,
}

/// What became of the last folder chosen with Add Folder.
#[derive(Clone, Debug, PartialEq)]
enum Added {
    /// Already in the list.
    Duplicate(String),
    /// Under a folder in the list, named second.
    Covered(String, String),
    /// The picker could not be opened.
    Failed,
}

/// The last answers the background threads got.
#[derive(Clone, Debug, Default, PartialEq)]
struct Snapshot {
    /// `None` until the indexer has been asked once.
    status: Option<Status>,
    /// Files in the index, when it was running to count them.
    files: Option<u64>,
    reindex: Reindex,
    added: Option<Added>,
}

fn snapshot() -> &'static Mutex<Snapshot> {
    static SNAPSHOT: OnceLock<Mutex<Snapshot>> = OnceLock::new();
    SNAPSHOT.get_or_init(|| Mutex::new(Snapshot::default()))
}

/// Whether the pane is on screen, set by `main.rs` every update.
static SHOWN: AtomicBool = AtomicBool::new(false);
/// Whether the polling thread is alive.
static POLLING: AtomicBool = AtomicBool::new(false);
/// Set when the snapshot moved, until `main.rs` repaints for it.
static DIRTY: AtomicBool = AtomicBool::new(false);

/// Change the snapshot, and wake the window if that changed anything.
fn update(change: impl FnOnce(&mut Snapshot)) {
    let mut snapshot = snapshot().lock().unwrap();
    let before = snapshot.clone();
    change(&mut snapshot);
    if *snapshot != before {
        DIRTY.store(true, Ordering::Relaxed);
        otto_kit::AppContext::request_wakeup();
    }
}

/// Whether the pane's state moved since the last call; `on_update` polls
/// this to repaint.
pub fn take_dirty() -> bool {
    DIRTY.swap(false, Ordering::Relaxed)
}

/// Tell the pane whether it is on screen, which starts and stops the polling.
pub fn set_shown(shown: bool) {
    SHOWN.store(shown, Ordering::Relaxed);
    if !shown || POLLING.swap(true, Ordering::Relaxed) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("search-status".into())
        .spawn(|| loop {
            refresh();
            std::thread::sleep(POLL);
            if !SHOWN.load(Ordering::Relaxed) {
                POLLING.store(false, Ordering::Relaxed);
                // Shown again between the check and the store, with no
                // thread started for it: this one carries on.
                if SHOWN.load(Ordering::Relaxed) && !POLLING.swap(true, Ordering::Relaxed) {
                    continue;
                }
                return;
            }
        });
    if let Err(err) = spawned {
        POLLING.store(false, Ordering::Relaxed);
        eprintln!("search: could not watch the file index: {err}");
    }
}

/// Ask the indexer how it is doing.
fn refresh() {
    let status = index::status();
    // Counting starts LocalSearch when it is not running, so it is only
    // asked of one that is.
    let files = match status.state {
        State::Idle | State::Indexing | State::Paused => index::counts().ok().map(|c| c.files),
        State::Missing | State::Stopped => None,
    };
    update(|snapshot| {
        snapshot.status = Some(status);
        snapshot.files = files;
    });
}

/// Run `work` on a thread of its own, then look at the indexer again at once
/// rather than on the next poll.
fn in_background(name: &str, work: impl FnOnce() + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            work();
            refresh();
        });
    if let Err(err) = spawned {
        eprintln!("search: could not start {name}: {err}");
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// The configured folders, as `search.folders` holds them. `None` while the
/// compositor is not serving settings.
fn folders() -> Option<Vec<String>> {
    match settings_client::value(FOLDERS_ID) {
        Some(Value::List(items)) => Some(items),
        _ => None,
    }
}

fn set_folders(folders: Vec<String>) {
    if let settings_client::SetOutcome::Failed(why) =
        settings_client::set(FOLDERS_ID, Value::List(folders))
    {
        eprintln!("{FOLDERS_ID}: {why}");
    }
}

/// The folder a configured entry stands for.
///
/// `~` and `$HOME` are the home folder, a path under either is joined to it,
/// and a `&` keyword is the folder `user-dirs.dirs` names for it. `None` for a
/// keyword that names nothing.
fn expand(entry: &str, home: &Path, dirs: &str) -> Option<PathBuf> {
    if entry == "~" || entry == "$HOME" {
        return Some(home.to_path_buf());
    }
    if let Some(rest) = entry
        .strip_prefix("~/")
        .or_else(|| entry.strip_prefix("$HOME/"))
    {
        return Some(home.join(rest));
    }
    if let Some(keyword) = entry.strip_prefix('&') {
        return xdg_variable(keyword).and_then(|variable| user_dir(dirs, variable, home));
    }
    Some(PathBuf::from(entry))
}

/// How a chosen folder is written into the list: from `~` when it is under
/// the home folder, absolute otherwise.
fn entry_for(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// `folders` with `path` added, or why it was not.
///
/// A folder already in the list, or under one that is, is left out: the index
/// already looks there. The reason names both folders the way the pane does.
fn with_folder(
    folders: &[String],
    path: &Path,
    home: &Path,
    dirs: &str,
) -> Result<Vec<String>, Added> {
    let entry = entry_for(path, home);
    for existing in folders {
        let Some(covered) = expand(existing, home, dirs) else {
            continue;
        };
        if covered == path {
            return Err(Added::Duplicate(folder_name(&entry, home, dirs)));
        }
        if path.starts_with(&covered) {
            return Err(Added::Covered(
                folder_name(&entry, home, dirs),
                folder_name(existing, home, dirs),
            ));
        }
    }
    let mut next = folders.to_vec();
    next.push(entry);
    Ok(next)
}

/// `folders` without `entry`.
fn without_folder(folders: &[String], entry: &str) -> Vec<String> {
    folders
        .iter()
        .filter(|folder| folder.as_str() != entry)
        .cloned()
        .collect()
}

/// A folder row's identifier, which carries the entry it stands for.
fn folder_row_id(entry: &str) -> &'static str {
    intern(format!("{FOLDER_ROW}{entry}"))
}

/// `text` as a `&'static str`, kept once however often it is asked for: row
/// labels and identifiers are static, and folder names are not known until
/// the list is read.
fn intern(text: String) -> &'static str {
    static INTERNED: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let mut interned = INTERNED.get_or_init(Default::default).lock().unwrap();
    interned
        .entry(text)
        .or_insert_with_key(|text| text.clone().leak())
}

/// A press on a folder row's "−" button.
pub fn remove(id: &str) {
    let Some(entry) = id.strip_prefix(FOLDER_ROW) else {
        return;
    };
    if let Some(folders) = folders() {
        set_folders(without_folder(&folders, entry));
        update(|snapshot| snapshot.added = None);
    }
}

/// Ask for a folder through the desktop portal and add it, on a thread of its
/// own: the call blocks for as long as the dialog is up.
fn add_folder() {
    let spawned = std::thread::Builder::new()
        .name("search-add-folder".into())
        .spawn(|| {
            let title = otto_kit::t!("settings-search-choose-folder-title");
            let path = match file_picker::open_folder(title) {
                file_picker::Outcome::Chosen(paths) => paths.into_iter().next(),
                file_picker::Outcome::Dismissed => None,
                file_picker::Outcome::Failed(why) => {
                    eprintln!("search: {why}");
                    update(|snapshot| snapshot.added = Some(Added::Failed));
                    None
                }
            };
            let (Some(path), Some(folders)) = (path, folders()) else {
                return;
            };
            let outcome = match with_folder(&folders, &path, &home(), &user_dirs_file()) {
                Ok(next) => {
                    set_folders(next);
                    None
                }
                Err(why) => Some(why),
            };
            update(|snapshot| snapshot.added = outcome);
        });
    if let Err(err) = spawned {
        eprintln!("search: could not open the folder picker: {err}");
    }
}

/// A press on one of this pane's push buttons.
pub fn press(row: &str, button: &str) {
    if row == status_label() && button == start_label() {
        in_background("search-start", || {
            match std::process::Command::new("systemctl")
                .args(["--user", "start", UNIT])
                .status()
            {
                Ok(status) if status.success() => {}
                Ok(status) => eprintln!("search: systemctl start {UNIT} failed: {status}"),
                Err(err) => eprintln!("search: could not run systemctl: {err}"),
            }
        });
    } else if row == reindex_label() && button == reindex_button() {
        in_background("search-reindex", || {
            let outcome = match index::reindex(&home()) {
                Ok(()) => Reindex::Asked,
                Err(err) => {
                    eprintln!("search: {err}");
                    Reindex::Failed
                }
            };
            update(|snapshot| snapshot.reindex = outcome);
        });
    } else if row == add_label() && button == choose_label() {
        add_folder();
    }
}

/// `count` with its digits grouped in threes by `separator`: 48,210.
fn group_digits(count: u64, separator: &str) -> String {
    let digits = count.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3 * separator.len());
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push_str(separator);
        }
        grouped.push(digit);
    }
    grouped
}

/// Progress as a whole percentage, which reads 100 only once the indexer
/// says it is done.
fn percent(progress: f64) -> u32 {
    ((progress.clamp(0.0, 1.0) * 100.0).floor() as u32).min(99)
}

/// How long is left, as the status row words it.
#[derive(Debug, PartialEq, Eq)]
enum Left {
    Minutes(u64),
    Hours(u64),
}

/// Round `remaining` up to minutes, and to hours past an hour and a half,
/// where counting minutes suggests a precision the estimate does not have.
fn left(remaining: Duration) -> Left {
    let minutes = remaining.as_secs().div_ceil(60).max(1);
    if minutes < 90 {
        Left::Minutes(minutes)
    } else {
        Left::Hours((minutes + 30) / 60)
    }
}

/// The status row's value.
fn status_text(status: &Status, files: Option<u64>) -> String {
    match status.state {
        State::Idle => match files {
            Some(files) => otto_kit::t_owned!(
                "settings-search-idle",
                files = group_digits(files, otto_kit::t!("settings-search-digit-separator"))
            ),
            None => otto_kit::t_owned!("settings-search-idle-uncounted"),
        },
        State::Indexing => {
            let percent = percent(status.progress);
            match status.remaining.map(left) {
                Some(Left::Minutes(minutes)) => otto_kit::t_owned!(
                    "settings-search-indexing-minutes",
                    percent = percent,
                    minutes = minutes
                ),
                Some(Left::Hours(hours)) => otto_kit::t_owned!(
                    "settings-search-indexing-hours",
                    percent = percent,
                    hours = hours
                ),
                None => otto_kit::t_owned!("settings-search-indexing", percent = percent),
            }
        }
        State::Paused => otto_kit::t_owned!("settings-search-paused"),
        State::Stopped => otto_kit::t_owned!("settings-search-stopped"),
        State::Missing => otto_kit::t_owned!("settings-search-missing"),
    }
}

/// The folder variable `user-dirs.dirs` sets for one of LocalSearch's `&`
/// keywords.
fn xdg_variable(keyword: &str) -> Option<&'static str> {
    Some(match keyword {
        "DESKTOP" => "XDG_DESKTOP_DIR",
        "DOCUMENTS" => "XDG_DOCUMENTS_DIR",
        "DOWNLOAD" => "XDG_DOWNLOAD_DIR",
        "MUSIC" => "XDG_MUSIC_DIR",
        "PICTURES" => "XDG_PICTURES_DIR",
        "VIDEOS" => "XDG_VIDEOS_DIR",
        "PUBLIC_SHARE" => "XDG_PUBLICSHARE_DIR",
        "TEMPLATES" => "XDG_TEMPLATES_DIR",
        _ => return None,
    })
}

/// The folder `variable` names in the text of a `user-dirs.dirs` file,
/// `$HOME` expanded.
fn user_dir(dirs: &str, variable: &str, home: &Path) -> Option<PathBuf> {
    otto_kit::xdg::parse_user_dirs(dirs, home)
        .into_iter()
        .find_map(|(key, path)| (key == variable).then_some(path))
}

/// The `user-dirs.dirs` file, empty when there is none.
fn user_dirs_file() -> String {
    otto_kit::xdg::config_home()
        .and_then(|config| std::fs::read_to_string(config.join("user-dirs.dirs")).ok())
        .unwrap_or_default()
}

/// One entry of LocalSearch's folder lists as the pane names it.
///
/// `$HOME` is Home, a `&` keyword is the folder it stands for by its own name
/// (Desktop, Downloads), and anything under the home folder is written from
/// `~`.
fn folder_name(entry: &str, home: &Path, dirs: &str) -> String {
    if entry == "~" || entry == "$HOME" || Path::new(entry) == home {
        return otto_kit::t_owned!("settings-search-folder-home");
    }
    if let Some(keyword) = entry.strip_prefix('&') {
        let found = xdg_variable(keyword).and_then(|variable| user_dir(dirs, variable, home));
        return match found.as_deref().and_then(Path::file_name) {
            Some(name) => name.to_string_lossy().into_owned(),
            None => keyword.to_string(),
        };
    }
    let path = match entry
        .strip_prefix("~/")
        .or_else(|| entry.strip_prefix("$HOME"))
    {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(entry),
    };
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

fn status_label() -> &'static str {
    otto_kit::t!("settings-search-index")
}
fn start_label() -> &'static str {
    otto_kit::t!("settings-search-start")
}
fn reindex_label() -> &'static str {
    otto_kit::t!("settings-search-reindex")
}
fn reindex_button() -> &'static str {
    otto_kit::t!("settings-search-reindex-button")
}

fn start_buttons() -> &'static [&'static str] {
    static BUTTONS: OnceLock<Vec<&'static str>> = OnceLock::new();
    BUTTONS.get_or_init(|| vec![start_label()])
}

fn reindex_buttons() -> &'static [&'static str] {
    static BUTTONS: OnceLock<Vec<&'static str>> = OnceLock::new();
    BUTTONS.get_or_init(|| vec![reindex_button()])
}

fn add_label() -> &'static str {
    otto_kit::t!("settings-search-add-folder")
}
fn choose_label() -> &'static str {
    otto_kit::t!("settings-search-choose")
}

fn choose_buttons() -> &'static [&'static str] {
    static BUTTONS: OnceLock<Vec<&'static str>> = OnceLock::new();
    BUTTONS.get_or_init(|| vec![choose_label()])
}

/// The row saying what the indexer is doing.
fn status_row(snapshot: &Snapshot) -> Row {
    let label = status_label();
    let Some(status) = snapshot.status else {
        return Row::new(
            label,
            Control::Value(otto_kit::t_owned!("settings-search-checking")),
        );
    };
    let text = status_text(&status, snapshot.files);
    match status.state {
        State::Stopped => Row::new(label, Control::Button(start_buttons())).detail(text),
        State::Missing => Row::new(label, Control::Value(text))
            .detail(otto_kit::t!("settings-search-missing-detail")),
        State::Paused => Row::new(label, Control::Value(text))
            .detail(otto_kit::t!("settings-search-paused-detail")),
        State::Idle | State::Indexing => Row::new(label, Control::Value(text)),
    }
}

/// One row per configured folder, each with its "−", then Add Folder.
fn folder_rows(folders: &[String], added: Option<&Added>) -> Vec<Row> {
    let home = home();
    let dirs = user_dirs_file();
    let mut rows: Vec<Row> = folders
        .iter()
        .map(|entry| {
            let mut row = Row::new(
                intern(folder_name(entry, &home, &dirs)),
                Control::Value(String::new()),
            )
            .removable(true);
            row.id = Some(folder_row_id(entry));
            row
        })
        .collect();
    if rows.is_empty() {
        rows.push(
            Row::new(
                otto_kit::t!("settings-search-folders"),
                Control::Value(otto_kit::t_owned!("settings-search-folders-none")),
            )
            .detail(otto_kit::t!("settings-search-folders-empty")),
        );
    }
    let detail = match added {
        None => otto_kit::t_owned!("settings-search-add-folder-detail"),
        Some(Added::Duplicate(folder)) => {
            otto_kit::t_owned!("settings-search-folder-duplicate", folder = folder.as_str())
        }
        Some(Added::Covered(folder, parent)) => otto_kit::t_owned!(
            "settings-search-folder-covered",
            folder = folder.as_str(),
            parent = parent.as_str()
        ),
        Some(Added::Failed) => otto_kit::t_owned!("settings-search-picker-failed"),
    };
    rows.push(Row::new(add_label(), Control::Button(choose_buttons())).detail(detail));
    rows
}

pub fn build() -> Pane {
    let snapshot = snapshot().lock().unwrap().clone();
    let name = otto_kit::t!("settings-pane-search");
    let intro = Some(otto_kit::t!("settings-search-intro"));

    let mut groups = vec![untitled(vec![status_row(&snapshot)])];

    let missing = snapshot
        .status
        .is_some_and(|status| status.state == State::Missing);
    if !missing {
        let running = snapshot.status.is_some_and(|status| {
            matches!(status.state, State::Idle | State::Indexing | State::Paused)
        });
        let reindex_detail = match snapshot.reindex {
            Reindex::NotAsked => otto_kit::t!("settings-search-reindex-detail"),
            Reindex::Asked => otto_kit::t!("settings-search-reindex-asked"),
            Reindex::Failed => otto_kit::t!("settings-search-reindex-failed"),
        };

        let mut rows = folders()
            .map(|folders| folder_rows(&folders, snapshot.added.as_ref()))
            .unwrap_or_default();
        rows.extend([
            Row::new(
                otto_kit::t!("settings-search-skip-repos"),
                Control::Toggle(true),
            )
            .detail(otto_kit::t!("settings-search-skip-repos-detail"))
            .id(SKIP_REPOS_ID),
            Row::new(
                otto_kit::t!("settings-search-removable"),
                Control::Toggle(false),
            )
            .detail(otto_kit::t!("settings-search-removable-detail"))
            .id(REMOVABLE_ID),
            Row::new(reindex_label(), Control::Button(reindex_buttons()))
                .detail(reindex_detail)
                .inactive(!running),
        ]);
        groups.push(group(otto_kit::t!("settings-search-looks-in"), rows));
    }

    Pane {
        name,
        icon: "search",
        intro,
        groups,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    const DIRS: &str = "XDG_DESKTOP_DIR=\"$HOME/Scrivania\"\n\
                        XDG_MUSIC_DIR=\"/srv/music\"\n";

    #[test]
    fn a_new_folder_is_added_from_home() {
        let home = Path::new("/home/ada");
        let folders = strings(&["/mnt/data"]);
        assert_eq!(
            with_folder(&folders, Path::new("/home/ada/My Notes"), home, DIRS),
            Ok(strings(&["/mnt/data", "~/My Notes"]))
        );
        assert_eq!(
            with_folder(&folders, home, home, DIRS),
            Ok(strings(&["/mnt/data", "~"]))
        );
        assert_eq!(
            with_folder(&[], Path::new("/srv/Ürün"), home, DIRS),
            Ok(strings(&["/srv/Ürün"]))
        );
    }

    #[test]
    fn a_folder_already_searched_is_not_added_twice() {
        let home = Path::new("/home/ada");
        let folders = strings(&["~/src", "&MUSIC"]);
        assert_eq!(
            with_folder(&folders, Path::new("/home/ada/src"), home, DIRS),
            Err(Added::Duplicate("~/src".into()))
        );
        assert_eq!(
            with_folder(&folders, Path::new("/srv/music"), home, DIRS),
            Err(Added::Duplicate("/srv/music".into()))
        );
    }

    #[test]
    fn a_folder_under_one_searched_is_not_added() {
        let home = Path::new("/home/ada");
        assert_eq!(
            with_folder(
                &strings(&["~"]),
                Path::new("/home/ada/src/otto"),
                home,
                DIRS
            ),
            Err(Added::Covered("~/src/otto".into(), "Home".into()))
        );
        assert_eq!(
            with_folder(
                &strings(&["&DESKTOP"]),
                Path::new("/home/ada/Scrivania/tmp"),
                home,
                DIRS
            ),
            Err(Added::Covered("~/Scrivania/tmp".into(), "Scrivania".into()))
        );
        // A sibling with a common prefix is not under it.
        assert_eq!(
            with_folder(
                &strings(&["~/src"]),
                Path::new("/home/ada/srcs"),
                home,
                DIRS
            ),
            Ok(strings(&["~/src", "~/srcs"]))
        );
    }

    #[test]
    fn removing_takes_out_only_that_folder() {
        let folders = strings(&["~", "/mnt/data", "&DESKTOP"]);
        assert_eq!(
            without_folder(&folders, "/mnt/data"),
            strings(&["~", "&DESKTOP"])
        );
        assert_eq!(without_folder(&strings(&["~"]), "~"), Vec::<String>::new());
        assert_eq!(without_folder(&folders, "/elsewhere"), folders);
    }

    #[test]
    fn a_folder_row_names_its_entry() {
        let id = folder_row_id("~/it's here");
        assert_eq!(id.strip_prefix(FOLDER_ROW), Some("~/it's here"));
        assert!(std::ptr::eq(id, folder_row_id("~/it's here")));
    }

    #[test]
    fn counts_are_grouped_in_threes() {
        assert_eq!(group_digits(0, ","), "0");
        assert_eq!(group_digits(999, ","), "999");
        assert_eq!(group_digits(1000, ","), "1,000");
        assert_eq!(group_digits(48_210, ","), "48,210");
        assert_eq!(group_digits(1_234_567, "."), "1.234.567");
        assert_eq!(group_digits(123_456, "\u{202f}"), "123\u{202f}456");
    }

    #[test]
    fn progress_reads_100_only_when_done() {
        assert_eq!(percent(0.0), 0);
        assert_eq!(percent(0.625), 62);
        assert_eq!(percent(0.999), 99);
        assert_eq!(percent(1.0), 99);
        assert_eq!(percent(-1.0), 0);
    }

    #[test]
    fn time_left_rounds_up_to_minutes_then_hours() {
        assert_eq!(left(Duration::from_secs(1)), Left::Minutes(1));
        assert_eq!(left(Duration::from_secs(60)), Left::Minutes(1));
        assert_eq!(left(Duration::from_secs(61)), Left::Minutes(2));
        assert_eq!(left(Duration::from_secs(300)), Left::Minutes(5));
        assert_eq!(left(Duration::from_secs(89 * 60)), Left::Minutes(89));
        assert_eq!(left(Duration::from_secs(90 * 60)), Left::Hours(2));
        assert_eq!(left(Duration::from_secs(4 * 3600)), Left::Hours(4));
    }

    #[test]
    fn folders_are_named_the_way_people_know_them() {
        let home = Path::new("/home/ada");
        let dirs = "# written by xdg-user-dirs-update\n\
                    XDG_DESKTOP_DIR=\"$HOME/Scrivania\"\n\
                    XDG_DOWNLOAD_DIR=\"$HOME/Downloads\"\n\
                    XDG_MUSIC_DIR=\"/srv/music\"\n";
        assert_eq!(folder_name("&DESKTOP", home, dirs), "Scrivania");
        assert_eq!(folder_name("&DOWNLOAD", home, dirs), "Downloads");
        assert_eq!(folder_name("&MUSIC", home, dirs), "music");
        // Not in the file: the keyword itself rather than nothing.
        assert_eq!(folder_name("&VIDEOS", home, dirs), "VIDEOS");
        assert_eq!(folder_name("$HOME/src", home, dirs), "~/src");
        assert_eq!(folder_name("~", home, dirs), "Home");
        assert_eq!(folder_name("~/src", home, dirs), "~/src");
        assert_eq!(folder_name("/home/ada/notes", home, dirs), "~/notes");
        assert_eq!(folder_name("/mnt/data", home, dirs), "/mnt/data");
    }
}
