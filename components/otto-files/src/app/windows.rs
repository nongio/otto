//! One Files per session: every browser and Trash window lives in the same
//! process.
//!
//! The first start owns `org.otto.FilesWindows1` on the session bus; a later
//! one hands its arguments to that one and leaves. The owner opens a window
//! for them, or brings the Trash forward when it is already open. Requests,
//! from the bus or from a window asking for another (Ctrl+N, Open in New
//! Window), land in one inbox the UI loop drains. Closing a window closes
//! that window; the last one closed ends the process.
//!
//! The desk and the file picker stay processes of their own: the desk is a
//! layer surface that lasts the session, and the picker is a dialog the bus
//! starts and stops. Both run one window through the same [`FilesApp`].
//!
//! The view layer keeps a few per-window settings in process-wide cells (the
//! shell, the decoration, the search strip, the icon size), so every lock of
//! a window's [`Browser`] through [`Browse::browse`] puts that window's own
//! values back first.

use super::*;

use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::MutexGuard;

use smithay_client_toolkit::shell::WaylandSurface;

pub const DBUS_NAME: &str = "org.otto.FilesWindows1";
pub const DBUS_PATH: &str = "/org/otto/FilesWindows1";

/// A window asked for.
#[derive(Debug, Clone)]
pub enum Request {
    /// A browser window, with the command line that asked for it and the
    /// folder that command line was given in.
    Browse {
        args: Vec<OsString>,
        cwd: PathBuf,
        token: Option<String>,
    },
    /// A browser window on `folder`, asked for by a window of this process.
    Folder(PathBuf),
    /// The Trash, with its Empty Trash question up when `empty`.
    Trash { empty: bool, token: Option<String> },
}

/// Requests waiting for the UI loop.
static INBOX: Mutex<Vec<Request>> = Mutex::new(Vec::new());

/// Whether this process hosts the browser windows, and so takes requests
/// for more.
static HOSTING: AtomicBool = AtomicBool::new(false);

/// Ask for a window. Answered on the UI loop's next pass.
pub fn open(request: Request) {
    INBOX.lock().unwrap().push(request);
    AppContext::request_wakeup();
}

/// Whether a window asked for here opens in this process. Not in the desk
/// or the picker, which start a Files of their own for it.
pub fn hosting() -> bool {
    HOSTING.load(Ordering::Relaxed)
}

struct Service;

#[zbus::interface(name = "org.otto.FilesWindows1")]
impl Service {
    /// Open a browser window for `args`, a command line as `otto-files` takes
    /// it, given in the folder `cwd`. `token` is an xdg-activation token,
    /// empty when there is none.
    fn open(&self, args: Vec<Vec<u8>>, cwd: Vec<u8>, token: String) {
        open(Request::Browse {
            args: args.into_iter().map(OsString::from_vec).collect(),
            cwd: PathBuf::from(OsString::from_vec(cwd)),
            token: (!token.is_empty()).then_some(token),
        });
    }

    /// Open the Trash, or bring it forward, with the Empty Trash question up
    /// when `empty`.
    fn open_trash(&self, empty: bool, token: String) {
        open(Request::Trash {
            empty,
            token: (!token.is_empty()).then_some(token),
        });
    }
}

/// Who this start turned out to be.
pub enum Claim {
    /// The owner: hosts the windows for as long as the connection is held.
    Owner(zbus::Connection),
    /// Another Files owns the name and has been handed the request.
    Forwarded,
}

