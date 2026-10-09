//! The window: its setup, its input, and the work it sends off the UI thread.
//!
//! The decode, the page rasteriser, the text layer and the recogniser all
//! block, so each runs on a blocking task and lands in the shared [`Viewer`]
//! under its lock, then wakes the loop. A window showing a still picture is
//! not committing frames, so without the wake nothing would notice.

// Rust guideline compliant 2026-02-21

use std::cell::RefCell;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use otto_files::peek::{self, Session, VideoPointer};
use otto_files::watch::{Change, DirWatch};
use otto_kit::components::titlebar::WindowControl;
use otto_kit::components::window::resize;
use otto_kit::prelude::*;
use otto_kit::preview::Preview;
use otto_kit::skia::{Contains, Point};
use otto_kit::CursorShape;
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind};
use smithay_client_toolkit::shell::xdg::window::WindowConfigure;
use smithay_client_toolkit::shell::WaylandSurface;
use wayland_client::protocol::wl_keyboard;

use crate::chat::Chat;
use crate::chrome::{self, Tool};
use crate::instance::{Documents, Inbox, Request};
use crate::viewer::{KeyOutcome, Stamp, Viewer};
use crate::Shape;

/// The desktop entry this binary is installed under, and the window's app id.
pub const APP_ID: &str = "otto-preview";
/// The smallest the window may be made, in points.
pub const MIN_W: f32 = 480.0;
pub const MIN_H: f32 = 360.0;
/// The frame's corner radius while it floats, as other Otto windows.
const CORNER: f32 = 12.0;
/// The evdev code of the primary button.
const BTN_LEFT: u32 = 0x110;
/// How often the loop turns while something moves on its own.
const FRAME: Duration = Duration::from_millis(16);
/// How often the loop turns while the chat's caret blinks.
const BLINK: Duration = Duration::from_millis(100);

/// The application: a window for each open file.
pub struct PreviewApp {
    docs: Vec<Doc>,
    /// The file this start was asked for, opened once the app is ready.
    first: Option<Request>,
    /// Files asked for by later starts, handed over the bus.
    inbox: Inbox,
    /// The windows' viewers, for the document tools on the bus.
    documents: Documents,
    /// When the loop last turned, for the caret's blink.
    last_update: Instant,
}

/// One file in its window.
struct Doc {
    viewer: Arc<Mutex<Viewer>>,
    /// The chat beside the document. It stays on the UI thread: its log
    /// holds fonts that may not cross to another, so it is painted into the
    /// viewer's [`Viewer::chat_picture`] for the draw to show.
    chat: Rc<RefCell<Chat>>,
    window: Window,
    /// The file with its links resolved, to tell a second request for it.
    key: PathBuf,
    /// What the opening size is fitted to.
    shape: Shape,
    /// Whether the first configure has been answered, which is when the
    /// window takes its opening size.
    sized: bool,
    /// Show the chat once the window knows its room: asked for on opening.
    chat_on_configure: bool,
    /// The file's folder, watched so the file is shown again when something
    /// changes it: an agent, an editor, a script. A folder rather than the
    /// file, because most tools save by writing a new file over the old.
    watch: Option<DirWatch>,
}

impl PreviewApp {
    /// An application that opens `first` when it is ready, and then whatever
    /// arrives in `inbox`.
    pub fn new(first: Request, inbox: Inbox, documents: Documents) -> Self {
        Self {
            docs: Vec::new(),
            first: Some(first),
            inbox,
            documents,
            last_update: Instant::now(),
        }
    }

    /// Open the file `request` names in a window of its own, or bring
    /// forward the window already showing it.
    fn open(&mut self, request: Request) -> Result<(), Box<dyn std::error::Error>> {
        let key = resolved(&request.path);
        if let Some(doc) = self.docs.iter().find(|doc| doc.key == key) {
            if let Some(session) = request.session {
                doc.chat.borrow_mut().carry_on(session);
            }
            if request.chat {
                doc.show_chat();
            }
            if let Some(surface) = doc.window.surface() {
                AppContext::activate(surface.xdg_window().wl_surface(), request.token);
            }
            return Ok(());
        }
        let shape = Shape::of(&request.path);
        let mut doc = Doc::open(request.path, key, shape)?;
        doc.chat_on_configure = request.chat;
        if let Some(session) = request.session {
            doc.chat.borrow_mut().carry_on(session);
        }
        self.documents
            .lock()
            .unwrap()
            .insert(doc.key.clone(), Arc::clone(&doc.viewer));
        self.docs.push(doc);
        Ok(())
    }

