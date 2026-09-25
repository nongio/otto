//! The agents pane.
//!
//! Edits `agents.toml`, the file the otto-agents service reads when it
//! starts. This is the one pane that writes a file itself: the service serves
//! no settings interface, so there is nothing on the bus to send a change to.
//!
//! Changes are held here until Apply rather than written as they are made.
//! The service only reads its configuration at startup, so a change means a
//! restart, and a restart ends whatever the agents were doing; five edits
//! should cost one restart, not five. Apply writes only the keys that changed,
//! with `toml_edit`, so comments and everything the pane does not show stay as
//! the user wrote them, then restarts the service if it is running.
//!
//! Which file holds what follows the service's own rules (`config::load` in
//! otto-agents): `/etc/otto/agents.toml` then the user's, each top-level key
//! taken from the last file that sets it, and the agent list from the last
//! file that lists any. Apply always writes the user's file. When the agents
//! came from the system file, the first change to one copies that list into
//! the user's file, since a user file listing agents replaces the system list
//! wholesale.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};

use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Table};

use crate::model::{group, untitled, Control, Pane, Row};

mod harness;
mod instructions;

use harness::Harness;
use instructions::Instructions;

/// The permission policies `agents.toml` accepts, in the order the pop-up
/// offers them. `deny` is what an agent gets when the key is unset.
const PERMISSIONS: [&str; 3] = ["deny", "ask", "allow"];

/// The frosted materials an agent's `colour` names — `Colour::NAMES` in
/// otto-agents, whose list this has to match.
const COLOURS: [&str; 12] = [
    "red", "orange", "amber", "yellow", "lime", "green", "teal", "cyan", "blue", "indigo",
    "violet", "magenta",
];

/// The pop-up identifier of the default-agent row.
const DEFAULT_ID: &str = "agents.default";

/// The unit Apply restarts.
const SERVICE: &str = "otto-agents.service";

/// One agent's settings as the pane edits them. Empty strings stand for an
/// unset key, which is how the file says "the agent's own default".
#[derive(Clone, Debug, PartialEq)]
struct Agent {
    id: String,
    name: String,
    /// `command` followed by `args`.
    command: Vec<String>,
    /// The plugin agent it runs as, which a harness switch carries over.
    persona: String,
    /// A harness picked from the pop-up, whose setup Apply writes along with
    /// the command. `None` once written, and for a command typed by hand.
    preset: Option<Harness>,
    permissions: String,
    model: String,
    /// As written, `~` and all: this is text for the user to read and edit,
    /// not a path the pane opens.
    folder: String,
    colour: String,
}

impl Agent {
    /// An agent with nothing set but its id: what a new agent is compared
    /// against, so that everything it has is written.
    fn blank(id: &str) -> Self {
        Self {
            id: id.to_string(),
            name: id.to_string(),
            command: Vec::new(),
            persona: String::new(),
            preset: None,
            permissions: PERMISSIONS[0].to_string(),
            model: String::new(),
            folder: String::new(),
            colour: String::new(),
        }
    }

    fn from_table(table: &Table) -> Option<Self> {
        let text = |key: &str| {
            table
                .get(key)
                .and_then(Item::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let id = text("id");
        if id.is_empty() {
            return None;
        }
        let permissions = match text("permissions") {
            p if p.is_empty() => PERMISSIONS[0].to_string(),
            p => p,
        };
        let colour = match text("colour") {
            c if c.is_empty() => text("color"),
            c => c,
        };
        Some(Self {
            name: match text("name") {
                n if n.is_empty() => id.clone(),
                n => n,
            },
            id,
            command: std::iter::once(text("command"))
                .filter(|command| !command.is_empty())
                .chain(
                    table
                        .get("args")
                        .and_then(Item::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|arg| arg.as_str().map(str::to_string)),
                )
                .collect(),
            persona: harness::persona(table),
            preset: None,
            permissions,
            model: text("model"),
            folder: text("folder"),
            colour,
        })
    }
}

/// What the files configure, as far as the pane shows it.
#[derive(Clone, Debug, Default, PartialEq)]
struct Config {
    agents: Vec<Agent>,
    /// An agent id, empty when unset — the service then takes the first.
    default_agent: String,
}

/// What was read, and from where.
#[derive(Default)]
struct State {
    /// As the files hold it.
    saved: Config,
    /// As the pane shows it. Differs from `saved` until Apply or Revert.
    draft: Config,
    /// The file the agent list came from, which Apply copies it out of when
    /// that is not the user's own.
    source: Option<PathBuf>,
    /// Why the last load or Apply failed, shown on the Apply row.
    error: Option<String>,
    /// The agent whose name is being edited, whose first row is a name field
    /// rather than its buttons.
    renaming: Option<usize>,
    /// A name field waiting for `main.rs` to start editing it.
    rename_request: Option<usize>,
    /// The agent files the plugins hold, found when the files are read.
    instructions: Vec<Instructions>,
}

fn state() -> &'static RwLock<State> {
    static STATE: OnceLock<RwLock<State>> = OnceLock::new();
    STATE.get_or_init(|| RwLock::new(load()))
}

fn system_path() -> PathBuf {
    PathBuf::from("/etc/otto/agents.toml")
}

/// The user's `agents.toml`, the file Apply writes.
fn user_path() -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_default();
    config.join("otto").join("agents.toml")
}

/// The user's file as the pane names it, with the home folder as `~`.
fn user_path_shown() -> String {
    shown(&user_path())
}

/// Read both files the way the service does.
fn load() -> State {
    let mut state = State::default();
    let mut files = Vec::new();
    for path in [system_path(), user_path()] {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        match text.parse::<DocumentMut>() {
            Ok(doc) => files.push((path, doc)),
            Err(err) => state.error = Some(format!("{}: {err}", path.display())),
        }
    }
    let (config, source) = merge(&files);
    state.saved = config.clone();
    state.draft = config;
    state.source = source;
    state.instructions = instructions::discover();
    state
}

