//! The desktop widget: a full-screen page drawn by ewwii over the wallpaper,
//! run and supervised by the compositor.
//!
//! Which page is the `desktop.widget` setting (`specs/desktop-widget.md`).
//! The themes live in one ewwii configuration, shipped in
//! `/usr/share/otto/widgets/ewwii` and overridden by
//! `~/.config/otto/widgets/ewwii`. Their backdrops are drawn for the size of
//! the usable area and GTK only loads them by absolute URL, so the configuration
//! is first copied to `~/.cache/otto/widgets/ewwii`, where the generators in
//! its `generators/` folder draw them and the stylesheet gets the variables
//! they print.
//!
//! The compositor then runs `ewwii daemon` on that copy the way it runs the
//! desk: spawned with the session's environment, watched through a pidfd and
//! restarted with a back-off if it dies while a widget is chosen. The window
//! is opened by ewwii's own client on a worker thread, which waits for the
//! daemon to answer. Choosing another widget closes the open window and
//! opens the new one on the same daemon; choosing none stops it.
//!
//! The page keeps clear of the top bar, the dock and any other panel: the
//! compositor writes the reserved area to the theme's `reserved-area` file
//! whenever it changes, and the theme follows that file with a `Listen` and
//! pads its window by it. The area is measured from the window's own edges,
//! since a layer-shell surface without an exclusive zone of -1, which ewwii
//! cannot ask for, is already moved clear of the panels' exclusive zones. The
//! backdrops are drawn for the usable area, so a change of its size prepares
//! the theme again.

// Rust guideline compliant 2026-02-21

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use std::{fmt, fs, io};

use smithay::desktop::layer_map_for_output;
use smithay::output::Output;
use smithay::reexports::calloop::{
    channel::{self, Event},
    generic::Generic,
    timer::{TimeoutAction, Timer},
    Interest, Mode, PostAction, RegistrationToken,
};
use smithay::reexports::wayland_server::Resource;
use smithay::utils::{Logical, Rectangle};
use tracing::{info, warn};

use crate::config::default_apps::{xdg_config_home, xdg_data_dirs};
use crate::config::Config;
use crate::desk::{failures_after_crash, pidfd, restart_delay};
use crate::state::{Backend, Otto};

/// The values `desktop.widget` takes: no widget, then the shipped themes'
/// windows, named as in `ewwii.nbcl`.
pub const WIDGET_CHOICES: &[&str] = &["none", "calendar", "stay_focused", "dont_be_busy"];

/// The value that shows no widget.
const NO_WIDGET: &str = "none";

/// The program that draws the widgets, looked up on `PATH`.
const PROGRAM: &str = "ewwii";

/// Where the theme sits under a configuration or data directory, and under
/// the cache directory once it is prepared.
const THEME_DIR: &str = "otto/widgets/ewwii";

/// The theme's folder of backdrop generators.
const GENERATORS_DIR: &str = "generators";

/// The theme's stylesheet, which gets the generators' variables ahead of it.
const STYLESHEET: &str = "ewwii.scss";

/// The file in the prepared theme that holds the reserved area, which the
/// theme follows with `tail -F`. Not a configuration file to ewwii, whose
/// watcher only reloads on `.nbcl`, `.scss` and `.css` changes, so rewriting
/// it never reopens the window.
const RESERVED_AREA_FILE: &str = "reserved-area";

/// How many times the worker asks ewwii to show a window before giving up.
/// Each try waits up to a second for the daemon's socket, so together they
/// cover a slow first start.
const SHOW_ATTEMPTS: u32 = 15;

/// The pause between two tries at showing a window.
const SHOW_RETRY: Duration = Duration::from_millis(500);