    /// The window holding the keyboard.
    fn focused(&self) -> Option<&Doc> {
        let focus = AppContext::keyboard_focus()?;
        self.docs
            .iter()
            .find(|doc| doc.window.surface_id().as_ref() == Some(&focus))
    }

    /// The window under the pointer.
    fn hovered(&self) -> Option<&Doc> {
        self.docs
            .iter()
            .find(|doc| doc.viewer.lock().unwrap().pointer.is_some())
    }
}

/// `path` with its links resolved, or as given when it cannot be.
fn resolved(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

impl Doc {
    /// A window on `path`, sized for `shape`.
    fn open(path: PathBuf, key: PathBuf, shape: Shape) -> Result<Self, Box<dyn std::error::Error>> {
        let chat = Rc::new(RefCell::new(Chat::new(path.clone(), dark())));
        let viewer = Viewer::new(path, shape.opening_size(None));
        let (name, (width, height)) = (viewer.name.clone(), viewer.size);
        let viewer = Arc::new(Mutex::new(viewer));
        let mut window = Window::new(&name, width as i32, height as i32)?;
        window.set_min_size(MIN_W as u32, MIN_H as u32);
        // Opaque, and the same ground the preview is drawn on, so a resize
        // that outruns the repaint shows paper rather than the desktop.
        window.set_background(ground(&AppContext::current_theme()));
        // Named after the desktop entry, so the dock and the switcher find
        // `otto-preview.desktop` directly.
        if let Some(surface) = window.surface() {
            surface.xdg_window().set_app_id(APP_ID.to_string());
        }
        window.set_frame_corner_radius(CORNER);

        let drawn = Arc::clone(&viewer);
        window.on_draw(move |canvas| {
            let theme = AppContext::current_theme();
            let viewer = drawn.lock().unwrap();
            canvas.clear(ground(&theme));
            crate::content::draw(canvas, &viewer, &theme);
            if viewer.chat_open {
                if let Some(picture) = &viewer.chat_picture {
                    canvas.draw_picture(picture, None, None);
                }
            }
            chrome::draw(canvas, &viewer, &theme);
        });

        let pointed = Arc::clone(&viewer);
        let pointed_chat = Rc::clone(&chat);
        let handle = window.clone();
        window.on_pointer_event(move |events| {
            handle_pointer(&pointed, &pointed_chat, &handle, events)
        });

        // The compositor's close (the dock's, a shortcut's) closes this
        // window, not the application with every other file in it.
        let closed = Arc::clone(&viewer);
        window.on_close_request(move || {
            closed.lock().unwrap().closing = true;
            AppContext::request_wakeup();
        });

        AppContext::register_window(window.clone());
        let watch = key.parent().map(DirWatch::new);
        Ok(Self {
            viewer,
            chat,
            window,
            key,
            shape,
            sized: false,
            chat_on_configure: false,
            watch,
        })
    }

    /// Repaint the window, with the chat painted afresh.
    fn redraw(&self) {
        paint_chat(
            &mut self.viewer.lock().unwrap(),
            &mut self.chat.borrow_mut(),
        );
        self.window.request_frame();
    }

    /// Decode the file off the UI thread, then recognise its text if it is a
    /// picture with none remembered.
    fn start_decode(&self, path: PathBuf, panel: Rect, scale: f32) {
        let viewer = Arc::clone(&self.viewer);
        tokio::task::spawn_blocking(move || {
            let (name, generation) = {
                let mut viewer = viewer.lock().unwrap();
                viewer.stamp = Stamp::of(&path);
                (viewer.name.clone(), viewer.generation)
            };
            let mut preview = peek::decode_document(&path, panel, scale, 1);
            let video = (path.is_file() && otto_media_kit::player::available())
                .then(|| peek::video_options(panel, scale, true));

            // Words already remembered ride in with the picture; otherwise
            // the picture goes up now and the recogniser follows.
            let recogniser = recogniser();
            let picture = recogniser.is_some()
                && otto_files::command::is_picture_name(&name)
                && matches!(&preview, Preview::Pixels { pixels, .. } if pixels.words.is_empty());
            let mut needs_recognising = false;
            if picture {
                match peek::remembered_words(&path, panel, scale, 1) {
                    Some(words) => {
                        if let Preview::Pixels { pixels, .. } = &mut preview {
                            pixels.words = words;
                        }
                    }
                    None => {
                        needs_recognising =
                            recogniser.as_deref().is_some_and(otto_peek::ocr::available);
                    }
                }
            }

            {
                let mut viewer = viewer.lock().unwrap();
                let mut session = Session::new(preview, name, Rect::new_empty(), Instant::now());
                session.dir = path.parent().map(Path::to_path_buf);
                // Only once the decoder has said what the file is: the player
                // is started on the sniffed type, never on the name.
                if let Some(options) = video {
                    session.attach_video(&path, options, AppContext::request_wakeup);
                }
                if needs_recognising {
                    session.start_recognising(Instant::now());
                }
                if !viewer.land(generation, session) {
                    return;
                }
            }
            AppContext::request_wakeup();

            let Some(command) = recogniser.filter(|_| needs_recognising) else {
                return;
            };
            let words = peek::recognise(
                &path,
                panel,
                scale,
                1,
                &command,
                peek::Priority::Interactive,
            );
            {
                let mut viewer = viewer.lock().unwrap();
                if viewer.generation != generation {
                    return;
                }
                match words {
                    Some(words) => viewer.session.attach_words(words),
                    None => viewer.session.stop_recognising(),
                }
                viewer.dirty = true;
            }
            AppContext::request_wakeup();
        });
    }

    /// Fill in an open document: the pages scrolled into view and its text
    /// layer, both off the UI thread.
    fn follow_document(&self) {
        let scale = AppContext::scale_factor().max(1) as f32;
        // The sidebar's thumbnails, small decodes of their own pages.
        let (thumbs, generation) = {
            let mut viewer = self.viewer.lock().unwrap();
            (viewer.thumb_work(scale), viewer.generation)
        };
        if let Some((requests, path)) = thumbs {
            for request in requests {
                let viewer = Arc::clone(&self.viewer);
                let path = path.clone();
                tokio::task::spawn_blocking(move || {
                    let image = match peek::decode_page(&path, request.page, request.width) {
                        Preview::Pixels { pixels, .. } => pixels.to_image(),
                        _ => None,
                    };
                    viewer
                        .lock()
                        .unwrap()
                        .finish_thumb(generation, request.page, image);
                    AppContext::request_wakeup();
                });
            }
        }
        let work = {
            let mut viewer = self.viewer.lock().unwrap();
            viewer
                .document_work(scale)
                .map(|work| (work, viewer.path.clone(), viewer.decode_panel))
        };
        let Some(((pages, text), path, panel)) = work else {
            return;
        };
        for request in pages {
            let viewer = Arc::clone(&self.viewer);
            let path = path.clone();
            tokio::task::spawn_blocking(move || {
                let pixels = match peek::decode_page(&path, request.page, request.width) {
                    Preview::Pixels { pixels, .. } => Some(pixels),
                    _ => None,
                };
                viewer
                    .lock()
                    .unwrap()
                    .finish_page(generation, request.page, pixels);
                AppContext::request_wakeup();
            });
        }
        if text {
            let viewer = Arc::clone(&self.viewer);
            tokio::task::spawn_blocking(move || {
                let Some((measured, words)) = peek::text_layer(&path, panel, scale) else {
                    return;
                };
                let mut viewer = viewer.lock().unwrap();
                if viewer.generation != generation {
                    return;
                }
                if viewer.session.attach_text_layer(measured, words) {
                    viewer.dirty = true;
                }
                drop(viewer);
                AppContext::request_wakeup();
            });
        }
    }
}

/// The recogniser's command line, when recognition is on: the one configured
/// in `files.toml`'s `[peek]` section, else the default.
fn recogniser() -> Option<String> {
    let config = otto_files::places_config::peek();
    if !config.recognise_text {
        return None;
    }
    let configured = config.recogniser.trim();
    Some(if configured.is_empty() {
        otto_peek::ocr::DEFAULT_COMMAND.to_string()
    } else {
        configured.to_string()
    })
}

/// Hand `target` to the desktop's opener, detached, so whatever it starts
/// outlives this window.
fn xdg_open(target: &OsStr) {
    let spawned = Command::new("xdg-open")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match spawned {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(err) => tracing::warn!(%err, "could not run xdg-open"),
    }
}

/// The ground under the chrome and the content: Peek's, solid.
///
/// [`otto_kit::preview::background`] is the card material, which is
/// translucent wherever the compositor frosts what lies under a card. Peek
/// floats over a blurred window and wants that; this window is opaque, and a
/// translucent ground lets whatever is behind it show through the margins
/// around a picture.
fn ground(theme: &Theme) -> Color {
    let mut solid = theme.clone();
    solid.with_solid_materials(theme.is_dark());
    let color = otto_kit::preview::background(&solid);
    Color::from_rgb(color.r(), color.g(), color.b())
}

/// Tell the compositor the window is opaque everywhere but its rounded
/// corners.
fn apply_opaque_region(window: &Window, (width, height): (f32, f32)) {
    let radius = window.frame_corner_radius();
    window.set_opaque_region(&[
        Rect::from_ltrb(0.0, radius, width, height - radius),
        Rect::from_ltrb(radius, 0.0, width - radius, height),
    ]);
}

/// The first seat, which every press this window acts on came from.
fn seat() -> Option<wayland_client::protocol::wl_seat::WlSeat> {
    AppContext::seat_state().seats().next()
}

/// Whether the colour scheme is dark.
fn dark() -> bool {
    otto_kit::color_scheme::current_color_scheme() == otto_kit::theme::ColorScheme::Dark
}

/// Everything the pointer does over the window.
fn handle_pointer(
    viewer: &Mutex<Viewer>,
    chat: &RefCell<Chat>,
    window: &Window,
    events: &[PointerEvent],
) {
    let mut redraw = false;
    for event in events {
        let at = Point::new(event.position.0 as f32, event.position.1 as f32);
        let mut v = viewer.lock().unwrap();
        let panel = v.chat_rect();
        let in_chat = panel.is_some_and(|panel| panel.contains(at));
        let (width, height) = v.size;
        let content = v.content();
        let edge = (!window.is_maximized())
            .then(|| resize::edge_at(Rect::from_wh(width, height), at.x, at.y))
            .flatten();
        match &event.kind {
            PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                v.pointer = Some(at);
                if v.chat_resizing {
                    v.resize_chat_to(at.x);
                    redraw |= std::mem::take(&mut v.dirty);
                    continue;
                }
                let control = chrome::control_at(&v, at.x, at.y);
                v.dirty |= v.controls.on_motion(control);
                let tool = v
                    .toolbar()
                    .tool_at(at.x, at.y)
                    .filter(|tool| v.tool_enabled(*tool));
                if tool != v.hovered_tool {
                    v.hovered_tool = tool;
                    v.dirty = true;
                }
                // Always forwarded: a selection or a bar dragged past the
                // content's edge keeps going.
                if v.mark_busy() {
                    v.mark_motion(at);
                } else {
                    v.mark_hover(content.contains(at).then_some(at));
                }
                v.content_pointer(VideoPointer::Motion, at);
                v.sidebar_motion(at);
                let mut chat_cursor = None;
                if let Some(panel) = panel {
                    let (repaint, cursor) = chat.borrow_mut().motion(panel, in_chat.then_some(at));
                    v.dirty |= repaint;
                    chat_cursor = in_chat.then_some(cursor);
                }
                let shape = match edge {
                    Some(edge) if v.drag.is_none() => edge.cursor(),
                    _ if v.drag.is_none() && v.on_chat_edge(at) => CursorShape::ColResize,
                    _ if content.contains(at) || v.drag.is_some() => v.content_cursor(at),
                    _ => chat_cursor.unwrap_or(CursorShape::Default),
                };
                if shape != v.cursor {
                    v.cursor = shape;
                    crate::cursors::show(shape);
                }
            }
            PointerEventKind::Press {
                button,
                serial,
                time,
            } => {
                if *button != BTN_LEFT {
                    continue;
                }
                if let Some(edge) = edge {
                    if let Some(seat) = seat() {
                        window.start_resize(&seat, *serial, edge);
                    }
                    continue;
                }
                if v.on_chat_edge(at) {
                    v.chat_resizing = true;
                    continue;
                }
                let control = chrome::control_at(&v, at.x, at.y);
                if v.controls.on_press(control) {
                    v.dirty = true;
                    continue;
                }
                if at.y < content.top {
                    // A toolbar button arms on the press and fires on the
                    // release; a disabled one takes the press and does
                    // nothing. Anywhere else in the chrome moves the window.
                    match v.toolbar().tool_at(at.x, at.y) {
                        Some(tool) if v.tool_enabled(tool) => {
                            v.pressed_tool = Some(tool);
                            v.dirty = true;
                        }
                        Some(_) => {}
                        None => {
                            drop(v);
                            if let Some(seat) = seat() {
                                window.titlebar_press(&seat, *serial, at.x, at.y);
                            }
                        }
                    }
                    continue;
                }
                if let Some(panel) = panel.filter(|_| in_chat) {
                    v.dirty |= chat.borrow_mut().press(panel, at, *time, *serial);
                    continue;
                }
                // A press on the document gives it the keyboard back.
                v.dirty |= std::mem::replace(&mut chat.borrow_mut().focused, false);
                if v.sidebar().is_some_and(|sidebar| sidebar.rect.contains(at)) {
                    v.sidebar_press(at);
                    continue;
                }
                // A click on a mark's badge deletes the mark.
                if content.contains(at)
                    && (v.mark_delete_at(at) || v.mark_grab(at) || v.mark_press(at))
                {
                    continue;
                }
                v.content_pointer(VideoPointer::Press, at);
                if v.drag.is_some() && v.cursor != CursorShape::Grabbing {
                    v.cursor = CursorShape::Grabbing;
                    AppContext::set_cursor_shape(CursorShape::Grabbing);
                }
            }
            PointerEventKind::Release { button, .. } => {
                if *button != BTN_LEFT {
                    continue;
                }
                if std::mem::take(&mut v.chat_resizing) {
                    // New windows open their panel at this width.
                    crate::chat::remember_width(v.chat_w);
                    continue;
                }
                let control = chrome::control_at(&v, at.x, at.y);
                let armed = v.controls.pressed().is_some();
                let fired = v.controls.on_release(control);
                v.dirty |= armed;
                match fired {
                    Some(WindowControl::Close) => {
                        v.closing = true;
                        AppContext::request_wakeup();
                    }
                    Some(WindowControl::Minimize) => window.minimize(),
                    Some(WindowControl::Zoom) => window.toggle_maximized(),
                    None => {}
                }
                if let Some(tool) = v.pressed_tool.take() {
                    v.dirty = true;
                    if v.toolbar().tool_at(at.x, at.y) == Some(tool) {
                        if tool == Tool::Chat {
                            let resized = v.toggle_chat();
                            chat_toggled(&v, chat, window, resized);
                        } else {
                            v.run_tool(tool);
                        }
                    }
                }
                if let Some(panel) = panel {
                    v.dirty |= chat.borrow_mut().release(panel, at);
                }
                v.sidebar_release();
                if v.mark_busy() {
                    v.mark_release();
                }
                let link = v.content_pointer(VideoPointer::Release, at);
                if v.cursor == CursorShape::Grabbing {
                    v.cursor = v.content_cursor(at);
                    crate::cursors::show(v.cursor);
                }
                if let Some(link) = link {
                    xdg_open(&link);
                }
            }
            PointerEventKind::Leave { .. } => {
                v.pointer = None;
                v.dirty |= v.controls.on_leave();
                if v.hovered_tool.take().is_some() | v.pressed_tool.take().is_some() {
                    v.dirty = true;
                }
                v.content_pointer(VideoPointer::Leave, at);
                v.mark_hover(None);
                v.sidebar_scroll.on_pointer_leave();
                v.dirty |= chat.borrow_mut().leave();
                v.cursor = CursorShape::Default;
            }
            PointerEventKind::Axis {
                horizontal,
                vertical,
                ..
            } => {
                let stop = vertical.stop || horizontal.stop;
                let discrete = vertical.discrete != 0 || horizontal.discrete != 0;
                if in_chat {
                    v.dirty |= chat
                        .borrow_mut()
                        .wheel(vertical.absolute as f32, discrete, stop);
                    redraw |= std::mem::take(&mut v.dirty);
                    continue;
                }
                if v.wheel_goes_to_sidebar(at, stop, discrete) {
                    v.sidebar_wheel(vertical.absolute as f32, stop, discrete);
                    redraw |= std::mem::take(&mut v.dirty);
                    continue;
                }
                v.wheel(
                    horizontal.absolute as f32,
                    vertical.absolute as f32,
                    vertical.stop || horizontal.stop,
                    vertical.discrete != 0 || horizontal.discrete != 0,
                    at,
                );
            }
        }
        redraw |= std::mem::take(&mut v.dirty);
    }
    if redraw {
        paint_chat(&mut viewer.lock().unwrap(), &mut chat.borrow_mut());
        window.request_frame();
    }
}