/// The configuration a list of files adds up to, and which of them the agent
/// list came from.
fn merge(files: &[(PathBuf, DocumentMut)]) -> (Config, Option<PathBuf>) {
    let mut config = Config::default();
    let mut source = None;
    for (path, doc) in files {
        let agents: Vec<Agent> = doc
            .get("agents")
            .and_then(Item::as_array_of_tables)
            .map(|list| list.iter().filter_map(Agent::from_table).collect())
            .unwrap_or_default();
        if !agents.is_empty() {
            config.agents = agents;
            source = Some(path.clone());
        }
        if let Some(default) = doc.get("default_agent").and_then(Item::as_str) {
            config.default_agent = default.to_string();
        }
    }
    (config, source)
}

/// Write the changes between `saved` and `draft` into `user`, the user's file
/// as it stands. `source` is the file the agents were read from, for when that
/// was not the user's.
///
/// `home` is the home folder, for the harness route that needs an absolute
/// path.
fn render(
    user: &str,
    source: Option<&str>,
    saved: &Config,
    draft: &Config,
    home: &str,
) -> Result<String, String> {
    let mut doc = user.parse::<DocumentMut>().map_err(|err| err.to_string())?;

    if draft.default_agent != saved.default_agent {
        set_or_remove(doc.as_table_mut(), "default_agent", &draft.default_agent);
    }

    if draft.agents != saved.agents {
        let listed = doc
            .get("agents")
            .and_then(Item::as_array_of_tables)
            .is_some_and(|list| !list.is_empty());
        if !listed {
            let copied = source
                .map(|text| text.parse::<DocumentMut>().map_err(|err| err.to_string()))
                .transpose()?
                .and_then(|doc| {
                    doc.get("agents")
                        .and_then(Item::as_array_of_tables)
                        .cloned()
                })
                .unwrap_or_else(ArrayOfTables::new);
            doc.insert("agents", Item::ArrayOfTables(copied));
        }
        let list = doc
            .get_mut("agents")
            .and_then(Item::as_array_of_tables_mut)
            .ok_or("`agents` is not a list of tables")?;
        // Removed agents go first, so a new agent reusing an id is not taken
        // for the one it replaces.
        let removed = |id: &str| {
            saved.agents.iter().any(|a| a.id == id) && !draft.agents.iter().any(|a| a.id == id)
        };
        list.retain(|table| !table.get("id").and_then(Item::as_str).is_some_and(removed));
        for after in &draft.agents {
            let blank = Agent::blank(&after.id);
            let before = saved
                .agents
                .iter()
                .find(|a| a.id == after.id)
                .unwrap_or(&blank);
            if before == after {
                continue;
            }
            let position = list.iter().position(|table| {
                table.get("id").and_then(Item::as_str) == Some(after.id.as_str())
            });
            let position = match position {
                Some(position) => position,
                None if before == &blank => {
                    let mut table = Table::new();
                    table.insert("id", value(&after.id));
                    list.push(table);
                    list.len() - 1
                }
                None => return Err(format!("agent `{}` is no longer in the file", after.id)),
            };
            let table = list
                .get_mut(position)
                .ok_or("`agents` changed while it was written")?;
            if before.name != after.name {
                set_or_remove(table, "name", &after.name);
            }
            if before.command != after.command {
                if let Some((command, args)) = after.command.split_first() {
                    set_or_remove(table, "command", command);
                    harness::set_list(table, "args", args);
                }
            }
            if let Some(preset) = after.preset {
                preset.write(table, &after.persona, home);
            }
            if before.permissions != after.permissions {
                set_or_remove(table, "permissions", &after.permissions);
            }
            if before.model != after.model {
                set_or_remove(table, "model", &after.model);
            }
            if before.folder != after.folder {
                set_or_remove(table, "folder", &after.folder);
            }
            if before.colour != after.colour {
                // Written under whichever spelling the file already uses.
                let key = if table.contains_key("color") {
                    "color"
                } else {
                    "colour"
                };
                set_or_remove(table, key, &after.colour);
            }
        }
    }
    Ok(doc.to_string())
}

/// Set `key` to `text`, or remove it when `text` is empty.
///
/// An existing key has its value replaced in place: inserting would replace
/// the key too, and the comment above a key belongs to the key.
fn set_or_remove(table: &mut Table, key: &str, text: &str) {
    if text.is_empty() {
        table.remove(key);
    } else if let Some(item) = table.get_mut(key) {
        *item = value(text);
    } else {
        table.insert(key, value(text));
    }
}

/// Write the held changes and restart the service.
fn apply() {
    let mut state = state().write().unwrap();
    if state.draft == state.saved {
        return;
    }
    let path = user_path();
    let result = (|| {
        let user = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(err.to_string()),
        };
        let source = state
            .source
            .as_ref()
            .filter(|source| **source != path)
            .map(std::fs::read_to_string)
            .transpose()
            .map_err(|err| err.to_string())?;
        let home = std::env::var("HOME").unwrap_or_default();
        let text = render(&user, source.as_deref(), &state.saved, &state.draft, &home)?;
        write_atomically(&path, &text).map_err(|err| err.to_string())
    })();
    match result {
        Ok(()) => {
            for agent in &mut state.draft.agents {
                agent.preset = None;
            }
            state.saved = state.draft.clone();
            state.source = Some(path);
            state.error = None;
            reload_service();
        }
        Err(err) => {
            eprintln!("agents: could not save {}: {err}", path.display());
            state.error = Some(err);
        }
    }
}

/// Replace `path` with `text` in one step, so the service never starts on a
/// half-written file.
fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension(format!("toml.tmp.{}", std::process::id()));
    let mut file = std::fs::File::create(&temp)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&temp, path)
}

