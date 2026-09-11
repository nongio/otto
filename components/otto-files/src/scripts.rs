//! Commands that live outside the process, in
//! `$XDG_CONFIG_HOME/otto/files-scripts/` (`~/.config/otto/files-scripts/`).
//!
//! One more provider on the command seam, with a process boundary in the
//! middle: every executable in that directory is a script that describes its
//! commands once, is asked for a dry run while an argument is typed, and is
//! run with the situation and the request handed over as data. The first two,
//! to prove the boundary, are zip and unzip — see `components/otto-files/scripts/`.
//!
//! # The protocol
//!
//! A script is called with one word on its command line and, for the last two,
//! a JSON object on standard input. It answers with JSON on standard output.
//!
//! `script describe` — once, when the window starts:
//!
//! ```json
//! { "commands": [ {
//!     "id": "compress",
//!     "title": "Compress to Zip",
//!     "keywords": ["archive", "zip"],
//!     "group": "file",
//!     "when": { "targets": "some", "extensions": ["zip"], "kinds": "files" },
//!     "arg": { "prompt": "Archive name", "label": "name",
//!              "placeholder": "Archive.zip", "initial": "Archive.zip",
//!              "preview": true },
//!     "undo": "Compress"
//! } ] }
//! ```
//!
//! Every string a person reads — `title`, `undo`, the `arg`'s `prompt`,
//! `label`, `placeholder` and `initial` — may instead be an object keyed by
//! locale, `{ "en": "Compress to Zip", "it": "Comprimi in Zip" }`; the host
//! picks the window's locale, then the same language, then English.
//! `keywords` may be keyed the same way, and every locale's words match.
//! The locale is also handed to the script itself, as `OTTO_LOCALE` in the
//! environment on every call and as `locale` in the JSON below, for what it
//! says back at preview and run time.
//!
//! Everything but `id` and `title` is optional. `when` is the whole of what
//! decides whether the command is offered, so that opening the palette never
//! runs a script: `targets` is `"some"` (default), `"one"`, `"none"` or
//! `"any"`; `extensions` requires every target to carry one of them;
//! `kinds` is `"files"`, `"folders"` or `"any"`. A command with no `arg`
//! runs on Return; one with an `arg` opens the field, and with `preview`
//! set is asked for a dry run after every keystroke.
//!
//! `script preview` and `script run` — with this on standard input:
//!
//! ```json
//! { "command": "compress", "arg": "Holiday.zip", "locale": "en-GB",
//!   "targets": ["/home/me/Pictures/a.png", "/home/me/Pictures/b.png"],
//!   "situation": { "path": "/home/me/Pictures", "selection": [...], ... } }
//! ```
//!
//! `targets` is the selection or, when nothing is selected, the item under
//! the cursor — what the command acts on. `situation` is the window's
//! [`Situation`] in full. A preview answers with what it would do and must do
//! nothing:
//!
//! ```json
//! { "rows": [ { "from": "a.png", "to": "a.jpg", "conflict": false } ],
//!   "note": "2 items converted" }
//! ```
//!
//! A row is what one target would become; a row with no `to` just names the
//! target — for a command whose outcome is one thing made of them all, like
//! an archive, the rows say what goes in and the note says where. A person
//! can toggle a row out of the run.
//!
//! A run answers with what it did, and the host records the changes for undo:
//!
//! ```json
//! { "status": "Compressed 2 items",
//!   "changes": [ { "kind": "created", "path": "/home/me/Pictures/Holiday.zip" },
//!                { "kind": "moved", "from": "...", "to": "..." } ],
//!   "reload": true }
//! ```
//!
//! A non-zero exit is a failure, and the last line of standard error is what
//! the person reads. Describing and previewing are held to a short deadline,
//! since one runs at startup and the other under the keyboard; a run takes as
//! long as it takes, on a worker thread, and its effect arrives through
//! [`CommandProvider::poll`].

use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Process, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::command::{
    ArgKind, ArgSpec, Command, CommandProvider, Effect, Group, Preview, PreviewRow, Request,
    Situation,
};
use crate::model::Change;

/// The provider's namespace, and so the prefix on every script command's id.
pub const NAMESPACE: &str = "scripts";

/// How long a script may take to describe itself.
const DESCRIBE_DEADLINE: Duration = Duration::from_secs(3);
/// How long a dry run may take. It runs under the keyboard, so a script that
/// cannot answer in this is treated as having nothing to show.
const PREVIEW_DEADLINE: Duration = Duration::from_millis(600);