/// Paint the chat, when it shows, into the picture the draw puts beside the
/// document. Called before every repaint the chat may have changed for.
fn paint_chat(viewer: &mut Viewer, chat: &mut Chat) {
    chat.pending_marks = viewer.marks.pending();
    viewer.chat_picture = viewer.chat_rect().and_then(|panel| {
        let mut recorder = otto_kit::skia::PictureRecorder::new();
        let canvas = recorder.begin_recording(panel, false);
        chat.draw(canvas, panel, &AppContext::current_theme());
        recorder.finish_recording_as_picture(None)
    });
}

/// The chat was just shown or hidden: a panel shown takes the keyboard, and
/// connects the first time.
fn chat_toggled(
    viewer: &Viewer,
    chat: &RefCell<Chat>,
    window: &Window,
    resized: Option<(f32, f32)>,
) {
    if let Some((width, height)) = resized {
        window.resize(width as i32, height as i32);
        apply_opaque_region(window, (width, height));
    }
    let mut chat = chat.borrow_mut();
    if viewer.chat_open {
        chat.opened();
    } else {
        chat.focused = false;
    }
}

impl Doc {
    /// Answer a configure for this window.
    fn configure(&mut self, configure: WindowConfigure) {
        // The first configure leaves the size to the window unless the window
        // is tiled or maximized, and carries the room a new window has. The
        // opening size is fitted to that room: the size the window was
        // created at only knew the fixed limit.
        let first = !std::mem::replace(&mut self.sized, true);
        let fitted = match (first, configure.new_size, configure.suggested_bounds) {
            (true, (None, None), Some((width, height))) => {
                let (width, height) = self.shape.opening_size(Some((width as f32, height as f32)));
                self.window.resize(width as i32, height as i32);
                Some((width, height))
            }
            _ => None,
        };
        let size = {
            let mut viewer = self.viewer.lock().unwrap();
            if let (Some(width), Some(height)) = configure.new_size {
                viewer.size = (width.get() as f32, height.get() as f32);
            } else if let Some(size) = fitted {
                viewer.size = size;
            }
            viewer.variant = self.window.decoration_variant();
            viewer.active = self.window.is_activated();
            viewer.floating = !self.window.is_maximized()
                && viewer.variant == otto_kit::components::titlebar::DecorationVariant::Floating;
            if let Some((width, height)) = configure.suggested_bounds {
                if width > 0 && height > 0 {
                    viewer.room = Some((width as f32, height as f32));
                }
            }
            viewer.dirty = true;
            viewer.size
        };
        self.window.sync_frame_corners();
        apply_opaque_region(&self.window, size);
        if std::mem::take(&mut self.chat_on_configure) {
            self.show_chat();
        }
        self.redraw();
    }

