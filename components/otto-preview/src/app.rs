//! The window: its setup, its input, and the work it sends off the UI thread.
//!
//! The decode, the page rasteriser, the text layer and the recogniser all
//! block, so each runs on a blocking task and lands in the shared [`Viewer`]
//! under its lock, then wakes the loop. A window showing a still picture is
//! not committing frames, so without the wake nothing would notice.

// Rust guideline compliant 2026-02-21

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use otto_files::peek::{self, Session, VideoPointer};
use otto_kit::components::titlebar::WindowControl;
use otto_kit::components::window::resize;
use otto_kit::prelude::*;
use otto_kit::preview::Preview;
use otto_kit::skia::{Contains, Point};
use otto_kit::CursorShape;
use smithay_client_toolkit::seat::keyboard::KeyEvent;
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind};
use smithay_client_toolkit::shell::xdg::window::WindowConfigure;
use wayland_client::protocol::wl_keyboard;

use crate::chrome;
use crate::viewer::{KeyOutcome, Viewer};
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

/// The application: one window on one file.
pub struct PreviewApp {
    viewer: Arc<Mutex<Viewer>>,
    window: Option<Window>,
    /// What the opening size is fitted to.
    shape: Shape,
    /// Whether the first configure has been answered, which is when the
    /// window takes its opening size.
    sized: bool,
}

impl PreviewApp {
    /// An application that will open `path` in a window sized for `shape`.
    pub fn new(path: PathBuf, shape: Shape) -> Self {
        Self {
            viewer: Arc::new(Mutex::new(Viewer::new(path, shape.opening_size(None)))),
            window: None,
            shape,
            sized: false,
        }
    }

    fn redraw(&self) {
        if let Some(window) = &self.window {
            window.request_frame();
        }
    }