/// Bring the harnesses and the service up to date with the saved file.
///
/// Every harness but Claude Code reads its instructions from a copy
/// `otto-agents plugins install` writes in its own dialect and place, so that
/// runs first: an agent just pointed at a set of instructions finds its copy
/// there when the service starts it. The installer only writes what changed,
/// and never touches a file that is not its own.
///
/// Then the service restarts to read the file, with `try-restart` rather than
/// `restart`: the service is off until the user turns it on, and saving a
/// setting is not asking for it to start. On a thread, because both wait on
/// other processes and that is not a wait the window should share. A failed
/// install is shown on the Apply row.
fn reload_service() {
    let spawned = std::thread::Builder::new()
        .name("agents-reload".into())
        .spawn(|| {
            match std::process::Command::new("otto-agents")
                .args(["plugins", "install"])
                .output()
            {
                Ok(output) if output.status.success() => {}
                Ok(output) => {
                    let why = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    eprintln!("agents: otto-agents plugins install failed: {why}");
                    state().write().unwrap().error = Some(why);
                }
                Err(err) => eprintln!("agents: could not run otto-agents: {err}"),
            }
            match std::process::Command::new("systemctl")
                .args(["--user", "try-restart", SERVICE])
                .status()
            {
                Ok(status) if status.success() => {}
                Ok(status) => eprintln!("agents: restarting {SERVICE} failed: {status}"),
                Err(err) => eprintln!("agents: could not run systemctl: {err}"),
            }
        });
    if let Err(err) = spawned {
        eprintln!("agents: could not reload {SERVICE}: {err}");
    }
}

/// What systemd says about the service.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Service {
    /// Not asked yet.
    Checking,
    Running,
    Stopped,
    /// Stopped by an error rather than by someone.
    Failed,
    /// No such unit: otto-agents is not installed, or not for this user.
    Missing,
    /// No `systemctl` to ask: a session without systemd.
    Unmanaged,
}

impl Service {
    const ALL: [Service; 6] = [
        Service::Checking,
        Service::Running,
        Service::Stopped,
        Service::Failed,
        Service::Missing,
        Service::Unmanaged,
    ];

    /// Read `systemctl show -p LoadState,ActiveState`.
    fn parse(show: &str) -> Service {
        let value = |key: &str| {
            show.lines()
                .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
                .unwrap_or_default()
        };
        match (value("LoadState"), value("ActiveState")) {
            ("not-found", _) => Service::Missing,
            (_, "active" | "reloading" | "activating") => Service::Running,
            (_, "failed") => Service::Failed,
            _ => Service::Stopped,
        }
    }
}

/// The service's state as last seen, an index into [`Service::ALL`].
static SERVICE_STATE: AtomicU8 = AtomicU8::new(0);
/// Set when the state moved, until `main.rs` repaints for it.
static SERVICE_DIRTY: AtomicBool = AtomicBool::new(false);

fn service() -> Service {
    Service::ALL[SERVICE_STATE.load(Ordering::Relaxed) as usize]
}

/// Ask systemd, and wake the window if the answer changed. Returns the
/// answer, so the watcher can stop asking a system that has no systemd.
fn refresh_service() -> Service {
    let found = match std::process::Command::new("systemctl")
        .args(["--user", "show", "-p", "LoadState,ActiveState", SERVICE])
        .output()
    {
        Ok(output) if output.status.success() => {
            Service::parse(&String::from_utf8_lossy(&output.stdout))
        }
        // There, but with no user manager to answer.
        Ok(_) => Service::Unmanaged,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Service::Unmanaged,
        Err(_) => Service::Missing,
    };
    let index = Service::ALL.iter().position(|s| *s == found).unwrap_or(0) as u8;
    if SERVICE_STATE.swap(index, Ordering::Relaxed) != index {
        SERVICE_DIRTY.store(true, Ordering::Relaxed);
        otto_kit::AppContext::request_wakeup();
    }
    found
}

/// Keep [`service`] current, from the first time the pane is built.
///
/// A poll, every five seconds, on a thread of its own: the service can stop
/// or start from anywhere, systemd has nothing this app can cheaply wait on
/// without a bus connection of its own, and the draw path must never run a
/// process. The thread only touches the two atomics and wakes the main loop,
/// the arrangement `settings_client::spawn_change_listener` uses.
fn watch_service() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let spawned = std::thread::Builder::new()
            .name("agents-service".into())
            .spawn(|| {
                // Without systemd the answer cannot change, so it is asked once.
                while refresh_service() != Service::Unmanaged {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                }
            });
        if let Err(err) = spawned {
            eprintln!("agents: could not watch {SERVICE}: {err}");
        }
    });
}

/// Whether the service's state moved since the last call; `on_update` polls
/// this to repaint.
pub fn take_service_dirty() -> bool {
    SERVICE_DIRTY.swap(false, Ordering::Relaxed)
}

/// Start or restart the service, then look again at once rather than on the
/// next poll.
fn run_service(verb: &'static str) {
    let spawned = std::thread::Builder::new()
        .name("agents-service-run".into())
        .spawn(move || {
            match std::process::Command::new("systemctl")
                .args(["--user", verb, SERVICE])
                .status()
            {
                Ok(status) if status.success() => {}
                Ok(status) => eprintln!("agents: systemctl {verb} {SERVICE} failed: {status}"),
                Err(err) => eprintln!("agents: could not run systemctl: {err}"),
            }
            let _ = refresh_service();
        });
    if let Err(err) = spawned {
        eprintln!("agents: could not {verb} {SERVICE}: {err}");
    }
}

/// Whether the pane holds changes the files do not have yet.
fn dirty() -> bool {
    let state = state().read().unwrap();
    state.draft != state.saved
}

/// Throw the held changes away and read the files again, picking up anything
/// edited by hand since.
fn revert() {
    *state().write().unwrap() = load();
}

