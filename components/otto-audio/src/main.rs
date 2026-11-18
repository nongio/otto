//! `otto-audio`: the PipeWire graph as a patchbay, and the engine's settings.
//!
//! One window. PipeWire is followed on its own thread (otto-audio-graph);
//! its events land in an inbox the run loop drains in `on_update`, so the
//! window repaints when the graph changes and never polls.
//!
//! `otto-audio --png OUT [--engine]` draws the window offscreen instead.

mod preview;
mod ui;

use std::sync::{Arc, Mutex};

use otto_audio::WINDOW_APP;
use otto_audio_graph::{Connection, Event};
use otto_kit::events::{KeyEvent, KeyState, WindowConfigure};
use otto_kit::prelude::*;
use otto_kit::t;

use crate::ui::Ui;

/// The app id, which names the desktop entry.
const APP_ID: &str = "otto-audio";
/// The window's opening size.
const WINDOW_W: i32 = 1100;
const WINDOW_H: i32 = 700;
/// The smallest the window goes: room for the sidebar, the toolbar and a
/// column of cards.
const MIN_W: u32 = 720;
const MIN_H: u32 = 460;

struct AudioApp {
    ui: Option<Arc<Mutex<Ui>>>,
    inbox: Arc<Mutex<Vec<Event>>>,
    window: Option<Window>,
}

impl AudioApp {
    fn redraw(&self) {
        if let Some(window) = &self.window {
            window.request_frame();
        }
    }

    fn with_ui<R>(&self, f: impl FnOnce(&mut Ui) -> R) -> Option<R> {
        let ui = self.ui.as_ref()?;
        let mut ui = ui.lock().ok()?;
        Some(f(&mut ui))
    }
}

impl App for AudioApp {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let inbox = Arc::clone(&self.inbox);
        let connection = Connection::start(WINDOW_APP, move |event| {
            if let Ok(mut inbox) = inbox.lock() {
                inbox.push(event);
            }
            AppContext::request_wakeup();
        });
        let ui = Arc::new(Mutex::new(Ui::new(connection)));

        let mut window = Window::new(t!("audio-title"), WINDOW_W, WINDOW_H)?;
        window.set_min_size(MIN_W, MIN_H);
        window.set_background(ui::ground(&AppContext::current_theme()));
        if let Some(surface) = window.surface() {
            surface.xdg_window().set_app_id(APP_ID.to_string());
        }

        let drawn = Arc::clone(&ui);
        window.on_draw(move |canvas| {
            let theme = AppContext::current_theme();
            if let Ok(mut ui) = drawn.lock() {
                ui.draw(canvas, &theme);
            }
        });

        let pointed = Arc::clone(&ui);
        let handle = window.clone();
        window.on_pointer_event(move |events| {
            let redraw = pointed
                .lock()
                .map(|mut ui| ui.pointer(events, &handle))
                .unwrap_or(false);
            if redraw {
                handle.request_frame();
            }
        });

        AppContext::register_window(window.clone());
        self.ui = Some(ui);
        self.window = Some(window);
        Ok(())
    }

    fn on_configure(&mut self, _ctx: &AppContext, configure: WindowConfigure, _serial: u32) {
        let Some(window) = self.window.clone() else {
            return;
        };
        self.with_ui(|ui| {
            if let (Some(w), Some(h)) = configure.new_size {
                ui.size = (w.get() as f32, h.get() as f32);
            }
            ui.variant = window.decoration_variant();
            ui.active = window.is_activated();
        });
        window.sync_frame_corners();
        window.request_frame();
    }

    fn on_modifiers(&mut self, _ctx: &AppContext, modifiers: Modifiers) {
        self.with_ui(|ui| ui.modifiers = modifiers);
    }

    fn on_key_event(&mut self, _ctx: &AppContext, event: &KeyEvent, state: KeyState, _serial: u32) {
        if state != KeyState::Pressed {
            return;
        }
        if self.with_ui(|ui| ui.key(event.keysym)).unwrap_or(false) {
            self.redraw();
        }
    }

    fn on_theme_changed(&mut self, _ctx: &AppContext) {
        if let Some(window) = &mut self.window {
            window.set_background(ui::ground(&AppContext::current_theme()));
        }
        self.redraw();
    }

    fn on_pointer_pinch_begin(&mut self, _ctx: &AppContext, fingers: u32) {
        if fingers == 2 {
            self.with_ui(Ui::pinch_begin);
        }
    }

    fn on_pointer_pinch_update(
        &mut self,
        _ctx: &AppContext,
        _dx: f64,
        _dy: f64,
        scale: f64,
        _rotation: f64,
    ) {
        if self
            .with_ui(|ui| ui.pinch_update(scale as f32))
            .unwrap_or(false)
        {
            self.redraw();
        }
    }

    fn on_pointer_pinch_end(&mut self, _ctx: &AppContext, _cancelled: bool) {
        self.with_ui(Ui::pinch_end);
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        let events = match self.inbox.lock() {
            Ok(mut inbox) => std::mem::take(&mut *inbox),
            Err(_) => return,
        };
        if events.is_empty() {
            return;
        }
        self.with_ui(|ui| {
            for event in events {
                ui.apply(event);
            }
        });
        self.redraw();
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    otto_kit::logging::init("info");
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("otto-audio {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if let Some(at) = args.iter().position(|a| a == "--png") {
        let out = args.get(at + 1).ok_or("--png needs a file to write")?;
        let pane = if args.iter().any(|a| a == "--engine") {
            ui::Pane::Engine
        } else {
            ui::Pane::Graph
        };
        return preview::render(out, pane);
    }
    tokio::runtime::Runtime::new()?.block_on(async {
        AppRunner::new(AudioApp {
            ui: None,
            inbox: Arc::new(Mutex::new(Vec::new())),
            window: None,
        })
        .run()
    })
}
