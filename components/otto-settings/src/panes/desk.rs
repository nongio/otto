//! The desk's group in the Appearance pane.
//!
//! Whether the desk runs is a session setting (`desk.enabled`) and goes
//! through `org.otto.Settings` like any other. Its folder and its panel's
//! size and position belong to the desk itself, in the `[desk]` section of
//! `~/.config/otto/files.toml` (see `specs/desk.md`), so this group writes
//! them there, keeping everything else in the file, and the desk follows the
//! file as it changes.
//!
//! *When icons don't fit* writes `overflow`: the grid scrolls, or its last
//! cell becomes the overflow tile, which opens the rest in the overflow
//! panel. *Icon size* writes `icon_size`, in points.
//!
//! The size and position are set on the desk, not here: *Edit…* asks the
//! running desk for its edit mode over `org.otto.Desk1`, where the panel is
//! dragged into place. *Reset* puts it back to filling the screen.

// Rust guideline compliant 2026-02-21

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use toml_edit::DocumentMut;

use crate::discovery::Choice;
use crate::file_picker;
use crate::model::{group, Control, Group, Row};
use crate::settings_client::{self, Value};

/// The session setting that runs the desk.
const ENABLED_ID: &str = "desk.enabled";

/// The *When icons don't fit* pop-up. Not a session setting: the choice is
/// the desk's own, `overflow` in `files.toml`, and this group answers for it.
const OVERFLOW_ID: &str = "files.desk.overflow";

/// What `overflow` may say, the default first.
const OVERFLOWS: [&str; 2] = ["scroll", "stack"];

/// The *Icon size* slider. Not a session setting either: `icon_size` in
/// `files.toml`.
const ICON_SIZE_ID: &str = "files.desk.icon_size";

/// The icon size when the file sets none, the desk's own default.
const DEFAULT_ICON_SIZE: f32 = 64.0;

/// The slider's range, in points, inside the 32 to 256 the desk accepts.
const ICON_SIZES: (f32, f32) = (32.0, 160.0);

/// The slider moves in steps of this many points.
const ICON_SIZE_STEP: f32 = 4.0;

/// What this group shows of the `[desk]` section.
#[derive(Debug, Clone, PartialEq)]
struct DeskFile {
    /// `folder` as written, `None` for the default.
    folder: Option<String>,
    /// Whether the panel is anything other than the default `fill`.
    placed: bool,
    /// `overflow`, one of [`OVERFLOWS`]; the default when unset or unknown,
    /// which is what the desk makes of it too.
    overflow: &'static str,
    /// `icon_size`, in points, kept to [`ICON_SIZES`].
    icon_size: f32,
}

impl Default for DeskFile {
    fn default() -> Self {
        Self {
            folder: None,
            placed: false,
            overflow: OVERFLOWS[0],
            icon_size: DEFAULT_ICON_SIZE,
        }
    }
}

/// The `[desk]` section as last read, with the file's modification time
/// then, so the file is only read again once something has written it: this
/// group, the desk's edit mode, or a person with an editor.
type Cache = Option<(Option<SystemTime>, DeskFile)>;

fn cached() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Set when a write from a background thread changed what the group shows.
static DIRTY: AtomicBool = AtomicBool::new(false);

/// Whether the group changed since the last call; `main.rs` polls this to
/// repaint.
pub fn take_dirty() -> bool {
    DIRTY.swap(false, Ordering::Relaxed)
}

/// `~/.config/otto/files.toml`, where the desk reads its own settings —
/// the file otto-files' `places_config::config_path` names.
fn config_path() -> Option<PathBuf> {
    otto_kit::xdg::otto_config_file("files.toml")
}

/// Read the parts of `text`'s `[desk]` section this group shows.
fn read_desk(text: &str) -> DeskFile {
    let Ok(doc) = text.parse::<DocumentMut>() else {
        return DeskFile::default();
    };
    let desk = doc.get("desk");
    let string = |key: &str| {
        desk.and_then(|desk| desk.get(key))
            .and_then(|item| item.as_str())
            .map(str::to_string)
    };
    DeskFile {
        folder: string("folder").filter(|folder| !folder.trim().is_empty()),
        placed: string("anchor").is_some_and(|anchor| !anchor.trim().eq_ignore_ascii_case("fill")),
        overflow: string("overflow")
            .and_then(|value| {
                let value = value.trim().to_ascii_lowercase();
                OVERFLOWS.into_iter().find(|known| *known == value)
            })
            .unwrap_or(OVERFLOWS[0]),
        icon_size: desk
            .and_then(|desk| desk.get("icon_size"))
            .and_then(|item| {
                item.as_float()
                    .or_else(|| item.as_integer().map(|size| size as f64))
            })
            .filter(|size| *size > 0.0)
            .map(|size| snap_icon_size(size as f32))
            .unwrap_or(DEFAULT_ICON_SIZE),
    }
}