/// Claim the bus name, or hand `request` to the Files that holds it.
///
/// Claimed without replacement: a Files still running keeps its windows,
/// and one that has exited no longer holds the name. An error means no
/// session bus, and the caller carries on alone.
pub async fn claim_or_forward(request: &Request) -> zbus::Result<Claim> {
    use zbus::fdo::{DBusProxy, RequestNameFlags, RequestNameReply};

    let connection = zbus::connection::Builder::session()?.build().await?;
    // Served before the name is claimed, so no request can reach the name
    // ahead of the object.
    connection.object_server().at(DBUS_PATH, Service).await?;
    let reply = DBusProxy::new(&connection)
        .await?
        .request_name(DBUS_NAME.try_into()?, RequestNameFlags::DoNotQueue.into())
        .await?;
    if reply == RequestNameReply::PrimaryOwner {
        tracing::info!(name = DBUS_NAME, "files windows service running");
        return Ok(Claim::Owner(connection));
    }

    match request {
        Request::Browse { args, cwd, token } => {
            let args: Vec<&[u8]> = args.iter().map(|arg| arg.as_bytes()).collect();
            connection
                .call_method(
                    Some(DBUS_NAME),
                    DBUS_PATH,
                    Some(DBUS_NAME),
                    "Open",
                    &(
                        args,
                        cwd.as_os_str().as_bytes(),
                        token.as_deref().unwrap_or(""),
                    ),
                )
                .await?;
        }
        Request::Trash { empty, token } => {
            connection
                .call_method(
                    Some(DBUS_NAME),
                    DBUS_PATH,
                    Some(DBUS_NAME),
                    "OpenTrash",
                    &(*empty, token.as_deref().unwrap_or("")),
                )
                .await?;
        }
        // The folder is absolute, so any working directory will do.
        Request::Folder(folder) => {
            connection
                .call_method(
                    Some(DBUS_NAME),
                    DBUS_PATH,
                    Some(DBUS_NAME),
                    "Open",
                    &(vec![folder.as_os_str().as_bytes()], b"/".as_slice(), ""),
                )
                .await?;
        }
    }
    Ok(Claim::Forwarded)
}

/// Host the browser and Trash windows, starting with the one `first` asks
/// for, until the last of them is closed.
pub fn run_windows(first: Request) -> Result<(), Box<dyn std::error::Error>> {
    HOSTING.store(true, Ordering::Relaxed);
    UI_THREAD.with(|ui| ui.set(true));
    let selection_queue = Some(serve_files_interface());
    let mut app = FilesApp::new(Vec::new(), selection_queue);
    app.hosting = true;
    app.first = Some(first);
    prewarm_sounds();
    AppRunner::new(app).run()
}

/// Run one window over `browser`: the desk, or the picker serving
/// `picker_queue`.
pub(super) fn run_single(
    browser: Browser,
    picker_queue: Option<crate::dbus::SharedQueue>,
) -> Result<(), Box<dyn std::error::Error>> {
    UI_THREAD.with(|ui| ui.set(true));
    let selection_queue = picker_queue.is_none().then(serve_files_interface);
    let window = FilesWindow::new(browser, picker_queue);
    prewarm_sounds();
    AppRunner::new(FilesApp::new(vec![window], selection_queue)).run()
}

/// Resolve the file-operation sounds off the main thread now, so the first
/// delete of the session does not pay for a walk of every theme directory at
/// the moment it wants to make a noise.
fn prewarm_sounds() {
    let sounds: Vec<&str> = SOUND_DESTROYED
        .iter()
        .chain(SOUND_REMOVED.iter())
        .chain(SOUND_RESTORED.iter())
        .chain(SOUND_ARRIVED.iter())
        .copied()
        .collect();
    otto_kit::sound::prewarm(&sounds);
}

/// Start serving `org.otto.Files1` for this process's windows.
///
/// Outside a tokio runtime the queue is never fed and the windows simply
/// cannot be asked. A bus that refuses us costs nothing but that either.
fn serve_files_interface() -> crate::files_service::SharedQueue {
    let queue = crate::files_service::SharedQueue::default();
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        let service_queue = Arc::clone(&queue);
        runtime.spawn(async move {
            if let Err(error) = crate::files_service::serve(service_queue).await {
                tracing::warn!(%error, "cannot serve org.otto.Files1");
            }
        });
    }
    queue
}

/// The browser a [`Request::Browse`] or [`Request::Folder`] opens on.
fn browser_for(start: crate::launch::Start) -> Browser {
    let mut browser = Browser::new(start.folder);
    browser.remember(&remembered::Remembered::load());
    if !start.select.is_empty() {
        browser.pending_select = Some(
            start
                .select
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
        );
    }
    if let Some((query, scope)) = start.search {
        browser.start_search(query, scope);
    }
    browser
}

