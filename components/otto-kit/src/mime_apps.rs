//! Which applications open which file types, and opening files with them.
//!
//! This follows the freedesktop MIME Applications Associations specification:
//! the `mimeapps.list` files say what the user and the system chose, and each
//! installed desktop entry's `MimeType=` says what it can open. [`Associations`]
//! holds both, read once, and answers the questions a file manager asks —
//! "what opens this by default" and "what else could" — the same way every
//! other spec-following desktop component does, so a double-click here and
//! `xdg-open` elsewhere agree.
//!
//! [`set_default`] writes the user's choice back where the specification puts
//! it, and [`open`] launches an application on files, expanding its `Exec=`
//! line's field codes.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use freedesktop_desktop_entry::DesktopEntry;

/// An installed application, as its desktop entry describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    /// The desktop file ID, `.desktop` suffix included (`org.gnome.Evince.desktop`).
    ///
    /// This is the name `mimeapps.list` uses, so it is what a remembered
    /// choice is written as.
    pub id: String,
    /// Localized `Name=`.
    pub name: String,
    /// `Icon=`, for a theme lookup.
    pub icon_name: Option<String>,
    /// `Exec=`, field codes and all.
    pub exec: Option<String>,
    /// `Terminal=true`: the program wants a terminal around it.
    pub terminal: bool,
    /// `Path=`: the directory to start the program in.
    pub working_dir: Option<PathBuf>,
    /// `MimeType=`: the types the entry says it can open.
    pub mime_types: Vec<String>,
    /// `NoDisplay=true`: a handler, not something to list among applications.
    pub no_display: bool,
    /// The desktop file itself, for the `%k` field code.
    pub entry_path: PathBuf,
}

/// The three groups of one `mimeapps.list` file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ListFile {
    defaults: HashMap<String, Vec<String>>,
    added: HashMap<String, Vec<String>>,
    removed: HashMap<String, Vec<String>>,
}

/// Every installed application and every association, read once.
///
/// Reading is a walk of the application directories and a parse of each
/// entry, so this is built when it is needed — opening a chooser — and
/// dropped with it, rather than kept up to date.
#[derive(Clone, Debug, Default)]
pub struct Associations {
    /// By desktop file ID. The first directory in precedence order wins, so a
    /// user's own copy of an entry replaces the system's.
    apps: HashMap<String, App>,
    /// The `mimeapps.list` files, highest precedence first.
    lists: Vec<ListFile>,
}

impl Associations {
    /// Read the installed applications and the association files.
    pub fn load() -> Self {
        let locales = crate::desktop_entry::entry_locales();
        let desktops = current_desktops();
        let mut apps = HashMap::new();
        for dir in freedesktop_desktop_entry::default_paths() {
            collect_apps(&dir, &dir, &locales, &desktops, &mut apps);
        }
        let lists = list_paths(&desktops)
            .iter()
            .filter_map(|path| std::fs::read_to_string(path).ok())
            .map(|text| parse_list(&text))
            .collect();
        Self { apps, lists }
    }

    /// What opens `mime` by default: the user's or the system's choice, and
    /// failing that the most preferred application that can open it.
    ///
    /// `ancestors` is `mime` and the types it descends from, nearest first —
    /// see [`crate::filetype::ancestors`]. A Rust file with no default of its
    /// own opens in whatever opens plain text.
    pub fn default_for(&self, ancestors: &[String]) -> Option<&App> {
        for mime in ancestors {
            let chosen = self
                .lists
                .iter()
                .filter_map(|list| list.defaults.get(mime))
                .flatten()
                .find_map(|id| self.openable(id));
            if chosen.is_some() {
                return chosen;
            }
            // With nothing chosen, a handler is not a guess worth making
            // when an application proper can open the file.
            let associated = self.associated(mime);
            let first = associated
                .iter()
                .find(|app| !app.no_display)
                .or(associated.first());
            if let Some(first) = first {
                return Some(first);
            }
        }
        None
    }

