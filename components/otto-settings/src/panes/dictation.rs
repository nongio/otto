//! The dictation pane.
//!
//! Dictation runs inside otto-kit apps (the launcher's Ctrl+D) and the
//! otto-dictate input method, not in the compositor, so it serves no setting
//! on the bus. Its settings are its own file, `dictation.toml` in Otto's
//! config folder, which otto-kit's `dictation::Config` reads as each
//! dictation starts (see `specs/dictation.md`). This pane writes it with
//! `toml_edit`, one key at a time, so comments and keys it does not show stay
//! as they were. A change applies to the next dictation; nothing reloads.
//!
//! The engine is also a systemd user unit, `otto-stt-<engine>.service`. The
//! units conflict, so starting one stops the others; picking an engine
//! enables and starts its unit and disables the rest, so the next login
//! brings back the same one. The server row says how the picked engine's
//! unit is doing, asked on a thread while the pane is on screen, the way the
//! Agents pane watches its service.
//!
//! *Start dictation at login* is otto-dictate's XDG autostart entry, see
//! [`set_autostart`].

// Rust guideline compliant 2026-02-21

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use toml_edit::DocumentMut;

use crate::discovery::Choice;
use crate::model::{untitled, Control, Pane, Row};
use crate::settings_client::Value;

/// The Engine pop-up. Like every identifier here it is the pane's own: the
/// settings store has never heard of it.
const ENGINE_ID: &str = "dictation.engine";
/// The Language pop-up.
const LANGUAGE_ID: &str = "dictation.language";
/// The Hotword boost slider.
const BOOST_ID: &str = "dictation.hotwords_boost";
/// The Start dictation at login switch.
const AUTOSTART_ID: &str = "dictation.autostart";
/// The speech server row, keyed by an identifier so its press cannot be
/// taken for another pane's row of the same label.
const SERVER_ID: &str = "dictation.server";

/// The engines `dictation.toml` names, the default first. Each is served by
/// `otto-stt-<engine>.service`.
const ENGINES: [&str; 3] = ["parakeet", "whisper", "crispasr"];

/// The engine that takes hotwords, and so the one the boost is shown for.
const HOTWORD_ENGINE: &str = "crispasr";

/// The languages the pop-up offers after Automatic: those Otto has a
/// catalogue for, by their ISO 639-1 code and their own name. A language
/// set by hand or taken from `LANG` that is not here still works, and shows
/// as its code.
const LANGUAGES: [(&str, &str); 11] = [
    ("en", "English"),
    ("de", "Deutsch"),
    ("es", "Español"),
    ("fr", "Français"),
    ("it", "Italiano"),
    ("ja", "日本語"),
    ("pl", "Polski"),
    ("pt", "Português"),
    ("ru", "Русский"),
    ("uk", "Українська"),
    ("zh", "中文"),
];

/// What `language` says to let the engine detect the language.
const AUTO: &str = "auto";

/// The boost when the file sets none, otto-kit's own default.
const DEFAULT_BOOST: f32 = 4.0;
/// The slider's range. Past about 6 CrispASR garbles the words around a
/// name, so the top of the range is already too much; it is there to try.
const BOOSTS: (f32, f32) = (1.0, 8.0);
/// The slider moves in steps of this much.
const BOOST_STEP: f32 = 0.5;

/// The unit that serves `engine`.
fn unit(engine: &str) -> String {
    format!("otto-stt-{engine}.service")
}

// ---------------------------------------------------------------------------
// dictation.toml
// ---------------------------------------------------------------------------

/// What the pane shows of `dictation.toml`, each key `None` when unset.
#[derive(Debug, Clone, Default, PartialEq)]
struct DictationFile {
    /// One of [`ENGINES`]; an engine the pane does not know is ignored.
    engine: Option<&'static str>,
    language: Option<String>,
    hotwords_boost: Option<f32>,
}

/// Read what the pane shows of `text`. A file that does not parse shows as
/// unset, as otto-kit reads it.
fn read_file(text: &str) -> DictationFile {
    let Ok(doc) = text.parse::<DocumentMut>() else {
        return DictationFile::default();
    };
    let string = |key: &str| {
        doc.get(key)
            .and_then(|item| item.as_str())
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
    };
    DictationFile {
        engine: string("engine").and_then(|engine| ENGINES.into_iter().find(|e| *e == engine)),
        language: string("language"),
        hotwords_boost: doc.get("hotwords_boost").and_then(|item| {
            item.as_float()
                .or_else(|| item.as_integer().map(|n| n as f64))
                .map(|n| n as f32)
        }),
    }
}