// ---------------------------------------------------------------------------
// What a script says about itself
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
struct Description {
    #[serde(default)]
    commands: Vec<Described>,
}

/// A string a person reads, given once or once per locale:
/// `"title": "Compress to Zip"` or
/// `"title": { "en": "Compress to Zip", "it": "Comprimi in Zip" }`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
enum Text {
    One(String),
    ByLocale(BTreeMap<String, String>),
}

impl Text {
    /// The string for `locale`: the exact tag, then the same language, then
    /// English, then whatever was given first. Case does not matter and `_`
    /// reads as `-`, so `pt_BR` finds `pt-BR`.
    fn get(&self, locale: &str) -> String {
        match self {
            Text::One(text) => text.clone(),
            Text::ByLocale(map) => {
                let wanted = locale.replace('_', "-").to_ascii_lowercase();
                let language = wanted.split('-').next().unwrap_or_default().to_owned();
                let pick = |test: &dyn Fn(&str) -> bool| {
                    map.iter()
                        .find(|(tag, _)| test(&tag.replace('_', "-").to_ascii_lowercase()))
                        .map(|(_, text)| text.clone())
                };
                pick(&|tag| tag == wanted)
                    .or_else(|| pick(&|tag| tag.split('-').next() == Some(&language)))
                    .or_else(|| pick(&|tag| tag.split('-').next() == Some("en")))
                    .or_else(|| map.values().next().cloned())
                    .unwrap_or_default()
            }
        }
    }
}

/// Keywords, likewise: one list, or one per locale.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
enum Words {
    One(Vec<String>),
    ByLocale(BTreeMap<String, Vec<String>>),
}

