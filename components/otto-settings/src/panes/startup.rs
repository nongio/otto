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
//! Each component has a switch. Other commands are listed with a "−" that
//! takes them out, and *Add command* appends one.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use toml_edit::{ArrayOfTables, DocumentMut, Item};

use crate::model::{group, Control, Group, Row};
use crate::settings_client::{self, Value};

/// The key the compositor reads the list from.
const KEY: &str = "exec_once";

/// The system's configuration, the bottom layer.
const SYSTEM_CONFIG: &str = "/etc/otto/config.toml";

/// A component switch's identifier is this followed by the component's
/// program. Not session settings: the group answers for them.
const COMPONENT_ROW: &str = "startup.component.";

/// Another command's row identifier is this followed by its command line.
const COMMAND_ROW: &str = "startup.command.";

/// The *Add command* field.
const ADD_ID: &str = "startup.add";

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
    })
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
    Some(Entry { cmd, args })
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
}

/// The first layer that sets a list, given each layer's text.
fn list_in(texts: &[Option<String>]) -> List {
    texts
        .iter()
        .enumerate()
        .find_map(|(layer, text)| {
            let entries = read_list(text.as_deref()?)?;
            Some(List {
                entries,
                own: layer == 0,
            })
        })
        .unwrap_or_default()
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

/// `text` without the first command that reads as `line`.
fn without_command(text: &str, inherited: &[Entry], line: &str) -> Result<String, String> {
    edited(text, inherited, |tables| {
        let index = tables
            .iter()
            .position(|table| entry(table).is_some_and(|entry| entry.line() == line));
        if let Some(index) = index {
            tables.remove(index);
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

/// `text` as a `&'static str`, kept once however often it is asked for: row
/// labels and identifiers are static, and command lines are not known until
/// the list is read.
fn intern(text: String) -> &'static str {
    static INTERNED: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let mut interned = INTERNED.get_or_init(Default::default).lock().unwrap();
    interned
        .entry(text)
        .or_insert_with_key(|text| text.clone().leak())
}

/// The Start at login group: a switch per component, the other commands,
/// and the field that adds one.
pub fn group_rows() -> Group {
    let list = list();
    let mut rows: Vec<Row> = COMPONENTS
        .iter()
        .map(|(program, label, detail)| {
            let on = list.entries.iter().any(|entry| entry.runs(program));
            let mut row =
                Row::new(otto_kit::t!(*label), Control::Toggle(on)).detail(otto_kit::t!(*detail));
            row.id = Some(intern(format!("{COMPONENT_ROW}{program}")));
            row
        })
        .collect();
    let others = list
        .entries
        .iter()
        .filter(|entry| !COMPONENTS.iter().any(|(program, ..)| entry.runs(program)));
    for entry in others {
        let line = entry.line();
        let mut row = Row::new(intern(line.clone()), Control::Value(String::new())).removable(true);
        row.id = Some(intern(format!("{COMMAND_ROW}{line}")));
        rows.push(row);
    }
    let detail = if list.own {
        otto_kit::t!("settings-startup-add-detail")
    } else {
        otto_kit::t!("settings-startup-system-detail")
    };
    let mut add = Row::new(
        otto_kit::t!("settings-startup-add"),
        Control::Text(String::new()),
    )
    .detail(detail);
    add.id = Some(ADD_ID);
    rows.push(add);
    group(otto_kit::t!("settings-group-start-at-login"), rows)
}

/// Take a switch or a new command into the configuration. Returns whether
/// `id` was the group's, taken or not.
pub fn apply(id: &str, value: &Value) -> bool {
    if let Some(program) = id.strip_prefix(COMPONENT_ROW) {
        let Some((program, ..)) = COMPONENTS.iter().find(|(known, ..)| *known == program) else {
            return true;
        };
        if let Value::Bool(on) = value {
            let on = *on;
            write(move |text, inherited| with_component(text, inherited, program, on));
        }
        return true;
    }
    if id != ADD_ID {
        return false;
    }
    if let Some(entry) = match value {
        Value::Text(line) => parse_line(line),
        _ => None,
    } {
        write(move |text, inherited| with_command(text, inherited, &entry));
    }
    true
}

/// A press on another command's "−".
pub fn remove(id: &str) {
    let Some(line) = id.strip_prefix(COMMAND_ROW) else {
        return;
    };
    let line = line.to_string();
    write(move |text, inherited| without_command(text, inherited, &line));
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
        let back = without_command(&next, &system(), &entry.line()).unwrap();
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
    fn the_group_owns_only_its_own_ids() {
        assert!(!apply("desk.enabled", &Value::Bool(true)));
        assert!(apply("startup.component.otto-nothing", &Value::Bool(true)));
    }
}