thread_local! {
    /// Whether this thread runs the windows. Only it measures, draws and
    /// hit-tests, so only it may set the view layer's cells: a worker
    /// finishing a thumbnail must not change them under a window the UI
    /// thread is in the middle of drawing.
    static UI_THREAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A window's browser, locked, and on the UI thread with the view layer set
/// up for it.
pub(super) trait Browse {
    fn browse(&self) -> MutexGuard<'_, Browser>;
}

impl Browse for Mutex<Browser> {
    fn browse(&self) -> MutexGuard<'_, Browser> {
        let browser = self.lock().unwrap();
        if UI_THREAD.with(std::cell::Cell::get) {
            browser.enter_view();
        }
        browser
    }
}

impl Browser {
    /// Put this window's settings into the view layer's process-wide cells,
    /// so the geometry measured next is this window's. The desk is a process
    /// of its own and sets them once.
    pub(super) fn enter_view(&self) {
        if view::is_desk() {
            return;
        }
        view::set_shell(if self.trash {
            view::Shell::Trash
        } else {
            view::Shell::Browser
        });
        view::set_decoration_variant(self.decoration);
        view::set_search_band(self.search.is_some());
        view::set_grid_icon(self.grid_icon);
    }
}

/// The application: every window this process shows.
pub(super) struct FilesApp {
    windows: Vec<FilesWindow>,
    /// The window this start was asked for, opened once the app is ready.
    first: Option<Request>,
    /// Whether this process hosts the browser windows: it takes requests for
    /// more, and ends when the last is closed.
    hosting: bool,
    /// Questions from `org.otto.Files1` waiting for the UI thread. `None` in
    /// the picker, whose selection belongs to the application it serves.
    selection_queue: Option<crate::files_service::SharedQueue>,
    /// The main surface of the window that was last activated, which takes
    /// the keys when the keyboard is on a surface no window claims.
    last_active: Option<wayland_client::backend::ObjectId>,
    /// The surface the pointer last entered, which says which window a
    /// touchpad gesture is over.
    pointer_over: Rc<RefCell<Option<wayland_client::backend::ObjectId>>>,
    /// Whether any window is in front, as last told to the task reporter.
    watched: Option<bool>,
}

impl FilesApp {
    fn new(
        windows: Vec<FilesWindow>,
        selection_queue: Option<crate::files_service::SharedQueue>,
    ) -> Self {
        Self {
            windows,
            first: None,
            hosting: false,
            selection_queue,
            last_active: None,
            pointer_over: Rc::new(RefCell::new(None)),
            watched: None,
        }
    }

    /// Open the window `request` asks for, or bring forward the Trash.
    fn open(&mut self, request: Request) -> Result<(), Box<dyn std::error::Error>> {
        let mut activation = None;
        let browser = match request {
            // The start that sent this has already said what it could not
            // honour, on its own stderr.
            Request::Browse { args, cwd, token } => {
                activation = token;
                let home = crate::model::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                let args = crate::launch::parse(args);
                browser_for(crate::launch::resolve(
                    &args,
                    &home,
                    &cwd,
                    crate::launch::Probe::disk,
                ))
            }
            Request::Folder(folder) => browser_for(crate::launch::Start {
                folder,
                select: Vec::new(),
                search: None,
                warnings: Vec::new(),
            }),
            Request::Trash { empty, token } => {
                if let Some(window) = self.windows.iter().find(|w| w.state.browse().trash) {
                    if empty {
                        window.state.browse().pending_empty_ask = true;
                    }
                    if let Some(surface) = window.window.as_ref().and_then(Window::surface) {
                        AppContext::activate(surface.xdg_window().wl_surface(), token);
                    }
                    window.render();
                    return Ok(());
                }
                activation = token;
                let mut browser = Browser::for_trash();
                browser.pending_empty_ask = empty;
                browser
            }
        };
        let mut window = FilesWindow::new(browser, None);
        window.show()?;
        // A launcher's token lets the new window come forward the way the
        // launcher asked; the compositor spends it when the window maps.
        if let (Some(token), Some(surface)) =
            (activation, window.window.as_ref().and_then(Window::surface))
        {
            AppContext::activate(surface.xdg_window().wl_surface(), Some(token));
        }
        self.windows.push(window);
        Ok(())
    }

    /// The window `surface` belongs to: its own, or one it hangs beside it.
    fn owner(&self, surface: &wayland_client::backend::ObjectId) -> Option<usize> {
        self.windows.iter().position(|window| window.owns(surface))
    }

