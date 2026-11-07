//! The Dictation topic: what dictation needs, what is there, and the steps
//! that follow.
//!
//! Dictation (Ctrl+D in the launcher and the Ask chat, `otto-dictate
//! toggle` in any other field) recognises speech on a local speech server,
//! `otto-stt-<engine>.service`, one of three engines. The engines are not
//! packaged: `components/otto-dictate/engines/install.sh` in Otto's source
//! builds one, fetches its model and installs otto-dictate. Setup finds
//! what that left, picks the engine (`engine` in `dictation.toml`, the key
//! Settings › Dictation edits too), starts its service, and turns on
//! otto-dictate's autostart entry. For the rest it shows the commands.

// Rust guideline compliant 2026-02-21

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use otto_foundations::{desktop_file, xdg};

use crate::plan::{Action, State, Step};
use crate::probe::{self, Unit};

/// Where `install.sh` keeps the engines and their models, under the home.
pub const DATA_DIR: &str = ".local/share/otto-voice-poc";

/// The address every engine serves, and the apps dictate to.
pub const SERVER: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 8080);

/// Otto's source, for the instructions that need a checkout.
pub const SOURCE: &str = "https://github.com/nongio/otto";

/// The autostart entry's file name, the same in every autostart folder.
const AUTOSTART_FILE: &str = "otto-dictate.desktop";

/// The programs `install.sh` needs, by the name shown; the second is any
/// of the programs that provide it.
pub const TOOLS: [(&str, &[&str]); 5] = [
    ("git", &["git"]),
    ("cmake", &["cmake"]),
    ("C++ compiler", &["c++", "g++", "clang++"]),
    ("glslc", &["glslc"]),
    ("cargo", &["cargo"]),
];

/// A speech engine, each served by `otto-stt-<id>.service`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Parakeet,
    Whisper,
    CrispAsr,
}

/// The engines, the default first.
pub const ENGINES: [Engine; 3] = [Engine::Parakeet, Engine::Whisper, Engine::CrispAsr];

impl Engine {
    /// The id `dictation.toml` and `install.sh` use.
    pub fn id(self) -> &'static str {
        match self {
            Engine::Parakeet => "parakeet",
            Engine::Whisper => "whisper",
            Engine::CrispAsr => "crispasr",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        ENGINES.into_iter().find(|engine| engine.id() == id)
    }

    pub fn name(self) -> &'static str {
        match self {
            Engine::Parakeet => "Parakeet",
            Engine::Whisper => "Whisper",
            Engine::CrispAsr => "CrispASR",
        }
    }

    /// The unit that serves it.
    pub fn unit(self) -> &'static str {
        match self {
            Engine::Parakeet => "otto-stt-parakeet.service",
            Engine::Whisper => "otto-stt-whisper.service",
            Engine::CrispAsr => "otto-stt-crispasr.service",
        }
    }

    /// What it is good at, for the row.
    pub fn about(self) -> &'static str {
        match self {
            Engine::Parakeet => "25 European languages · the default",
            Engine::Whisper => "English only",
            Engine::CrispAsr => "25 European languages · favours the names it is given",
        }
    }

    /// The server program `install.sh` builds or fetches, under `DATA_DIR`.
    pub fn server(self) -> &'static str {
        match self {
            Engine::Parakeet => "whisper.cpp/build-vulkan/bin/parakeet-server",
            Engine::Whisper => "whisper.cpp/build-vulkan/bin/whisper-server",
            Engine::CrispAsr => "crispasr-bin/crispasr-linux-x86_64-vulkan/crispasr",
        }
    }

    /// The model it loads, under `DATA_DIR/models`.
    pub fn model(self) -> &'static str {
        match self {
            Engine::Parakeet => "ggml-parakeet-tdt-0.6b-v3-f16.bin",
            Engine::Whisper => "ggml-small.en-q5_1.bin",
            Engine::CrispAsr => "parakeet-tdt-0.6b-v3-q8_0.gguf",
        }
    }
}