/// `$XDG_CONFIG_HOME/otto/dictation.toml`, the file otto-kit reads.
fn config_path() -> Option<PathBuf> {
    otto_kit::xdg::otto_config_file("dictation.toml")
}

/// The file as last read, with its modification time then, so it is only
/// read again once something has written it.
type Cache = Option<(Option<SystemTime>, DictationFile)>;

fn cached() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// `dictation.toml`, read again whenever it has changed.
fn dictation_file() -> DictationFile {
    let path = config_path();
    let modified = path
        .as_deref()
        .and_then(|path| std::fs::metadata(path).ok())
        .and_then(|meta| meta.modified().ok());
    let mut cache = cached().lock().unwrap();
    if let Some((seen, file)) = cache.as_ref() {
        if *seen == modified {
            return file.clone();
        }
    }
    let file = path
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| read_file(&text))
        .unwrap_or_default();
    *cache = Some((modified, file.clone()));
    file
}

/// `text` with `key` set to `value`, everything else kept as it was. An
/// existing key has its value replaced in place, so the comment above it
/// stays with it.
///
/// # Errors
///
/// Fails when `text` is not TOML, so a file that does not parse is never
/// replaced by one that has lost everything in it.
fn with_key(text: &str, key: &str, value: toml_edit::Value) -> Result<String, String> {
    let mut doc = text.parse::<DocumentMut>().map_err(|err| err.to_string())?;
    match doc.get_mut(key).and_then(|item| item.as_value_mut()) {
        Some(old) => {
            // Keep the spacing and any comment trailing the old value.
            let decor = old.decor().clone();
            *old = value;
            *old.decor_mut() = decor;
        }
        None => {
            doc.insert(key, toml_edit::Item::Value(value));
        }
    }
    Ok(doc.to_string())
}

/// Set `key` in `dictation.toml` and remember what it now says.
fn write(key: &str, value: toml_edit::Value) {
    let Some(path) = config_path() else {
        eprintln!("dictation: no home folder to keep dictation.toml in");
        return;
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            eprintln!("dictation: cannot read {}: {err}", path.display());
            return;
        }
    };
    let next = match with_key(&text, key, value) {
        Ok(next) => next,
        Err(err) => {
            eprintln!(
                "dictation: {} does not parse, leaving it alone: {err}",
                path.display()
            );
            return;
        }
    };
    if let Err(err) = write_atomically(&path, &next) {
        eprintln!("dictation: cannot write {}: {err}", path.display());
        return;
    }
    *cached().lock().unwrap() = None;
    wake();
}

/// Replace `path` with `text` in one step, so a dictation starting just then
/// never reads it half written.
fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, path)
}

/// Set when what the pane shows moved off the main thread, until `main.rs`
/// repaints for it.
static DIRTY: AtomicBool = AtomicBool::new(false);

fn wake() {
    DIRTY.store(true, Ordering::Relaxed);
    otto_kit::AppContext::request_wakeup();
}

/// Whether the pane changed since the last call; `main.rs` polls this to
/// repaint.
pub fn take_dirty() -> bool {
    DIRTY.swap(false, Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// The engines' units
// ---------------------------------------------------------------------------

/// What systemd says about one engine's unit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Service {
    /// Not asked yet.
    Checking,
    Running,
    Stopped,
    /// Stopped by an error rather than by someone.
    Failed,
    /// No such unit: the engine is not installed.
    Missing,
    /// No `systemctl` to ask: a session without systemd.
    Unmanaged,
}

/// One engine's unit as last seen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Unit {
    service: Service,
    /// Enabled to start with the session.
    enabled: bool,
}

impl Unit {
    const CHECKING: Unit = Unit {
        service: Service::Checking,
        enabled: false,
    };
}