/// What a row edits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Field {
    Name,
    Harness,
    Command,
    Permissions,
    Model,
    Folder,
    Colour,
    Instructions,
    InstructionsFile,
}

impl Field {
    const ALL: [Field; 9] = [
        Field::Name,
        Field::Harness,
        Field::Command,
        Field::Instructions,
        Field::InstructionsFile,
        Field::Permissions,
        Field::Model,
        Field::Folder,
        Field::Colour,
    ];

    fn key(self) -> &'static str {
        match self {
            Field::Name => "name",
            Field::Harness => "harness",
            Field::Command => "command",
            Field::Permissions => "permissions",
            Field::Model => "model",
            Field::Folder => "folder",
            Field::Colour => "colour",
            Field::Instructions => "instructions",
            Field::InstructionsFile => "instructions-file",
        }
    }
}

/// The identifier of one agent's row, `agent.<index>.<field>`.
///
/// Rows need an identifier to own a pop-up or a text field — the menu pool and
/// the editor key on it, and labels repeat from one agent to the next. Leaked
/// once each, since the menu pool keeps `&'static str`s; the list only grows by
/// the fields of agents the files add, so the leak is bounded by the file.
fn row_id(index: usize, field: Field) -> &'static str {
    static IDS: OnceLock<Mutex<HashMap<(usize, &'static str), &'static str>>> = OnceLock::new();
    let mut ids = IDS.get_or_init(Default::default).lock().unwrap();
    ids.entry((index, field.key()))
        .or_insert_with(|| format!("agent.{index}.{}", field.key()).leak())
}

/// The agent and field an identifier names, or `None` for one not ours.
fn parse_id(id: &str) -> Option<(usize, Field)> {
    let rest = id.strip_prefix("agent.")?;
    let (index, key) = rest.split_once('.')?;
    let field = Field::ALL.into_iter().find(|field| field.key() == key)?;
    Some((index.parse().ok()?, field))
}

/// Whether a row identifier belongs to this pane, whose rows `main.rs` routes
/// here rather than onto the bus.
pub fn owns(id: &str) -> bool {
    id == DEFAULT_ID || parse_id(id).is_some()
}

/// The instructions Otto ships, listed straight after Default.
const OTTO_INSTRUCTIONS: &str = "otto";

/// How the Instructions pop-up names a set: none as Default, the harness
/// running as itself, and any other by the name in its file.
fn instructions_label(name: &str) -> String {
    if name.is_empty() {
        otto_kit::t!("settings-agent-instructions-default").to_string()
    } else {
        name.to_string()
    }
}

fn permission_label(policy: &str) -> &'static str {
    match policy {
        "ask" => otto_kit::t!("settings-agent-permissions-ask"),
        "allow" => otto_kit::t!("settings-agent-permissions-allow"),
        _ => otto_kit::t!("settings-agent-permissions-deny"),
    }
}

fn colour_label(colour: &str) -> &'static str {
    match colour {
        "red" => otto_kit::t!("settings-agent-colour-red"),
        "orange" => otto_kit::t!("settings-agent-colour-orange"),
        "amber" => otto_kit::t!("settings-agent-colour-amber"),
        "yellow" => otto_kit::t!("settings-agent-colour-yellow"),
        "lime" => otto_kit::t!("settings-agent-colour-lime"),
        "green" => otto_kit::t!("settings-agent-colour-green"),
        "teal" => otto_kit::t!("settings-agent-colour-teal"),
        "cyan" => otto_kit::t!("settings-agent-colour-cyan"),
        "blue" => otto_kit::t!("settings-agent-colour-blue"),
        "indigo" => otto_kit::t!("settings-agent-colour-indigo"),
        "violet" => otto_kit::t!("settings-agent-colour-violet"),
        "magenta" => otto_kit::t!("settings-agent-colour-magenta"),
        _ => otto_kit::t!("settings-agent-colour-none"),
    }
}

/// The choices one of this pane's pop-ups offers, or `None` for a pop-up that
/// is not ours.
///
/// Each choice is its own label: a pop-up row shows the value it holds, and a
/// row showing `ask` where the menu said "Ask me" would read as two settings.
/// [`choose`] maps the label back to what the file takes.
pub fn menu_choices(id: &str) -> Option<Vec<String>> {
    if id == DEFAULT_ID {
        let state = state().read().unwrap();
        return Some(state.draft.agents.iter().map(|a| a.name.clone()).collect());
    }
    match parse_id(id)? {
        (_, Field::Permissions) => Some(
            PERMISSIONS
                .iter()
                .map(|p| permission_label(p).to_string())
                .collect(),
        ),
        (_, Field::Harness) => Some(
            Harness::ALL
                .iter()
                .map(|harness| harness.label().to_string())
                .collect(),
        ),
        (_, Field::Instructions) => {
            let state = state().read().unwrap();
            let mut names: Vec<&str> = state.instructions.iter().map(|i| i.name.as_str()).collect();
            // Otto's own first, then whatever the user has added.
            names.sort_by_key(|name| *name != OTTO_INSTRUCTIONS);
            Some(
                std::iter::once("")
                    .chain(names)
                    .map(instructions_label)
                    .collect(),
            )
        }
        (_, Field::Colour) => Some(
            std::iter::once("")
                .chain(COLOURS)
                .map(|c| colour_label(c).to_string())
                .collect(),
        ),
        _ => None,
    }
}

