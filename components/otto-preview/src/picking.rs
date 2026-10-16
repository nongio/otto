//! Studio started with no file: an empty window, and the desktop's open
//! dialog over it.
//!
//! The window comes first so the dialog has a parent. The picker is a Files
//! window served through the portal; parented to ours it is Studio's dialog
//! (the bar names Studio, the dock shows no second app) and it centres over
//! us. The window is exported with xdg-foreign and its handle goes out as
//! the request's `parent_window`, once the compositor has sent it.
//!
//! The bus name is not claimed until a file is chosen: a Studio already
//! running then gets the file over the bus, as a start naming one would
//! give it, and this process leaves.

// Rust guideline compliant 2026-02-21

use std::sync::{Arc, Mutex};

use otto_kit::components::label::Label;
use otto_kit::components::titlebar::{DecorationVariant, WindowControl, WindowDecoration};
use otto_kit::portal_file_chooser::Outcome;
use otto_kit::prelude::*;
use otto_kit::typography::styles;
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind};
use smithay_client_toolkit::shell::xdg::window::WindowConfigure;

use crate::file_dialog;
use crate::instance::{self, Documents, Inbox, Request, Role};

/// The empty window's size, in points: a document's, which is what most
/// often follows.
const SIZE: (f32, f32) = crate::DOCUMENT_SIZE;

/// What the dialog, and the bus after it, came back with.
pub enum Picked {
    /// A file to open here: this process owns the bus name, or has no bus.
    Open(Request, Option<zbus::Connection>),
    /// A Studio already running was handed the file; nothing more to do.
    Forwarded,
    /// No file: the person cancelled, or no dialog could be shown.
    Nothing,
}

/// The empty window while the dialog is up.
pub struct Picking {
    window: Window,
    /// The window as last configured, shared with its draw.
    size: Arc<Mutex<Frame>>,
    /// The window's handle for the picker; `None` on a compositor without
    /// xdg-foreign, when the dialog is asked without a parent.
    exported: Option<otto_kit::foreign::Exported>,
    /// Whether the dialog has been asked, which waits on the handle.
    asked: bool,
    /// The chat and session the start asked for, carried to the file.
    chat: bool,
    session: Option<String>,
    /// The answer, written by the dialog's thread.
    answer: Arc<Mutex<Option<Picked>>>,
    /// Whether the window was closed under the dialog.
    closed: Arc<std::sync::atomic::AtomicBool>,
    inbox: Inbox,
    documents: Documents,
}

impl Picking {
    /// An empty Studio window, named for the picker.
    pub fn open(
        chat: bool,
        session: Option<String>,
        inbox: Inbox,
        documents: Documents,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // What the desktop entry calls the program, in every language.
        let mut window = Window::new("Studio", SIZE.0 as i32, SIZE.1 as i32)?;
        let size = Arc::new(Mutex::new(Frame {
            width: SIZE.0,
            height: SIZE.1,
            variant: window.decoration_variant(),
            active: false,
            dark: AppContext::current_theme().is_dark(),
        }));
        window.set_min_size(crate::app::MIN_W as u32, crate::app::MIN_H as u32);
        window.set_background(crate::app::ground(&AppContext::current_theme()));
        if let Some(surface) = window.surface() {
            surface
                .xdg_window()
                .set_app_id(crate::app::APP_ID.to_string());
        }
        window.set_frame_corner_radius(crate::app::CORNER);
        let drawn = Arc::clone(&size);
        window.on_draw(move |canvas| {
            let theme = AppContext::current_theme();
            canvas.clear(crate::app::ground(&theme));
            let frame = *drawn.lock().unwrap();
            let (width, height) = (frame.width, frame.height);
            frame.decoration().draw(canvas);
            Label::new(otto_kit::t_owned!("studio-pick-hint"))
                .with_style(styles::BODY)
                .with_color(theme.text_tertiary)
                .centered_at(width / 2.0, height / 2.0)
                .render(canvas);
        });
        let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        // The titlebar: its lights, and a drag anywhere else on it.
        let pressed = Arc::clone(&closed);
        let pointed = Arc::clone(&size);
        let handle = window.clone();
        window.on_pointer_event(move |events: &[PointerEvent]| {
            for event in events {
                let PointerEventKind::Press { serial, .. } = event.kind else {
                    continue;
                };
                let (x, y) = (event.position.0 as f32, event.position.1 as f32);
                let frame = *pointed.lock().unwrap();
                if y >= WindowDecoration::height_for(frame.variant) {
                    continue;
                }
                match frame.decoration().control_at(x, y) {
                    Some(WindowControl::Close) => {
                        pressed.store(true, std::sync::atomic::Ordering::Relaxed);
                        AppContext::request_wakeup();
                    }
                    Some(WindowControl::Minimize) => handle.minimize(),
                    Some(WindowControl::Zoom) => handle.toggle_maximized(),
                    None => {
                        if let Some(seat) = AppContext::seat_state().seats().next() {
                            handle.titlebar_press(&seat, serial, x, y);
                        }
                    }
                }
            }
        });
        let on_close = Arc::clone(&closed);
        window.on_close_request(move || {
            on_close.store(true, std::sync::atomic::Ordering::Relaxed);
            AppContext::request_wakeup();
        });
        AppContext::register_window(window.clone());
        let exported = window.export();
        Ok(Self {
            window,
            size,
            exported,
            asked: false,
            chat,
            session,
            answer: Arc::new(Mutex::new(None)),
            closed,
            inbox,
            documents,
        })
    }