/// Read `systemctl show -p Id,LoadState,ActiveState,UnitFileState` for the
/// engines' units: one block per unit, separated by a blank line, each
/// known by its `Id`. An engine with no block keeps `Checking`.
fn parse_units(show: &str) -> [Unit; 3] {
    let mut units = [Unit::CHECKING; 3];
    for block in show.split("\n\n") {
        let value = |key: &str| {
            block
                .lines()
                .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
                .unwrap_or_default()
        };
        let Some(index) = ENGINES.iter().position(|e| unit(e) == value("Id")) else {
            continue;
        };
        let service = match (value("LoadState"), value("ActiveState")) {
            ("not-found", _) => Service::Missing,
            (_, "active" | "reloading" | "activating") => Service::Running,
            (_, "failed") => Service::Failed,
            _ => Service::Stopped,
        };
        units[index] = Unit {
            service,
            enabled: value("UnitFileState").starts_with("enabled"),
        };
    }
    units
}

fn units() -> &'static Mutex<[Unit; 3]> {
    static UNITS: OnceLock<Mutex<[Unit; 3]>> = OnceLock::new();
    UNITS.get_or_init(|| Mutex::new([Unit::CHECKING; 3]))
}

/// Ask systemd about every engine's unit, and wake the window if the answer
/// changed. Returns whether there is a systemd to ask.
fn refresh_units() -> bool {
    let names: Vec<String> = ENGINES.iter().map(|e| unit(e)).collect();
    let found = match std::process::Command::new("systemctl")
        .args([
            "--user",
            "show",
            "-p",
            "Id,LoadState,ActiveState,UnitFileState",
        ])
        .args(&names)
        .output()
    {
        Ok(output) if output.status.success() => {
            parse_units(&String::from_utf8_lossy(&output.stdout))
        }
        // There, but with no user manager to answer; or not there at all.
        _ => {
            [Unit {
                service: Service::Unmanaged,
                enabled: false,
            }; 3]
        }
    };
    let managed = found[0].service != Service::Unmanaged;
    let mut units = units().lock().unwrap();
    if *units != found {
        *units = found;
        drop(units);
        wake();
    }
    managed
}

/// The engine the pane shows as picked: the file's, else the one running,
/// else the one enabled, else the default. `install.sh` picks an engine
/// without writing the file, and the pane should show the one it picked.
fn current_engine(file: &DictationFile, units: &[Unit; 3]) -> &'static str {
    file.engine
        .or_else(|| {
            let index = units
                .iter()
                .position(|u| u.service == Service::Running)
                .or_else(|| units.iter().position(|u| u.enabled))?;
            Some(ENGINES[index])
        })
        .unwrap_or(ENGINES[0])
}

/// How often the units are asked about while the pane is on screen.
const POLL: std::time::Duration = std::time::Duration::from_secs(5);

/// Whether the pane is on screen.
static SHOWN: AtomicBool = AtomicBool::new(false);
/// Whether a watcher thread is running.
static WATCHING: AtomicBool = AtomicBool::new(false);

/// Tell the pane whether it is on screen, which starts and stops keeping the
/// units' state current: a poll every five seconds on a thread of its own,
/// as the Agents pane does, so the draw path never runs a process and a
/// hidden pane asks nothing.
pub fn set_shown(shown: bool) {
    SHOWN.store(shown, Ordering::Relaxed);
    // Without systemd the answer cannot change, so it is not asked again.
    let unmanaged = units().lock().unwrap()[0].service == Service::Unmanaged;
    if !shown || unmanaged || WATCHING.swap(true, Ordering::Relaxed) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("dictation-units".into())
        .spawn(|| loop {
            if !refresh_units() {
                WATCHING.store(false, Ordering::Relaxed);
                return;
            }
            std::thread::sleep(POLL);
            if !SHOWN.load(Ordering::Relaxed) {
                WATCHING.store(false, Ordering::Relaxed);
                // Shown again between the check and the store, with no
                // thread started for it: this one carries on.
                if SHOWN.load(Ordering::Relaxed) && !WATCHING.swap(true, Ordering::Relaxed) {
                    continue;
                }
                return;
            }
        });
    if let Err(err) = spawned {
        WATCHING.store(false, Ordering::Relaxed);
        eprintln!("dictation: could not watch the engines: {err}");
    }
}

/// Run `systemctl --user` with each of `calls` in turn on a thread of its
/// own, then look at the units again at once rather than on the next poll.
fn systemctl(calls: Vec<Vec<String>>) {
    let spawned = std::thread::Builder::new()
        .name("dictation-systemctl".into())
        .spawn(move || {
            for args in calls {
                match std::process::Command::new("systemctl")
                    .arg("--user")
                    .args(&args)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                {
                    Ok(status) if status.success() => {}
                    Ok(status) => {
                        eprintln!("dictation: systemctl {} failed: {status}", args.join(" "))
                    }
                    Err(err) => eprintln!("dictation: could not run systemctl: {err}"),
                }
            }
            let _ = refresh_units();
        });
    if let Err(err) = spawned {
        eprintln!("dictation: could not run systemctl: {err}");
    }
}