impl Words {
    /// Every locale's words at once: a keyword is never shown, only matched,
    /// so a person who thinks of the command in another language still finds
    /// it.
    fn all(&self) -> Vec<String> {
        match self {
            Words::One(words) => words.clone(),
            Words::ByLocale(map) => map.values().flatten().cloned().collect(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct Described {
    id: String,
    title: Text,
    #[serde(default)]
    keywords: Option<Words>,
    #[serde(default)]
    group: GroupName,
    #[serde(default)]
    when: When,
    #[serde(default)]
    arg: Option<DescribedArg>,
    /// What to call the run in the undo history. The title when absent.
    #[serde(default)]
    undo: Option<Text>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum GroupName {
    Go,
    #[default]
    File,
    Edit,
    View,
}

impl From<GroupName> for Group {
    fn from(name: GroupName) -> Self {
        match name {
            GroupName::Go => Group::Go,
            GroupName::File => Group::File,
            GroupName::Edit => Group::Edit,
            GroupName::View => Group::View,
        }
    }
}

/// The conditions under which a command is offered, checked against the
/// situation and nothing else.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
struct When {
    targets: Targets,
    /// Every target must end in one of these, compared without case and
    /// without the dot.
    extensions: Vec<String>,
    kinds: Kinds,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Targets {
    /// One or more.
    #[default]
    Some,
    One,
    None,
    Any,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Kinds {
    #[default]
    Any,
    Files,
    Folders,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct DescribedArg {
    prompt: Text,
    #[serde(default)]
    label: Option<Text>,
    #[serde(default)]
    placeholder: Option<Text>,
    #[serde(default)]
    initial: Option<Text>,
    #[serde(default)]
    preview: bool,
}

/// One command a script offers, with the script it came from.
#[derive(Debug, Clone, PartialEq)]
struct ScriptCommand {
    script: PathBuf,
    /// `scripts:<script>.<id>` — the script's file name keeps two scripts'
    /// commands apart.
    full_id: String,
    described: Described,
}

impl ScriptCommand {
    fn offered(&self, situation: &Situation, targets: &[PathBuf]) -> bool {
        let when = &self.described.when;
        let count_ok = match when.targets {
            Targets::Some => !targets.is_empty(),
            Targets::One => targets.len() == 1,
            Targets::None => targets.is_empty(),
            Targets::Any => true,
        };
        if !count_ok {
            return false;
        }
        if !when.extensions.is_empty() {
            let wanted: Vec<String> = when
                .extensions
                .iter()
                .map(|e| e.trim_start_matches('.').to_ascii_lowercase())
                .collect();
            let all = targets.iter().all(|target| {
                target
                    .extension()
                    .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
                    .is_some_and(|ext| wanted.contains(&ext))
            });
            if !all {
                return false;
            }
        }
        match when.kinds {
            Kinds::Any => true,
            // Only the cursor's kind is known without I/O; a selection is
            // taken at its word when it is more than the cursor.
            Kinds::Files => !(targets.len() == 1 && situation.cursor_is_dir),
            Kinds::Folders => targets.len() == 1 && situation.cursor_is_dir,
        }
    }

    fn command(&self, locale: &str) -> Command {
        let d = &self.described;
        let mut command = Command::new(&self.full_id, d.title.get(locale), Group::from(d.group))
            .with_keywords(d.keywords.as_ref().map(Words::all).unwrap_or_default());
        if let Some(arg) = &d.arg {
            let mut spec = ArgSpec::new(
                arg.prompt.get(locale),
                arg.label
                    .as_ref()
                    .map(|label| label.get(locale))
                    .unwrap_or_else(|| otto_kit::t_owned!("files-command-arg-name")),
                ArgKind::Text,
            );
            if let Some(placeholder) = &arg.placeholder {
                spec = spec.with_placeholder(placeholder.get(locale));
            }
            if let Some(initial) = &arg.initial {
                spec = spec.with_initial(initial.get(locale));
            }
            if arg.preview {
                spec = spec.previewed();
            }
            command = command.with_arg(spec);
        }
        command
    }
}

// ---------------------------------------------------------------------------
// What crosses the boundary
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct Input<'a> {
    command: &'a str,
    arg: Option<&'a str>,
    /// The window's locale, as a BCP 47 tag, so what a script says back is
    /// in the person's language. Also in the environment as `OTTO_LOCALE`.
    locale: &'a str,
    targets: &'a [PathBuf],
    situation: &'a Situation,
}

#[derive(Debug, Default, Deserialize)]
struct PreviewReply {
    #[serde(default)]
    rows: Vec<PreviewRowReply>,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PreviewRowReply {
    from: String,
    #[serde(default)]
    to: String,
    #[serde(default)]
    conflict: bool,
}

#[derive(Debug, Default, Deserialize)]
struct RunReply {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    changes: Vec<ChangeReply>,
    #[serde(default)]
    reload: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ChangeReply {
    Moved { from: PathBuf, to: PathBuf },
    Created { path: PathBuf },
}

impl From<ChangeReply> for Change {
    fn from(reply: ChangeReply) -> Self {
        match reply {
            ChangeReply::Moved { from, to } => Change::Moved { from, to },
            ChangeReply::Created { path } => Change::Created { path },
        }
    }
}

/// The selection, or the item under the cursor when nothing is selected.
fn targets_of(situation: &Situation) -> Vec<PathBuf> {
    if !situation.selection.is_empty() {
        return situation.selection.clone();
    }
    situation
        .cursor_name
        .as_ref()
        .map(|name| vec![situation.path.join(name)])
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Running scripts
// ---------------------------------------------------------------------------

struct Outcome {
    success: bool,
    stdout: String,
    stderr: String,
}

impl Outcome {
    /// The line a person reads when the script refused: the last thing it
    /// said on stderr, or the fact that it said nothing.
    fn complaint(&self) -> String {
        self.stderr
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| otto_kit::t_owned!("files-script-failed-quietly"))
    }
}

/// Run `script verb` with `input` on stdin. With a deadline, a script that
/// overruns is killed and the outcome is a failure.
fn call(
    script: &Path,
    verb: &str,
    locale: &str,
    input: Option<&[u8]>,
    deadline: Option<Duration>,
) -> Result<Outcome, String> {
    let mut child = Process::new(script)
        .arg(verb)
        .env("OTTO_LOCALE", locale)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("{}: {err}", script.display()))?;

    if let (Some(bytes), Some(mut stdin)) = (input, child.stdin.take()) {
        // A script that exits without reading gets a broken pipe here, and
        // its exit status is the story, not the write.
        let _ = stdin.write_all(bytes);
    }

    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);

    let status = match deadline {
        Some(deadline) => wait_until(&mut child, deadline),
        None => child.wait().ok(),
    };
    let Some(status) = status else {
        // Killed for overrunning. Anything it started may still hold the
        // pipes open, so the readers are left to finish on their own rather
        // than waited for.
        return Ok(Outcome {
            success: false,
            stdout: String::new(),
            stderr: otto_kit::t_owned!("files-script-timed-out"),
        });
    };
    Ok(Outcome {
        success: status.success(),
        stdout: stdout.and_then(|h| h.join().ok()).unwrap_or_default(),
        stderr: stderr.and_then(|h| h.join().ok()).unwrap_or_default(),
    })
}

fn drain(mut pipe: impl Read + Send + 'static) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = pipe.read_to_string(&mut text);
        text
    })
}

/// Wait for the child up to `deadline`; `None` if it had to be killed.
fn wait_until(child: &mut Child, deadline: Duration) -> Option<std::process::ExitStatus> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if started.elapsed() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// Ask one script what it offers.
fn describe(script: &Path, locale: &str) -> Vec<ScriptCommand> {
    let stem = script
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let outcome = match call(script, "describe", locale, None, Some(DESCRIBE_DEADLINE)) {
        Ok(outcome) => outcome,
        Err(err) => {
            tracing::warn!("files-scripts: {err}");
            return Vec::new();
        }
    };
    if !outcome.success {
        tracing::warn!(
            "files-scripts: {} would not describe itself: {}",
            script.display(),
            outcome.complaint()
        );
        return Vec::new();
    }
    let description: Description = match serde_json::from_str(&outcome.stdout) {
        Ok(description) => description,
        Err(err) => {
            tracing::warn!(
                "files-scripts: {} described itself badly: {err}",
                script.display()
            );
            return Vec::new();
        }
    };
    description
        .commands
        .into_iter()
        .map(|described| ScriptCommand {
            script: script.to_path_buf(),
            full_id: format!("{NAMESPACE}:{stem}.{}", described.id),
            described,
        })
        .collect()
}

/// Every executable in `dir`, by name, described.
fn discover(dir: &Path, locale: &str) -> Vec<ScriptCommand> {
    use std::os::unix::fs::PermissionsExt;

    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut scripts: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            !path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with('.'))
        })
        .filter(|path| {
            std::fs::metadata(path)
                .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        })
        .collect();
    scripts.sort();
    scripts
        .iter()
        .flat_map(|script| describe(script, locale))
        .collect()
}

/// Where the scripts are, honouring `XDG_CONFIG_HOME`. `OTTO_FILES_SCRIPTS`
/// overrides the whole path, for tests and for trying a directory out.
pub fn scripts_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("OTTO_FILES_SCRIPTS") {
        return Some(PathBuf::from(path));
    }
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
        })?;
    Some(base.join("otto").join("files-scripts"))
}