    /// Show the chat, if it isn't showing.
    fn show_chat(&self) {
        let mut viewer = self.viewer.lock().unwrap();
        if viewer.chat_open {
            return;
        }
        let resized = viewer.toggle_chat();
        chat_toggled(&viewer, &self.chat, &self.window, resized);
        drop(viewer);
        self.redraw();
    }

    /// One turn of the loop: start the decode once the window is
    /// configured, follow the document, and step whatever moves.
    fn update(&self) {
        self.follow_file();
        // The decode waits for the first configure, by which time the window
        // knows its real size and the output's scale.
        let start = {
            let mut viewer = self.viewer.lock().unwrap();
            (!viewer.started && self.window.is_configured()).then(|| {
                viewer.started = true;
                viewer.decode_panel = viewer.content();
                (viewer.path.clone(), viewer.decode_panel)
            })
        };
        if let Some((path, panel)) = start {
            let scale = AppContext::scale_factor().max(1) as f32;
            self.start_decode(path, panel, scale);
        }
        self.follow_document();
        if self.viewer.lock().unwrap().tick() {
            self.window.request_frame();
        }
    }
}

impl Doc {
    /// The marks and the agent session agree: a session carried on here
    /// brings back the marks it kept, and marks changed since are kept with
    /// it, once a stroke or a drag is over.
    fn sync_marks(&self, chat: &mut Chat) {
        let mut viewer = self.viewer.lock().unwrap();
        if let Some(kept) = chat.restored_marks() {
            viewer.marks.restore(&kept);
            chat.marks_restored(&viewer.marks);
            viewer.dirty = true;
            drop(viewer);
            self.window.request_frame();
            return;
        }
        if !viewer.mark_busy() {
            chat.keep_marks(&viewer.marks);
        }
    }