/// Everything the Dictation topic decides from.
#[derive(Debug, Clone)]
pub struct Facts {
    pub home: PathBuf,
    /// `DATA_DIR` under the home.
    pub data: PathBuf,
    /// `dictation.toml`, the file setup writes `engine` into.
    pub config_path: PathBuf,
    /// Its text; empty when there is none.
    pub config: String,
    /// A file that would not parse, with why.
    pub config_error: Option<String>,
    /// The engine the file names, when it names one setup knows.
    pub engine_in_file: Option<Engine>,
    /// Each engine's unit.
    pub units: Vec<(Engine, Unit)>,
    /// Each engine's server program and model, whether both are there.
    pub files: Vec<(Engine, bool)>,
    /// Whether something answers at `SERVER` now.
    pub answering: bool,
    /// otto-dictate, on the PATH or where `install.sh` puts it.
    pub dictate: Option<PathBuf>,
    /// The user's autostart entry for it.
    pub autostart_path: PathBuf,
    /// What that entry says, if there is one.
    pub autostart_entry: Option<String>,
    /// A system entry a package may have installed.
    pub system_entry: Option<String>,
    /// The key Otto's `config.toml` binds to `otto-dictate`, when one is.
    pub shortcut: Option<String>,
    /// PipeWire, which captures the microphone.
    pub pipewire: Option<PathBuf>,
    /// Whether it is running: its socket is in the runtime directory.
    pub pipewire_running: bool,
    /// Whether a Vulkan driver is installed: the engines run on the GPU.
    pub vulkan: bool,
    /// Where each of `TOOLS` is, by the name shown.
    pub tools: BTreeMap<&'static str, Option<PathBuf>>,
    /// A checkout of Otto's source with `install.sh` in it.
    pub source: Option<PathBuf>,
    pub package_manager: Option<&'static str>,
}

impl Facts {
    /// Reads the machine.
    pub fn read() -> Self {
        let home = xdg::home().unwrap_or_default();
        let config_home = xdg::config_home().unwrap_or_else(|| home.join(".config"));
        let data = home.join(DATA_DIR);
        let config_path = config_home.join("otto").join("dictation.toml");
        let config = std::fs::read_to_string(&config_path).unwrap_or_default();
        let (engine_in_file, config_error) = match config.parse::<toml_edit::DocumentMut>() {
            Ok(doc) => (
                doc.get("engine")
                    .and_then(|v| v.as_str())
                    .and_then(Engine::from_id),
                None,
            ),
            Err(err) => (None, Some(err.to_string())),
        };
        let shell_path = probe::shell_path();
        let dictate = probe::find("otto-dictate", &shell_path)
            .or_else(|| Some(home.join(".local/bin/otto-dictate")).filter(|p| p.is_file()));
        let autostart_path = config_home.join("autostart").join(AUTOSTART_FILE);
        let system_entry = xdg::config_dirs().into_iter().find_map(|dir| {
            std::fs::read_to_string(dir.join("autostart").join(AUTOSTART_FILE)).ok()
        });
        let otto_config = std::fs::read_to_string(config_home.join("otto/config.toml"));
        let tools = TOOLS
            .into_iter()
            .map(|(name, programs)| {
                let path = programs
                    .iter()
                    .find_map(|program| probe::find(program, &shell_path));
                (name, path)
            })
            .collect();
        let runtime = xdg::runtime_dir();
        Self {
            data: data.clone(),
            config_path,
            config,
            config_error,
            engine_in_file,
            units: ENGINES
                .into_iter()
                .map(|engine| (engine, probe::unit(engine.unit())))
                .collect(),
            files: ENGINES
                .into_iter()
                .map(|engine| (engine, files_present(&data, engine)))
                .collect(),
            answering: answering(),
            dictate,
            autostart_entry: std::fs::read_to_string(&autostart_path).ok(),
            autostart_path,
            system_entry,
            shortcut: otto_config.ok().and_then(|text| shortcut(&text)),
            pipewire: probe::find("pipewire", &shell_path),
            pipewire_running: runtime.is_some_and(|dir| dir.join("pipewire-0").exists()),
            vulkan: vulkan_driver(),
            tools,
            source: source(&home),
            package_manager: probe::package_manager(&shell_path),
            home,
        }
    }

    /// A path as setup shows it, with the home folder as `~`.
    pub fn shown(&self, path: &Path) -> String {
        xdg::tilde_in(path, Some(&self.home))
    }

    pub fn unit(&self, engine: Engine) -> Unit {
        self.units
            .iter()
            .find(|(e, _)| *e == engine)
            .map_or(Unit::Unmanaged, |(_, unit)| *unit)
    }

    pub fn has_files(&self, engine: Engine) -> bool {
        self.files
            .iter()
            .any(|(e, present)| *e == engine && *present)
    }