/// Hold a choice made in one of this pane's pop-ups.
pub fn choose(id: &str, label: &str) {
    let mut state = state().write().unwrap();
    if id == DEFAULT_ID {
        if let Some(agent) = state.draft.agents.iter().find(|a| a.name == label) {
            state.draft.default_agent = agent.id.clone();
        }
        return;
    }
    let Some((index, field)) = parse_id(id) else {
        return;
    };
    let Some(agent) = state.draft.agents.get_mut(index) else {
        return;
    };
    match field {
        // Picking the harness the agent already runs changes nothing, and
        // Custom has nothing to set up: it is what a command the pane does
        // not recognise shows as.
        Field::Harness => {
            let Some(picked) = Harness::ALL.into_iter().find(|h| h.label() == label) else {
                return;
            };
            if picked == Harness::Custom || picked == Harness::detect(&agent.command) {
                return;
            }
            agent.command = picked.command(&agent.persona);
            agent.preset = Some(picked);
            // Models are named differently by every harness, so one set for
            // the old harness would be refused by the new one.
            agent.model.clear();
        }
        // A harness takes its instructions by its own route, so picking
        // another set rewrites that route along with the rest of the setup.
        // A command the pane does not recognise has no route it knows.
        Field::Instructions => {
            let harness = Harness::detect(&agent.command);
            if harness == Harness::Custom {
                return;
            }
            let name = if label == instructions_label("") {
                ""
            } else {
                label
            };
            if agent.persona == name {
                return;
            }
            agent.persona = name.to_string();
            if harness == Harness::Hermes {
                agent.command = harness.command(name);
            }
            agent.preset = Some(harness);
        }
        Field::Permissions => {
            if let Some(policy) = PERMISSIONS.iter().find(|p| permission_label(p) == label) {
                agent.permissions = policy.to_string();
            }
        }
        Field::Colour => {
            agent.colour = COLOURS
                .iter()
                .find(|c| colour_label(c) == label)
                .map(|c| c.to_string())
                .unwrap_or_default();
        }
        _ => {}
    }
}

/// Hold a committed edit from one of this pane's text fields.
pub fn commit_text(id: &str, text: &str) {
    let Some((index, field)) = parse_id(id) else {
        return;
    };
    let mut state = state().write().unwrap();
    let Some(agent) = state.draft.agents.get_mut(index) else {
        return;
    };
    let text = text.trim().to_string();
    match field {
        // An emptied field keeps the name it had: the name is the group's title.
        Field::Name => {
            if !text.is_empty() {
                agent.name = text;
            }
            // An agent not yet saved takes its id from its name. One already
            // saved keeps its id: its sessions are filed under it.
            let saved = state
                .saved
                .agents
                .iter()
                .any(|a| a.id == state.draft.agents[index].id);
            if !saved {
                let id = unique_id(
                    &state.draft.agents,
                    &slug(&state.draft.agents[index].name),
                    Some(index),
                );
                let old = std::mem::replace(&mut state.draft.agents[index].id, id.clone());
                if state.draft.default_agent == old {
                    state.draft.default_agent = id;
                }
            }
            state.renaming = None;
        }
        // A harness needs a command; an emptied field keeps the one it had.
        Field::Command => {
            let words = harness::split(&text);
            if !words.is_empty() {
                agent.command = words;
            }
        }
        Field::Model => agent.model = text,
        Field::Folder => agent.folder = text,
        _ => {}
    }
}

/// How many agents the pane can hold.
///
/// A cap for the same reason the shortcut list has one: every pop-up needs a
/// menu, and menus can only be made at window setup, so an agent added later
/// has to find its menus already there.
pub const MAX_AGENTS: usize = 32;

/// Every pop-up identifier the pane can ever show, for the menu pool.
pub fn slot_ids() -> Vec<&'static str> {
    std::iter::once(DEFAULT_ID)
        .chain((0..MAX_AGENTS).flat_map(|index| {
            [
                Field::Harness,
                Field::Instructions,
                Field::Permissions,
                Field::Colour,
            ]
            .map(|field| row_id(index, field))
        }))
        .collect()
}

/// An agent's name as the label of its first row.
///
/// Row labels are `&'static str`, so each distinct name is leaked once and
/// reused; there are as many as there are names the pane has shown.
fn name_label(name: &str) -> &'static str {
    static LABELS: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let mut labels = LABELS.get_or_init(Default::default).lock().unwrap();
    labels
        .entry(name.to_string())
        .or_insert_with(|| name.to_string().leak())
}

/// An id made from a name: lower case, words joined by `-`.
fn slug(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() {
            slug.extend(c.to_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "agent".into()
    } else {
        slug.into()
    }
}

/// `base`, or `base-2`, `base-3`… whichever no other agent has.
fn unique_id(agents: &[Agent], base: &str, skip: Option<usize>) -> String {
    let taken = |id: &str| {
        agents
            .iter()
            .enumerate()
            .any(|(index, agent)| Some(index) != skip && agent.id == id)
    };
    (1..)
        .map(|n| match n {
            1 => base.to_string(),
            n => format!("{base}-{n}"),
        })
        .find(|id| !taken(id))
        .expect("an unused id exists")
}

/// Append an agent on Claude Code, and open its name for editing.
fn add() {
    let mut state = state().write().unwrap();
    if state.draft.agents.len() >= MAX_AGENTS {
        return;
    }
    let name = otto_kit::t!("settings-agents-new-name");
    let id = unique_id(&state.draft.agents, &slug(name), None);
    let mut agent = Agent::blank(&id);
    agent.name = name.to_string();
    agent.persona = OTTO_INSTRUCTIONS.to_string();
    agent.command = Harness::Claude.command(&agent.persona);
    agent.preset = Some(Harness::Claude);
    // An agent that refuses everything it asks is not much use, and one made
    // here is one the user is about to try.
    agent.permissions = "ask".into();
    state.draft.agents.push(agent);
    let index = state.draft.agents.len() - 1;
    state.renaming = Some(index);
    state.rename_request = Some(index);
}

/// Drop an agent from the list. A default that named it is cleared, since
/// the service refuses a default that is not configured.
fn remove(index: usize) {
    let mut state = state().write().unwrap();
    if index >= state.draft.agents.len() {
        return;
    }
    let agent = state.draft.agents.remove(index);
    if state.draft.default_agent == agent.id {
        state.draft.default_agent.clear();
    }
    state.renaming = None;
}

/// The name field `main.rs` should start editing, with the text it starts
/// on, once per Rename.
pub fn take_rename() -> Option<(&'static str, String)> {
    let mut state = state().write().unwrap();
    let index = state.rename_request.take()?;
    let name = state.draft.agents.get(index)?.name.clone();
    Some((row_id(index, Field::Name), name))
}

/// Put an agent's buttons back when its name field is left without a commit.
pub fn stop_renaming() {
    state().write().unwrap().renaming = None;
}

fn service_row_label() -> &'static str {
    otto_kit::t!("settings-agents-service")
}
fn start_label() -> &'static str {
    otto_kit::t!("settings-agents-start")
}
fn restart_label() -> &'static str {
    otto_kit::t!("settings-agents-restart")
}