/// What switching to `engine` asks of systemd: the others disabled, so the
/// next login starts the same one, then its unit enabled and started, which
/// stops whichever other one was running through `Conflicts=`. One call per
/// unit, so an engine that is not installed does not stop the rest.
fn switch_calls(engine: &str) -> Vec<Vec<String>> {
    ENGINES
        .iter()
        .filter(|other| **other != engine)
        .map(|other| vec!["disable".to_string(), unit(other)])
        .chain(std::iter::once(vec![
            "enable".to_string(),
            "--now".to_string(),
            unit(engine),
        ]))
        .collect()
}

// ---------------------------------------------------------------------------
// The autostart entry
// ---------------------------------------------------------------------------

/// The entry's file name, the same in every autostart folder.
const AUTOSTART_FILE: &str = "otto-dictate.desktop";

/// The user's entry, `$XDG_CONFIG_HOME/autostart/otto-dictate.desktop`.
fn user_entry() -> Option<PathBuf> {
    Some(
        otto_kit::xdg::config_home()?
            .join("autostart")
            .join(AUTOSTART_FILE),
    )
}

/// The system's entry a package may install, from the first of
/// `$XDG_CONFIG_DIRS` that has one.
fn system_entry() -> Option<String> {
    otto_kit::xdg::config_dirs()
        .into_iter()
        .find_map(|dir| std::fs::read_to_string(dir.join("autostart").join(AUTOSTART_FILE)).ok())
}

/// `key`'s value in the `[Desktop Entry]` group of `entry`.
fn entry_key<'a>(entry: &'a str, key: &str) -> Option<&'a str> {
    let mut in_group = false;
    for line in entry.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
        } else if in_group {
            if let Some((name, value)) = line.split_once('=') {
                if name.trim() == key {
                    return Some(value.trim());
                }
            }
        }
    }
    None
}

/// Whether `entry` starts with the session: not `Hidden`, which the
/// autostart spec reads as deleted, and not switched off the GNOME way.
fn entry_starts(entry: &str) -> bool {
    entry_key(entry, "Hidden") != Some("true")
        && entry_key(entry, "X-GNOME-Autostart-enabled") != Some("false")
}

/// Whether otto-dictate starts at login: the user's entry decides where
/// there is one, since it overrides the system's of the same name, and the
/// system's where there is not.
fn autostart(user: Option<&str>, system: Option<&str>) -> bool {
    user.or(system).is_some_and(entry_starts)
}

/// `entry` with `key` set to `value` in its `[Desktop Entry]` group, added at
/// the group's end when it is not there.
fn with_entry_key(entry: &str, key: &str, value: &str) -> String {
    let mut lines: Vec<String> = entry.lines().map(str::to_string).collect();
    let mut in_group = false;
    let mut group_end = None;
    for (index, line) in lines.iter_mut().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_group = trimmed == "[Desktop Entry]";
            if in_group {
                group_end = Some(index + 1);
            }
            continue;
        }
        if !in_group {
            continue;
        }
        if trimmed
            .split_once('=')
            .is_some_and(|(name, _)| name.trim() == key)
        {
            *line = format!("{key}={value}");
            return lines.join("\n") + "\n";
        }
        if !trimmed.is_empty() {
            group_end = Some(index + 1);
        }
    }
    match group_end {
        Some(at) => lines.insert(at, format!("{key}={value}")),
        None => {
            lines.push("[Desktop Entry]".into());
            lines.push(format!("{key}={value}"));
        }
    }
    lines.join("\n") + "\n"
}

/// An entry for a user without one, running otto-dictate from where
/// `install.sh` puts it, or from `PATH` when it is not there.
fn new_entry(home: Option<&Path>) -> String {
    let exec = home
        .map(|home| home.join(".local/bin/otto-dictate"))
        .filter(|path| path.is_file())
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "otto-dictate".into());
    format!(
        "[Desktop Entry]\nType=Application\nName=Dictation\n\
         Comment=Speech to text in any text field\nExec={exec}\nNoDisplay=true\n"
    )
}