    /// Whether otto-dictate starts at login: the user's entry decides where
    /// there is one, since it overrides the system's of the same name.
    pub fn autostart(&self) -> bool {
        self.autostart_entry
            .as_deref()
            .or(self.system_entry.as_deref())
            .is_some_and(entry_starts)
    }

    /// The tools `install.sh` needs that are not there.
    pub fn missing_tools(&self) -> Vec<&'static str> {
        self.tools
            .iter()
            .filter(|(_, path)| path.is_none())
            .map(|(name, _)| *name)
            .collect()
    }
}

/// Whether the engine's server program and model are both where
/// `install.sh` puts them.
fn files_present(data: &Path, engine: Engine) -> bool {
    data.join(engine.server()).is_file() && data.join("models").join(engine.model()).is_file()
}

/// Whether something answers at `SERVER` now.
fn answering() -> bool {
    TcpStream::connect_timeout(&SERVER, Duration::from_millis(300)).is_ok()
}

/// Whether a Vulkan driver is installed: a loader manifest in one of the
/// places the loader looks.
fn vulkan_driver() -> bool {
    [
        "/usr/share/vulkan/icd.d",
        "/etc/vulkan/icd.d",
        "/usr/local/share/vulkan/icd.d",
    ]
    .into_iter()
    .filter_map(|dir| std::fs::read_dir(dir).ok())
    .flatten()
    .filter_map(Result::ok)
    .any(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
}

/// A checkout of Otto's source with the engines' `install.sh`: the one this
/// was built from, or one in the usual places under the home.
fn source(home: &Path) -> Option<PathBuf> {
    let built_from = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let candidates = [
        built_from,
        home.join("otto"),
        home.join("dev/otto"),
        home.join("src/otto"),
        home.join("code/otto"),
        home.join("projects/otto"),
    ];
    candidates
        .into_iter()
        .find(|dir| dir.join(INSTALL_SH).is_file())
        .and_then(|dir| std::fs::canonicalize(dir).ok())
}

/// `install.sh`, relative to a checkout.
pub const INSTALL_SH: &str = "components/otto-dictate/engines/install.sh";

/// The key `config.toml` binds to otto-dictate, from its `[keyboard_shortcuts]`.
pub fn shortcut(config: &str) -> Option<String> {
    let doc = config.parse::<toml_edit::DocumentMut>().ok()?;
    let table = doc.get("keyboard_shortcuts")?.as_table_like()?;
    let found = table.iter().find_map(|(key, value)| {
        let cmd = value.get("run")?.get("cmd")?.as_str()?;
        Path::new(cmd)
            .file_name()
            .is_some_and(|name| name == "otto-dictate")
            .then(|| key.to_owned())
    });
    found
}

/// Whether an autostart entry starts with the session: not `Hidden`, which
/// the autostart spec reads as deleted, and not switched off the GNOME way.
fn entry_starts(entry: &str) -> bool {
    desktop_file::value(entry, "Hidden") != Some("true")
        && desktop_file::value(entry, "X-GNOME-Autostart-enabled") != Some("false")
}

/// The user's autostart entry with otto-dictate on: their own entry or the
/// system's switched back on, else a new one running `dictate`.
pub fn autostart_entry(facts: &Facts, dictate: &Path) -> String {
    let base = match (&facts.autostart_entry, &facts.system_entry) {
        (Some(user), _) => user.clone(),
        (None, Some(system)) => system.clone(),
        (None, None) => [
            ("Type", "Application"),
            ("Name", "Dictation"),
            ("Comment", "Speech to text in any text field"),
            ("Exec", &dictate.display().to_string()),
            ("NoDisplay", "true"),
        ]
        .into_iter()
        .fold(String::new(), |entry, (key, value)| {
            desktop_file::with_value(&entry, key, value)
        }),
    };
    let entry = desktop_file::with_value(&base, "Hidden", "false");
    desktop_file::with_value(&entry, "X-GNOME-Autostart-enabled", "true")
}

/// What the user picks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choices {
    /// The engine to dictate with; `None` while none is installed.
    pub engine: Option<Engine>,
    /// Start otto-dictate at login.
    pub dictate: bool,
}

