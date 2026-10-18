//! The Start at login group in the Lock & Login pane.
//!
//! What the session starts when it begins is `[[exec_once]]` in the
//! configuration (`docs/user/autostart.md`): Otto's own components — the top
//! bar, the dynamic island, the stash and the Agents panel — and any other
//! command the user listed. The compositor runs the list once, at login, so
//! a change here applies at the next login.
//!
//! There is no session setting for the list, so this group reads and writes
//! the file itself, in the writable configuration the General pane names,
//! keeping everything else in it. An array in the user's file replaces the
//! system's rather than adding to it: until the user's file has a list of
//! its own, the group shows the system's, and its first change copies that
//! list in before changing it, so turning one component off does not turn
//! off the rest with it.
//!
//! A pop-up says where startup apps come from: Otto's list alone, or the
//! XDG autostart folders too (`xdg_autostart`, written here the same way).
//! Under it, one list box holds every entry in the order the compositor
//! starts them: a checkbox switches one off where it stands (`enabled =
//! false`), a drag moves it, "+" adds an app or program through the portal
//! or a typed command, and "−" takes the picked one out. Otto's own apps are
//! only ever switched off, never taken out; one missing from the list is
//! offered unticked at its end. The footer's text button opens the file.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use toml_edit::{ArrayOfTables, DocumentMut, Item};

use otto_kit::portal_file_chooser::{Outcome, Request, Rule};

use crate::discovery::Choice;
use crate::model::{group, Control, Group, ListBox, ListBoxItem, Row};
use crate::settings_client::{self, Value};
use crate::view::ListBoxHit;

/// The key the compositor reads the list from.
const KEY: &str = "exec_once";

/// The system's configuration, the bottom layer.
const SYSTEM_CONFIG: &str = "/etc/otto/config.toml";

/// The list box. Not session settings, this and the ids below: the group
/// answers for them.
const LIST_ID: &str = "startup.list";

/// The menu the list box's "+" opens, and its two choices.
pub const ADD_MENU_ID: &str = "startup.add-menu";
const ADD_APP: &str = "app";
const ADD_COMMAND: &str = "command";

/// The field a typed command goes into, shown after "Command…".
pub const ADD_ID: &str = "startup.add";

/// The key that also starts the autostart folders' entries.
const XDG_KEY: &str = "xdg_autostart";

/// The pop-up that writes it, and what it may hold: off, then on.
const XDG_ID: &str = "startup.xdg";
const SOURCES: [&str; 2] = ["otto", "otto-xdg"];

/// Where the picker opens: the installed applications.
const APPLICATIONS: &str = "/usr/share/applications";

/// Otto's components that start with the session, in the order the system
/// configuration lists them: the program, its label and its detail.
const COMPONENTS: [(&str, &str, &str); 4] = [
    (
        "otto-bar",
        "settings-startup-bar",
        "settings-startup-bar-detail",
    ),
    (
        "otto-islands",
        "settings-startup-islands",
        "settings-startup-islands-detail",
    ),
    (
        "otto-stash",
        "settings-startup-stash",
        "settings-startup-stash-detail",
    ),
    (
        "otto-canvas",
        "settings-startup-canvas",
        "settings-startup-canvas-detail",
    ),
];

/// One `[[exec_once]]` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    cmd: String,
    args: Vec<String>,
    /// `enabled`, which the compositor reads as true when it is missing.
    enabled: bool,
}

impl Entry {
    /// Whether this entry runs `program`, by name or by a path to it.
    fn runs(&self, program: &str) -> bool {
        Path::new(&self.cmd)
            .file_name()
            .is_some_and(|name| name == program)
    }