/// The user's entry after turning autostart `on`, given what it says now
/// (`user`) and what the system's says (`system`). `None` when there is
/// nothing to write: off, with no entry anywhere to switch off.
///
/// Off writes `Hidden=true` into the user's entry rather than deleting it.
/// The autostart spec reads a `Hidden` entry as deleted, and a user's entry
/// overrides the system's of the same name, so this is the one way that also
/// switches off an entry a package installed in `/etc/xdg/autostart`, which
/// deleting the user's file would bring back. It also keeps the user's own
/// `Exec` line for when the switch goes back on. `X-GNOME-Autostart-enabled`
/// is kept in step for session managers that read it instead of `Hidden`.
fn autostart_entry(
    on: bool,
    user: Option<&str>,
    system: Option<&str>,
    home: Option<&Path>,
) -> Option<String> {
    let base = match (user, system) {
        (Some(user), _) => user.to_string(),
        (None, Some(system)) => system.to_string(),
        (None, None) if on => new_entry(home),
        (None, None) => return None,
    };
    let entry = with_entry_key(&base, "Hidden", if on { "false" } else { "true" });
    Some(with_entry_key(
        &entry,
        "X-GNOME-Autostart-enabled",
        if on { "true" } else { "false" },
    ))
}

/// Start otto-dictate at login, or stop doing so. Only its own entry is
/// touched.
fn set_autostart(on: bool) {
    let Some(path) = user_entry() else {
        eprintln!("dictation: no config folder to keep the autostart entry in");
        return;
    };
    let user = std::fs::read_to_string(&path).ok();
    let system = system_entry();
    let home = otto_kit::xdg::home();
    let Some(entry) = autostart_entry(on, user.as_deref(), system.as_deref(), home.as_deref())
    else {
        return;
    };
    if let Err(err) = write_atomically(&path, &entry) {
        eprintln!("dictation: cannot write {}: {err}", path.display());
    }
}

/// Whether otto-dictate starts at login, read from the entries now.
fn autostart_now() -> bool {
    let user = user_entry().and_then(|path| std::fs::read_to_string(path).ok());
    autostart(user.as_deref(), system_entry().as_deref())
}

// ---------------------------------------------------------------------------
// The pane
// ---------------------------------------------------------------------------

fn engine_label(engine: &str) -> String {
    match engine {
        "whisper" => otto_kit::t!("settings-dictation-engine-whisper"),
        "crispasr" => otto_kit::t!("settings-dictation-engine-crispasr"),
        _ => otto_kit::t!("settings-dictation-engine-parakeet"),
    }
    .to_string()
}

fn language_label(language: &str) -> String {
    if language == AUTO {
        return otto_kit::t!("settings-dictation-language-auto").to_string();
    }
    LANGUAGES
        .iter()
        .find(|(code, _)| *code == language)
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| language.to_string())
}

/// The language a dictation would use now: the file's, else the one `LANG`
/// names, as otto-kit works it out.
fn current_language(file: &DictationFile) -> String {
    file.language.clone().unwrap_or_else(|| {
        std::env::var("LANG")
            .ok()
            .and_then(|lang| lang.get(..2).map(str::to_ascii_lowercase))
            .filter(|code| code.chars().all(|c| c.is_ascii_lowercase()) && code != "c")
            .unwrap_or_else(|| AUTO.into())
    })
}

/// The choices for one of the pane's pop-ups, or `None` for any other.
pub fn menu_choices(id: &str) -> Option<Vec<Choice>> {
    match id {
        ENGINE_ID => Some(
            ENGINES
                .iter()
                .map(|engine| Choice {
                    label: engine_label(engine),
                    value: (*engine).to_string(),
                })
                .collect(),
        ),
        LANGUAGE_ID => Some(
            std::iter::once(AUTO)
                .chain(LANGUAGES.iter().map(|(code, _)| *code))
                .map(|code| Choice {
                    label: language_label(code),
                    value: code.to_string(),
                })
                .collect(),
        ),
        _ => None,
    }
}

/// What one of the pane's pop-ups shows for `value`, or `None` for any other.
pub fn display(id: &str, value: &str) -> Option<String> {
    match id {
        ENGINE_ID => Some(engine_label(value)),
        LANGUAGE_ID => Some(language_label(value)),
        _ => None,
    }
}