    /// Decode the file off the UI thread, then recognise its text if it is a
    /// picture with none remembered.
    fn start_decode(&self, path: PathBuf, panel: Rect, scale: f32) {
        let viewer = Arc::clone(&self.viewer);
        tokio::task::spawn_blocking(move || {
            let name = viewer.lock().unwrap().name.clone();
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
                viewer.session = session;
                viewer.dirty = true;
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
                viewer.lock().unwrap().finish_page(request.page, pixels);
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

/// Everything the pointer does over the window.
fn handle_pointer(viewer: &Mutex<Viewer>, window: &Window, events: &[PointerEvent]) {
    let mut redraw = false;
    for event in events {
        let at = Point::new(event.position.0 as f32, event.position.1 as f32);
        let mut v = viewer.lock().unwrap();
        let (width, height) = v.size;
        let content = v.content();
        let edge = (!window.is_maximized())
            .then(|| resize::edge_at(Rect::from_wh(width, height), at.x, at.y))
            .flatten();
        match &event.kind {
            PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                v.pointer = Some(at);
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
                v.content_pointer(VideoPointer::Motion, at);
                let shape = match edge {
                    Some(edge) if v.drag.is_none() => edge.cursor(),
                    _ if content.contains(at) || v.drag.is_some() => v.content_cursor(at),
                    _ => CursorShape::Default,
                };
                if shape != v.cursor {
                    v.cursor = shape;
                    AppContext::set_cursor_shape(shape);
                }
            }
            PointerEventKind::Press { button, serial, .. } => {
                if *button != BTN_LEFT {
                    continue;
                }
                if let Some(edge) = edge {
                    if let Some(seat) = seat() {
                        window.start_resize(&seat, *serial, edge);
                    }
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
                let control = chrome::control_at(&v, at.x, at.y);
                let armed = v.controls.pressed().is_some();
                let fired = v.controls.on_release(control);
                v.dirty |= armed;
                match fired {
                    Some(WindowControl::Close) => std::process::exit(0),
                    Some(WindowControl::Minimize) => window.minimize(),
                    Some(WindowControl::Zoom) => window.toggle_maximized(),
                    None => {}
                }
                if let Some(tool) = v.pressed_tool.take() {
                    v.dirty = true;
                    if v.toolbar().tool_at(at.x, at.y) == Some(tool) {
                        v.run_tool(tool);
                    }
                }
                let link = v.content_pointer(VideoPointer::Release, at);
                if v.cursor == CursorShape::Grabbing {
                    v.cursor = v.content_cursor(at);
                    AppContext::set_cursor_shape(v.cursor);
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
                v.cursor = CursorShape::Default;
            }
            PointerEventKind::Axis {
                horizontal,
                vertical,
                ..
            } => {
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
        window.request_frame();
    }
}

impl App for PreviewApp {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let (name, (width, height)) = {
            let viewer = self.viewer.lock().unwrap();
            (viewer.name.clone(), viewer.size)
        };
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

        let viewer = Arc::clone(&self.viewer);
        window.on_draw(move |canvas| {
            let theme = AppContext::current_theme();
            let viewer = viewer.lock().unwrap();
            canvas.clear(ground(&theme));
            crate::content::draw(canvas, &viewer, &theme);
            chrome::draw(canvas, &viewer, &theme);
        });

        let viewer = Arc::clone(&self.viewer);
        let handle = window.clone();
        window.on_pointer_event(move |events| handle_pointer(&viewer, &handle, events));

        AppContext::register_window(window.clone());
        self.window = Some(window);
        Ok(())
    }

    fn on_configure(&mut self, _ctx: &AppContext, configure: WindowConfigure, _serial: u32) {
        let Some(window) = &self.window else {
            return;
        };
        // The first configure leaves the size to the window unless the window
        // is tiled or maximized, and carries the room a new window has. The
        // opening size is fitted to that room: the size the window was
        // created at only knew the fixed limit.
        let first = !std::mem::replace(&mut self.sized, true);
        let fitted = match (first, configure.new_size, configure.suggested_bounds) {
            (true, (None, None), Some((width, height))) => {
                let (width, height) = self.shape.opening_size(Some((width as f32, height as f32)));
                window.resize(width as i32, height as i32);
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
            viewer.variant = window.decoration_variant();
            viewer.active = window.is_activated();
            viewer.dirty = true;
            viewer.size
        };
        window.sync_frame_corners();
        apply_opaque_region(window, size);
        window.request_frame();
    }

    fn on_modifiers(&mut self, _ctx: &AppContext, modifiers: Modifiers) {
        self.viewer.lock().unwrap().modifiers = modifiers;
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
        let outcome = self.viewer.lock().unwrap().key(event.keysym);
        match outcome {
            KeyOutcome::Close => std::process::exit(0),
            KeyOutcome::Copy(text) => {
                otto_kit::clipboard::set_text(&text, serial);
            }
            KeyOutcome::Handled | KeyOutcome::Ignored => {}
        }
        if std::mem::take(&mut self.viewer.lock().unwrap().dirty) {
            self.redraw();
        }
    }

    fn on_theme_changed(&mut self, _ctx: &AppContext) {
        if let Some(window) = &mut self.window {
            window.set_background(ground(&AppContext::current_theme()));
        }
        self.redraw();
    }

    fn on_pointer_pinch_begin(&mut self, _ctx: &AppContext, fingers: u32) {
        let mut viewer = self.viewer.lock().unwrap();
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
        let mut viewer = self.viewer.lock().unwrap();
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
            self.redraw();
        }
    }

    fn on_pointer_pinch_end(&mut self, _ctx: &AppContext, _cancelled: bool) {
        self.viewer.lock().unwrap().pinch_base = None;
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        let Some(window) = &self.window else {
            return;
        };
        // The decode waits for the first configure, by which time the window
        // knows its real size and the output's scale.
        let start = {
            let mut viewer = self.viewer.lock().unwrap();
            (!viewer.started && window.is_configured()).then(|| {
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
            window.request_frame();
        }
    }

    fn idle_timeout(&self) -> Option<Duration> {
        self.viewer.lock().unwrap().animating().then_some(FRAME)
    }
}