impl Choices {
    /// The engine the file names, else the one running, else the one
    /// enabled, else the first installed; and otto-dictate at login when it
    /// is installed and not yet started that way.
    pub fn initial(facts: &Facts) -> Self {
        let installed: Vec<Engine> = ENGINES
            .into_iter()
            .filter(|e| status(facts, *e).installed())
            .collect();
        let running = installed
            .iter()
            .copied()
            .find(|e| matches!(facts.unit(*e), Unit::Loaded { active: true, .. }));
        let enabled = installed
            .iter()
            .copied()
            .find(|e| matches!(facts.unit(*e), Unit::Loaded { enabled: true, .. }));
        let engine = facts
            .engine_in_file
            .filter(|e| installed.contains(e))
            .or(running)
            .or(enabled)
            .or_else(|| installed.first().copied());
        Self {
            engine,
            dictate: facts.dictate.is_some() && !facts.autostart(),
        }
    }
}

/// Where an engine stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Serving now.
    Running { enabled: bool },
    /// Installed, not running.
    Stopped { enabled: bool },
    /// Not installed: what is missing of its service, server and model.
    Missing(Vec<&'static str>),
    /// No systemd user session to ask.
    Unmanaged,
}

impl Status {
    pub fn installed(&self) -> bool {
        matches!(self, Status::Running { .. } | Status::Stopped { .. })
    }
}

pub fn status(facts: &Facts, engine: Engine) -> Status {
    let mut missing = Vec::new();
    if !facts.has_files(engine) {
        missing.push("its server and model");
    }
    match facts.unit(engine) {
        Unit::Unmanaged if missing.is_empty() => Status::Unmanaged,
        Unit::Unmanaged => Status::Missing(missing),
        Unit::Missing => {
            missing.push("its service");
            Status::Missing(missing)
        }
        Unit::Loaded { .. } if !missing.is_empty() => Status::Missing(missing),
        Unit::Loaded {
            enabled,
            active: true,
        } => Status::Running { enabled },
        Unit::Loaded { enabled, .. } => Status::Stopped { enabled },
    }
}

/// The engine serving now, if one is.
pub fn running(facts: &Facts) -> Option<Engine> {
    ENGINES
        .into_iter()
        .find(|e| matches!(status(facts, *e), Status::Running { .. }))
}

/// Where otto-dictate, the dictation in every other field, stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dictate {
    NotInstalled,
    /// Installed; not started at login.
    Off,
    /// Starts at login.
    On,
}

pub fn dictate(facts: &Facts) -> Dictate {
    match (&facts.dictate, facts.autostart()) {
        (None, _) => Dictate::NotInstalled,
        (Some(_), false) => Dictate::Off,
        (Some(_), true) => Dictate::On,
    }
}

/// What the user has to fix by hand before dictation works: an engine to
/// install, the microphone, the GPU driver.
pub fn to_fix(facts: &Facts) -> usize {
    let no_engine = !ENGINES.into_iter().any(|e| status(facts, e).installed());
    usize::from(no_engine) + usize::from(facts.pipewire.is_none()) + usize::from(!facts.vulkan)
}

/// `dictation.toml` with `engine` set, the rest as it was.
///
/// # Errors
///
/// The file's parse error, when it has one.
pub fn render_config(text: &str, engine: Engine) -> Result<String, String> {
    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|err| err.to_string())?;
    doc["engine"] = toml_edit::value(engine.id());
    Ok(doc.to_string())
}

/// The steps for `choices`.
pub fn steps(facts: &Facts, choices: &Choices) -> Vec<Step> {
    let mut steps = vec![config_step(facts, choices), server_step(facts, choices)];
    if choices.dictate || dictate(facts) == Dictate::On {
        steps.push(dictate_step(facts, choices));
    }
    steps
}