/// A `&'static str` for an undo label that came out of a script.
///
/// The undo stack names its steps with static strings. There are as many of
/// these as there are script commands, so keeping them for the life of the
/// process is bounded, and the same label is only ever kept once.
fn keep(label: &str) -> &'static str {
    static KEPT: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut kept = KEPT.get_or_init(Default::default).lock().unwrap();
    if let Some(existing) = kept.get(label) {
        return existing;
    }
    let leaked: &'static str = Box::leak(label.to_owned().into_boxed_str());
    kept.insert(leaked);
    leaked
}

// ---------------------------------------------------------------------------
// The provider
// ---------------------------------------------------------------------------

/// The scripts directory, as a provider.
pub struct ScriptProvider {
    /// The window's locale, told to every script and used to pick among
    /// the texts a description gives per locale.
    locale: String,
    /// What has been described so far. Behind a mutex because the seam asks
    /// for commands through `&self` and discovery lands from another thread.
    commands: Mutex<Vec<ScriptCommand>>,
    /// Discovery in flight, drained the first time it is asked for.
    discovering: Mutex<Option<Receiver<Vec<ScriptCommand>>>>,
    /// Runs in flight report here.
    finished: Receiver<Result<Effect, String>>,
    report: Sender<Result<Effect, String>>,
    /// How many runs are still going — what they did changes what is
    /// offered next (an Undo), so the provider is not settled until they land.
    running: Arc<AtomicUsize>,
}

impl ScriptProvider {
    /// The provider over the configured directory. Discovery starts now, on
    /// its own thread, so the window's start is not held up by a slow script
    /// and Ctrl+P finds whatever has answered by then.
    pub fn new() -> Self {
        Self::over(scripts_dir(), otto_kit::i18n::current_locale())
    }