/// The compositor's hold on the ewwii daemon.
#[derive(Default)]
pub struct DesktopWidget {
    running: Option<Running>,
    /// A daemon sent `SIGTERM` that has not exited yet. A new one waits for
    /// it, since both would claim the same socket.
    stopping: Option<Running>,
    /// The pending restart, while the daemon waits out its back-off.
    restart: Option<RegistrationToken>,
    /// Crashes in a row, each one soon after its start.
    failures: u32,
    /// The prepared theme the daemon runs on, once there is one.
    theme: Option<PathBuf>,
    /// Hands a prepared theme back to the event loop.
    prepared: Option<channel::Sender<Result<PathBuf, String>>>,
    /// The worker that talks to the daemon.
    client: Option<mpsc::Sender<Show>>,
    /// The window last asked for on the running daemon.
    shown: Option<String>,
    /// The usable size the theme was last prepared for.
    prepared_size: Option<(i32, i32)>,
    /// The reserved area last written to the theme.
    reserved: Option<Insets>,
}

/// How far the usable area sits in from each edge of the widget's window, in
/// logical points: what the top bar, the dock and other panels reserve.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Insets {
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub left: i32,
}

impl Insets {
    /// The space between the edges of `window` and those of `usable`. A side
    /// where `usable` reaches past `window` counts as 0.
    pub fn between(window: Rectangle<i32, Logical>, usable: Rectangle<i32, Logical>) -> Self {
        let window_end = window.loc + window.size;
        let usable_end = usable.loc + usable.size;
        Self {
            top: (usable.loc.y - window.loc.y).max(0),
            right: (window_end.x - usable_end.x).max(0),
            bottom: (window_end.y - usable_end.y).max(0),
            left: (usable.loc.x - window.loc.x).max(0),
        }
    }
}

impl fmt::Display for Insets {
    /// The insets as a CSS shorthand, top first and clockwise, which is the
    /// line written to the theme's `reserved-area` file.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}px {}px {}px {}px",
            self.top, self.right, self.bottom, self.left
        )
    }
}

impl fmt::Debug for DesktopWidget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DesktopWidget")
            .field("running", &self.running)
            .field("stopping", &self.stopping)
            .field("failures", &self.failures)
            .field("theme", &self.theme)
            .field("shown", &self.shown)
            .field("prepared_size", &self.prepared_size)
            .field("reserved", &self.reserved)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct Running {
    child: Child,
    started: Instant,
    /// The event source that fires when the process exits.
    watch: Option<RegistrationToken>,
}

/// A request to the worker: show `window` of the theme in `dir`, alone.
#[derive(Debug)]
struct Show {
    dir: PathBuf,
    window: String,
}

/// The widget this session should show, if any.
///
/// Any name other than `none` is a window of the theme, so a customised theme
/// can add windows of its own and name them in the configuration file.
fn wanted_widget() -> Option<String> {
    if crate::login::is_login_mode() {
        return None;
    }
    let widget = Config::with(|c| c.desktop.widget.trim().to_string());
    (!widget.is_empty() && widget != NO_WIDGET).then_some(widget)
}

/// Whether `program` is an executable file in one of the `PATH` directories.
fn on_path(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| {
            fs::metadata(dir.join(program))
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
    })
}

/// The theme to prepare: the user's own copy if there is one, otherwise the
/// first one installed in the data directories.
fn theme_source() -> Option<PathBuf> {
    xdg_config_home()
        .into_iter()
        .chain(xdg_data_dirs())
        .map(|dir| dir.join(THEME_DIR))
        .find(|dir| dir.join("ewwii.nbcl").is_file())
}

/// Where the prepared theme goes: `$XDG_CACHE_HOME/otto/widgets/ewwii`.
fn prepared_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".cache")))
        .map(|dir| dir.join(THEME_DIR))
}

/// `path` as a `file://` URL, with everything but unreserved characters and
/// separators percent-encoded.
fn file_url(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut url = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            url.push(char::from(byte));
        } else {
            url.push_str(&format!("%{byte:02X}"));
        }
    }
    url
}

/// Whether the files at `a` and `b` both exist and hold the same bytes.
fn same_contents(a: &Path, b: &Path) -> bool {
    matches!((fs::read(a), fs::read(b)), (Ok(a), Ok(b)) if a == b)
}