    /// The command line as the row shows it and *Add command* takes it.
    fn line(&self) -> String {
        std::iter::once(&self.cmd)
            .chain(&self.args)
            .map(|word| {
                if word.is_empty() || word.contains(char::is_whitespace) {
                    format!("\"{word}\"")
                } else {
                    word.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn table(&self) -> toml_edit::Table {
        let mut table = toml_edit::Table::new();
        table["cmd"] = toml_edit::value(self.cmd.as_str());
        let mut args = toml_edit::Array::new();
        for arg in &self.args {
            args.push(arg.as_str());
        }
        table["args"] = toml_edit::value(args);
        if !self.enabled {
            table["enabled"] = toml_edit::value(false);
        }
        table
    }
}

/// `line` split into words: spaces separate them, and quotes, single or
/// double, keep spaces inside one. `None` when it has no command.
fn parse_line(line: &str) -> Option<Entry> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut started = false;
    for ch in line.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => word.push(ch),
            None if ch == '"' || ch == '\'' => {
                quote = Some(ch);
                started = true;
            }
            None if ch.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            None => {
                word.push(ch);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    let mut words = words.into_iter();
    let cmd = words.next().filter(|cmd| !cmd.is_empty())?;
    Some(Entry {
        cmd,
        args: words.collect(),
        enabled: true,
    })
}

/// The command a chosen file stands for: an application's `.desktop` entry
/// runs its `Exec=` line, with the field codes dropped since nothing is
/// opened at login, and any other file runs itself. `text` is the file's
/// contents, read only for an entry.
fn entry_for_file(path: &Path, text: Option<&str>) -> Option<Entry> {
    if path.extension().is_some_and(|ext| ext == "desktop") {
        let exec = desktop_exec(text?)?;
        let mut argv = otto_kit::mime_apps::exec_argv(&exec)?.into_iter();
        return Some(Entry {
            cmd: argv.next()?,
            args: argv.collect(),
            enabled: true,
        });
    }
    Some(Entry {
        cmd: path.to_str()?.to_string(),
        args: Vec::new(),
        enabled: true,
    })
}

/// The `Exec=` value of an entry's `[Desktop Entry]` group.
fn desktop_exec(text: &str) -> Option<String> {
    let mut in_entry = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if in_entry {
            if let Some(exec) = line.strip_prefix("Exec").map(str::trim_start) {
                if let Some(exec) = exec.strip_prefix('=') {
                    return Some(exec.trim().to_string());
                }
            }
        }
    }
    None
}

/// Whether `path` is a file that may be run.
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// One entry of the list, as a table of either kind.
fn entry(table: &dyn toml_edit::TableLike) -> Option<Entry> {
    let cmd = table.get("cmd")?.as_str()?.to_string();
    let args = table
        .get("args")
        .and_then(Item::as_array)
        .map(|args| {
            args.iter()
                .filter_map(|arg| arg.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let enabled = table.get("enabled").and_then(Item::as_bool).unwrap_or(true);
    Some(Entry { cmd, args, enabled })
}

/// The list `item` holds: `[[exec_once]]` tables, or an inline array.
fn entries_of(item: &Item) -> Vec<Entry> {
    if let Some(tables) = item.as_array_of_tables() {
        return tables.iter().filter_map(|table| entry(table)).collect();
    }
    item.as_array()
        .map(|array| {
            array
                .iter()
                .filter_map(|value| value.as_inline_table())
                .filter_map(|table| entry(table))
                .collect()
        })
        .unwrap_or_default()
}

/// The list `text` sets, or `None` when it sets none (or does not parse) and
/// the layer below decides.
fn read_list(text: &str) -> Option<Vec<Entry>> {
    let doc = text.parse::<DocumentMut>().ok()?;
    doc.get(KEY).map(entries_of)
}

/// The file this group writes: the one every setting goes to, or the user's
/// own while no compositor says which that is.
fn writable_path() -> Option<PathBuf> {
    settings_client::config_path()
        .map(PathBuf::from)
        .or_else(|| otto_kit::xdg::otto_config_file("config.toml"))
}

/// The layers the list may come from, topmost first.
fn layers() -> Vec<PathBuf> {
    let mut layers: Vec<PathBuf> = writable_path().into_iter().collect();
    for below in [
        otto_kit::xdg::otto_config_file("config.toml"),
        Some(PathBuf::from(SYSTEM_CONFIG)),
    ]
    .into_iter()
    .flatten()
    {
        if !layers.contains(&below) {
            layers.push(below);
        }
    }
    layers
}

/// What the group shows: the list in force, and whether it is the writable
/// file's own rather than one from a layer below.
#[derive(Debug, Clone, Default, PartialEq)]
struct List {
    entries: Vec<Entry>,
    own: bool,
    /// `xdg_autostart` from the first layer that sets it, off by default as
    /// in the compositor.
    xdg: bool,
}

/// `xdg_autostart` in `text`, or `None` when it does not say.
fn read_xdg(text: &str) -> Option<bool> {
    text.parse::<DocumentMut>().ok()?.get(XDG_KEY)?.as_bool()
}

/// The first layer that sets a list, and the first that sets the XDG
/// switch, given each layer's text.
fn list_in(texts: &[Option<String>]) -> List {
    let mut list = texts
        .iter()
        .enumerate()
        .find_map(|(layer, text)| {
            let entries = read_list(text.as_deref()?)?;
            Some(List {
                entries,
                own: layer == 0,
                xdg: false,
            })
        })
        .unwrap_or_default();
    list.xdg = texts
        .iter()
        .find_map(|text| read_xdg(text.as_deref()?))
        .unwrap_or(false);
    list
}

/// The list as last read, with each layer's modification time then, so the
/// files are only read again once one of them has changed.
type Cache = Option<(Vec<Option<SystemTime>>, List)>;

fn cached() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Set when a write changed what the group shows.
static DIRTY: AtomicBool = AtomicBool::new(false);

/// Whether the group changed since the last call; `main.rs` polls this to
/// repaint.
pub fn take_dirty() -> bool {
    DIRTY.swap(false, Ordering::Relaxed)
}

/// The list in force, read again whenever a layer has changed.
fn list() -> List {
    let layers = layers();
    let modified: Vec<Option<SystemTime>> = layers
        .iter()
        .map(|path| {
            std::fs::metadata(path)
                .and_then(|meta| meta.modified())
                .ok()
        })
        .collect();
    let mut cache = cached().lock().unwrap();
    if let Some((seen, list)) = cache.as_ref() {
        if *seen == modified {
            return list.clone();
        }
    }
    let texts: Vec<Option<String>> = layers
        .iter()
        .map(|path| std::fs::read_to_string(path).ok())
        .collect();
    let list = list_in(&texts);
    *cache = Some((modified, list.clone()));
    list
}

/// `text` with its list changed by `change`, everything else kept as it was.
/// A file with no list of its own starts from `inherited`, the list in force
/// from the layers below.
///
/// # Errors
///
/// Fails when `text` is not TOML, or its list is neither tables nor an
/// array, so a file that does not parse is never replaced by one that has
/// lost everything in it.
fn edited(
    text: &str,
    inherited: &[Entry],
    change: impl FnOnce(&mut ArrayOfTables),
) -> Result<String, String> {
    let mut doc = text.parse::<DocumentMut>().map_err(|err| err.to_string())?;
    let mut tables = match doc.remove(KEY) {
        None => inherited.iter().map(Entry::table).collect(),
        Some(Item::ArrayOfTables(tables)) => tables,
        Some(item @ Item::Value(toml_edit::Value::Array(_))) => {
            entries_of(&item).iter().map(Entry::table).collect()
        }
        Some(_) => return Err(format!("{KEY} is not a list of commands")),
    };
    change(&mut tables);
    // An empty list of tables writes nothing at all, which would hand the
    // list back to the layer below: an empty array says there is none.
    doc[KEY] = if tables.is_empty() {
        toml_edit::value(toml_edit::Array::new())
    } else {
        Item::ArrayOfTables(tables)
    };
    Ok(doc.to_string())
}

/// `text` with `program` started at login, or not, given the list it
/// inherits.
fn with_component(
    text: &str,
    inherited: &[Entry],
    program: &str,
    on: bool,
) -> Result<String, String> {
    edited(text, inherited, |tables| {
        let runs = |table: &toml_edit::Table| entry(table).is_some_and(|entry| entry.runs(program));
        if on {
            if !tables.iter().any(runs) {
                tables.push(
                    Entry {
                        cmd: program.to_string(),
                        args: Vec::new(),
                        enabled: true,
                    }
                    .table(),
                );
            }
        } else {
            tables.retain(|table| !runs(table));
        }
    })
}

/// `text` with `line` appended to the list.
fn with_command(text: &str, inherited: &[Entry], line: &Entry) -> Result<String, String> {
    edited(text, inherited, |tables| tables.push(line.table()))
}

/// `text` with the autostart folders' entries started at login, or not.
fn with_xdg(text: &str, on: bool) -> Result<String, String> {
    let mut doc = text.parse::<DocumentMut>().map_err(|err| err.to_string())?;
    doc[XDG_KEY] = toml_edit::value(on);
    Ok(doc.to_string())
}

/// Where the entry a row showed at `index`, reading `line`, is now: still
/// there, or the first that reads the same if the file moved it.
fn find_entry(tables: &ArrayOfTables, index: usize, line: &str) -> Option<usize> {
    let reads = |table: &toml_edit::Table| entry(table).is_some_and(|entry| entry.line() == line);
    if tables.get(index).is_some_and(reads) {
        return Some(index);
    }
    tables.iter().position(reads)
}

/// `text` with the entry the list showed at `index` switched on or off where
/// it stands. On drops the key rather than writing `true`, which is what a
/// missing one means.
fn with_enabled(
    text: &str,
    inherited: &[Entry],
    index: usize,
    line: &str,
    on: bool,
) -> Result<String, String> {
    edited(text, inherited, |tables| {
        let Some(index) = find_entry(tables, index, line) else {
            return;
        };
        if let Some(table) = tables.get_mut(index) {
            if on {
                table.remove("enabled");
            } else {
                table["enabled"] = toml_edit::value(false);
            }
        }
    })
}

/// `text` without the entry a row showed at `index`.
fn without_entry(
    text: &str,
    inherited: &[Entry],
    index: usize,
    line: &str,
) -> Result<String, String> {
    edited(text, inherited, |tables| {
        if let Some(index) = find_entry(tables, index, line) {
            tables.remove(index);
        }
    })
}

/// `text` with the entry a row showed at `index` swapped with its neighbour
/// `step` places away, its comment going with it. Nothing moves past either
/// end.
fn moved(
    text: &str,
    inherited: &[Entry],
    index: usize,
    line: &str,
    step: isize,
) -> Result<String, String> {
    edited(text, inherited, |tables| {
        let Some(from) = find_entry(tables, index, line) else {
            return;
        };
        let Some(to) = from
            .checked_add_signed(step)
            .filter(|to| *to < tables.len())
        else {
            return;
        };
        // Tables are written in the order of their place in the file, not
        // of the list, so each takes the other's place along with its slot.
        let (Some(mut a), Some(mut b)) = (tables.get(from).cloned(), tables.get(to).cloned())
        else {
            return;
        };
        let (place_a, place_b) = (a.position(), b.position());
        a.set_position(place_b);
        b.set_position(place_a);
        if let Some(slot) = tables.get_mut(from) {
            *slot = b;
        }
        if let Some(slot) = tables.get_mut(to) {
            *slot = a;
        }
    })
}

/// Apply `change` to the writable configuration and repaint.
fn write(change: impl FnOnce(&str, &[Entry]) -> Result<String, String>) {
    let Some(path) = writable_path() else {
        eprintln!("startup: no home folder to keep config.toml in");
        return;
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            eprintln!("startup: cannot read {}: {err}", path.display());
            return;
        }
    };
    let inherited = list().entries;
    let next = match change(&text, &inherited) {
        Ok(next) => next,
        Err(err) => {
            eprintln!(
                "startup: {} does not parse, leaving it alone: {err}",
                path.display()
            );
            return;
        }
    };
    if let Err(err) = write_atomically(&path, &next) {
        eprintln!("startup: cannot write {}: {err}", path.display());
        return;
    }
    *cached().lock().unwrap() = None;
    DIRTY.store(true, Ordering::Relaxed);
    otto_kit::AppContext::request_wakeup();
}

/// Replace `path` with `text` in one step, so nothing reads it half written.
fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension(format!("toml.tmp.{}", std::process::id()));
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, path)
}

/// What the list box keeps between frames: the item picked, the one being
/// dragged, and whether the field for a typed command is showing.
#[derive(Default)]
struct Ui {
    selected: Option<usize>,
    dragging: Option<usize>,
    adding: bool,
    /// Set when "Command…" is chosen, until `main.rs` puts the keyboard in
    /// the field.
    focus_field: bool,
}

fn ui() -> std::sync::MutexGuard<'static, Ui> {
    static UI: OnceLock<Mutex<Ui>> = OnceLock::new();
    UI.get_or_init(Default::default).lock().unwrap()
}

/// Repaint after a change to [`Ui`].
fn changed() {
    DIRTY.store(true, Ordering::Relaxed);
    otto_kit::AppContext::request_wakeup();
}

/// One line of the list box: an entry the list has, or a component it does
/// not, offered unticked at the end so it can be put back.
enum Line {
    Entry(usize, Entry),
    Missing(&'static str),
}

/// The list box's lines, in order.
fn lines(entries: &[Entry]) -> Vec<Line> {
    let mut lines: Vec<Line> = entries
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, entry)| Line::Entry(index, entry))
        .collect();
    lines.extend(
        COMPONENTS
            .iter()
            .filter(|(program, ..)| !entries.iter().any(|entry| entry.runs(program)))
            .map(|(program, ..)| Line::Missing(program)),
    );
    lines
}

/// The component `entry` starts, if it is one of Otto's.
fn component_of(entry: &Entry) -> Option<&'static (&'static str, &'static str, &'static str)> {
    COMPONENTS.iter().find(|(program, ..)| entry.runs(program))
}