    fn over(dir: Option<PathBuf>, locale: String) -> Self {
        let (report, finished) = mpsc::channel();
        let discovering = dir.map(|dir| {
            let (tx, rx) = mpsc::channel();
            let locale = locale.clone();
            std::thread::Builder::new()
                .name("files-scripts".into())
                .spawn(move || {
                    let _ = tx.send(discover(&dir, &locale));
                })
                .expect("spawn discovery thread");
            rx
        });
        Self {
            locale,
            commands: Mutex::new(Vec::new()),
            discovering: Mutex::new(discovering),
            finished,
            report,
            running: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// The provider over `dir`, with discovery already done. For tests.
    #[cfg(test)]
    fn discovered(dir: &Path) -> Self {
        Self::discovered_in(dir, "en-GB")
    }

    #[cfg(test)]
    fn discovered_in(dir: &Path, locale: &str) -> Self {
        let provider = Self::over(None, locale.to_owned());
        *provider.commands.lock().unwrap() = discover(dir, locale);
        provider
    }

    fn take_discovered(&self) {
        let mut discovering = self.discovering.lock().unwrap();
        let Some(rx) = discovering.as_ref() else {
            return;
        };
        if let Ok(found) = rx.try_recv() {
            *self.commands.lock().unwrap() = found;
            *discovering = None;
        }
    }

    fn find(&self, id: &str) -> Option<ScriptCommand> {
        self.take_discovered();
        self.commands
            .lock()
            .unwrap()
            .iter()
            .find(|command| command.full_id == id)
            .cloned()
    }

    fn input(&self, command: &ScriptCommand, request: &Request, situation: &Situation) -> Vec<u8> {
        let targets = targets_of(situation);
        let input = Input {
            command: &command.described.id,
            arg: request.arg.as_deref(),
            locale: &self.locale,
            targets: &targets,
            situation,
        };
        serde_json::to_vec(&input).unwrap_or_default()
    }

    /// Run `command` to completion and turn what it says into an effect.
    fn run_now(command: &ScriptCommand, locale: &str, input: &[u8]) -> Result<Effect, String> {
        let outcome = call(&command.script, "run", locale, Some(input), None)?;
        if !outcome.success {
            return Err(outcome.complaint());
        }
        let reply: RunReply = if outcome.stdout.trim().is_empty() {
            RunReply::default()
        } else {
            serde_json::from_str(&outcome.stdout).map_err(|err| {
                format!(
                    "{}: {err}",
                    command
                        .script
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                )
            })?
        };
        let changes: Vec<Change> = reply.changes.into_iter().map(Change::from).collect();
        let label = command
            .described
            .undo
            .as_ref()
            .unwrap_or(&command.described.title)
            .get(locale);
        Ok(Effect {
            status: reply.status,
            undo_label: (!changes.is_empty()).then(|| keep(&label)),
            changes,
            reload: reply.reload,
        })
    }
}

impl Default for ScriptProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandProvider for ScriptProvider {
    fn namespace(&self) -> &'static str {
        NAMESPACE
    }

    fn commands(&self, situation: &Situation) -> Vec<Command> {
        // Scripts act on real paths in a real directory; the Trash and
        // Recent are neither.
        if situation.trash || situation.recent {
            return Vec::new();
        }
        self.take_discovered();
        let targets = targets_of(situation);
        self.commands
            .lock()
            .unwrap()
            .iter()
            .filter(|command| command.offered(situation, &targets))
            .map(|command| command.command(&self.locale))
            .collect()
    }

    fn settled(&self) -> bool {
        self.take_discovered();
        self.discovering.lock().unwrap().is_none() && self.running.load(Ordering::Acquire) == 0
    }

    fn preview(&self, request: &Request, situation: &Situation) -> Option<Preview> {
        let command = self.find(&request.id)?;
        let input = self.input(&command, request, situation);
        let outcome = call(
            &command.script,
            "preview",
            &self.locale,
            Some(&input),
            Some(PREVIEW_DEADLINE),
        )
        .ok()?;
        if !outcome.success {
            return Some(Preview {
                rows: Vec::new(),
                note: Some(outcome.complaint()),
            });
        }
        let reply: PreviewReply = serde_json::from_str(&outcome.stdout).ok()?;
        Some(Preview {
            rows: reply
                .rows
                .into_iter()
                .map(|row| PreviewRow {
                    from: row.from,
                    to: row.to,
                    conflict: row.conflict,
                })
                .collect(),
            note: reply.note,
        })
    }

    fn run(&mut self, request: &Request, situation: &Situation) -> Result<Effect, String> {
        let command = self
            .find(&request.id)
            .ok_or_else(|| format!("{} is not a script here", request.id))?;
        let input = self.input(&command, request, situation);
        let report = self.report.clone();
        let locale = self.locale.clone();
        let title = command.described.title.get(&locale);
        let running = Arc::clone(&self.running);
        running.fetch_add(1, Ordering::AcqRel);
        let spawned = std::thread::Builder::new()
            .name("files-script-run".into())
            .spawn(move || {
                let _ = report.send(Self::run_now(&command, &locale, &input));
                running.fetch_sub(1, Ordering::AcqRel);
            });
        if let Err(err) = spawned {
            self.running.fetch_sub(1, Ordering::AcqRel);
            return Err(err.to_string());
        }
        // The palette closes on this; what the script did arrives through
        // `poll`, and the status line says so meanwhile.
        Ok(Effect {
            status: Some(otto_kit::t_owned!("files-script-running", title = title)),
            ..Effect::default()
        })
    }

    fn poll(&mut self) -> Vec<Result<Effect, String>> {
        self.finished.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "otto-files-scripts-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn script(&self, name: &str, body: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A script that offers one previewed command and, when run, creates a
    /// file named by its argument next to the first target.
    const TOUCH: &str = r#"
case "$1" in
  describe)
    printf '%s' '{"commands":[{"id":"touch","title":"Touch a File","keywords":["create"],
      "when":{"targets":"some","extensions":["txt"]},
      "arg":{"prompt":"File name","initial":"{stem}.out","preview":true},"undo":"Touch"}]}'
    ;;
  preview)
    input=$(cat)
    arg=$(printf '%s' "$input" | sed 's/.*"arg":"\([^"]*\)".*/\1/')
    printf '{"rows":[{"from":"first","to":"%s","conflict":false}],"note":"would touch %s"}' "$arg" "$arg"
    ;;
  run)
    input=$(cat)
    arg=$(printf '%s' "$input" | sed 's/.*"arg":"\([^"]*\)".*/\1/')
    dir=$(printf '%s' "$input" | sed 's/.*"path":"\([^"]*\)".*/\1/')
    : > "$dir/$arg"
    printf '{"status":"touched %s","changes":[{"kind":"created","path":"%s/%s"}],"reload":true}' "$arg" "$dir" "$arg"
    ;;