    /// Every application that can open a file of this type, the default first.
    ///
    /// Types nearer the file come before their parents, so an application
    /// that knows the exact type is offered above one that only knows a
    /// general one. `NoDisplay` entries are left out unless one is the
    /// default: they are handlers — a viewer's second entry for one format, a
    /// helper — and listing them puts the same name in the list twice.
    pub fn apps_for(&self, ancestors: &[String]) -> Vec<&App> {
        let mut out: Vec<&App> = self.default_for(ancestors).into_iter().collect();
        for mime in ancestors {
            for app in self.associated(mime) {
                if !app.no_display && !out.iter().any(|seen| seen.id == app.id) {
                    out.push(app);
                }
            }
        }
        out
    }

    /// Every application worth listing, by name: the ones a person would
    /// recognize as applications, for "open with something else".
    pub fn all(&self) -> Vec<&App> {
        let mut out: Vec<&App> = self
            .apps
            .values()
            .filter(|app| !app.no_display && app.exec.is_some())
            .collect();
        out.sort_by_cached_key(|app| app.name.to_lowercase());
        out
    }

    /// The applications associated with exactly `mime`, most preferred first.
    ///
    /// Added associations come first, in file precedence order; then every
    /// entry whose `MimeType=` lists the type. A removal hides an application
    /// from the files below the one that says so, and from `MimeType=`.
    fn associated(&self, mime: &str) -> Vec<&App> {
        let mut out: Vec<&App> = Vec::new();
        let mut removed: HashSet<&str> = HashSet::new();
        for list in &self.lists {
            for id in list.added.get(mime).into_iter().flatten() {
                if removed.contains(id.as_str()) || out.iter().any(|app| app.id == *id) {
                    continue;
                }
                if let Some(app) = self.openable(id) {
                    out.push(app);
                }
            }
            removed.extend(
                list.removed
                    .get(mime)
                    .into_iter()
                    .flatten()
                    .map(String::as_str),
            );
        }
        let mut declared: Vec<&App> = self
            .apps
            .values()
            .filter(|app| app.exec.is_some() && app.mime_types.iter().any(|m| m == mime))
            .filter(|app| !removed.contains(app.id.as_str()))
            .filter(|app| !out.iter().any(|seen| seen.id == app.id))
            .collect();
        declared.sort_by_cached_key(|app| app.name.to_lowercase());
        out.extend(declared);
        out
    }

    /// An installed application that can actually be run.
    fn openable(&self, id: &str) -> Option<&App> {
        self.apps.get(id).filter(|app| app.exec.is_some())
    }
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// `XDG_CURRENT_DESKTOP`, lowercased, in its own order.
fn current_desktops() -> Vec<String> {
    freedesktop_desktop_entry::current_desktop().unwrap_or_default()
}

/// Walk one application directory, recursively.
///
/// A desktop file ID is the path below the `applications` directory with `/`
/// turned into `-`: `kde/konsole.desktop` is `kde-konsole.desktop`.
fn collect_apps(
    root: &Path,
    dir: &Path,
    locales: &[String],
    desktops: &[String],
    apps: &mut HashMap<String, App>,
) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_apps(root, &path, locales, desktops, apps);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let id = relative.to_string_lossy().replace('/', "-");
        // Earlier directories take precedence, and a hidden entry still
        // counts: `Hidden=true` in the user's directory is how a system entry
        // is deleted.
        if apps.contains_key(&id) {
            continue;
        }
        let Ok(entry) = DesktopEntry::from_path(path.clone(), Some(locales)) else {
            continue;
        };
        if let Some(app) = app_from_entry(&id, &entry, locales, desktops) {
            apps.insert(id, app);
        } else {
            // Remember the ID so a lower-precedence copy cannot come back.
            apps.insert(id.clone(), hidden_placeholder(id, path));
        }
    }
}

/// An entry that exists only to hide lower-precedence ones. It has no
/// `Exec`, so nothing offers or launches it.
fn hidden_placeholder(id: String, entry_path: PathBuf) -> App {
    App {
        id,
        name: String::new(),
        icon_name: None,
        exec: None,
        terminal: false,
        working_dir: None,
        mime_types: Vec::new(),
        no_display: true,
        entry_path,
    }
}