/// `size` on the slider's steps and inside its range.
fn snap_icon_size(size: f32) -> f32 {
    ((size / ICON_SIZE_STEP).round() * ICON_SIZE_STEP).clamp(ICON_SIZES.0, ICON_SIZES.1)
}

/// The `[desk]` section, read again whenever the file has changed.
fn desk_file() -> DeskFile {
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
        .map(|text| read_desk(&text))
        .unwrap_or_default();
    *cache = Some((modified, file.clone()));
    file
}

/// `text` with the `[desk]` section changed by `change`, everything else
/// kept as it was.
///
/// # Errors
///
/// Fails when `text` is not TOML, so a file that does not parse is never
/// replaced by one that has lost everything in it.
fn edited(text: &str, change: impl FnOnce(&mut toml_edit::Table)) -> Result<String, String> {
    let mut doc = text.parse::<DocumentMut>().map_err(|err| err.to_string())?;
    if !doc.contains_table("desk") {
        doc["desk"] = toml_edit::table();
    }
    let desk = doc["desk"]
        .as_table_mut()
        .ok_or_else(|| "[desk] is not a table".to_string())?;
    change(desk);
    Ok(doc.to_string())
}

/// `text` with the desk's folder set to `folder`.
fn with_folder(text: &str, folder: &str) -> Result<String, String> {
    edited(text, |desk| {
        desk["folder"] = toml_edit::value(folder);
    })
}

/// `text` with the desk's `overflow` set to `value`.
fn with_overflow(text: &str, value: &str) -> Result<String, String> {
    edited(text, |desk| {
        desk["overflow"] = toml_edit::value(value);
    })
}

/// `text` with the desk's `icon_size` set to `size` points.
fn with_icon_size(text: &str, size: f32) -> Result<String, String> {
    edited(text, |desk| {
        desk["icon_size"] = toml_edit::value(i64::from(size.round() as i32));
    })
}

/// `text` with the desk back to filling the usable area.
fn with_fill(text: &str) -> Result<String, String> {
    edited(text, |desk| {
        desk["anchor"] = toml_edit::value("fill");
        desk.remove("size");
        desk.remove("position");
    })
}

/// Apply `change` to `files.toml` and remember what it now says.
fn write(change: impl FnOnce(&str) -> Result<String, String>) {
    let Some(path) = config_path() else {
        eprintln!("desk: no home folder to keep files.toml in");
        return;
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            eprintln!("desk: cannot read {}: {err}", path.display());
            return;
        }
    };
    let next = match change(&text) {
        Ok(next) => next,
        Err(err) => {
            eprintln!(
                "desk: {} does not parse, leaving it alone: {err}",
                path.display()
            );
            return;
        }
    };
    if let Err(err) = write_atomically(&path, &next) {
        eprintln!("desk: cannot write {}: {err}", path.display());
        return;
    }
    // Forgotten rather than filled in: the next read stats the new file and
    // takes it from there.
    *cached().lock().unwrap() = None;
    DIRTY.store(true, Ordering::Relaxed);
    otto_kit::AppContext::request_wakeup();
}

/// Replace `path` with `text` in one step, so the desk, which watches the
/// file, never reads it half written.
fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension(format!("toml.tmp.{}", std::process::id()));
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, path)
}

fn folder_label() -> &'static str {
    otto_kit::t!("settings-desk-folder")
}
fn choose_label() -> &'static str {
    otto_kit::t!("settings-search-choose")
}
fn layout_label() -> &'static str {
    otto_kit::t!("settings-desk-layout")
}
fn edit_label() -> &'static str {
    otto_kit::t!("settings-desk-layout-edit")
}
fn reset_label() -> &'static str {
    otto_kit::t!("settings-desk-layout-reset")
}

fn overflow_label(value: &str) -> String {
    match value {
        "stack" => otto_kit::t_owned!("settings-desk-overflow-stack"),
        _ => otto_kit::t_owned!("settings-desk-overflow-scroll"),
    }
}