esac
"#;

    fn situation(dir: &Path, selection: &[&str]) -> Situation {
        Situation {
            path: dir.to_path_buf(),
            selection: selection.iter().map(|name| dir.join(name)).collect(),
            cursor_name: Some("notes.txt".into()),
            has_entries: true,
            ..Situation::default()
        }
    }

    #[test]
    fn a_script_is_described_once_and_offered_on_its_conditions() {
        let dir = Dir::new("describe");
        dir.script("touch", TOUCH);
        let provider = ScriptProvider::discovered(&dir.0);

        let offered = provider.commands(&situation(&dir.0, &["a.txt", "b.TXT"]));
        assert_eq!(offered.len(), 1);
        let command = &offered[0];
        assert_eq!(command.id, "scripts:touch.touch");
        assert_eq!(command.title, "Touch a File");
        assert_eq!(command.namespace(), Some("scripts"));
        let arg = command.arg.as_ref().expect("takes an argument");
        assert!(arg.preview);
        assert_eq!(arg.initial.as_deref(), Some("{stem}.out"));

        // The wrong extension, the Trash and Recent all keep it away.
        assert!(provider.commands(&situation(&dir.0, &["a.png"])).is_empty());
        assert!(provider
            .commands(&Situation {
                trash: true,
                ..situation(&dir.0, &["a.txt"])
            })
            .is_empty());
        assert!(provider
            .commands(&Situation {
                recent: true,
                ..situation(&dir.0, &["a.txt"])
            })
            .is_empty());
    }

    #[test]
    fn the_cursor_stands_in_for_an_empty_selection() {
        let dir = Dir::new("cursor");
        dir.script("touch", TOUCH);
        let provider = ScriptProvider::discovered(&dir.0);
        let offered = provider.commands(&situation(&dir.0, &[]));
        assert_eq!(offered.len(), 1, "notes.txt under the cursor is a target");
        assert!(provider
            .commands(&Situation {
                cursor_name: None,
                ..situation(&dir.0, &[])
            })
            .is_empty());
    }

    #[test]
    fn a_preview_is_the_scripts_dry_run() {
        let dir = Dir::new("preview");
        dir.script("touch", TOUCH);
        let provider = ScriptProvider::discovered(&dir.0);
        let preview = provider
            .preview(
                &Request::new("scripts:touch.touch", Some("hello.out".into())),
                &situation(&dir.0, &["a.txt"]),
            )
            .expect("a preview");
        assert_eq!(preview.rows.len(), 1);
        assert_eq!(preview.rows[0].to, "hello.out");
        assert_eq!(preview.note.as_deref(), Some("would touch hello.out"));
        assert!(
            !dir.0.join("hello.out").exists(),
            "a preview changes nothing"
        );
    }

    #[test]
    fn a_run_finishes_later_and_lands_through_poll() {
        let dir = Dir::new("run");
        dir.script("touch", TOUCH);
        let mut provider = ScriptProvider::discovered(&dir.0);
        let situation = situation(&dir.0, &["a.txt"]);
        let first = provider
            .run(
                &Request::new("scripts:touch.touch", Some("made.out".into())),
                &situation,
            )
            .expect("accepted");
        assert!(
            first.changes.is_empty(),
            "nothing is recorded until it lands"
        );
        assert!(first.status.is_some());

        let landed = wait_for(&mut provider);
        let effect = landed.expect("the script succeeded");
        assert_eq!(effect.status.as_deref(), Some("touched made.out"));
        assert_eq!(effect.undo_label, Some("Touch"));
        assert!(effect.reload);
        assert!(matches!(
            effect.changes.as_slice(),
            [Change::Created { path }] if path == &dir.0.join("made.out")
        ));
        assert!(dir.0.join("made.out").exists());
    }

    #[test]
    fn a_script_that_fails_is_read_from_its_last_line_of_stderr() {
        let dir = Dir::new("fail");
        dir.script(
            "grumpy",
            r#"
case "$1" in
  describe) printf '%s' '{"commands":[{"id":"no","title":"No"}]}' ;;
  run) echo "thinking..." >&2; echo "not today" >&2; exit 3 ;;