fn list_item(line: &Line) -> ListBoxItem {
    match line {
        Line::Entry(_, entry) => match component_of(entry) {
            Some((_, label, detail)) => ListBoxItem {
                title: otto_kit::t_owned!(*label),
                subtitle: Some(otto_kit::t_owned!(*detail)),
                checked: entry.enabled,
            },
            None => ListBoxItem {
                title: Path::new(&entry.cmd)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&entry.cmd)
                    .to_string(),
                subtitle: Some(entry.line()),
                checked: entry.enabled,
            },
        },
        Line::Missing(program) => {
            let (_, label, detail) = COMPONENTS
                .iter()
                .find(|(known, ..)| known == program)
                .expect("a missing line is one of the components");
            ListBoxItem {
                title: otto_kit::t_owned!(*label),
                subtitle: Some(otto_kit::t_owned!(*detail)),
                checked: false,
            }
        }
    }
}

/// The Start at login group: where startup apps come from, the list of them
/// in launch order, and the field a typed command goes into while it is
/// being added.
pub fn groups() -> Vec<Group> {
    let list = list();
    let lines = lines(&list.entries);
    let ui = ui();
    let selected = ui.selected.filter(|index| *index < lines.len());
    // Otto's own apps are switched off, never taken out.
    let can_remove = selected
        .and_then(|index| lines.get(index))
        .is_some_and(|line| matches!(line, Line::Entry(_, entry) if component_of(entry).is_none()));
    let mut source = Row::new(
        otto_kit::t!("settings-startup-apps"),
        Control::Select(source_value(list.xdg).to_string()),
    )
    .detail(otto_kit::t!("settings-startup-xdg-detail"));
    source.id = Some(XDG_ID);
    let mut rows = vec![source];
    let mut items = Row::new(
        otto_kit::t!("settings-startup-list"),
        Control::ListBox(ListBox {
            items: lines.iter().map(list_item).collect(),
            selected,
            can_remove,
            action: Some(otto_kit::t!("settings-startup-open-config")),
        }),
    );
    items.id = Some(LIST_ID);
    rows.push(items);
    if ui.adding {
        let mut field = Row::new(
            otto_kit::t!("settings-startup-add"),
            Control::Text(String::new()),
        )
        .detail(otto_kit::t!("settings-startup-add-detail"));
        field.id = Some(ADD_ID);
        rows.push(field);
    }
    vec![group(otto_kit::t!("settings-group-start-at-login"), rows)]
}