    /// Show the file again when its folder has changed and the file with
    /// it. A file that has gone keeps showing what it was.
    fn follow_file(&self) {
        let Some(watch) = &self.watch else {
            return;
        };
        if watch.take() != Some(Change::Modified) {
            return;
        }
        let mut viewer = self.viewer.lock().unwrap();
        let now = Stamp::of(&viewer.path);
        if now.is_none() || now == viewer.stamp {
            return;
        }
        tracing::debug!(path = %viewer.path.display(), "the file changed; reloading");
        viewer.reload();
        viewer.track_version();
        drop(viewer);
        self.window.request_frame();
    }
}

impl App for PreviewApp {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        match self.first.take() {
            Some(first) => self.open(first),
            None => Ok(()),
        }
    }

    fn on_configure(&mut self, _ctx: &AppContext, configure: WindowConfigure, _serial: u32) {
        let Some((id, ..)) = AppContext::current_surface_configure() else {
            return;
        };
        if let Some(doc) = self
            .docs
            .iter_mut()
            .find(|doc| doc.window.surface_id().as_ref() == Some(&id))
        {
            doc.configure(configure);
        }
    }

    fn on_modifiers(&mut self, _ctx: &AppContext, modifiers: Modifiers) {
        for doc in &self.docs {
            doc.viewer.lock().unwrap().modifiers = modifiers;
        }
    }

    fn on_key_event(
        &mut self,
        _ctx: &AppContext,
        event: &KeyEvent,
        state: wl_keyboard::KeyState,
        serial: u32,
    ) {
        if state != wl_keyboard::KeyState::Pressed {
            return;
        }
        let Some(doc) = self.focused() else {
            return;
        };
        {
            let mut viewer = doc.viewer.lock().unwrap();
            let modifiers = viewer.modifiers;
            // Ctrl+K shows or hides the chat, from anywhere.
            if modifiers.ctrl && matches!(event.keysym, Keysym::k | Keysym::K) {
                let resized = viewer.toggle_chat();
                chat_toggled(&viewer, &doc.chat, &doc.window, resized);
                drop(viewer);
                doc.redraw();
                return;
            }
            if viewer.chat_open {
                let mut chat = doc.chat.borrow_mut();
                if chat.focused {
                    // The marks not yet sent go with this message, as files
                    // the agent reads and the chat doesn't list.
                    if chat.sends(event) {
                        // The file as it is before the agent hears of it, so
                        // there is always a version to come back to.
                        viewer.keep_version(None);
                        let dir = crate::marks::dir();
                        let (path, files) = {
                            let viewer = &mut *viewer;
                            let files =
                                viewer
                                    .marks
                                    .export(&viewer.path, &viewer.session.preview, &dir);
                            (viewer.path.clone(), files)
                        };
                        if !files.is_empty() {
                            tracing::debug!(path = %path.display(), count = files.len(), "marks go with the message");
                            viewer.dirty = true;
                        }
                        chat.attach_unlisted(files);
                    }
                    let handled = chat.key(event, modifiers, serial);
                    drop(chat);
                    drop(viewer);
                    if handled {
                        doc.redraw();
                        return;
                    }
                } else if modifiers.ctrl
                    && matches!(event.keysym, Keysym::c | Keysym::C)
                    && chat.copy(serial)
                {
                    // Text selected in the log, with the document focused.
                    return;
                }
            }
        }
        let outcome = doc.viewer.lock().unwrap().key(event.keysym);
        match outcome {
            KeyOutcome::Close => {
                doc.viewer.lock().unwrap().closing = true;
                AppContext::request_wakeup();
            }
            KeyOutcome::Copy(text) => {
                otto_kit::clipboard::set_text(&text, serial);
            }
            KeyOutcome::Handled | KeyOutcome::Ignored => {}
        }
        if std::mem::take(&mut doc.viewer.lock().unwrap().dirty) {
            doc.redraw();
        }
    }

    fn on_theme_changed(&mut self, _ctx: &AppContext) {
        for doc in &mut self.docs {
            doc.window
                .set_background(ground(&AppContext::current_theme()));
            doc.chat.borrow_mut().set_dark(dark());
            doc.redraw();
        }
    }

    fn on_pointer_pinch_begin(&mut self, _ctx: &AppContext, fingers: u32) {
        let Some(doc) = self.hovered() else {
            return;
        };
        let mut viewer = doc.viewer.lock().unwrap();
        viewer.pinch_base =
            (fingers == 2 && viewer.zoomable()).then_some(viewer.session.zoom.scale);
    }

    fn on_pointer_pinch_update(
        &mut self,
        _ctx: &AppContext,
        dx: f64,
        dy: f64,
        scale: f64,
        _rotation: f64,
    ) {
        let Some(doc) = self.hovered() else {
            return;
        };
        let mut viewer = doc.viewer.lock().unwrap();
        let Some(base) = viewer.pinch_base else {
            return;
        };
        // The pinch reports only how far its focal point has drifted, so it
        // zooms about where the pointer last was.
        let content = viewer.content();
        let origin = viewer
            .pointer
            .unwrap_or_else(|| Point::new(content.center_x(), content.center_y()));
        let focus = (origin.x + dx as f32, origin.y + dy as f32);
        let moved = viewer.zoom_about(base * scale as f32, focus);
        drop(viewer);
        if moved {
            doc.redraw();
        }
    }

    fn on_pointer_pinch_end(&mut self, _ctx: &AppContext, _cancelled: bool) {
        for doc in &self.docs {
            doc.viewer.lock().unwrap().pinch_base = None;
        }
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        let requests = std::mem::take(&mut *self.inbox.lock().unwrap());
        for request in requests {
            if let Err(err) = self.open(request) {
                tracing::warn!(%err, "could not open a window");
            }
        }
        let documents = &self.documents;
        self.docs.retain(|doc| {
            let closing = doc.viewer.lock().unwrap().closing;
            if closing {
                documents.lock().unwrap().remove(&doc.key);
                doc.window.close();
            }
            !closing
        });
        // The last window closed is the application closed; the bus name
        // goes with the process, and the next start owns it.
        if self.docs.is_empty() {
            std::process::exit(0);
        }
        let now = Instant::now();
        let delta = now.duration_since(self.last_update).as_secs_f32();
        self.last_update = now;
        for doc in &self.docs {
            doc.update();
            let mut chat = doc.chat.borrow_mut();
            doc.sync_marks(&mut chat);
            if chat.pump() | chat.tick(delta) {
                drop(chat);
                doc.redraw();
            }
        }
    }

    fn idle_timeout(&self) -> Option<Duration> {
        if self
            .docs
            .iter()
            .any(|doc| doc.viewer.lock().unwrap().animating() || doc.chat.borrow().scrolling())
        {
            return Some(FRAME);
        }
        self.docs
            .iter()
            .any(|doc| doc.viewer.lock().unwrap().chat_open && doc.chat.borrow().focused)
            .then_some(BLINK)
    }

    fn poll_fds(&self) -> Vec<std::os::fd::RawFd> {
        self.docs
            .iter()
            .filter_map(|doc| doc.chat.borrow().poll_fd())
            .collect()
    }
}