esac
"#,
        );
        let mut provider = ScriptProvider::discovered(&dir.0);
        let situation = situation(&dir.0, &["a.txt"]);
        provider
            .run(&Request::new("scripts:grumpy.no", None), &situation)
            .unwrap();
        assert_eq!(wait_for(&mut provider).unwrap_err(), "not today");
    }

    #[test]
    fn a_script_that_cannot_describe_itself_is_left_out() {
        let dir = Dir::new("bad");
        dir.script("broken", "printf 'this is not json'");
        dir.script("silent", "exit 1");
        dir.script("fine", r#"[ "$1" = describe ] && printf '%s' '{"commands":[{"id":"ok","title":"OK","when":{"targets":"none"}}]}'"#);
        std::fs::write(dir.0.join("notes.txt"), "not a script").unwrap();
        let provider = ScriptProvider::discovered(&dir.0);
        let offered = provider.commands(&Situation {
            path: dir.0.clone(),
            ..Situation::default()
        });
        assert_eq!(offered.len(), 1);
        assert_eq!(offered[0].id, "scripts:fine.ok");
    }

    #[test]
    fn a_slow_dry_run_is_given_up_on() {
        let dir = Dir::new("slow");
        dir.script(
            "slow",
            r#"
case "$1" in
  describe) printf '%s' '{"commands":[{"id":"s","title":"Slow","arg":{"prompt":"x","preview":true}}]}' ;;
  preview) sleep 5; printf '{"rows":[]}' ;;
esac
"#,
        );
        let provider = ScriptProvider::discovered(&dir.0);
        let started = Instant::now();
        let preview = provider.preview(
            &Request::new("scripts:slow.s", Some("x".into())),
            &situation(&dir.0, &["a.txt"]),
        );
        assert!(started.elapsed() < Duration::from_secs(3));
        let preview = preview.expect("a note saying so");
        assert!(preview.rows.is_empty());
        assert!(preview.note.is_some());
    }

    #[test]
    fn a_missing_directory_offers_nothing() {
        let provider = ScriptProvider::discovered(Path::new("/nonexistent/otto-files-scripts"));
        assert!(provider
            .commands(&situation(Path::new("/tmp"), &["a.txt"]))
            .is_empty());
    }

    /// The shipped samples, through the provider: zip the fixture, then
    /// unzip what came out. Skipped where `zip` is not installed.
    #[test]
    fn the_shipped_zip_and_unzip_scripts_round_trip() {
        if !["zip", "unzip", "python3"].iter().all(|tool| {
            std::process::Command::new("sh")
                .args(["-c", &format!("command -v {tool}")])
                .stdout(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        }) {
            eprintln!("zip, unzip or python3 missing; skipping");
            return;
        }
        let shipped = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts");
        let dir = Dir::new("shipped");
        std::fs::write(dir.0.join("a.txt"), "a").unwrap();
        std::fs::create_dir(dir.0.join("Photos")).unwrap();
        std::fs::write(dir.0.join("Photos/b.txt"), "b").unwrap();
        let mut provider = ScriptProvider::discovered(&shipped);

        let situation = situation(&dir.0, &["a.txt", "Photos"]);
        let offered: Vec<String> = provider
            .commands(&situation)
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert!(offered.contains(&"scripts:zip.compress".to_string()));
        assert!(
            !offered.contains(&"scripts:unzip.extract".to_string()),
            "not everything selected is a .zip"
        );

        let request = Request::new("scripts:zip.compress", Some("Bundle".into()));
        let preview = provider.preview(&request, &situation).unwrap();
        assert_eq!(preview.rows.len(), 2);
        assert_eq!(preview.rows[0].from, "a.txt");
        assert_eq!(
            preview.rows[0].to, "",
            "an archive's rows only name what goes in"
        );
        assert_eq!(preview.note.as_deref(), Some("2 items into “Bundle.zip”"));
        provider.run(&request, &situation).unwrap();
        let effect = wait_for(&mut provider).unwrap();
        assert!(dir.0.join("Bundle.zip").is_file());
        assert_eq!(effect.undo_label, Some("Compress"));

        let situation = situation_with(&dir.0, &["Bundle.zip"]);
        let offered: Vec<String> = provider
            .commands(&situation)
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert!(offered.contains(&"scripts:unzip.extract".to_string()));
        // Extracting asks nothing: each archive goes into a folder named
        // after it.
        let request = Request::new("scripts:unzip.extract", None);
        let preview = provider.preview(&request, &situation).unwrap();
        assert_eq!(preview.rows[0].to, "Bundle/");
        assert!(!preview.rows[0].conflict);
        provider.run(&request, &situation).unwrap();
        wait_for(&mut provider).unwrap();
        assert!(dir.0.join("Bundle/Photos/b.txt").is_file());

        // Running again would land on the folder that is now there.
        let preview = provider.preview(&request, &situation).unwrap();
        assert!(preview.rows[0].conflict);
        provider.run(&request, &situation).unwrap();
        assert!(wait_for(&mut provider).is_err());
    }

    #[test]
    fn a_script_may_speak_several_languages() {
        let dir = Dir::new("l10n");
        dir.script(
            "hello",
            r#"
case "$1" in
  describe)
    printf '%s' '{"commands":[{"id":"hi","title":{"en":"Say Hello","it":"Saluta","pt-BR":"Diga oi"},
      "keywords":{"en":["greet"],"it":["ciao"]},
      "arg":{"prompt":{"en":"To whom","it":"A chi"},"placeholder":"..."}}]}'
    ;;
  preview) printf '{"note":"%s"}' "$OTTO_LOCALE" ;;