/// What the source pop-up holds for `xdg`.
fn source_value(xdg: bool) -> &'static str {
    if xdg {
        SOURCES[1]
    } else {
        SOURCES[0]
    }
}

fn source_label(value: &str) -> String {
    if value == SOURCES[1] {
        otto_kit::t_owned!("settings-startup-apps-xdg")
    } else {
        otto_kit::t_owned!("settings-startup-apps-otto")
    }
}

/// The choices for the source pop-up and for the menu "+" opens.
pub fn menu_choices(id: &str) -> Option<Vec<Choice>> {
    let choice = |label: String, value: &str| Choice {
        label,
        value: value.to_string(),
    };
    match id {
        XDG_ID => Some(
            SOURCES
                .iter()
                .map(|value| choice(source_label(value), value))
                .collect(),
        ),
        ADD_MENU_ID => Some(vec![
            choice(otto_kit::t_owned!("settings-startup-menu-app"), ADD_APP),
            choice(
                otto_kit::t_owned!("settings-startup-menu-command"),
                ADD_COMMAND,
            ),
        ]),
        _ => None,
    }
}

/// What the source pop-up shows for `value`, or `None` for any other.
pub fn display(id: &str, value: &str) -> Option<String> {
    (id == XDG_ID).then(|| source_label(value))
}