fn app_from_entry(
    id: &str,
    entry: &DesktopEntry,
    locales: &[String],
    desktops: &[String],
) -> Option<App> {
    if entry.type_() != Some("Application") || entry.hidden() {
        return None;
    }
    let shown_here = |list: Option<Vec<&str>>| {
        list.map(|names| {
            names
                .iter()
                .any(|name| desktops.iter().any(|d| d.eq_ignore_ascii_case(name)))
        })
    };
    if shown_here(entry.only_show_in()) == Some(false)
        || shown_here(entry.not_show_in()) == Some(true)
    {
        return None;
    }
    if let Some(try_exec) = entry.try_exec() {
        if !program_exists(try_exec) {
            return None;
        }
    }
    let name = entry.name(locales)?.to_string();
    Some(App {
        id: id.to_string(),
        name,
        icon_name: entry.icon().map(str::to_string),
        exec: entry
            .exec()
            .map(str::to_string)
            .filter(|e| !e.trim().is_empty()),
        terminal: entry.terminal(),
        working_dir: entry.path().map(PathBuf::from),
        mime_types: entry
            .mime_type()
            .unwrap_or_default()
            .into_iter()
            .map(str::to_string)
            .collect(),
        no_display: entry.no_display(),
        entry_path: entry.path.clone(),
    })
}

/// Is `program` an absolute path that exists, or a name found on `PATH`?
fn program_exists(program: &str) -> bool {
    let program = Path::new(program);
    if program.is_absolute() {
        return program.exists();
    }
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

/// The `mimeapps.list` files in the specification's precedence order, highest
/// first: config before data, user before system, and in each directory the
/// desktop-specific file (`otto-mimeapps.list`) before the generic one. The
/// legacy `defaults.list` comes last in each data directory.
fn list_paths(desktops: &[String]) -> Vec<PathBuf> {
    let names: Vec<String> = desktops
        .iter()
        .map(|d| format!("{d}-mimeapps.list"))
        .chain(["mimeapps.list".to_string()])
        .collect();

    let mut paths = Vec::new();
    let mut config_dirs: Vec<PathBuf> = config_home().into_iter().collect();
    config_dirs.extend(split_dirs("XDG_CONFIG_DIRS", "/etc/xdg"));
    for dir in &config_dirs {
        paths.extend(names.iter().map(|name| dir.join(name)));
    }

    let mut data_dirs: Vec<PathBuf> = data_home().into_iter().collect();
    data_dirs.extend(split_dirs("XDG_DATA_DIRS", "/usr/local/share:/usr/share"));
    for dir in &data_dirs {
        let dir = dir.join("applications");
        paths.extend(names.iter().map(|name| dir.join(name)));
        paths.push(dir.join("defaults.list"));
    }
    paths
}

fn config_home() -> Option<PathBuf> {
    non_empty_var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| non_empty_var("HOME").map(|home| Path::new(&home).join(".config")))
}

fn data_home() -> Option<PathBuf> {
    non_empty_var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| non_empty_var("HOME").map(|home| Path::new(&home).join(".local/share")))
}

fn split_dirs(var: &str, fallback: &str) -> Vec<PathBuf> {
    non_empty_var(var)
        .unwrap_or_else(|| fallback.to_string())
        .split(':')
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn non_empty_var(var: &str) -> Option<String> {
    std::env::var(var).ok().filter(|value| !value.is_empty())
}

/// Parse one `mimeapps.list` (or `defaults.list`) body.
fn parse_list(text: &str) -> ListFile {
    let mut list = ListFile::default();
    let mut group: Option<&mut HashMap<String, Vec<String>>> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            group = match line {
                "[Default Applications]" => Some(&mut list.defaults),
                "[Added Associations]" => Some(&mut list.added),
                "[Removed Associations]" => Some(&mut list.removed),
                _ => None,
            };
            continue;
        }
        let (Some(group), Some((mime, ids))) = (group.as_deref_mut(), line.split_once('=')) else {
            continue;
        };
        let ids: Vec<String> = ids
            .split(';')
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .collect();
        // A key repeated within one file: the first one stands.
        group.entry(mime.trim().to_string()).or_insert(ids);
    }
    list
}

// ---------------------------------------------------------------------------
// Remembering a choice
// ---------------------------------------------------------------------------