    /// This window's surface, to tell its configures from the documents'.
    pub fn surface_id(&self) -> Option<wayland_client::backend::ObjectId> {
        self.window.surface_id()
    }

    /// Follow the size the compositor gives the window.
    pub fn configure(&self, configure: WindowConfigure) {
        {
            let mut frame = self.size.lock().unwrap();
            if let (Some(width), Some(height)) = configure.new_size {
                (frame.width, frame.height) = (width.get() as f32, height.get() as f32);
            }
            frame.variant = self.window.decoration_variant();
            frame.active = self.window.is_activated();
            frame.dark = AppContext::current_theme().is_dark();
        }
        self.window.sync_frame_corners();
        self.window.request_frame();
    }

    /// Ask the dialog once the window's handle is known, on a thread of its
    /// own: the call blocks for as long as the dialog is up.
    fn ask(&mut self) {
        if self.asked {
            return;
        }
        let parent = match &self.exported {
            Some(exported) => match exported.parent_window() {
                Some(handle) => handle,
                None => return,
            },
            None => String::new(),
        };
        self.asked = true;
        let answer = Arc::clone(&self.answer);
        let (chat, session) = (self.chat, self.session.clone());
        let (inbox, documents) = (self.inbox.clone(), self.documents.clone());
        let runtime = tokio::runtime::Handle::current();
        let title = otto_kit::t_owned!("studio-open-title");
        let fallback = Arc::clone(&answer);
        // The kit wakes the run loop once the answer is in, whatever it is.
        let spawned = file_dialog::open(&title, &parent).spawn(move |outcome| {
            let path = match outcome {
                Outcome::Chosen(paths) => paths.into_iter().next(),
                Outcome::Dismissed => None,
                Outcome::Failed(why) => {
                    tracing::warn!(%why, "no open dialog");
                    None
                }
            };
            let picked = match path {
                Some(path) => {
                    let request = Request {
                        path,
                        token: None,
                        chat,
                        session,
                    };
                    match runtime.block_on(instance::claim_or_forward(&request, inbox, documents)) {
                        Ok(Role::Owner(connection)) => Picked::Open(request, Some(connection)),
                        Ok(Role::Forwarded) => Picked::Forwarded,
                        Err(err) => {
                            tracing::warn!(%err, "no session bus; this Studio stands alone");
                            Picked::Open(request, None)
                        }
                    }
                }
                None => Picked::Nothing,
            };
            *answer.lock().unwrap() = Some(picked);
        });
        if let Err(err) = spawned {
            tracing::warn!(%err, "no thread for the open dialog");
            *fallback.lock().unwrap() = Some(Picked::Nothing);
        }
    }

    /// Move the pick along, and take its answer once there is one. The
    /// window is closed with the answer; closing the window first counts as
    /// cancelling.
    pub fn poll(&mut self) -> Option<Picked> {
        self.ask();
        if self.closed.load(std::sync::atomic::Ordering::Relaxed) {
            self.window.close();
            return Some(Picked::Nothing);
        }
        let picked = self.answer.lock().unwrap().take()?;
        self.window.close();
        Some(picked)
    }
}

/// What the draw and the pointer need of the window, as last configured.
#[derive(Debug, Clone, Copy)]
struct Frame {
    width: f32,
    height: f32,
    variant: DecorationVariant,
    active: bool,
    dark: bool,
}

impl Frame {
    /// The window's titlebar: Studio's, with nothing but the name on it.
    fn decoration(&self) -> WindowDecoration {
        WindowDecoration::new("Studio", self.width)
            .with_variant(self.variant)
            .with_active(self.active)
            .with_dark(self.dark)
            // Opaque: nothing is blurred behind this window.
            .with_blurred(false)
    }
}
