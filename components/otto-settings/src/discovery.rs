//! Discovers the valid choices for settings whose set is not fixed at
//! compile time — it depends on what is installed on this machine, not on
//! anything the compositor's schema can declare. `Describe` only marks six
//! settings as `enum`; the rest (fonts, cursor/icon/sound themes, the lock
//! and greeter commands) are `string` on the wire because the compositor
//! should not have to know what fonts a given machine has installed.
//!
//! Every lookup here touches the filesystem or the font manager, so results
//! are cached for the lifetime of the app: `open_menu` runs on a pointer
//! press and must not block visibly, and re-scanning `/usr/share/icons` on
//! every click would be a needless stat storm.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// One entry in a discovered dropdown: what the field shows, and what gets
/// sent in a `Set`. They differ exactly once — the auto-detect entry, whose
/// label says so but whose value is the empty string the compositor already
/// uses to mean "no override".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub label: String,
    pub value: String,
}

/// The setting ids this module knows how to discover choices for. Anything
/// else is not ours to answer — `open_menu` falls back to this only when the
/// served schema has no `choices` of its own.
pub fn choices_for(id: &str, current: &str) -> Option<Vec<Choice>> {
    if id == AUTO_LOCK_ID {
        return Some(auto_lock_choices(current));
    }
    let discovered: &'static [String] = match id {
        "font_family" => font_families(),
        "cursor_theme" => cursor_themes(),
        "icon_theme" => icon_themes(),
        "audio.sound_theme" => sound_themes(),
        "lock.locker_command" => locker_commands(),
        "login.greeter_command" => greeter_commands(),
        _ => return None,
    };

    // Nothing found, and nothing already set: there is genuinely nothing to
    // offer, so no menu — not a menu with zero rows.
    if discovered.is_empty() && current.is_empty() {
        return None;
    }

    Some(merge_with_current(discovered, current))
}

/// Combine a discovered, sorted, de-duplicated list with whatever is
/// currently set, so the menu always shows the live value even if discovery
/// missed it (a theme installed by hand outside the usual directories, a
/// locker command this list does not know about).
fn merge_with_current(discovered: &[String], current: &str) -> Vec<Choice> {
    let mut out = Vec::with_capacity(discovered.len() + 1);

    if current.is_empty() {
        // An empty effective value means "auto-detect", which is a real
        // state worth its own row rather than a blank one.
        out.push(Choice {
            label: otto_kit::t_owned!("settings-choice-automatic"),
            value: String::new(),
        });
    } else if !discovered.iter().any(|d| d == current) {
        out.push(Choice {
            label: current.to_string(),
            value: current.to_string(),
        });
    }

    out.extend(discovered.iter().map(|d| Choice {
        label: d.clone(),
        value: d.clone(),
    }));

    out
}

// ---------------------------------------------------------------------
// Fonts, from the font manager Skia renders with — on Linux that is
// fontconfig's view of the machine, the same one the compositor draws text
// from, without spawning `fc-list` for it.
// ---------------------------------------------------------------------

static FONT_FAMILIES: OnceLock<Vec<String>> = OnceLock::new();

fn font_families() -> &'static [String] {
    FONT_FAMILIES.get_or_init(|| sorted_families(otto_kit::skia::FontMgr::new().family_names()))
}

/// Family names, sorted for a picker: blanks dropped, each name once.
fn sorted_families(names: impl IntoIterator<Item = String>) -> Vec<String> {
    names
        .into_iter()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

// ---------------------------------------------------------------------
// Cursor and icon themes, per the XDG icon theme spec: a directory under an
// icon search path is a theme if it declares itself with the files the spec
// looks for. Sound themes follow the analogous freedesktop Sound Theme spec.
// ---------------------------------------------------------------------

static CURSOR_THEMES: OnceLock<Vec<String>> = OnceLock::new();
static ICON_THEMES: OnceLock<Vec<String>> = OnceLock::new();
static SOUND_THEMES: OnceLock<Vec<String>> = OnceLock::new();

fn cursor_themes() -> &'static [String] {
    CURSOR_THEMES.get_or_init(|| scan_themes(&icon_search_dirs(), is_cursor_theme_dir))
}