/// Copy the files of `source` into `dest`, keeping their modes, except the
/// top-level stylesheet, which [`prepare_theme`] writes itself.
///
/// Files are overwritten in place rather than the folder replaced: the
/// running daemon's working directory is `dest`, and its scripts are found
/// relative to it. A file already there with the same contents is left
/// alone, since the daemon reloads, and reopens its window, on every change
/// to its configuration.
fn copy_theme(source: &Path, dest: &Path, top: bool) -> io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        if top && name == OsStr::new(STYLESHEET) {
            continue;
        }
        let from = entry.path();
        let to = dest.join(&name);
        if entry.file_type()?.is_dir() {
            copy_theme(&from, &to, false)?;
        } else if !same_contents(&from, &to) {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Copy the theme in `source` to `dest`, draw its backdrops for an area of
/// `size` logical pixels, and write its stylesheet.
///
/// Each executable in the theme's `generators/` folder runs from the theme's
/// root with the width and height as arguments (none when `size` is unknown,
/// and the generator picks its own). What they print is SCSS, put ahead of the
/// stylesheet together with `$theme-dir`, the theme's absolute `file://` URL,
/// which is the only way GTK loads an image from the stylesheet, and the size
/// as `$usable-width` and `$usable-height`.
///
/// # Errors
///
/// Fails when a file cannot be copied or written, or a generator cannot run
/// or exits unsuccessfully: without its variables the stylesheet would not
/// compile.
pub fn prepare_theme(source: &Path, dest: &Path, size: Option<(i32, i32)>) -> io::Result<()> {
    copy_theme(source, dest, true)?;

    let mut stylesheet = format!("$theme-dir: \"{}\";\n", file_url(dest));
    // Also makes the stylesheet differ for every size, so ewwii reloads, and
    // GTK loads the redrawn backdrops, even when no generator's variables
    // changed.
    if let Some((width, height)) = size {
        stylesheet.push_str(&format!(
            "$usable-width: {width}px;\n$usable-height: {height}px;\n"
        ));
    }
    let mut generators: Vec<PathBuf> = match fs::read_dir(dest.join(GENERATORS_DIR)) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .collect(),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(err) => return Err(err),
    };
    generators.sort();
    for generator in generators {
        let mut command = Command::new(&generator);
        command.current_dir(dest).stdin(Stdio::null());
        if let Some((width, height)) = size {
            command.arg(width.to_string()).arg(height.to_string());
        }
        let output = command.output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "{} failed ({}): {}",
                generator.display(),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        stylesheet.push_str(&String::from_utf8_lossy(&output.stdout));
    }
    stylesheet.push('\n');
    stylesheet.push_str(&fs::read_to_string(source.join(STYLESHEET))?);

    // Written whole and renamed into place: the daemon reloads when the
    // stylesheet changes, and must never read half of one. Unchanged, it is
    // not written at all.
    let target = dest.join(STYLESHEET);
    if fs::read_to_string(&target).is_ok_and(|current| current == stylesheet) {
        return Ok(());
    }
    let partial = dest.join(format!(".{STYLESHEET}.new"));
    fs::write(&partial, stylesheet)?;
    fs::rename(&partial, target)
}

/// Write `insets` to the theme in `dir` as one line, replacing the file in
/// one step so the `tail -F` following it never reads half of one.
fn write_reserved_area(dir: &Path, insets: Insets) -> io::Result<()> {
    let partial = dir.join(format!(".{RESERVED_AREA_FILE}.new"));
    fs::write(&partial, format!("{insets}\n"))?;
    fs::rename(&partial, dir.join(RESERVED_AREA_FILE))
}