/// Make `app_id` the default for `mime`, in the user's own `mimeapps.list`.
///
/// The application also moves to the front of the type's added associations,
/// so it is listed for the type even if its desktop entry does not say it
/// handles it. The rest of the file is kept as it was.
///
/// # Errors
///
/// When there is no home directory to write to, or the file cannot be
/// written. The write goes to a temporary file first and is renamed into
/// place, so a failure leaves the old file whole. When the file is a symlink
/// (a dotfiles manager's), the file it points to is the one rewritten, so the
/// link survives.
pub fn set_default(mime: &str, app_id: &str) -> io::Result<()> {
    let dir = config_home()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no config directory"))?;
    std::fs::create_dir_all(&dir)?;
    let link = dir.join("mimeapps.list");
    let path = match std::fs::canonicalize(&link) {
        Ok(target) => target,
        Err(err) if err.kind() == io::ErrorKind::NotFound => link,
        Err(err) => return Err(err),
    };
    let current = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err),
    };
    let updated = with_default(&current, mime, app_id);
    // Beside the file, so the rename stays on one filesystem, and named for
    // this process, so two windows saving at once do not share one.
    let temporary = path.with_file_name(format!(".mimeapps.list.{}.tmp", std::process::id()));
    let written = std::fs::File::create(&temporary).and_then(|mut file| {
        file.write_all(updated.as_bytes())?;
        file.sync_all()
    });
    if let Err(err) = written.and_then(|()| std::fs::rename(&temporary, &path)) {
        let _ = std::fs::remove_file(&temporary);
        return Err(err);
    }
    Ok(())
}

/// `text`, a `mimeapps.list` body, with `app_id` set as the default for
/// `mime` and first among its added associations.
fn with_default(text: &str, mime: &str, app_id: &str) -> String {
    let text = set_in_group(text, "[Default Applications]", mime, |_| {
        vec![app_id.to_string()]
    });
    set_in_group(&text, "[Added Associations]", mime, |current| {
        let mut ids = vec![app_id.to_string()];
        ids.extend(current.into_iter().filter(|id| id != app_id));
        ids
    })
}