fn icon_themes() -> &'static [String] {
    ICON_THEMES.get_or_init(|| scan_themes(&icon_search_dirs(), is_icon_theme_dir))
}

fn sound_themes() -> &'static [String] {
    SOUND_THEMES.get_or_init(|| scan_themes(&sound_search_dirs(), is_sound_theme_dir))
}

/// A cursor theme is a directory containing a `cursors/` subdirectory of
/// cursor files (the XDG cursor spec). It need not have an `index.theme` —
/// plenty of hand-installed cursor themes skip it.
fn is_cursor_theme_dir(dir: &Path) -> bool {
    dir.join("cursors").is_dir()
}

/// An icon theme declares itself with `index.theme` (XDG icon theme spec).
fn is_icon_theme_dir(dir: &Path) -> bool {
    dir.join("index.theme").is_file()
}

/// A sound theme declares itself with `index.theme` too (freedesktop Sound
/// Theme spec, which mirrors the icon theme one).
fn is_sound_theme_dir(dir: &Path) -> bool {
    dir.join("index.theme").is_file()
}

fn icon_search_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/usr/share/icons"),
        PathBuf::from("/usr/local/share/icons"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.push(home.join(".local/share/icons"));
        dirs.push(home.join(".icons"));
    }
    if let Some(xdg_data_home) = std::env::var_os("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(xdg_data_home).join("icons"));
    }
    dirs
}

fn sound_search_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/usr/share/sounds"),
        PathBuf::from("/usr/local/share/sounds"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/sounds"));
    }
    if let Some(xdg_data_home) = std::env::var_os("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(xdg_data_home).join("sounds"));
    }
    dirs
}

/// Every subdirectory of `dirs` that `is_theme` accepts, named for its
/// directory basename, sorted and de-duplicated (the same theme often lives
/// under more than one search path).
fn scan_themes(dirs: &[PathBuf], is_theme: impl Fn(&Path) -> bool) -> Vec<String> {
    let mut names = BTreeSet::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() || !is_theme(&path) {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                names.insert(name.to_string());
            }
        }
    }
    names.into_iter().collect()
}

// ---------------------------------------------------------------------
// Lock and greeter commands: rather than list every executable on `PATH` —
// a menu of 3000 entries is not a menu — offer the plausible candidates and
// keep only the ones that actually exist.
// ---------------------------------------------------------------------

static LOCKER_COMMANDS: OnceLock<Vec<String>> = OnceLock::new();
static GREETER_COMMANDS: OnceLock<Vec<String>> = OnceLock::new();

/// Screen lockers otto might plausibly be pointed at: Otto's own, plus the
/// common wlroots/GTK ones.
const LOCKER_CANDIDATES: &[&str] = &["otto-lock", "swaylock", "gtklock", "hyprlock", "waylock"];

/// Greeters a display manager might plausibly hand off to: Otto's own, plus
/// the common greetd/LightDM/SDDM ones.
const GREETER_CANDIDATES: &[&str] = &[
    "otto-greeter",
    "gtkgreet",
    "regreet",
    "lightdm-gtk-greeter",
    "sddm-greeter",
];

fn locker_commands() -> &'static [String] {
    LOCKER_COMMANDS.get_or_init(|| find_on_path(LOCKER_CANDIDATES))
}

fn greeter_commands() -> &'static [String] {
    GREETER_COMMANDS.get_or_init(|| find_on_path(GREETER_CANDIDATES))
}

/// Which of `candidates` exist as executable files somewhere on `PATH`, in
/// candidate order (not sorted — the list is hand-curated and short enough
/// that "Otto's own first" reads better than alphabetical).
fn find_on_path(candidates: &[&str]) -> Vec<String> {
    let dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    find_in_dirs(candidates, &dirs)
}