/// Take a choice from the source pop-up or the "+" menu. Returns whether `id`
/// was the group's.
pub fn choose(id: &str, value: &str) -> bool {
    match id {
        XDG_ID => {
            let on = value == SOURCES[1];
            write(move |text, _| with_xdg(text, on));
            true
        }
        ADD_MENU_ID => {
            match value {
                ADD_APP => choose_app(),
                ADD_COMMAND => {
                    let mut ui = ui();
                    ui.adding = true;
                    ui.focus_field = true;
                    drop(ui);
                    changed();
                }
                _ => {}
            }
            true
        }
        _ => false,
    }
}

/// Whether "Command…" asked for the keyboard to go to the command field
/// since the last call.
pub fn take_command_request() -> bool {
    std::mem::take(&mut ui().focus_field)
}

/// A press on an item's checkbox: an entry switched on or off where it
/// stands, or a missing component put back at the end.
pub fn list_check(row: &str, index: usize) {
    if row != LIST_ID {
        return;
    }
    match lines(&list().entries).into_iter().nth(index) {
        Some(Line::Entry(index, entry)) => {
            let line = entry.line();
            let on = !entry.enabled;
            write(move |text, inherited| with_enabled(text, inherited, index, &line, on));
        }
        Some(Line::Missing(program)) => {
            write(move |text, inherited| with_component(text, inherited, program, true));
        }
        None => {}
    }
}