/// Rewrite `mime`'s line in `group`, adding the line — and the group — when
/// they are missing. `value` gets the IDs the line holds now.
fn set_in_group(
    text: &str,
    group: &str,
    mime: &str,
    value: impl FnOnce(Vec<String>) -> Vec<String>,
) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let header = lines.iter().position(|line| line.trim() == group);
    let format = |ids: Vec<String>| format!("{mime}={};", ids.join(";"));

    let Some(header) = header else {
        if lines.last().is_some_and(|line| !line.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push(group.to_string());
        lines.push(format(value(Vec::new())));
        return lines.join("\n") + "\n";
    };

    let end = lines[header + 1..]
        .iter()
        .position(|line| line.trim().starts_with('['))
        .map_or(lines.len(), |offset| header + 1 + offset);
    let existing = (header + 1..end).find(|&i| {
        lines[i]
            .split_once('=')
            .is_some_and(|(key, _)| key.trim() == mime)
    });
    match existing {
        Some(i) => {
            let current = lines[i]
                .split_once('=')
                .map(|(_, ids)| {
                    ids.split(';')
                        .map(str::trim)
                        .filter(|id| !id.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            lines[i] = format(value(current));
        }
        None => {
            // After the group's last entry, before any blank lines that
            // separate it from the next group.
            let mut at = end;
            while at > header + 1 && lines[at - 1].trim().is_empty() {
                at -= 1;
            }
            lines.insert(at, format(value(Vec::new())));
        }
    }
    lines.join("\n") + "\n"
}

// ---------------------------------------------------------------------------
// Launching
// ---------------------------------------------------------------------------

/// Why an application could not be started.
#[derive(Debug)]
pub enum OpenError {
    /// The entry has no usable `Exec=` line.
    NoCommand,
    /// The `Exec=` line does not parse: an unterminated quote, say.
    BadCommand(String),
    /// The program could not be spawned.
    Spawn(io::Error),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCommand => f.write_str("the application has no command to run"),
            Self::BadCommand(reason) => {
                write!(f, "the application's command is malformed: {reason}")
            }
            Self::Spawn(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for OpenError {}

/// Open `paths` with `app`.
///
/// Its `Exec=` line decides how the files are passed: `%F` and `%U` take them
/// all in one process, `%f` and `%u` one each, so an application that takes a
/// single file is started once per file. Relative paths are made absolute
/// first, since the application starts in a directory of its own choosing.
///
/// The processes are detached: stdio closed, a process group of their own so
/// a signal to the caller's terminal does not reach them, and reaped on a
/// thread so they outlive the caller.
///
/// # Errors
///
/// [`OpenError`] when the entry has no command, the command does not parse, or
/// the program cannot be started. Nothing is started unless every command
/// parses.
pub fn open(app: &App, paths: &[PathBuf]) -> Result<(), OpenError> {
    use std::os::unix::process::CommandExt;

    let exec = app.exec.as_deref().ok_or(OpenError::NoCommand)?;
    let commands = command_lines(exec, paths, app)?;
    for mut argv in commands {
        if app.terminal {
            let mut wrapped: Vec<OsString> =
                terminal_command().into_iter().map(OsString::from).collect();
            wrapped.append(&mut argv);
            argv = wrapped;
        }
        let (program, args) = argv.split_first().ok_or(OpenError::NoCommand)?;
        let mut command = std::process::Command::new(program);
        command
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0);
        if let Some(dir) = app.working_dir.as_deref().filter(|dir| dir.is_dir()) {
            command.current_dir(dir);
        }
        let mut child = command.spawn().map_err(OpenError::Spawn)?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
    Ok(())
}

/// The command that runs a `Terminal=true` entry's program in a terminal,
/// to be followed by that program's argv.
///
/// `$TERMINAL` if the session names one, then the freedesktop terminal
/// launcher, then the first common terminal installed.
pub fn terminal_command() -> Vec<String> {
    if let Some(terminal) = non_empty_var("TERMINAL") {
        return vec![terminal, "-e".to_string()];
    }
    if program_exists("xdg-terminal-exec") {
        return vec!["xdg-terminal-exec".to_string()];
    }
    let terminal = ["ghostty", "alacritty", "foot", "kitty"]
        .into_iter()
        .find(|candidate| program_exists(candidate))
        .unwrap_or("xterm");
    vec![terminal.to_string(), "-e".to_string()]
}

/// One argument of an `Exec=` line, after quoting is undone.
#[derive(Debug, PartialEq, Eq)]
enum Token {
    /// A literal argument, which may hold single-file codes (`--file=%f`).
    Text(String),
    /// `%F`: every file, as paths.
    Files,
    /// `%U`: every file, as URIs.
    Uris,
    /// `%i`: `--icon <Icon>`, or nothing.
    Icon,
}

/// Split an `Exec=` value into arguments.
///
/// Arguments are separated by spaces. One in double quotes may hold spaces,
/// and inside the quotes a backslash escapes `"`, `` ` ``, `$` and `\`. Field
/// codes stand for themselves only outside quotes.
fn tokenize(exec: &str) -> Result<Vec<Token>, OpenError> {
    let mut tokens = Vec::new();
    let mut chars = exec.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let Some(&first) = chars.peek() else {
            return Ok(tokens);
        };
        if first == '"' {
            chars.next();
            let mut arg = String::new();
            loop {
                match chars.next() {
                    None => return Err(OpenError::BadCommand("unterminated quote".to_string())),
                    Some('"') => break,
                    Some('\\') => match chars.next() {
                        Some(c @ ('"' | '`' | '$' | '\\')) => arg.push(c),
                        Some(c) => {
                            arg.push('\\');
                            arg.push(c);
                        }
                        None => {
                            return Err(OpenError::BadCommand("unterminated quote".to_string()))
                        }
                    },
                    Some(c) => arg.push(c),
                }
            }
            // `%%` still means a percent sign inside quotes; nothing else
            // is expanded there.
            tokens.push(Token::Text(arg.replace('%', "%%")));
            continue;
        }
        let mut arg = String::new();
        while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
            arg.push(c);
        }
        tokens.push(match arg.as_str() {
            "%F" => Token::Files,
            "%U" => Token::Uris,
            "%i" => Token::Icon,
            _ => Token::Text(arg),
        });
    }
}

/// The command lines that open `paths` with an `Exec=` value.
///
/// More than one when the line takes a single file (`%f`, `%u`) and several
/// were given; exactly one otherwise, including when the line takes no files
/// at all — the application is started without them, as the specification
/// says it must be.
fn command_lines(
    exec: &str,
    paths: &[PathBuf],
    app: &App,
) -> Result<Vec<Vec<OsString>>, OpenError> {
    let tokens = tokenize(exec)?;
    let paths: Vec<PathBuf> = paths
        .iter()
        .map(|path| std::path::absolute(path).unwrap_or_else(|_| path.clone()))
        .collect();
    let single = tokens.iter().any(|token| match token {
        Token::Text(text) => has_code(text, 'f') || has_code(text, 'u'),
        _ => false,
    });
    let groups: Vec<&[PathBuf]> = if single && paths.len() > 1 {
        paths.chunks(1).collect()
    } else {
        vec![&paths]
    };

    let mut lines = Vec::new();
    for files in groups {
        let mut argv = Vec::new();
        for token in &tokens {
            match token {
                Token::Files => argv.extend(files.iter().map(|p| p.clone().into_os_string())),
                Token::Uris => argv.extend(
                    files
                        .iter()
                        .map(|p| crate::clipboard::path_to_uri(p).into()),
                ),
                Token::Icon => {
                    if let Some(icon) = &app.icon_name {
                        argv.push("--icon".into());
                        argv.push(icon.into());
                    }
                }
                Token::Text(text) => {
                    // A lone `%f` with no file is dropped, not left empty.
                    if files.is_empty() && matches!(text.as_str(), "%f" | "%u") {
                        continue;
                    }
                    let expanded = expand(text, files.first().map(PathBuf::as_path), app);
                    if !expanded.is_empty() || text.is_empty() {
                        argv.push(expanded);
                    }
                }
            }
        }
        if argv.is_empty() {
            return Err(OpenError::NoCommand);
        }
        lines.push(argv);
    }
    Ok(lines)
}

/// Does `text` hold the field code `%<code>` (and not a `%%` before it)?
fn has_code(text: &str, code: char) -> bool {
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some(next) if next == code => return true,
                _ => {}
            }
        }
    }
    false
}