fn start_buttons() -> &'static [&'static str] {
    static BUTTONS: OnceLock<Vec<&'static str>> = OnceLock::new();
    BUTTONS.get_or_init(|| vec![start_label()])
}

fn restart_buttons() -> &'static [&'static str] {
    static BUTTONS: OnceLock<Vec<&'static str>> = OnceLock::new();
    BUTTONS.get_or_init(|| vec![restart_label()])
}

/// The row saying whether the service is up, with the button that fixes it
/// when it is not.
fn service_row() -> Row {
    let label = service_row_label();
    let (control, detail) = match service() {
        Service::Checking => (
            Control::Value(String::new()),
            otto_kit::t!("settings-agents-service-checking"),
        ),
        Service::Running => (
            Control::Button(restart_buttons()),
            otto_kit::t!("settings-agents-service-running"),
        ),
        Service::Stopped => (
            Control::Button(start_buttons()),
            otto_kit::t!("settings-agents-service-stopped"),
        ),
        Service::Failed => (
            Control::Button(start_buttons()),
            otto_kit::t!("settings-agents-service-failed"),
        ),
        Service::Missing => (
            Control::Value(String::new()),
            otto_kit::t!("settings-agents-service-missing"),
        ),
        Service::Unmanaged => (
            Control::Value(String::new()),
            otto_kit::t!("settings-agents-service-unmanaged"),
        ),
    };
    Row::new(label, control).detail(detail)
}

fn config_file() -> &'static str {
    otto_kit::t!("settings-agents-file")
}
fn open_label() -> &'static str {
    otto_kit::t!("common-open")
}

fn open_buttons() -> &'static [&'static str] {
    static BUTTONS: OnceLock<Vec<&'static str>> = OnceLock::new();
    BUTTONS.get_or_init(|| vec![open_label()])
}

/// Hand a file to whatever the desktop opens it with, the way the General
/// pane opens the compositor's config.
fn open(path: &Path) {
    if let Err(err) = std::process::Command::new("xdg-open").arg(path).spawn() {
        eprintln!("agents: could not open {}: {err}", path.display());
    }
}

/// Where Ask starts a session whose agent names no folder: a scratch folder
/// of its own (`ask.rs` in otto-launcher).
fn ask_scratch_folder() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .unwrap_or_default()
        .join("otto/ask")
}

/// The Hermes profile an agent running as `persona` starts in.
fn hermes_profile(persona: &str) -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".hermes/profiles")
        .join(persona)
}

/// A path as the pane shows it, with the home folder as `~`.
fn shown(path: &Path) -> String {
    match std::env::var_os("HOME")
        .and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf))
    {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

fn changes() -> &'static str {
    otto_kit::t!("settings-agents-changes")
}
fn rename_label() -> &'static str {
    otto_kit::t!("settings-agents-rename")
}
fn remove_label() -> &'static str {
    otto_kit::t!("common-remove")
}
fn add_label() -> &'static str {
    otto_kit::t!("common-add")
}
fn new_agent() -> &'static str {
    otto_kit::t!("settings-agents-add")
}

fn agent_buttons() -> &'static [&'static str] {
    static BUTTONS: OnceLock<Vec<&'static str>> = OnceLock::new();
    BUTTONS.get_or_init(|| vec![rename_label(), remove_label()])
}

fn add_buttons() -> &'static [&'static str] {
    static BUTTONS: OnceLock<Vec<&'static str>> = OnceLock::new();
    BUTTONS.get_or_init(|| vec![add_label()])
}

fn revert_label() -> &'static str {
    otto_kit::t!("settings-agents-revert")
}
fn apply_label() -> &'static str {
    otto_kit::t!("settings-agents-apply")
}

fn change_buttons() -> &'static [&'static str] {
    static BUTTONS: OnceLock<Vec<&'static str>> = OnceLock::new();
    BUTTONS.get_or_init(|| vec![revert_label(), apply_label()])
}

/// A press on one of this pane's push buttons.
///
/// `row` is the row's handle: an agent's row is keyed by its identifier, the
/// same one whichever agent sits at that index.
pub fn press(row: &str, button: &str) {
    if row == changes() {
        match button {
            b if b == apply_label() => apply(),
            // Reached from the keyboard too, which does not see the dimmed
            // buttons: with nothing held, there is nothing to throw away.
            b if b == revert_label() && dirty() => revert(),
            _ => {}
        }
        return;
    }
    if row == new_agent() {
        add();
        return;
    }
    if row == service_row_label() {
        match button {
            b if b == start_label() => run_service("start"),
            b if b == restart_label() => run_service("restart"),
            _ => {}
        }
        return;
    }
    if row == config_file() {
        // The user's file once it exists; until then, the one being read.
        let user = user_path();
        let path = if user.is_file() {
            Some(user)
        } else {
            state().read().unwrap().source.clone()
        };
        if let Some(path) = path {
            open(&path);
        }
        return;
    }
    if let Some((index, Field::InstructionsFile)) = parse_id(row) {
        let state = state().read().unwrap();
        let path = state
            .draft
            .agents
            .get(index)
            .and_then(|agent| state.instructions.iter().find(|i| i.name == agent.persona))
            .map(|found| found.path.clone());
        drop(state);
        if let Some(path) = path {
            open(&path);
        }
        return;
    }
    let Some((index, Field::Name)) = parse_id(row) else {
        return;
    };
    match button {
        b if b == rename_label() => {
            let mut state = state().write().unwrap();
            state.renaming = Some(index);
            state.rename_request = Some(index);
        }
        b if b == remove_label() => remove(index),
        _ => {}
    }
}