/// A press on an item anywhere but its checkbox: it is picked, and held for
/// a drag if it is in the list.
pub fn list_pick(row: &str, index: usize) {
    if row != LIST_ID {
        return;
    }
    let in_list = index < list().entries.len();
    let mut ui = ui();
    ui.selected = Some(index);
    ui.dragging = in_list.then_some(index);
    drop(ui);
    changed();
}

/// The list box an item is being dragged in, if one is.
pub fn dragging() -> Option<&'static str> {
    ui().dragging.map(|_| LIST_ID)
}

/// The held item is over the item at `over`: it moves a place towards it,
/// and the next motion moves it on.
pub fn drag_over(over: usize) {
    let Some(from) = ui().dragging else {
        return;
    };
    let entries = list().entries;
    let over = over.min(entries.len().saturating_sub(1));
    if over == from {
        return;
    }
    let Some(line) = entries.get(from).map(Entry::line) else {
        return;
    };
    let step: isize = if over > from { 1 } else { -1 };
    write(move |text, inherited| moved(text, inherited, from, &line, step));
    let to = from.saturating_add_signed(step);
    let mut ui = ui();
    ui.dragging = Some(to);
    ui.selected = Some(to);
}

/// The pointer let go: nothing is held any more.
pub fn end_drag() {
    ui().dragging = None;
}

/// A footer button let go on: "−" takes the picked entry out, the text
/// button opens the file.
pub fn list_activate(row: &str, hit: ListBoxHit) {
    if row != LIST_ID {
        return;
    }
    match hit {
        ListBoxHit::Remove => {
            let picked = ui().selected;
            let entries = list().entries;
            let Some((index, entry)) = picked.and_then(|index| Some((index, entries.get(index)?)))
            else {
                return;
            };
            if component_of(entry).is_some() {
                return;
            }
            let line = entry.line();
            write(move |text, inherited| without_entry(text, inherited, index, &line));
            ui().selected = None;
        }
        ListBoxHit::Action => {
            if let Some(path) = writable_path() {
                super::open_in_default_app(&path);
            }
        }
        ListBoxHit::Add | ListBoxHit::Item(_) | ListBoxHit::Check(_) => {}
    }
}

/// Ask for an application or a program through the desktop portal and add
/// what it runs. The portal blocks, so the request runs on its own thread.
fn choose_app() {
    let spawned = Request::open_file(otto_kit::t!("settings-startup-choose-title"))
        .current_folder(APPLICATIONS)
        .filter(
            otto_kit::t!("settings-startup-filter-apps"),
            &[Rule::Glob("*.desktop".into())],
        )
        .filter(
            otto_kit::t!("settings-startup-filter-all"),
            &[Rule::Glob("*".into())],
        )
        .spawn(|outcome| {
            let path = match outcome {
                Outcome::Chosen(paths) => paths.into_iter().next(),
                Outcome::Dismissed => None,
                Outcome::Failed(why) => {
                    eprintln!("startup: {why}");
                    None
                }
            };
            let Some(path) = path else {
                return;
            };
            let is_entry = path.extension().is_some_and(|ext| ext == "desktop");
            if !is_entry && !is_executable(&path) {
                eprintln!("startup: {} is not a program", path.display());
                return;
            }
            let text = is_entry
                .then(|| std::fs::read_to_string(&path).ok())
                .flatten();
            let Some(entry) = entry_for_file(&path, text.as_deref()) else {
                eprintln!("startup: {} runs nothing", path.display());
                return;
            };
            write(move |text, inherited| with_command(text, inherited, &entry));
        });
    if let Err(err) = spawned {
        eprintln!("startup: could not open the file picker: {err}");
    }
}