    /// The window the keys go to.
    fn keyed(&self) -> Option<usize> {
        AppContext::keyboard_focus()
            .and_then(|focus| self.owner(&focus))
            .or_else(|| {
                let last = self.last_active.as_ref()?;
                self.windows
                    .iter()
                    .position(|window| window.surface_id().as_ref() == Some(last))
            })
            .or_else(|| (self.windows.len() == 1).then_some(0))
    }

    /// The window under the pointer.
    fn pointed(&self) -> Option<usize> {
        let over = self.pointer_over.borrow().clone();
        over.and_then(|surface| self.owner(&surface))
            .or_else(|| self.keyed())
    }

    /// Take down the windows that were asked to close.
    fn close_closing(&mut self) {
        self.windows.retain_mut(|window| {
            let closing = {
                let mut browser = window.state.browse();
                // A file dialog closed is a pick cancelled: the requester
                // is waiting for an answer.
                if browser.closing && browser.picker.is_some() {
                    browser.closing = false;
                    browser.picker_cancel();
                }
                browser.closing
            };
            if closing {
                window.close();
            }
            !closing
        });
    }

    /// Answer `org.otto.Files1.FocusedSelection` for every call waiting on
    /// it, from the window holding the keyboard. Any surface of a window
    /// counts: its Get Info panel and its menus are about the same
    /// selection.
    fn answer_selection_asks(&self) {
        let Some(queue) = self.selection_queue.as_ref() else {
            return;
        };
        let asks = queue.take();
        if asks.is_empty() {
            return;
        }
        let answer = AppContext::keyboard_focus()
            .and_then(|_| self.keyed())
            .and_then(|index| self.windows[index].state.browse().focused_selection(true));
        for ask in asks {
            let _ = ask.send(answer.clone());
        }
    }

    /// A running job is reported on the island only while the user is
    /// somewhere else; in front of a Files window the status line has it.
    fn follow_watched(&mut self) {
        let watched = self.windows.iter().any(|w| w.state.browse().focused);
        if self.watched != Some(watched) {
            self.watched = Some(watched);
            crate::tasks::set_watched(watched);
        }
    }
}

impl App for FilesApp {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        // Before any window: a surface builds its own root layer node at
        // construction, and that node is what the scene hangs off.
        AppContext::enable_layer_engine(view::WINDOW_W, view::WINDOW_H);

        let over = Rc::clone(&self.pointer_over);
        AppContext::register_pointer_callback(move |events| {
            use wayland_client::Proxy;
            for event in events {
                if let PointerEventKind::Enter { .. } = event.kind {
                    *over.borrow_mut() = Some(event.surface.id());
                }
            }
        });