/// A row this pane routes to itself, bound to one of its identifiers without
/// asking the settings store about it: the store has never heard of them.
fn row(label: &'static str, control: Control, id: &'static str) -> Row {
    let mut row = Row::new(label, control);
    row.id = Some(id);
    row
}

pub fn build() -> Pane {
    watch_service();
    let state = state().read().unwrap();
    let name = otto_kit::t!("settings-pane-agents");
    let intro = Some(otto_kit::t!("settings-agents-intro"));

    let draft = &state.draft;
    let status = match (&state.error, draft == &state.saved) {
        (Some(error), _) => otto_kit::t_owned!("settings-agents-failed", error = error.as_str()),
        (None, true) => otto_kit::t_owned!("settings-agents-saved"),
        (None, false) => otto_kit::t_owned!("settings-agents-unsaved"),
    };
    let changes_row = Row::new(changes(), Control::Button(change_buttons()))
        .detail(status)
        .inactive(draft == &state.saved);
    let file_row =
        Row::new(config_file(), Control::Button(open_buttons())).detail(user_path_shown());
    let add_row = (draft.agents.len() < MAX_AGENTS)
        .then(|| Row::new(new_agent(), Control::Button(add_buttons())));

    if draft.agents.is_empty() {
        let none = Row::new(
            otto_kit::t!("settings-agents-none"),
            Control::Value(String::new()),
        )
        .detail(Cow::Owned(otto_kit::t_owned!(
            "settings-agents-none-detail",
            path = user_path_shown()
        )));
        return Pane {
            name,
            icon: "agent",
            intro,
            groups: vec![untitled(
                [service_row(), none, changes_row, file_row]
                    .into_iter()
                    .chain(add_row)
                    .collect(),
            )],
        };
    }

    let default_name = draft
        .agents
        .iter()
        .find(|a| a.id == draft.default_agent)
        .or_else(|| draft.agents.first())
        .map(|a| a.name.clone())
        .unwrap_or_default();
    let mut groups = vec![untitled(vec![
        service_row(),
        row(
            otto_kit::t!("settings-agents-default"),
            Control::Select(default_name),
            DEFAULT_ID,
        )
        .detail(otto_kit::t!("settings-agents-default-detail")),
        changes_row,
        file_row,
    ])];
    groups.extend(draft.agents.iter().enumerate().map(|(index, agent)| {
        let mut rows = vec![
            if state.renaming == Some(index) {
                row(
                    otto_kit::t!("settings-agent-name"),
                    Control::Text(agent.name.clone()),
                    row_id(index, Field::Name),
                )
            } else {
                row(
                    name_label(&agent.name),
                    Control::Button(agent_buttons()),
                    row_id(index, Field::Name),
                )
            },
            row(
                otto_kit::t!("settings-agent-harness"),
                Control::Select(Harness::detect(&agent.command).label().to_string()),
                row_id(index, Field::Harness),
            ),
            row(
                otto_kit::t!("settings-agent-command"),
                Control::Text(harness::join(&agent.command)),
                row_id(index, Field::Command),
            ),
        ];
        let found = state
            .instructions
            .iter()
            .find(|found| found.name == agent.persona);
        rows.push(
            row(
                otto_kit::t!("settings-agent-instructions"),
                Control::Select(instructions_label(&agent.persona)),
                row_id(index, Field::Instructions),
            )
            .detail(match found {
                // Hermes runs in a profile named after its instructions, and
                // the installer leaves making one to Hermes.
                Some(_)
                    if Harness::detect(&agent.command) == Harness::Hermes
                        && !hermes_profile(&agent.persona).is_dir() =>
                {
                    otto_kit::t_owned!(
                        "settings-agent-instructions-hermes",
                        name = agent.persona.as_str()
                    )
                }
                _ if agent.persona.is_empty() || found.is_some() => {
                    otto_kit::t_owned!("settings-agent-instructions-detail")
                }
                _ => otto_kit::t_owned!(
                    "settings-agent-instructions-missing",
                    name = agent.persona.as_str()
                ),
            }),
        );
        if let Some(found) = found {
            rows.push(
                row(
                    otto_kit::t!("settings-agent-instructions-file"),
                    Control::Button(open_buttons()),
                    row_id(index, Field::InstructionsFile),
                )
                .detail(shown(&found.path)),
            );
        }
        rows.extend([
            row(
                otto_kit::t!("settings-agent-permissions"),
                Control::Select(permission_label(&agent.permissions).to_string()),
                row_id(index, Field::Permissions),
            ),
            row(
                otto_kit::t!("settings-agent-model"),
                Control::Text(agent.model.clone()),
                row_id(index, Field::Model),
            ),
            row(
                otto_kit::t!("settings-agent-folder"),
                Control::Text(agent.folder.clone()),
                row_id(index, Field::Folder),
            )
            .detail(if agent.folder.is_empty() {
                otto_kit::t_owned!(
                    "settings-agent-folder-unset",
                    path = shown(&ask_scratch_folder())
                )
            } else {
                otto_kit::t_owned!("settings-agent-folder-detail")
            }),
            row(
                otto_kit::t!("settings-agent-colour"),
                Control::Select(colour_label(&agent.colour).to_string()),
                row_id(index, Field::Colour),
            ),
        ]);
        group(
            otto_kit::t_owned!("settings-agent-id", id = agent.id.as_str()),
            rows,
        )
    }));

    groups.extend(add_row.map(|row| untitled(vec![row])));
    Pane {
        name,
        icon: "agent",
        intro,
        groups,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = r#"# Agents that otto-agents runs.
default_agent = "claude"

[[agents]]
id = "claude"
name = "Otto"
# Each new session switches to this model.
model = "haiku"
permissions = "ask"
agent = "otto"

[[agents]]
id = "pi"
command = "pi-acp"
color = "teal"
"#;

    fn parsed(text: &str) -> Config {
        merge(&[(PathBuf::from("user"), text.parse().unwrap())]).0
    }

    #[test]
    fn reads_agents_with_their_defaults() {
        let config = parsed(USER);
        assert_eq!(config.default_agent, "claude");
        assert_eq!(config.agents[0].name, "Otto");
        assert_eq!(config.agents[0].permissions, "ask");
        // No name falls back to the id, no permissions to deny, and the
        // American spelling of colour is read too.
        assert_eq!(config.agents[1].name, "pi");
        assert_eq!(config.agents[1].permissions, "deny");
        assert_eq!(config.agents[1].colour, "teal");
    }

    #[test]
    fn writes_only_what_changed_and_keeps_comments() {
        let saved = parsed(USER);
        let mut draft = saved.clone();
        draft.default_agent = "pi".into();
        draft.agents[0].model = String::new();
        draft.agents[0].name = "Helper".into();
        draft.agents[1].folder = "~/dev".into();
        draft.agents[1].colour = "blue".into();

        let text = render(USER, None, &saved, &draft, "/home/u").unwrap();
        assert!(text.contains("# Agents that otto-agents runs."));
        assert!(text.contains("default_agent = \"pi\""));
        assert!(!text.contains("model = \"haiku\""));
        assert!(text.contains("name = \"Helper\""));
        assert!(text.contains("folder = \"~/dev\""));
        assert!(text.contains("color = \"blue\""));
        assert!(!text.contains("colour"));
        assert!(text.contains("command = \"pi-acp\""));
        assert_eq!(parsed(&text), draft);
    }

    #[test]
    fn copies_the_system_list_into_a_user_file_without_one() {
        let user = "default_agent = \"claude\"\n";
        let saved = merge(&[
            (PathBuf::from("system"), USER.parse().unwrap()),
            (PathBuf::from("user"), user.parse().unwrap()),
        ])
        .0;
        let mut draft = saved.clone();
        draft.agents[1].permissions = "allow".into();

        let text = render(user, Some(USER), &saved, &draft, "/home/u").unwrap();
        assert_eq!(parsed(&text), draft);
    }

    #[test]
    fn picking_a_harness_writes_its_setup() {
        let saved = parsed(USER);
        let mut draft = saved.clone();
        let otto = &mut draft.agents[0];
        otto.command = Harness::Codex.command(&otto.persona);
        otto.preset = Some(Harness::Codex);
        otto.model.clear();

        let text = render(USER, None, &saved, &draft, "/home/u").unwrap();
        let reread = parsed(&text);
        assert_eq!(Harness::detect(&reread.agents[0].command), Harness::Codex);
        assert!(text.contains("CODEX_CONFIG"));
        assert!(text.contains("collaboration_mode = \"plan\""));
        // Only the edited agent changed.
        assert_eq!(reread.agents[1], saved.agents[1]);
    }

    #[test]
    fn adds_and_removes_agents() {
        let saved = parsed(USER);
        let mut draft = saved.clone();
        draft.agents.remove(1);
        let mut reviewer = Agent::blank("review");
        reviewer.name = "Reviewer".into();
        reviewer.command = Harness::Claude.command("otto");
        reviewer.preset = Some(Harness::Claude);
        reviewer.permissions = "ask".into();
        draft.agents.push(reviewer);

        let text = render(USER, None, &saved, &draft, "/home/u").unwrap();
        assert!(!text.contains("pi-acp"));
        let reread = parsed(&text);
        assert_eq!(reread.agents.len(), 2);
        assert_eq!(reread.agents[1].name, "Reviewer");
        assert_eq!(Harness::detect(&reread.agents[1].command), Harness::Claude);
        assert!(text.contains("skills = \"claude\""));
    }

    #[test]
    fn ids_come_from_names_and_stay_unique() {
        assert_eq!(slug("New agent"), "new-agent");
        assert_eq!(slug("  Rita's desk!"), "rita-s-desk");
        assert_eq!(slug("…"), "agent");
        let agents = [Agent::blank("new-agent"), Agent::blank("new-agent-2")];
        assert_eq!(unique_id(&agents, "new-agent", None), "new-agent-3");
        assert_eq!(unique_id(&agents, "new-agent", Some(0)), "new-agent");
    }

    #[test]
    fn reads_the_service_state_systemd_reports() {
        let show = |load: &str, active: &str| {
            Service::parse(&format!("LoadState={load}\nActiveState={active}\n"))
        };
        assert_eq!(show("loaded", "active"), Service::Running);
        assert_eq!(show("loaded", "inactive"), Service::Stopped);
        assert_eq!(show("loaded", "failed"), Service::Failed);
        assert_eq!(show("not-found", "inactive"), Service::Missing);
    }

    #[test]
    fn row_ids_round_trip() {
        for field in Field::ALL {
            assert_eq!(parse_id(row_id(3, field)), Some((3, field)));
        }
        assert!(owns(DEFAULT_ID));
        assert!(!owns("agent.x.model"));
        assert!(!owns("theme_scheme"));
    }
}