/// Take a typed command into the list. Returns whether `id` was the
/// group's, taken or not.
pub fn apply(id: &str, value: &Value) -> bool {
    if id != ADD_ID {
        return false;
    }
    if let Some(entry) = match value {
        Value::Text(line) => parse_line(line),
        _ => None,
    } {
        write(move |text, inherited| with_command(text, inherited, &entry));
    }
    ui().adding = false;
    changed();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system() -> Vec<Entry> {
        COMPONENTS
            .iter()
            .map(|(program, ..)| Entry {
                cmd: program.to_string(),
                args: Vec::new(),
                enabled: true,
            })
            .collect()
    }

    #[test]
    fn the_first_layer_with_a_list_decides() {
        let mine = "[rendering]\nrenderer = \"vulkan\"\n".to_string();
        let etc = "[[exec_once]]\ncmd = \"otto-bar\"\n".to_string();
        let list = list_in(&[Some(mine), None, Some(etc)]);
        assert!(!list.own);
        assert_eq!(list.entries.len(), 1);
        assert!(list.entries[0].runs("otto-bar"));

        let mine = "exec_once = []\n".to_string();
        let list = list_in(&[Some(mine), Some("[[exec_once]]\ncmd = \"x\"\n".into())]);
        assert!(list.own);
        assert!(list.entries.is_empty());
    }

    #[test]
    fn a_component_is_known_by_its_path_too() {
        let entry = Entry {
            cmd: "/home/me/.local/bin/otto-stash".into(),
            args: vec!["add".into()],
            enabled: true,
        };
        assert!(entry.runs("otto-stash"));
        assert!(!entry.runs("otto-bar"));
    }

    #[test]
    fn the_first_change_copies_the_inherited_list_in() {
        let text = "# mine\n[rendering]\nrenderer = \"vulkan\"\n";
        let next = with_component(text, &system(), "otto-stash", false).unwrap();
        assert!(next.starts_with("# mine"));
        assert!(next.contains("renderer = \"vulkan\""));
        let list = read_list(&next).unwrap();
        let programs: Vec<&str> = list.iter().map(|entry| entry.cmd.as_str()).collect();
        assert_eq!(programs, ["otto-bar", "otto-islands", "otto-canvas"]);
    }

    #[test]
    fn a_component_turned_back_on_is_added_once() {
        let text = "[[exec_once]]\ncmd = \"otto-bar\"\nargs = []\n";
        let next = with_component(text, &system(), "otto-stash", true).unwrap();
        let again = with_component(&next, &system(), "otto-stash", true).unwrap();
        let programs: Vec<String> = read_list(&again)
            .unwrap()
            .into_iter()
            .map(|entry| entry.cmd)
            .collect();
        assert_eq!(programs, ["otto-bar", "otto-stash"]);
    }

    #[test]
    fn the_last_one_off_leaves_an_empty_list_rather_than_none() {
        let text = "[[exec_once]]\ncmd = \"otto-bar\"\n";
        let next = with_component(text, &system(), "otto-bar", false).unwrap();
        assert_eq!(read_list(&next), Some(Vec::new()));
        let back = with_component(&next, &system(), "otto-bar", true).unwrap();
        assert_eq!(read_list(&back).unwrap().len(), 1);
    }

    #[test]
    fn comments_on_the_kept_entries_stay() {
        let text = "# The bar.\n[[exec_once]]\ncmd = \"otto-bar\"\n\n# Mine.\n[[exec_once]]\ncmd = \"foo\"\n";
        let next = with_component(text, &system(), "otto-bar", false).unwrap();
        assert!(!next.contains("# The bar."));
        assert!(next.contains("# Mine."));
    }

    #[test]
    fn a_command_is_added_and_removed_by_its_line() {
        let entry = parse_line("  notify-send 'Hello there' -t 5 ").unwrap();
        assert_eq!(entry.cmd, "notify-send");
        assert_eq!(entry.args, ["Hello there", "-t", "5"]);
        assert_eq!(entry.line(), "notify-send \"Hello there\" -t 5");
        assert!(parse_line("   ").is_none());

        let next = with_command("", &system(), &entry).unwrap();
        assert_eq!(read_list(&next).unwrap().len(), 5);
        let back = without_entry(&next, &system(), 4, &entry.line()).unwrap();
        assert_eq!(read_list(&back).unwrap(), system());
    }

    #[test]
    fn an_inline_list_is_read_and_rewritten_as_tables() {
        let text = "exec_once = [{ cmd = \"otto-bar\" }, { cmd = \"foo\", args = [\"-x\"] }]\n";
        let next = with_component(text, &system(), "otto-bar", false).unwrap();
        assert!(next.contains("[[exec_once]]"));
        assert_eq!(read_list(&next).unwrap()[0].line(), "foo -x");
    }

    #[test]
    fn a_file_that_does_not_parse_is_left_alone() {
        assert!(with_component("[x\n", &system(), "otto-bar", true).is_err());
        assert!(with_component("exec_once = 3\n", &system(), "otto-bar", true).is_err());
    }

    #[test]
    fn the_xdg_switch_is_read_from_the_first_layer_that_says() {
        assert!(!list_in(&[None]).xdg);
        let list = list_in(&[
            Some("[dock]\n".into()),
            Some("xdg_autostart = true\n".into()),
        ]);
        assert!(list.xdg);
        let next = with_xdg("# mine\n[dock]\nsize = 3\n", true).unwrap();
        assert_eq!(read_xdg(&next), Some(true));
        // A key at the top level, not inside the last table, and the
        // table's comment still over it.
        assert!(next.find("xdg_autostart").unwrap() < next.find("[dock]").unwrap());
        assert!(next.contains("# mine\n[dock]"));
    }

    #[test]
    fn a_desktop_entry_runs_its_exec_line() {
        let text = "[Desktop Entry]\nName=Foo\nExec=foo --bar %U\n\n[Desktop Action new]\nExec=foo --new\n";
        let entry = entry_for_file(Path::new("/x/foo.desktop"), Some(text)).unwrap();
        assert_eq!(entry.line(), "foo --bar");
        assert!(entry_for_file(Path::new("/x/foo.desktop"), Some("[Desktop Entry]\n")).is_none());
        let program = entry_for_file(Path::new("/usr/bin/foo"), None).unwrap();
        assert_eq!(program.line(), "/usr/bin/foo");
    }

    fn programs(text: &str) -> Vec<String> {
        read_list(text)
            .unwrap()
            .into_iter()
            .map(|entry| entry.cmd)
            .collect()
    }

    #[test]
    fn an_entry_moves_with_its_comment_and_stops_at_the_ends() {
        let text = "# The bar.\n[[exec_once]]\ncmd = \"otto-bar\"\n\n# Mine.\n[[exec_once]]\ncmd = \"foo\"\n\n[dock]\nsize = 3\n";
        let next = moved(text, &[], 1, "foo", -1).unwrap();
        assert_eq!(programs(&next), ["foo", "otto-bar"]);
        assert!(next.find("# Mine.").unwrap() < next.find("# The bar.").unwrap());
        assert!(next.find("otto-bar").unwrap() < next.find("[dock]").unwrap());
        assert_eq!(moved(&next, &[], 0, "foo", -1).unwrap(), next);
        assert_eq!(moved(&next, &[], 1, "otto-bar", 1).unwrap(), next);
    }

    #[test]
    fn a_move_finds_its_entry_when_the_file_moved_it() {
        let text =
            "[[exec_once]]\ncmd = \"a\"\n[[exec_once]]\ncmd = \"b\"\n[[exec_once]]\ncmd = \"c\"\n";
        // The row showed "c" at 1; it is at 2 now.
        let next = moved(text, &[], 1, "c", -1).unwrap();
        assert_eq!(programs(&next), ["a", "c", "b"]);
    }

    #[test]
    fn the_first_move_copies_the_inherited_list_in() {
        let next = moved("", &system(), 2, "otto-stash", -1).unwrap();
        assert_eq!(
            programs(&next),
            ["otto-bar", "otto-stash", "otto-islands", "otto-canvas"]
        );
    }

    #[test]
    fn an_entry_switched_off_keeps_its_place_and_back_on_drops_the_key() {
        let text = "[[exec_once]]\ncmd = \"otto-bar\"\n\n[[exec_once]]\ncmd = \"otto-stash\"\n";
        let off = with_enabled(text, &[], 1, "otto-stash", false).unwrap();
        let list = read_list(&off).unwrap();
        assert_eq!(list[1].cmd, "otto-stash");
        assert!(!list[1].enabled);
        assert!(off.contains("enabled = false"));
        let on = with_enabled(&off, &[], 1, "otto-stash", true).unwrap();
        assert!(read_list(&on).unwrap()[1].enabled);
        assert!(!on.contains("enabled"));
    }

    #[test]
    fn a_missing_component_is_offered_after_the_list() {
        let entries =
            read_list("[[exec_once]]\ncmd = \"foo\"\n[[exec_once]]\ncmd = \"otto-bar\"\n").unwrap();
        let lines = lines(&entries);
        assert!(matches!(&lines[0], Line::Entry(0, entry) if entry.cmd == "foo"));
        assert!(matches!(&lines[1], Line::Entry(1, entry) if entry.cmd == "otto-bar"));
        let missing: Vec<&str> = lines[2..]
            .iter()
            .map(|line| match line {
                Line::Missing(program) => *program,
                Line::Entry(..) => "",
            })
            .collect();
        assert_eq!(missing, ["otto-islands", "otto-stash", "otto-canvas"]);
    }

    #[test]
    fn the_pop_ups_offer_their_choices_and_own_only_their_ids() {
        let values = |id| -> Vec<String> {
            menu_choices(id)
                .unwrap()
                .into_iter()
                .map(|choice| choice.value)
                .collect()
        };
        assert_eq!(values(XDG_ID), SOURCES);
        assert_eq!(values(ADD_MENU_ID), [ADD_APP, ADD_COMMAND]);
        assert!(menu_choices("desk.enabled").is_none());
        assert!(display("desk.enabled", "otto").is_none());
        assert_eq!(source_value(true), "otto-xdg");
        assert!(!apply("desk.enabled", &Value::Bool(true)));
    }
}
