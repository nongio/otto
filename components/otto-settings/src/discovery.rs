//! Discovers the valid choices for settings whose set is not fixed at
//! compile time — it depends on what is installed on this machine, not on
//! anything the compositor's schema can declare. `Describe` only marks six
//! settings as `enum`; the rest (fonts, cursor/icon/sound themes, the lock
//! and greeter commands) are `string` on the wire because the compositor
//! should not have to know what fonts a given machine has installed.
//!
//! Every lookup here touches the filesystem or spawns a process, so results
//! are cached for the lifetime of the app: `open_menu` runs on a pointer
//! press and must not block visibly, and re-scanning `/usr/share/icons` on
//! every click would be a needless stat storm.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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
// Fonts, via fontconfig's `fc-list`. The project deliberately keeps its
// dependency count low, and fontconfig's own crate pulls in a build-time
// bindgen dependency for what is, from here, one command's output — so this
// shells out instead of linking it.
// ---------------------------------------------------------------------

static FONT_FAMILIES: OnceLock<Vec<String>> = OnceLock::new();

fn font_families() -> &'static [String] {
    FONT_FAMILIES.get_or_init(|| {
        let output = match Command::new("fc-list").arg(":").arg("family").output() {
            Ok(output) if output.status.success() => output,
            Ok(output) => {
                eprintln!(
                    "settings: fc-list exited with {}; font list unavailable",
                    output.status
                );
                return Vec::new();
            }
            Err(err) => {
                eprintln!("settings: fc-list not available ({err}); font list unavailable");
                return Vec::new();
            }
        };
        parse_fc_list(&String::from_utf8_lossy(&output.stdout))
    })
}

/// Parse `fc-list : family` output: one family (or comma-separated aliases,
/// commonly a Latin name and a localised one) per line. Only the first alias
/// is kept — good enough for a picker, and it keeps the list from doubling
/// up on every CJK font.
fn parse_fc_list(text: &str) -> Vec<String> {
    let mut names = BTreeSet::new();
    for line in text.lines() {
        if let Some(first) = line.split(',').next() {
            let name = first.trim();
            if !name.is_empty() {
                names.insert(name.to_string());
            }
        }
    }
    names.into_iter().collect()
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
    fn parses_fc_list_output_taking_first_alias_and_dedup_sorting() {
        let text =
            "DejaVu Sans,DejaVu Sans\nNoto Sans CJK JP,Noto Sans CJK JP\nDejaVu Sans\nInter\n";
        let names = parse_fc_list(text);
        assert_eq!(names, vec!["DejaVu Sans", "Inter", "Noto Sans CJK JP"]);
    }

    #[test]
    fn parses_fc_list_output_ignoring_blank_lines() {
        let names = parse_fc_list("\n\nMono\n");
        assert_eq!(names, vec!["Mono"]);
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
    fn merge_with_current_marks_empty_current_as_automatic() {
        let discovered = vec!["Adwaita".to_string(), "Breeze".to_string()];
        let choices = merge_with_current(&discovered, "");
        assert_eq!(choices[0].label, "Automatic");
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