fn find_in_dirs(candidates: &[&str], dirs: &[PathBuf]) -> Vec<String> {
    candidates
        .iter()
        .filter(|candidate| {
            dirs.iter()
                .any(|dir| is_executable_file(&dir.join(candidate)))
        })
        .map(|candidate| candidate.to_string())
        .collect()
}

fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

// ---------------------------------------------------------------------
// The auto-lock interval: an integer number of seconds on the wire, offered
// as a handful of intervals. Not discovered, but not the schema's to serve
// either — the setting takes any number of seconds, and these are only the
// ones worth a row in a menu.
// ---------------------------------------------------------------------

const AUTO_LOCK_ID: &str = "lock.auto_lock_timeout";

/// Seconds. 0 never locks.
const AUTO_LOCK_INTERVALS: &[u32] = &[0, 60, 120, 300, 600, 900, 1800, 3600];

/// The intervals, plus whatever is set now if it is not one of them (a value
/// written into `config.toml` by hand).
fn auto_lock_choices(current: &str) -> Vec<Choice> {
    let mut seconds: Vec<u32> = AUTO_LOCK_INTERVALS.to_vec();
    if let Ok(current) = current.trim().parse::<u32>() {
        if !seconds.contains(&current) {
            seconds.push(current);
            seconds.sort_unstable();
        }
    }
    seconds
        .into_iter()
        .map(|s| Choice {
            label: interval_label(s),
            value: s.to_string(),
        })
        .collect()
}

/// A name for `value` of `id` where the schema has none. Only the auto-lock
/// interval has one: seconds, shown as minutes.
pub fn label_for(id: &str, value: &str) -> Option<String> {
    if id != AUTO_LOCK_ID {
        return None;
    }
    value.trim().parse::<u32>().ok().map(interval_label)
}