esac
"#,
        );
        let situation = situation(&dir.0, &["a.txt"]);

        let italian = ScriptProvider::discovered_in(&dir.0, "it-IT");
        let command = italian.commands(&situation).remove(0);
        assert_eq!(command.title, "Saluta");
        assert_eq!(command.arg.as_ref().unwrap().prompt, "A chi");
        assert!(command.keywords.contains(&"greet".to_string()));
        assert!(command.keywords.contains(&"ciao".to_string()));

        let brazilian = ScriptProvider::discovered_in(&dir.0, "pt_BR");
        assert_eq!(brazilian.commands(&situation).remove(0).title, "Diga oi");

        let german = ScriptProvider::discovered_in(&dir.0, "de");
        assert_eq!(german.commands(&situation).remove(0).title, "Say Hello");

        // The script itself is told, too.
        let preview = italian
            .preview(
                &Request::new("scripts:hello.hi", Some("x".into())),
                &situation,
            )
            .unwrap();
        assert_eq!(preview.note.as_deref(), Some("it-IT"));
    }

    fn situation_with(dir: &Path, selection: &[&str]) -> Situation {
        Situation {
            cursor_name: selection.first().map(|s| s.to_string()),
            ..situation(dir, selection)
        }
    }

    fn wait_for(provider: &mut ScriptProvider) -> Result<Effect, String> {
        let started = Instant::now();
        loop {
            if let Some(result) = provider.poll().pop() {
                return result;
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "the run never landed"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