fn config_step(facts: &Facts, choices: &Choices) -> Step {
    let title = "Settings file";
    let Some(engine) = choices.engine else {
        return Step::blocked(title, "no engine is installed yet: Enter on one shows how");
    };
    if let Some(err) = &facts.config_error {
        return Step::blocked(
            title,
            format!("{} does not parse: {err}", facts.shown(&facts.config_path)),
        );
    }
    if facts.engine_in_file == Some(engine) {
        return Step::done(title, format!("engine = \"{}\" already", engine.id()));
    }
    let text = match render_config(&facts.config, engine) {
        Ok(text) => text,
        Err(err) => return Step::blocked(title, err),
    };
    let detail = format!(
        "Writes {}:\n{}",
        facts.shown(&facts.config_path),
        text.lines()
            .map(|l| format!("│ {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    Step {
        title,
        state: State::Todo,
        summary: format!("engine = \"{}\" in dictation.toml", engine.id()),
        detail,
        actions: vec![Action::Write {
            path: facts.config_path.clone(),
            text,
        }],
    }
}

fn server_step(facts: &Facts, choices: &Choices) -> Step {
    let title = "Speech server";
    let Some(engine) = choices.engine else {
        return Step::blocked(title, "no engine is installed yet");
    };
    match status(facts, engine) {
        Status::Unmanaged => {
            Step::blocked(title, "no systemd user session: start the engine yourself")
        }
        Status::Missing(_) => Step::blocked(title, format!("{} is not installed", engine.name())),
        Status::Running { enabled: true } => Step::done(
            title,
            format!("{} running, and starts at login", engine.name()),
        ),
        Status::Running { enabled: false } | Status::Stopped { .. } => {
            let others: Vec<Engine> = ENGINES
                .into_iter()
                .filter(|e| *e != engine)
                .filter(|e| matches!(facts.unit(*e), Unit::Loaded { enabled: true, .. }))
                .collect();
            let mut actions: Vec<Action> = others
                .iter()
                .map(|e| Action::run(&["systemctl", "--user", "disable", e.unit()]))
                .collect();
            actions.push(Action::run(&[
                "systemctl",
                "--user",
                "enable",
                "--now",
                engine.unit(),
            ]));
            let mut what = vec![format!("start {} now and at login", engine.name())];
            if let Some(other) = running(facts).filter(|e| *e != engine) {
                what.push(format!("stop {}", other.name()));
            }
            let detail = actions
                .iter()
                .filter_map(|action| match action {
                    Action::Run { args } => Some(format!("$ {}", args.join(" "))),
                    Action::Write { .. } => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            Step {
                title,
                state: State::Todo,
                summary: what.join(", "),
                detail,
                actions,
            }
        }
    }
}

fn dictate_step(facts: &Facts, choices: &Choices) -> Step {
    let title = "Dictation at login";
    match (dictate(facts), &facts.dictate) {
        (Dictate::NotInstalled, _) | (_, None) => {
            Step::blocked(title, "otto-dictate is not installed: Enter shows how")
        }
        (Dictate::On, _) => Step::done(title, "otto-dictate starts at login"),
        (Dictate::Off, _) if !choices.dictate => Step::done(title, "off"),
        (Dictate::Off, Some(dictate)) => {
            let text = autostart_entry(facts, dictate);
            let detail = format!(
                "Writes {}:\n{}",
                facts.shown(&facts.autostart_path),
                text.lines()
                    .map(|l| format!("│ {l}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            Step {
                title,
                state: State::Todo,
                summary: "start otto-dictate at login".into(),
                detail,
                actions: vec![Action::Write {
                    path: facts.autostart_path.clone(),
                    text,
                }],
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn facts() -> Facts {
        Facts {
            home: "/home/u".into(),
            data: PathBuf::from("/home/u").join(DATA_DIR),
            config_path: "/home/u/.config/otto/dictation.toml".into(),
            config: String::new(),
            config_error: None,
            engine_in_file: None,
            units: ENGINES.into_iter().map(|e| (e, Unit::Missing)).collect(),
            files: ENGINES.into_iter().map(|e| (e, false)).collect(),
            answering: false,
            dictate: None,
            autostart_path: "/home/u/.config/autostart/otto-dictate.desktop".into(),
            autostart_entry: None,
            system_entry: None,
            shortcut: None,
            pipewire: Some("/usr/bin/pipewire".into()),
            pipewire_running: true,
            vulkan: true,
            tools: TOOLS.into_iter().map(|(name, _)| (name, None)).collect(),
            source: None,
            package_manager: Some("pacman"),
        }
    }

    fn install(facts: &mut Facts, engine: Engine, enabled: bool, active: bool) {
        for (e, unit) in &mut facts.units {
            if *e == engine {
                *unit = Unit::Loaded { enabled, active };
            }
        }
        for (e, present) in &mut facts.files {
            if *e == engine {
                *present = true;
            }
        }
    }

    #[test]
    fn nothing_installed_blocks_every_step() {
        let facts = facts();
        let choices = Choices::initial(&facts);
        assert_eq!(choices.engine, None);
        let steps = steps(&facts, &choices);
        assert!(steps.iter().all(|s| s.state == State::Blocked));
        assert_eq!(
            status(&facts, Engine::Parakeet),
            Status::Missing(vec!["its server and model", "its service"])
        );
    }

    #[test]
    fn an_installed_engine_is_picked_written_and_started() {
        let mut facts = facts();
        install(&mut facts, Engine::Whisper, false, false);
        let choices = Choices::initial(&facts);
        assert_eq!(choices.engine, Some(Engine::Whisper));
        let steps = steps(&facts, &choices);
        assert_eq!(steps[0].state, State::Todo);
        assert!(
            matches!(&steps[0].actions[0], Action::Write { text, .. } if text.contains("engine = \"whisper\""))
        );
        assert_eq!(steps[1].summary, "start Whisper now and at login");
        assert_eq!(
            steps[1].actions,
            [Action::run(&[
                "systemctl",
                "--user",
                "enable",
                "--now",
                "otto-stt-whisper.service"
            ])]
        );
    }

    #[test]
    fn switching_engines_stops_the_other() {
        let mut facts = facts();
        install(&mut facts, Engine::CrispAsr, true, true);
        install(&mut facts, Engine::Parakeet, false, false);
        let mut choices = Choices::initial(&facts);
        assert_eq!(choices.engine, Some(Engine::CrispAsr));
        assert_eq!(steps(&facts, &choices)[1].state, State::Done);
        choices.engine = Some(Engine::Parakeet);
        let step = &steps(&facts, &choices)[1];
        assert_eq!(
            step.summary,
            "start Parakeet now and at login, stop CrispASR"
        );
        assert_eq!(step.actions.len(), 2);
        assert!(matches!(&step.actions[0], Action::Run { args } if args[2] == "disable"));
    }

    #[test]
    fn the_file_keeps_its_other_keys() {
        let text = "# mine\nlanguage = \"it\"\nengine = \"whisper\"\n";
        let out = render_config(text, Engine::Parakeet).unwrap();
        assert_eq!(out, "# mine\nlanguage = \"it\"\nengine = \"parakeet\"\n");
        assert!(render_config("engine = [", Engine::Parakeet).is_err());
    }

    #[test]
    fn the_file_naming_the_engine_is_done() {
        let mut facts = facts();
        install(&mut facts, Engine::Parakeet, true, true);
        facts.engine_in_file = Some(Engine::Parakeet);
        let steps = steps(&facts, &Choices::initial(&facts));
        assert!(steps.iter().all(|s| s.state == State::Done));
    }

    #[test]
    fn otto_dictate_gets_an_autostart_entry() {
        let mut facts = facts();
        facts.dictate = Some("/home/u/.local/bin/otto-dictate".into());
        let choices = Choices::initial(&facts);
        assert!(choices.dictate);
        let steps = steps(&facts, &choices);
        let step = &steps[2];
        assert_eq!(step.state, State::Todo);
        let Action::Write { path, text } = &step.actions[0] else {
            panic!("writes the entry");
        };
        assert_eq!(path, &facts.autostart_path);
        assert!(text.contains("Exec=/home/u/.local/bin/otto-dictate"));
        assert!(text.contains("Hidden=false"));
        // A hidden entry of the user's is switched back on, Exec kept.
        facts.autostart_entry = Some("[Desktop Entry]\nExec=mine\nHidden=true\n".into());
        assert_eq!(dictate(&facts), Dictate::Off);
        let text = autostart_entry(&facts, Path::new("/x"));
        assert!(text.contains("Exec=mine") && text.contains("Hidden=false"));
        facts.autostart_entry = Some(text);
        assert_eq!(dictate(&facts), Dictate::On);
    }

    #[test]
    fn finds_the_shortcut_bound_to_otto_dictate() {
        let config = r#"
[keyboard_shortcuts]
"Ctrl+Return" = { run = { cmd = "alacritty", args = [] } }
"Ctrl+Alt+m" = { run = { cmd = "/home/u/.local/bin/otto-dictate", args = ["toggle"] } }
"#;
        assert_eq!(shortcut(config).as_deref(), Some("Ctrl+Alt+m"));
        assert_eq!(shortcut("[keyboard_shortcuts]\n"), None);
        assert_eq!(shortcut("not toml ["), None);
    }
}