/// Expand the single-value field codes in one argument.
///
/// Deprecated and unknown codes are dropped, as the specification asks. Paths
/// go in as the bytes they are, so a name that is not UTF-8 still names the
/// file.
fn expand(text: &str, file: Option<&Path>, app: &App) -> OsString {
    let mut out = OsString::new();
    let mut literal = [0u8; 4];
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c.encode_utf8(&mut literal));
            continue;
        }
        match chars.next() {
            Some('%') => out.push("%"),
            Some('f') => {
                if let Some(file) = file {
                    out.push(file);
                }
            }
            Some('u') => {
                if let Some(file) = file {
                    out.push(crate::clipboard::path_to_uri(file));
                }
            }
            Some('c') => out.push(&app.name),
            Some('k') => out.push(&app.entry_path),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, name: &str, mimes: &[&str]) -> App {
        App {
            id: id.to_string(),
            name: name.to_string(),
            icon_name: Some(format!("{name}-icon")),
            exec: Some(format!("{} %U", name.to_lowercase())),
            terminal: false,
            working_dir: None,
            mime_types: mimes.iter().map(|m| m.to_string()).collect(),
            no_display: false,
            entry_path: PathBuf::from(format!("/usr/share/applications/{id}")),
        }
    }

    fn associations(apps: Vec<App>, lists: &[&str]) -> Associations {
        Associations {
            apps: apps.into_iter().map(|app| (app.id.clone(), app)).collect(),
            lists: lists.iter().map(|text| parse_list(text)).collect(),
        }
    }

    fn ids(apps: Vec<&App>) -> Vec<&str> {
        apps.into_iter().map(|app| app.id.as_str()).collect()
    }

    fn chain(mimes: &[&str]) -> Vec<String> {
        mimes.iter().map(|m| m.to_string()).collect()
    }

    #[test]
    fn the_listed_default_wins_over_declared_handlers() {
        let a = associations(
            vec![
                app("evince.desktop", "Evince", &["application/pdf"]),
                app("okular.desktop", "Okular", &["application/pdf"]),
            ],
            &["[Default Applications]\napplication/pdf=okular.desktop;\n"],
        );
        let pdf = chain(&["application/pdf"]);
        assert_eq!(
            a.default_for(&pdf).map(|app| app.id.as_str()),
            Some("okular.desktop")
        );
        assert_eq!(ids(a.apps_for(&pdf)), ["okular.desktop", "evince.desktop"]);
    }

    #[test]
    fn a_default_that_is_not_installed_falls_through() {
        let a = associations(
            vec![app("evince.desktop", "Evince", &["application/pdf"])],
            &["[Default Applications]\napplication/pdf=gone.desktop;evince.desktop;\n"],
        );
        let pdf = chain(&["application/pdf"]);
        assert_eq!(
            a.default_for(&pdf).map(|app| app.id.as_str()),
            Some("evince.desktop")
        );
    }

    #[test]
    fn with_no_default_the_first_handler_opens_it() {
        let a = associations(
            vec![
                app("zed.desktop", "Zed", &["text/plain"]),
                app("gedit.desktop", "Gedit", &["text/plain"]),
            ],
            &[],
        );
        // Declared handlers are offered by name.
        let rust = chain(&["text/x-rust", "text/plain"]);
        assert_eq!(
            a.default_for(&rust).map(|app| app.id.as_str()),
            Some("gedit.desktop")
        );
    }

    #[test]
    fn exact_types_are_offered_before_their_parents() {
        let a = associations(
            vec![
                app("gedit.desktop", "Gedit", &["text/plain"]),
                app("rustrover.desktop", "RustRover", &["text/x-rust"]),
            ],
            &[],
        );
        let rust = chain(&["text/x-rust", "text/plain"]);
        assert_eq!(
            ids(a.apps_for(&rust)),
            ["rustrover.desktop", "gedit.desktop"]
        );
    }

    #[test]
    fn a_removal_hides_lower_files_and_declarations() {
        let a = associations(
            vec![
                app("evince.desktop", "Evince", &["application/pdf"]),
                app("gimp.desktop", "Gimp", &[]),
            ],
            &[
                "[Removed Associations]\napplication/pdf=evince.desktop;gimp.desktop;\n",
                "[Added Associations]\napplication/pdf=gimp.desktop;\n",
            ],
        );
        assert!(a.apps_for(&chain(&["application/pdf"])).is_empty());
    }

    #[test]
    fn an_added_association_lists_an_app_that_does_not_declare_the_type() {
        let a = associations(
            vec![app("gimp.desktop", "Gimp", &[])],
            &["[Added Associations]\napplication/pdf=gimp.desktop;\n"],
        );
        assert_eq!(
            ids(a.apps_for(&chain(&["application/pdf"]))),
            ["gimp.desktop"]
        );
    }

    #[test]
    fn handlers_are_only_listed_as_the_default() {
        let mut handler = app("viewer-pdf.desktop", "Viewer", &["application/pdf"]);
        handler.no_display = true;
        let a = associations(
            vec![
                handler.clone(),
                app("viewer.desktop", "Viewer", &["application/pdf"]),
            ],
            &[],
        );
        let pdf = chain(&["application/pdf"]);
        assert_eq!(ids(a.apps_for(&pdf)), ["viewer.desktop"]);

        let a = associations(
            vec![
                handler,
                app("viewer.desktop", "Viewer", &["application/pdf"]),
            ],
            &["[Default Applications]\napplication/pdf=viewer-pdf.desktop;\n"],
        );
        assert_eq!(
            ids(a.apps_for(&pdf)),
            ["viewer-pdf.desktop", "viewer.desktop"]
        );
    }

    #[test]
    fn all_leaves_out_handlers_and_sorts_by_name() {
        let mut helper = app("helper.desktop", "Helper", &[]);
        helper.no_display = true;
        let a = associations(
            vec![
                app("b.desktop", "beta", &[]),
                app("a.desktop", "Alpha", &[]),
                helper,
            ],
            &[],
        );
        assert_eq!(ids(a.all()), ["a.desktop", "b.desktop"]);
    }

    #[test]
    fn a_new_default_goes_into_both_groups() {
        let text = with_default("", "application/pdf", "okular.desktop");
        assert_eq!(
            text,
            "[Default Applications]\napplication/pdf=okular.desktop;\n\n\
             [Added Associations]\napplication/pdf=okular.desktop;\n"
        );
    }

    #[test]
    fn changing_a_default_keeps_the_rest_of_the_file() {
        let before = "# mine\n[Default Applications]\ntext/plain=gedit.desktop;\n\
                      application/pdf=evince.desktop;\n\n\
                      [Added Associations]\napplication/pdf=evince.desktop;gimp.desktop;\n\n\
                      [Removed Associations]\nimage/png=gimp.desktop;\n";
        let after = with_default(before, "application/pdf", "okular.desktop");
        assert_eq!(
            after,
            "# mine\n[Default Applications]\ntext/plain=gedit.desktop;\n\
             application/pdf=okular.desktop;\n\n\
             [Added Associations]\napplication/pdf=okular.desktop;evince.desktop;gimp.desktop;\n\n\
             [Removed Associations]\nimage/png=gimp.desktop;\n"
        );
        let list = parse_list(&after);
        assert_eq!(list.defaults["text/plain"], ["gedit.desktop"]);
    }

    #[test]
    fn a_new_type_joins_the_end_of_its_group() {
        let before = "[Default Applications]\ntext/plain=gedit.desktop;\n\n[Added Associations]\n";
        let after = with_default(before, "image/png", "loupe.desktop");
        assert_eq!(
            after,
            "[Default Applications]\ntext/plain=gedit.desktop;\nimage/png=loupe.desktop;\n\n\
             [Added Associations]\nimage/png=loupe.desktop;\n"
        );
    }

    fn lines(exec: &str, paths: &[&str]) -> Vec<Vec<String>> {
        let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        command_lines(exec, &paths, &app("x.desktop", "X", &[]))
            .unwrap()
            .into_iter()
            .map(|argv| argv.into_iter().map(|a| a.into_string().unwrap()).collect())
            .collect()
    }

    #[test]
    fn a_file_list_code_takes_every_file_at_once() {
        assert_eq!(lines("gimp %F", &["/a", "/b"]), [["gimp", "/a", "/b"]]);
        assert_eq!(lines("app %U", &["/a b"]), [["app", "file:///a%20b"]]);
    }

    #[test]
    fn a_single_file_code_starts_one_process_per_file() {
        assert_eq!(
            lines("mpv -- %f", &["/a", "/b"]),
            [["mpv", "--", "/a"], ["mpv", "--", "/b"]]
        );
        assert_eq!(lines("app --file=%f", &["/a"]), [["app", "--file=/a"]]);
    }

    #[test]
    fn a_line_with_no_file_code_starts_once_without_files() {
        assert_eq!(
            lines("app --new-window", &["/a", "/b"]),
            [["app", "--new-window"]]
        );
        assert_eq!(lines("app %f", &[]), [["app"]]);
    }

    #[test]
    fn relative_paths_are_made_absolute() {
        let cwd = std::env::current_dir().unwrap();
        let expected = cwd.join("report.pdf");
        assert_eq!(
            lines("app %f", &["./report.pdf"]),
            [["app", expected.to_str().unwrap()]]
        );
        let uri = crate::clipboard::path_to_uri(&expected);
        assert_eq!(lines("app %U", &["report.pdf"]), [["app", uri.as_str()]]);
    }

    #[test]
    fn a_name_that_is_not_utf8_is_passed_as_its_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let path = PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/caf\xe9.txt"));
        let argv = command_lines(
            "app --file=%f %F",
            std::slice::from_ref(&path),
            &app("x.desktop", "X", &[]),
        )
        .unwrap();
        assert_eq!(argv[0][1].as_bytes(), b"--file=/tmp/caf\xe9.txt");
        assert_eq!(argv[0][2], path.into_os_string());
    }

    #[test]
    fn quoting_and_the_other_codes() {
        assert_eq!(
            lines(
                r#""/opt/My App/run" --name="%c" "a \"q\" \$x" 100%% %i %k %d %F"#,
                &["/f"]
            ),
            [[
                "/opt/My App/run",
                "--name=\"X\"",
                "a \"q\" $x",
                "100%",
                "--icon",
                "X-icon",
                "/usr/share/applications/x.desktop",
                "/f",
            ]]
        );
        assert!(matches!(tokenize("\"open"), Err(OpenError::BadCommand(_))));
    }
}