fn choose_buttons() -> &'static [&'static str] {
    static BUTTONS: OnceLock<Vec<&'static str>> = OnceLock::new();
    BUTTONS.get_or_init(|| vec![choose_label()])
}
fn layout_buttons() -> &'static [&'static str] {
    static BUTTONS: OnceLock<Vec<&'static str>> = OnceLock::new();
    BUTTONS.get_or_init(|| vec![edit_label(), reset_label()])
}

/// The Desk group: whether it is shown, which folder, and the panel's size
/// and position.
pub fn group_rows() -> Group {
    let file = desk_file();
    let running = matches!(settings_client::value(ENABLED_ID), Some(Value::Bool(true)));
    let folder = file
        .folder
        .clone()
        .unwrap_or_else(|| otto_kit::t_owned!("settings-desk-folder-default"));
    let layout = if file.placed {
        otto_kit::t!("settings-desk-layout-placed")
    } else {
        otto_kit::t!("settings-desk-layout-fill")
    };
    group(
        otto_kit::t!("settings-group-desk"),
        vec![
            Row::new(otto_kit::t!("settings-show-desk"), Control::Toggle(false))
                .detail(otto_kit::t!("settings-show-desk-detail"))
                .id(ENABLED_ID),
            Row::new(folder_label(), Control::Button(choose_buttons())).detail(folder),
            Row::new(
                otto_kit::t!("settings-desk-overflow"),
                Control::Select(file.overflow.to_string()),
            )
            .detail(otto_kit::t!("settings-desk-overflow-detail"))
            .id(OVERFLOW_ID),
            Row::new(
                otto_kit::t!("settings-desk-icon-size"),
                Control::Slider {
                    value: file.icon_size,
                    min: ICON_SIZES.0,
                    max: ICON_SIZES.1,
                    readout: format!("{} px", file.icon_size as i32),
                },
            )
            .id(ICON_SIZE_ID),
            // Edit mode is the running desk's, so there is nothing to edit
            // while it is off.
            Row::new(layout_label(), Control::Button(layout_buttons()))
                .detail(layout)
                .inactive(!running),
        ],
    )
}

/// A press on one of the group's push buttons.
pub fn press(row: &str, button: &str) {
    if row == folder_label() && button == choose_label() {
        choose_folder();
    } else if row == layout_label() && button == edit_label() {
        in_background("desk-edit", ask_for_edit_mode);
    } else if row == layout_label() && button == reset_label() {
        in_background("desk-reset", || write(with_fill));
    }
}

/// The choices for the group's pop-up, or `None` for any other.
pub fn menu_choices(id: &str) -> Option<Vec<Choice>> {
    (id == OVERFLOW_ID).then(|| {
        OVERFLOWS
            .iter()
            .map(|value| Choice {
                label: overflow_label(value),
                value: (*value).to_string(),
            })
            .collect()
    })
}

/// What the group's pop-up shows for `value`, or `None` for any other.
pub fn display(id: &str, value: &str) -> Option<String> {
    (id == OVERFLOW_ID).then(|| overflow_label(value))
}

/// Take a choice from the group's pop-up into `files.toml`. Returns whether
/// `id` was the group's, taken or not.
pub fn choose(id: &str, value: &str) -> bool {
    if id != OVERFLOW_ID {
        return false;
    }
    if OVERFLOWS.contains(&value) {
        let value = value.to_string();
        write(move |text| with_overflow(text, &value));
    }
    true
}

/// Take a value from the group's slider into `files.toml`. Returns whether
/// `id` was the group's. A drag sends a value per motion; only one that
/// lands on another step is written, so the desk reloads once per step.
pub fn apply(id: &str, value: &settings_client::Value) -> bool {
    if id != ICON_SIZE_ID {
        return false;
    }
    let size = match value {
        Value::Double(size) => *size as f32,
        Value::Int(size) => *size as f32,
        _ => return true,
    };
    let size = snap_icon_size(size);
    if size != desk_file().icon_size {
        write(move |text| with_icon_size(text, size));
    }
    true
}

/// Run `work` on a thread of its own: the portal and the bus both block.
fn in_background(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(err) = std::thread::Builder::new().name(name.into()).spawn(work) {
        eprintln!("desk: could not start {name}: {err}");
    }
}

/// Ask for a folder through the desktop portal and make it the desk's.
fn choose_folder() {
    in_background("desk-folder", || {
        let title = otto_kit::t!("settings-desk-choose-folder-title");
        let path = match file_picker::open_folder(title) {
            file_picker::Outcome::Chosen(paths) => paths.into_iter().next(),
            file_picker::Outcome::Dismissed => None,
            file_picker::Outcome::Failed(why) => {
                eprintln!("desk: {why}");
                None
            }
        };
        let Some(path) = path else {
            return;
        };
        let folder = otto_kit::xdg::tilde(&path);
        write(|text| with_folder(text, &folder));
    });
}