fn interval_label(seconds: u32) -> String {
    match seconds {
        0 => otto_kit::t_owned!("settings-lock-never"),
        s if s % 3600 == 0 => {
            otto_kit::t_owned!("settings-interval-hours", count = f64::from(s / 3600))
        }
        s if s % 60 == 0 => {
            otto_kit::t_owned!("settings-interval-minutes", count = f64::from(s / 60))
        }
        s => otto_kit::t_owned!("settings-interval-seconds", count = f64::from(s)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn scratch_dir(label: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "otto-settings-discovery-test-{label}-{}-{n}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn font_families_are_sorted_once_each_without_blanks() {
        let names = sorted_families(
            [
                "Inter",
                "DejaVu Sans",
                "",
                " ",
                "DejaVu Sans",
                "Noto Sans CJK JP",
            ]
            .map(String::from),
        );
        assert_eq!(names, vec!["DejaVu Sans", "Inter", "Noto Sans CJK JP"]);
    }

    #[test]
    fn scan_themes_finds_only_matching_dirs_and_dedups_across_search_paths() {
        let a = scratch_dir("a");
        let b = scratch_dir("b");

        fs::create_dir_all(a.join("Adwaita/cursors")).unwrap();
        fs::create_dir_all(a.join("NotATheme")).unwrap();
        fs::File::create(a.join("not-a-dir")).unwrap();
        // Same theme name present under both search paths.
        fs::create_dir_all(b.join("Adwaita/cursors")).unwrap();
        fs::create_dir_all(b.join("Breeze/cursors")).unwrap();

        let found = scan_themes(&[a.clone(), b.clone()], is_cursor_theme_dir);
        assert_eq!(found, vec!["Adwaita", "Breeze"]);

        fs::remove_dir_all(&a).ok();
        fs::remove_dir_all(&b).ok();
    }

    #[test]
    fn scan_themes_recognises_icon_theme_via_index_theme() {
        let dir = scratch_dir("icons");
        fs::create_dir_all(dir.join("Papirus")).unwrap();
        fs::write(dir.join("Papirus/index.theme"), "[Icon Theme]\n").unwrap();
        fs::create_dir_all(dir.join("Incomplete")).unwrap();

        let found = scan_themes(std::slice::from_ref(&dir), is_icon_theme_dir);
        assert_eq!(found, vec!["Papirus"]);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scan_themes_missing_directory_yields_empty_not_a_panic() {
        let missing = PathBuf::from("/does/not/exist/otto-settings-test");
        assert!(scan_themes(&[missing], is_icon_theme_dir).is_empty());
    }

    #[test]
    fn find_in_dirs_keeps_candidate_order_and_skips_non_executables() {
        let dir = scratch_dir("bin");
        let exe = dir.join("otto-lock");
        fs::write(&exe, "#!/bin/sh\n").unwrap();
        let mut perms = fs::metadata(&exe).unwrap().permissions();
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
        fs::set_permissions(&exe, perms).unwrap();

        // Present but not executable: must not count.
        fs::write(dir.join("swaylock"), "not executable").unwrap();

        let found = find_in_dirs(
            &["otto-lock", "swaylock", "hyprlock"],
            std::slice::from_ref(&dir),
        );
        assert_eq!(found, vec!["otto-lock"]);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn auto_lock_offers_intervals_and_keeps_a_hand_written_one() {
        let offered = auto_lock_choices("600");
        assert_eq!(offered.first().map(|c| c.value.as_str()), Some("0"));
        assert!(offered.iter().any(|c| c.value == "600"));
        assert_eq!(offered.len(), AUTO_LOCK_INTERVALS.len());

        let odd = auto_lock_choices("450");
        assert_eq!(odd.len(), AUTO_LOCK_INTERVALS.len() + 1);
        let values: Vec<&str> = odd.iter().map(|c| c.value.as_str()).collect();
        assert!(values
            .windows(2)
            .all(|w| w[0].parse::<u32>().unwrap() < w[1].parse::<u32>().unwrap()));
    }

    #[test]
    fn only_the_auto_lock_interval_gets_a_label() {
        assert!(label_for("lock.auto_lock_timeout", "300").is_some());
        assert_eq!(label_for("cursor_theme", "300"), None);
        assert_eq!(label_for("lock.auto_lock_timeout", "soon"), None);
    }

    #[test]
    fn merge_with_current_marks_empty_current_as_automatic() {
        let discovered = vec!["Adwaita".to_string(), "Breeze".to_string()];
        let choices = merge_with_current(&discovered, "");
        // Against the catalogue, not against English: the label follows the
        // desktop's language, and this asserts which entry is first.
        assert_eq!(
            choices[0].label,
            otto_kit::t_owned!("settings-choice-automatic")
        );
        assert_eq!(choices[0].value, "");
        assert_eq!(choices.len(), 3);
    }

    #[test]
    fn merge_with_current_does_not_duplicate_a_current_value_already_discovered() {
        let discovered = vec!["Adwaita".to_string(), "Breeze".to_string()];
        let choices = merge_with_current(&discovered, "Breeze");
        assert_eq!(choices.len(), 2);
        assert!(choices.iter().any(|c| c.value == "Breeze"));
    }

    #[test]
    fn merge_with_current_adds_a_current_value_discovery_missed() {
        let discovered = vec!["Adwaita".to_string()];
        let choices = merge_with_current(&discovered, "HandInstalled");
        assert_eq!(choices[0].value, "HandInstalled");
        assert_eq!(choices.len(), 2);
    }

    #[test]
    fn choices_for_unknown_id_returns_none() {
        assert!(choices_for("dock.position", "bottom").is_none());
    }

    #[test]
    fn choices_for_never_returns_an_empty_menu() {
        // font_family discovery may legitimately find nothing in a minimal
        // test environment; with no current value either there is nothing
        // to show, and that must mean no menu rather than an empty one.
        if let Some(choices) = choices_for("font_family", "") {
            assert!(!choices.is_empty());
        }
    }
}