/// Run one ewwii client command against the daemon for `dir`. Never starts a
/// daemon of its own: that one belongs to the compositor.
fn ewwii(dir: &Path, args: &[&str]) -> bool {
    Command::new(PROGRAM)
        .arg("--config")
        .arg(dir)
        .arg("--no-daemonize")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// The worker behind [`DesktopWidget::client`]: shows each requested window,
/// alone, once the daemon answers. A newer request replaces one still being
/// tried.
fn show_windows(requests: mpsc::Receiver<Show>) {
    let mut next = requests.recv().ok();
    while let Some(mut show) = next.take() {
        let mut shown = false;
        for attempt in 0..SHOW_ATTEMPTS {
            if let Some(newer) = requests.try_iter().last() {
                show = newer;
            }
            if ewwii(&show.dir, &["close-all"]) && ewwii(&show.dir, &["open", &show.window]) {
                shown = true;
                break;
            }
            if attempt + 1 < SHOW_ATTEMPTS {
                std::thread::sleep(SHOW_RETRY);
            }
        }
        if shown {
            info!(window = %show.window, "desktop widget shown");
        } else {
            warn!(window = %show.window, "ewwii did not show the desktop widget");
        }
        next = requests.try_iter().last().or_else(|| requests.recv().ok());
    }
}

impl<B: Backend + 'static> Otto<B> {
    /// Show the widget `desktop.widget` names, or stop ewwii for none.
    ///
    /// Called at startup, whenever the setting changes and when the usable
    /// area changes size. The theme is prepared again each time, for the
    /// size of the primary output's usable area, on a thread of its own;
    /// [`Self::desktop_widget_prepared`] carries on once it is ready.
    pub fn apply_desktop_widget_setting(&mut self) {
        let Some(widget) = wanted_widget() else {
            self.stop_desktop_widget();
            return;
        };
        if !on_path(PROGRAM) {
            warn!(%widget, "ewwii is not installed; no desktop widget");
            self.stop_desktop_widget();
            return;
        }
        let (Some(source), Some(dest)) = (theme_source(), prepared_dir()) else {
            warn!(%widget, "no desktop widget theme is installed");
            return;
        };
        let Some(sender) = self.desktop_widget_channel() else {
            return;
        };
        let size = self.desktop_widget_area().map(|(usable, _)| usable);
        self.desktop_widget.prepared_size = size;
        let spawned = std::thread::Builder::new()
            .name("desktop widget theme".into())
            .spawn(move || {
                let result = prepare_theme(&source, &dest, size)
                    .map(|()| dest)
                    .map_err(|err| format!("{}: {err}", source.display()));
                let _ = sender.send(result);
            });
        if let Err(err) = spawned {
            warn!(%err, "could not prepare the desktop widget theme");
        }
    }

    /// The size of the primary output's usable area and the insets between
    /// it and the widget's window. The usable area is the one maximized
    /// windows fill: the output less the exclusive zones of layer-shell
    /// panels and the dock's band. Until the window is mapped, the output
    /// stands in for it.
    fn desktop_widget_area(&self) -> Option<((i32, i32), Insets)> {
        let output = self.workspaces.primary_output()?;
        let geometry = self.workspaces.output_geometry(output)?;
        let usable = self.usable_zone(output);
        let window = self
            .desktop_widget_window(output)
            .map(|rect| Rectangle::new(rect.loc + geometry.loc, rect.size))
            .unwrap_or(geometry);
        Some((
            (usable.size.w, usable.size.h),
            Insets::between(window, usable),
        ))
    }

    /// Where the daemon's layer-shell window sits on `output`, relative to
    /// the output, if it is mapped there.
    fn desktop_widget_window(&self, output: &Output) -> Option<Rectangle<i32, Logical>> {
        let pid = i32::try_from(self.desktop_widget.running.as_ref()?.child.id()).ok()?;
        let map = layer_map_for_output(output);
        let layer = map.layers().find(|layer| {
            self.display_handle
                .get_client(layer.wl_surface().id())
                .ok()
                .and_then(|client| client.get_credentials(&self.display_handle).ok())
                .is_some_and(|credentials| credentials.pid == pid)
        })?;
        map.layer_geometry(layer)
    }

    /// Bring the widget up to date with the primary output's usable area:
    /// write the reserved area to the theme when it moved, and prepare the
    /// theme again when the area changed size, since the backdrops are drawn
    /// for it.
    ///
    /// Called whenever the usable area or the window may have moved: after
    /// any layer-shell change and once the dock settles. Does nothing while
    /// ewwii is not running, or when nothing changed.
    pub fn desktop_widget_area_changed(&mut self) {
        if self.desktop_widget.running.is_none() || self.desktop_widget.theme.is_none() {
            return;
        }
        let Some((size, _)) = self.desktop_widget_area() else {
            return;
        };
        self.write_desktop_widget_reserved_area();
        if self.desktop_widget.prepared_size != Some(size) {
            self.apply_desktop_widget_setting();
        }
    }

    /// Write the primary output's reserved area to the prepared theme, unless
    /// it holds that already.
    ///
    /// Written on the event loop: one short line into the cache, only when
    /// the area moved.
    fn write_desktop_widget_reserved_area(&mut self) {
        let Some(dir) = self.desktop_widget.theme.clone() else {
            return;
        };
        let Some((_, insets)) = self.desktop_widget_area() else {
            return;
        };
        if self.desktop_widget.reserved == Some(insets) {
            return;
        }
        match write_reserved_area(&dir, insets) {
            Ok(()) => {
                info!(%insets, "desktop widget reserved area");
                self.desktop_widget.reserved = Some(insets);
            }
            Err(err) => warn!(%err, "could not write the desktop widget's reserved area"),
        }
    }

    /// The sender prepared themes come back through, registering its
    /// receiving end on the event loop the first time.
    fn desktop_widget_channel(&mut self) -> Option<channel::Sender<Result<PathBuf, String>>> {
        if let Some(sender) = &self.desktop_widget.prepared {
            return Some(sender.clone());
        }
        let (sender, receiver) = channel::channel();
        match self.handle.insert_source(receiver, |event, _, state| {
            if let Event::Msg(result) = event {
                state.desktop_widget_prepared(result);
            }
        }) {
            Ok(_) => {
                self.desktop_widget.prepared = Some(sender.clone());
                Some(sender)
            }
            Err(err) => {
                warn!(err = %err.error, "could not listen for the desktop widget theme");
                None
            }
        }
    }

    /// A theme finished preparing: show the widget chosen by now, starting
    /// the daemon if it is not running.
    fn desktop_widget_prepared(&mut self, result: Result<PathBuf, String>) {
        let dir = match result {
            Ok(dir) => dir,
            Err(err) => {
                warn!(%err, "could not prepare the desktop widget theme");
                return;
            }
        };
        let Some(widget) = wanted_widget() else {
            return;
        };
        // Written again with every preparation, so a file lost from the cache
        // comes back.
        self.desktop_widget.reserved = None;
        self.desktop_widget.theme = Some(dir);
        self.write_desktop_widget_reserved_area();
        if self.desktop_widget.running.is_some() {
            // The same window again needs no reopening: ewwii reloads it by
            // itself when the stylesheet changed.
            if self.desktop_widget.shown.as_ref() != Some(&widget) {
                self.show_desktop_widget(widget);
            }
            return;
        }
        if let Some(token) = self.desktop_widget.restart.take() {
            self.handle.remove(token);
        }
        self.desktop_widget.failures = 0;
        self.start_desktop_widget();
    }

    /// Ask the worker to show `widget` on the running daemon.
    fn show_desktop_widget(&mut self, widget: String) {
        let Some(dir) = self.desktop_widget.theme.clone() else {
            return;
        };
        self.desktop_widget.shown = Some(widget.clone());
        let show = Show {
            dir,
            window: widget,
        };
        let show = match &self.desktop_widget.client {
            Some(client) => match client.send(show) {
                Ok(()) => return,
                Err(mpsc::SendError(show)) => show,
            },
            None => show,
        };
        let (client, requests) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("desktop widget".into())
            .spawn(move || show_windows(requests));
        match spawned {
            Ok(_) => {
                let _ = client.send(show);
                self.desktop_widget.client = Some(client);
            }
            Err(err) => warn!(%err, "could not start the desktop widget worker"),
        }
    }

    /// Spawn the daemon on the prepared theme and show the chosen widget,
    /// unless one is running or still on its way out; the latter starts it
    /// once it has exited.
    fn start_desktop_widget(&mut self) {
        if self.desktop_widget.running.is_some() || self.desktop_widget.stopping.is_some() {
            return;
        }
        let (Some(dir), Some(widget)) = (self.desktop_widget.theme.clone(), wanted_widget()) else {
            return;
        };
        let args = [
            "--config".to_string(),
            dir.to_string_lossy().into_owned(),
            "--no-daemonize".to_string(),
            "daemon".to_string(),
        ];
        let child = match self.program_command(PROGRAM, &args).spawn() {
            Ok(child) => child,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                warn!(program = PROGRAM, "ewwii is not installed");
                return;
            }
            Err(err) => {
                warn!(program = PROGRAM, %err, "could not start ewwii");
                self.desktop_widget.failures = self.desktop_widget.failures.saturating_add(1);
                self.schedule_desktop_widget_restart();
                return;
            }
        };
        info!(pid = child.id(), "ewwii started for the desktop widget");

        let pid = child.id();
        let watch = match pidfd(&child) {
            Ok(fd) => self
                .handle
                .insert_source(
                    Generic::new(fd, Interest::READ, Mode::Level),
                    move |_, _, state| Ok(state.desktop_widget_exited(pid)),
                )
                .map_err(|err| warn!(err = %err.error, "could not watch ewwii"))
                .ok(),
            Err(err) => {
                warn!(%err, "could not watch ewwii");
                None
            }
        };
        self.desktop_widget.running = Some(Running {
            child,
            started: Instant::now(),
            watch,
        });
        self.show_desktop_widget(widget);
    }

    /// The daemon `pid` has exited: reap it and, if that was not asked for,
    /// schedule a restart.
    fn desktop_widget_exited(&mut self, pid: u32) -> PostAction {
        let widget = &mut self.desktop_widget;
        if let Some(stopping) = widget.stopping.as_mut().filter(|s| s.child.id() == pid) {
            match stopping.child.try_wait() {
                Ok(Some(_)) => {}
                Ok(None) => return PostAction::Continue,
                Err(err) => warn!(%err, "could not reap the stopped ewwii"),
            }
            widget.stopping = None;
            if wanted_widget().is_some() && widget.restart.is_none() {
                self.start_desktop_widget();
            }
            return PostAction::Remove;
        }
        let Some(running) = widget.running.as_mut().filter(|r| r.child.id() == pid) else {
            return PostAction::Remove;
        };
        let status = match running.child.try_wait() {
            Ok(Some(status)) => status,
            Ok(None) => return PostAction::Continue,
            Err(err) => {
                warn!(%err, "could not reap ewwii");
                return PostAction::Continue;
            }
        };
        let uptime = running.started.elapsed();
        widget.running = None;
        widget.shown = None;

        // A clean exit is someone running `ewwii kill` on it: leave it be
        // until the setting changes.
        if status.success() {
            info!("ewwii exited");
            return PostAction::Remove;
        }

        widget.failures = failures_after_crash(widget.failures, uptime);
        warn!(
            %status,
            uptime_ms = uptime.as_millis() as u64,
            failures = widget.failures,
            "ewwii exited unexpectedly; restarting it"
        );
        self.schedule_desktop_widget_restart();
        PostAction::Remove
    }

    /// Start the daemon again once the back-off for the current crash count
    /// has passed.
    fn schedule_desktop_widget_restart(&mut self) {
        if let Some(token) = self.desktop_widget.restart.take() {
            self.handle.remove(token);
        }
        let timer = Timer::from_duration(restart_delay(self.desktop_widget.failures));
        match self.handle.insert_source(timer, |_, _, state| {
            state.desktop_widget.restart = None;
            state.start_desktop_widget();
            TimeoutAction::Drop
        }) {
            Ok(token) => self.desktop_widget.restart = Some(token),
            Err(err) => warn!(err = %err.error, "could not schedule an ewwii restart"),
        }
    }

    /// Terminate the daemon and cancel any pending restart.
    fn stop_desktop_widget(&mut self) {
        if let Some(token) = self.desktop_widget.restart.take() {
            self.handle.remove(token);
        }
        self.desktop_widget.failures = 0;
        self.desktop_widget.shown = None;
        let Some(running) = self.desktop_widget.running.take() else {
            return;
        };
        let pid = running.child.id();
        info!(pid, "stopping ewwii");
        if let Ok(pid) = libc::pid_t::try_from(pid) {
            // SAFETY: `kill` has no memory-safety preconditions. The pid is
            // our own child and has not been reaped, so it still names it.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
        // The watch stays registered and reaps it; without one a thread does.
        if running.watch.is_some() {
            self.desktop_widget.stopping = Some(running);
        } else {
            crate::input::actions::reap_in_background(PROGRAM, running.child);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_url_escapes_what_css_would_misread() {
        assert_eq!(
            file_url(Path::new("/home/a b/.cache/otto#1")),
            "file:///home/a%20b/.cache/otto%231"
        );
    }

    /// The shipped theme prepares into a stylesheet that defines every
    /// variable it uses, with the hand-tuned positions for a 1440 by 960
    /// screen.
    #[test]
    fn the_shipped_theme_prepares_for_the_screen() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/widgets/ewwii");
        let dest = std::env::temp_dir().join(format!("otto-widget-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dest);

        prepare_theme(&source, &dest, Some((1440, 960))).expect("theme prepares");

        let stylesheet = fs::read_to_string(dest.join(STYLESHEET)).expect("stylesheet");
        assert!(stylesheet.starts_with(&format!("$theme-dir: \"{}\";", file_url(&dest))));
        for line in [
            "$lines-note-left: 941px;",
            "$lines-note-top: 712px;",
            "$lines-date-shift-x: -45px;",
            "$focus-note-right: 90px;",
            "$focus-note-bottom: 88px;",
        ] {
            assert!(stylesheet.contains(line), "missing `{line}`");
        }
        for backdrop in ["lines/grid@2x.png", "focus/grid@2x.png"] {
            assert!(dest.join(backdrop).is_file(), "{backdrop} was not drawn");
        }
        // GTK drops a whole stylesheet over one non-ASCII byte.
        assert!(stylesheet.is_ascii());

        let _ = fs::remove_dir_all(&dest);
    }

    /// The theme pads its window by the `reserved-area` line as a CSS
    /// shorthand, so the sides must come top first and clockwise, measured
    /// from the window. Here the window has been moved down below a 30 point
    /// bar, keeps the output's height, and a 78 point dock sits on the left.
    #[test]
    fn insets_are_measured_from_the_window_in_css_order() {
        let window = Rectangle::new((0, 30).into(), (1440, 960).into());
        let usable = Rectangle::new((78, 30).into(), (1362, 930).into());

        let insets = Insets::between(window, usable);

        assert_eq!(insets.to_string(), "0px 0px 30px 78px");
    }

    #[test]
    fn every_shipped_widget_is_a_window_of_the_theme() {
        let nbcl = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/widgets/ewwii/ewwii.nbcl"),
        )
        .expect("theme");
        for widget in WIDGET_CHOICES.iter().filter(|w| **w != NO_WIDGET) {
            assert!(
                nbcl.contains(&format!("Window \"{widget}\"")),
                "`{widget}` is offered but the theme has no such window"
            );
        }
    }
}