/// Call `EditLayout` on the running desk. It returns at once; the desk shows
/// its outline and handles until Done or Cancel.
fn ask_for_edit_mode() {
    let result = zbus::blocking::Connection::session().and_then(|connection| {
        otto_kit::dbus::desk::DeskProxyBlocking::new(&connection)?.edit_layout()
    });
    if let Err(err) = result {
        eprintln!("desk: cannot reach the desk to edit its size and position: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_and_the_placement_are_read_from_the_desk_section() {
        let file = read_desk("[desk]\nfolder = \"~/Stuff\"\nanchor = \"top-left\"\n");
        assert_eq!(file.folder.as_deref(), Some("~/Stuff"));
        assert!(file.placed);
        assert_eq!(read_desk(""), DeskFile::default());
        assert!(!read_desk("[desk]\nanchor = \"fill\"\n").placed);
    }

    #[test]
    fn a_new_folder_keeps_the_rest_of_the_file() {
        let text = "# mine\n[sidebar]\nhide = [\"music\"]\n";
        let next = with_folder(text, "~/Stuff").unwrap();
        assert!(next.starts_with("# mine"));
        assert!(next.contains("hide = [\"music\"]"));
        assert_eq!(read_desk(&next).folder.as_deref(), Some("~/Stuff"));
    }

    #[test]
    fn reset_drops_the_size_and_position() {
        let text = "[desk]\nanchor = \"top-left\"\nsize = [\"40%\", \"30%\"]\nposition = [\"5%\", \"5%\"]\nsort = \"kind\"\n";
        let next = with_fill(text).unwrap();
        assert!(!next.contains("size"));
        assert!(!next.contains("position"));
        assert!(next.contains("sort = \"kind\""));
        assert!(!read_desk(&next).placed);
    }

    #[test]
    fn a_file_that_does_not_parse_is_left_alone() {
        assert!(with_folder("[desk\n", "~/x").is_err());
        assert!(with_fill("[desk\n").is_err());
        assert!(with_overflow("[desk\n", "stack").is_err());
    }

    #[test]
    fn overflow_is_read_and_defaults_to_scroll() {
        assert_eq!(read_desk("").overflow, "scroll");
        assert_eq!(
            read_desk("[desk]\noverflow = \"Stack\"\n").overflow,
            "stack"
        );
        assert_eq!(
            read_desk("[desk]\noverflow = \"pile\"\n").overflow,
            "scroll"
        );
    }

    #[test]
    fn a_new_overflow_keeps_the_rest_of_the_file() {
        let text = "# mine\n[desk]\nsort = \"kind\" # by kind\n";
        let next = with_overflow(text, "stack").unwrap();
        assert!(next.starts_with("# mine"));
        assert!(next.contains("sort = \"kind\" # by kind"));
        assert_eq!(read_desk(&next).overflow, "stack");
        let back = with_overflow(&next, "scroll").unwrap();
        assert_eq!(read_desk(&back).overflow, "scroll");
    }

    #[test]
    fn the_pop_up_offers_both_and_owns_only_its_own_id() {
        let values: Vec<String> = menu_choices(OVERFLOW_ID)
            .unwrap()
            .into_iter()
            .map(|choice| choice.value)
            .collect();
        assert_eq!(values, ["scroll", "stack"]);
        assert!(menu_choices("desk.enabled").is_none());
        assert!(!choose("desk.enabled", "stack"));
        assert!(display("desk.enabled", "stack").is_none());
    }

    #[test]
    fn icon_size_is_read_snapped_and_written_back() {
        assert_eq!(read_desk("").icon_size, DEFAULT_ICON_SIZE);
        assert_eq!(read_desk("[desk]\nicon_size = 96\n").icon_size, 96.0);
        assert_eq!(read_desk("[desk]\nicon_size = 97.5\n").icon_size, 96.0);
        assert_eq!(read_desk("[desk]\nicon_size = 8\n").icon_size, ICON_SIZES.0);
        assert_eq!(
            read_desk("[desk]\nicon_size = \"big\"\n").icon_size,
            DEFAULT_ICON_SIZE
        );
        let next = with_icon_size("# mine\n[desk]\nsort = \"kind\"\n", 80.0).unwrap();
        assert!(next.starts_with("# mine"));
        assert!(next.contains("icon_size = 80\n"));
        assert_eq!(read_desk(&next).icon_size, 80.0);
    }
}