/// Take a choice from one of the pane's pop-ups. Returns whether `id` was
/// the pane's, taken or not.
pub fn choose(id: &str, value: &str) -> bool {
    match id {
        ENGINE_ID => {
            if let Some(engine) = ENGINES.into_iter().find(|e| *e == value) {
                write("engine", engine.into());
                systemctl(switch_calls(engine));
            }
            true
        }
        LANGUAGE_ID => {
            if value == AUTO || LANGUAGES.iter().any(|(code, _)| *code == value) {
                write("language", value.into());
            }
            true
        }
        _ => false,
    }
}

/// `value` on the slider's steps and inside its range.
fn snap_boost(value: f32) -> f32 {
    ((value / BOOST_STEP).round() * BOOST_STEP).clamp(BOOSTS.0, BOOSTS.1)
}

/// Take a value from the pane's switch or slider. Returns whether `id` was
/// the pane's. A drag sends a value per motion; only one that lands on
/// another step is written.
pub fn apply(id: &str, value: &Value) -> bool {
    match (id, value) {
        (AUTOSTART_ID, Value::Bool(on)) => {
            set_autostart(*on);
            true
        }
        (BOOST_ID, Value::Double(boost)) => {
            set_boost(*boost as f32);
            true
        }
        (BOOST_ID, Value::Int(boost)) => {
            set_boost(*boost as f32);
            true
        }
        (AUTOSTART_ID | BOOST_ID, _) => true,
        _ => false,
    }
}

fn set_boost(boost: f32) {
    let boost = snap_boost(boost);
    if Some(boost) != dictation_file().hotwords_boost {
        write("hotwords_boost", f64::from(boost).into());
    }
}

fn start_label() -> &'static str {
    otto_kit::t!("settings-dictation-start")
}
fn restart_label() -> &'static str {
    otto_kit::t!("settings-dictation-restart")
}

fn start_buttons() -> &'static [&'static str] {
    static BUTTONS: OnceLock<Vec<&'static str>> = OnceLock::new();
    BUTTONS.get_or_init(|| vec![start_label()])
}

fn restart_buttons() -> &'static [&'static str] {
    static BUTTONS: OnceLock<Vec<&'static str>> = OnceLock::new();
    BUTTONS.get_or_init(|| vec![restart_label()])
}

/// A press on the server row's button: start or restart the picked engine.
pub fn press(row: &str, button: &str) {
    if row != SERVER_ID {
        return;
    }
    let engine = current_engine(&dictation_file(), &units().lock().unwrap());
    let verb = match button {
        b if b == start_label() => "start",
        b if b == restart_label() => "restart",
        _ => return,
    };
    systemctl(vec![vec![verb.to_string(), unit(engine)]]);
}

/// A row this pane routes to itself, bound to one of its identifiers without
/// asking the settings store about it.
fn row(label: &'static str, control: Control, id: &'static str) -> Row {
    let mut row = Row::new(label, control);
    row.id = Some(id);
    row
}

/// The row saying whether the picked engine's server is up, with the button
/// that fixes it when it is not.
fn server_row(engine: &str, unit_state: Unit) -> Row {
    let name = unit(engine);
    let (control, detail) = match unit_state.service {
        Service::Checking => (
            Control::Value(String::new()),
            otto_kit::t_owned!("settings-dictation-server-checking"),
        ),
        Service::Running => (
            Control::Button(restart_buttons()),
            otto_kit::t_owned!("settings-dictation-server-running"),
        ),
        Service::Stopped => (
            Control::Button(start_buttons()),
            otto_kit::t_owned!("settings-dictation-server-stopped"),
        ),
        Service::Failed => (
            Control::Button(start_buttons()),
            otto_kit::t_owned!("settings-dictation-server-failed", unit = name.as_str()),
        ),
        Service::Missing => (
            Control::Value(String::new()),
            otto_kit::t_owned!("settings-dictation-server-missing", engine = engine),
        ),
        Service::Unmanaged => (
            Control::Value(String::new()),
            otto_kit::t_owned!("settings-dictation-server-unmanaged"),
        ),
    };
    row(
        otto_kit::t!("settings-dictation-server"),
        control,
        SERVER_ID,
    )
    .detail(detail)
}