        for window in &mut self.windows {
            window.show()?;
        }
        if let Some(first) = self.first.take() {
            self.open(first)?;
        }
        Ok(())
    }

    fn accessibility(
        &mut self,
        ctx: &AppContext,
        surface: &wayland_client::backend::ObjectId,
    ) -> Option<A11yTree> {
        let index = self.owner(surface)?;
        self.windows[index].accessibility(ctx, surface)
    }

    fn on_accessibility_action(
        &mut self,
        ctx: &AppContext,
        surface: &wayland_client::backend::ObjectId,
        request: &ActionRequest,
    ) {
        if let Some(index) = self.owner(surface) {
            self.windows[index].on_accessibility_action(ctx, surface, request);
        }
    }

    fn on_update(&mut self, ctx: &AppContext) {
        let requests = std::mem::take(&mut *INBOX.lock().unwrap());
        for request in requests {
            if let Err(err) = self.open(request) {
                tracing::warn!(%err, "could not open a window");
            }
        }
        self.close_closing();
        // The last window closed is the application closed; the bus name
        // goes with the process, and the next start owns it.
        if self.hosting && self.windows.is_empty() {
            AppContext::request_exit();
            return;
        }
        for window in &mut self.windows {
            window.on_update(ctx);
        }
        self.answer_selection_asks();
        self.follow_watched();
    }

    fn on_pointer_hold_begin(&mut self, ctx: &AppContext, fingers: u32) {
        if let Some(index) = self.pointed() {
            self.windows[index].on_pointer_hold_begin(ctx, fingers);
        }
    }

    fn on_pointer_pinch_begin(&mut self, ctx: &AppContext, fingers: u32) {
        if let Some(index) = self.pointed() {
            self.windows[index].on_pointer_pinch_begin(ctx, fingers);
        }
    }

    fn on_pointer_pinch_update(
        &mut self,
        ctx: &AppContext,
        dx: f64,
        dy: f64,
        scale: f64,
        rotation: f64,
    ) {
        if let Some(index) = self.pointed() {
            self.windows[index].on_pointer_pinch_update(ctx, dx, dy, scale, rotation);
        }
    }

    fn on_pointer_pinch_end(&mut self, ctx: &AppContext, cancelled: bool) {
        for window in &mut self.windows {
            window.on_pointer_pinch_end(ctx, cancelled);
        }
    }

    fn idle_timeout(&self) -> Option<std::time::Duration> {
        self.windows
            .iter()
            .filter_map(FilesWindow::idle_timeout)
            .min()
    }

    fn on_configure_layer(&mut self, ctx: &AppContext, width: i32, height: i32, serial: u32) {
        for window in &mut self.windows {
            window.on_configure_layer(ctx, width, height, serial);
        }
    }

    fn on_configure(&mut self, ctx: &AppContext, configure: WindowConfigure, serial: u32) {
        let Some((id, ..)) = AppContext::current_surface_configure() else {
            return;
        };
        let Some(window) = self
            .windows
            .iter_mut()
            .find(|window| window.surface_id().as_ref() == Some(&id))
        else {
            return;
        };
        if configure.is_activated() {
            self.last_active = Some(id);
        }
        window.on_configure(ctx, configure, serial);
    }

    fn on_modifiers(&mut self, ctx: &AppContext, modifiers: Modifiers) {
        for window in &mut self.windows {
            window.on_modifiers(ctx, modifiers);
        }
    }

    fn on_keyboard_leave(&mut self, ctx: &AppContext, surface: &wl_surface::WlSurface) {
        for window in &mut self.windows {
            window.on_keyboard_leave(ctx, surface);
        }
    }

    fn on_key_event(
        &mut self,
        ctx: &AppContext,
        event: &KeyEvent,
        key_state: wl_keyboard::KeyState,
        serial: u32,
    ) {
        if let Some(index) = self.keyed() {
            self.windows[index].on_key_event(ctx, event, key_state, serial);
        }
    }
}

impl FilesWindow {
    /// The window's own surface.
    pub(super) fn surface_id(&self) -> Option<wayland_client::backend::ObjectId> {
        self.window.as_ref().and_then(Window::surface_id)
    }

    /// Whether `surface` is this window's, or one of the surfaces it hangs
    /// beside itself: Get Info, Open With, Peek, the palette, the desk's
    /// overflow panel.
    fn owns(&self, surface: &wayland_client::backend::ObjectId) -> bool {
        let is = |id: Option<wayland_client::backend::ObjectId>| id.as_ref() == Some(surface);
        if is(self.surface_id())
            || is(self
                .info_window
                .borrow()
                .as_ref()
                .and_then(Window::surface_id))
            || is(self
                .open_with_window
                .borrow()
                .as_ref()
                .and_then(Window::surface_id))
        {
            return true;
        }
        if let Some(target) = self.peek_target.lock().unwrap().as_ref() {
            if &target.card == surface || &target.overlay == surface {
                return true;
            }
        }
        // The desk's overflow panel is not counted: the desk is a process of
        // its own with one window, which takes every key anyway.
        self.palette_target
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|(palette, _)| palette == surface)
    }

    /// Take the window down, with everything it hangs beside itself.
    fn close(&mut self) {
        if let Some(window) = self.info_window.borrow_mut().take() {
            window.close();
        }
        if let Some(window) = self.open_with_window.borrow_mut().take() {
            window.close();
        }
        *self.peek_target.lock().unwrap() = None;
        *self.palette_target.lock().unwrap() = None;
        if let Some(mut surfaces) = self.pane_surfaces.take() {
            surfaces.close();
        }
        self.app_menu = None;
        if let Some(window) = self.window.take() {
            window.close();
        }
    }
}