pub fn build() -> Pane {
    let file = dictation_file();
    let units = *units().lock().unwrap();
    let engine = current_engine(&file, &units);
    let index = ENGINES.iter().position(|e| *e == engine).unwrap_or(0);

    let mut rows = vec![
        row(
            otto_kit::t!("settings-dictation-engine"),
            Control::Select(engine.to_string()),
            ENGINE_ID,
        )
        .detail(otto_kit::t!("settings-dictation-engine-detail")),
        server_row(engine, units[index]),
        row(
            otto_kit::t!("settings-dictation-language"),
            Control::Select(current_language(&file)),
            LANGUAGE_ID,
        )
        .detail(otto_kit::t!("settings-dictation-language-detail")),
    ];
    // Only CrispASR takes hotwords; for the others the boost does nothing.
    if engine == HOTWORD_ENGINE {
        let boost = snap_boost(file.hotwords_boost.unwrap_or(DEFAULT_BOOST));
        rows.push(
            row(
                otto_kit::t!("settings-dictation-hotwords-boost"),
                Control::Slider {
                    value: boost,
                    min: BOOSTS.0,
                    max: BOOSTS.1,
                    readout: format!("{boost:.1}"),
                },
                BOOST_ID,
            )
            .detail(otto_kit::t!("settings-dictation-hotwords-boost-detail")),
        );
    }

    Pane {
        name: otto_kit::t!("settings-pane-dictation"),
        icon: "microphone",
        intro: Some(otto_kit::t!("settings-dictation-intro")),
        groups: vec![
            untitled(rows),
            untitled(vec![row(
                otto_kit::t!("settings-dictation-autostart"),
                Control::Toggle(autostart_now()),
                AUTOSTART_ID,
            )
            .detail(otto_kit::t!("settings-dictation-autostart-detail"))]),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "# Speech to text.\n# The engine, kept in step with its unit.\nengine = \"whisper\" # by Settings\nlanguage = \"it\"\nurl = \"http://box:8080/inference\"\n";

    #[test]
    fn reads_the_keys_it_shows() {
        let file = read_file(FILE);
        assert_eq!(file.engine, Some("whisper"));
        assert_eq!(file.language.as_deref(), Some("it"));
        assert_eq!(file.hotwords_boost, None);
        assert_eq!(read_file("hotwords_boost = 5").hotwords_boost, Some(5.0));
        // An engine it does not know, or a file that does not parse, is unset.
        assert_eq!(read_file("engine = \"vosk\"").engine, None);
        assert_eq!(read_file("engine = "), DictationFile::default());
    }

    #[test]
    fn writing_a_key_keeps_comments_and_the_rest() {
        let text = with_key(FILE, "engine", "crispasr".into()).unwrap();
        assert!(
            text.contains(
                "# The engine, kept in step with its unit.\nengine = \"crispasr\" # by Settings\n"
            ),
            "{text}"
        );
        assert!(text.contains("url = \"http://box:8080/inference\""));
        let text = with_key(&text, "hotwords_boost", 5.5.into()).unwrap();
        let file = read_file(&text);
        assert_eq!(file.engine, Some("crispasr"));
        assert_eq!(file.language.as_deref(), Some("it"));
        assert_eq!(file.hotwords_boost, Some(5.5));
        // Into a file that is not there yet, and never over one that is broken.
        assert_eq!(
            with_key("", "language", "auto".into()).unwrap(),
            "language = \"auto\"\n"
        );
        assert!(with_key("engine = ", "language", "auto".into()).is_err());
    }

    #[test]
    fn reads_the_units_systemd_reports() {
        let show = "Id=otto-stt-parakeet.service\nLoadState=loaded\nActiveState=inactive\nUnitFileState=disabled\n\n\
                    Id=otto-stt-whisper.service\nLoadState=not-found\nActiveState=inactive\nUnitFileState=\n\n\
                    Id=otto-stt-crispasr.service\nLoadState=loaded\nActiveState=active\nUnitFileState=enabled\n";
        let units = parse_units(show);
        assert_eq!(
            units[0],
            Unit {
                service: Service::Stopped,
                enabled: false
            }
        );
        assert_eq!(units[1].service, Service::Missing);
        assert_eq!(
            units[2],
            Unit {
                service: Service::Running,
                enabled: true
            }
        );
        let failed =
            parse_units("Id=otto-stt-whisper.service\nLoadState=loaded\nActiveState=failed\n");
        assert_eq!(failed[1].service, Service::Failed);
        assert_eq!(failed[0], Unit::CHECKING);
    }

    #[test]
    fn the_picked_engine_is_the_files_then_the_one_running() {
        let mut units = [Unit::CHECKING; 3];
        assert_eq!(
            current_engine(&DictationFile::default(), &units),
            "parakeet"
        );
        units[1].enabled = true;
        assert_eq!(current_engine(&DictationFile::default(), &units), "whisper");
        units[2].service = Service::Running;
        assert_eq!(
            current_engine(&DictationFile::default(), &units),
            "crispasr"
        );
        let file = DictationFile {
            engine: Some("parakeet"),
            ..DictationFile::default()
        };
        assert_eq!(current_engine(&file, &units), "parakeet");
    }

    #[test]
    fn switching_disables_the_others_then_starts_the_one() {
        assert_eq!(
            switch_calls("whisper"),
            [
                vec!["disable", "otto-stt-parakeet.service"],
                vec!["disable", "otto-stt-crispasr.service"],
                vec!["enable", "--now", "otto-stt-whisper.service"],
            ]
        );
    }

    #[test]
    fn the_boost_snaps_to_its_steps() {
        assert_eq!(snap_boost(4.2), 4.0);
        assert_eq!(snap_boost(4.3), 4.5);
        assert_eq!(snap_boost(0.0), BOOSTS.0);
        assert_eq!(snap_boost(20.0), BOOSTS.1);
    }

    const INSTALLED: &str = "[Desktop Entry]\nType=Application\nName=Dictation\nExec=/home/u/.local/bin/otto-dictate\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n";

    #[test]
    fn autostart_follows_the_users_entry_over_the_systems() {
        assert!(!autostart(None, None));
        assert!(autostart(Some(INSTALLED), None));
        assert!(autostart(None, Some(INSTALLED)));
        let hidden = with_entry_key(INSTALLED, "Hidden", "true");
        assert!(!autostart(Some(&hidden), Some(INSTALLED)));
        assert!(!autostart(None, Some(&hidden)));
        let gnome_off = with_entry_key(INSTALLED, "X-GNOME-Autostart-enabled", "false");
        assert!(!autostart(Some(&gnome_off), None));
        // A key of the same name in another group is not the entry's.
        assert!(autostart(
            Some("[Desktop Entry]\nExec=x\n[Desktop Action a]\nHidden=true\n"),
            None
        ));
    }

    #[test]
    fn switching_autostart_off_hides_the_entry_and_on_brings_it_back() {
        let off = autostart_entry(false, Some(INSTALLED), None, None).unwrap();
        assert!(!autostart(Some(&off), None));
        assert!(
            off.contains("Exec=/home/u/.local/bin/otto-dictate\n"),
            "{off}"
        );
        assert!(off.contains("X-GNOME-Autostart-enabled=false\n"));
        let on = autostart_entry(true, Some(&off), None, None).unwrap();
        assert!(autostart(Some(&on), None));
        assert!(on.contains("Hidden=false\n"));
        // A system entry is masked by a hidden copy of it.
        let masked = autostart_entry(false, None, Some(INSTALLED), None).unwrap();
        assert!(!autostart(Some(&masked), Some(INSTALLED)));
        // Nothing to switch off, nothing written; on, an entry is made.
        assert_eq!(autostart_entry(false, None, None, None), None);
        let made = autostart_entry(true, None, None, None).unwrap();
        assert!(autostart(Some(&made), None));
        assert_eq!(entry_key(&made, "Exec"), Some("otto-dictate"));
    }

    #[test]
    fn the_autostart_entry_is_written_to_disk() {
        let dir =
            std::env::temp_dir().join(format!("otto-settings-autostart-{}", std::process::id()));
        let path = dir.join("autostart").join(AUTOSTART_FILE);
        let entry = autostart_entry(true, None, None, Some(&dir)).unwrap();
        write_atomically(&path, &entry).unwrap();
        let read = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(autostart(Some(&read), None));
    }

    #[test]
    fn languages_show_by_their_own_name() {
        assert_eq!(language_label("it"), "Italiano");
        assert_eq!(language_label("nl"), "nl");
        assert_eq!(display(LANGUAGE_ID, "de").as_deref(), Some("Deutsch"));
        assert_eq!(display("theme_scheme", "dark"), None);
        assert!(!choose("theme_scheme", "dark"));
    }
}
