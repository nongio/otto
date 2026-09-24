//! otto-dictate: dictation as a Wayland input method (proof of concept).
//!
//! Run it once; it binds `zwp_input_method_v2` and waits. `otto-dictate
//! toggle` starts listening in the focused text field, and stops it again.
//!
//! While you speak, a balloon under the caret shows bars moving with your
//! voice and the words heard so far. The audio buffer goes to a whisper.cpp
//! server twice a second; words two passes agree on are settled and shown
//! bright, the rest dimmed. The audio is trimmed up to the settled words, so a
//! pass only covers what is still unsettled. The field is not touched until
//! you stop: then the whole text goes in as one commit.
//!
//! While listening the keyboard is grabbed:
//! - Escape throws the balloon away.
//! - Backspace drops the last settled phrase from the balloon.
//! - Any other key stops listening, like the toggle. The key itself is not
//!   passed on.
//!
//! Environment:
//! - `OTTO_DICTATE_URL`: the server's `/inference` endpoint
//!   (default `http://127.0.0.1:8080/inference`).
//! - `OTTO_DICTATE_LANGUAGE`: an ISO 639-1 code or `auto` (default: from
//!   `LANG`).
//! - `OTTO_DICTATE_WAV`: a 16 kHz mono WAV played in place of the microphone.

mod balloon;

// Rust guideline compliant 2026-02-21

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use smithay_client_toolkit::delegate_shm;
use smithay_client_toolkit::reexports::calloop::channel::{self, Channel, Sender};
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{
    wl_compositor, wl_keyboard, wl_registry, wl_seat, wl_shm, wl_surface,
};
use wayland_client::{delegate_noop, Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_keyboard_grab_v2::{self, ZwpInputMethodKeyboardGrabV2},
    zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
    zwp_input_method_v2::{self, ZwpInputMethodV2},
    zwp_input_popup_surface_v2::{self, ZwpInputPopupSurfaceV2},
};

use otto_kit::dictation::{join, Agreement, Bars, Capture, Engine, Word, SAMPLE_RATE, WINDOW};

use crate::balloon::Balloon;

/// Initial size of the popup's buffer pool; it grows with the balloon.
const POOL_BYTES: usize = 512 * 256 * 4;
/// Popup buffers are drawn at this scale, so they stay sharp on HiDPI.
const POPUP_SCALE: i32 = 2;
/// Frame interval while listening (~24 fps).
const FRAME: Duration = Duration::from_millis(42);
/// Timer interval while idle; only bounds how soon the first frame appears.
const IDLE_TICK: Duration = Duration::from_millis(100);
/// How often a pass is sent while you speak.
const PASS_EVERY: Duration = Duration::from_millis(500);
/// Shortest clip worth sending: half a second.
const MIN_SAMPLES: usize = SAMPLE_RATE as usize / 2;
/// Once the buffer is longer than this, it is cut at the last settled word.
/// Shorter buffers keep the settled words as context for the engine.
const TRIM_AFTER: usize = SAMPLE_RATE as usize * 6;
/// Characters of already settled text sent as the prompt of each pass.
const PROMPT_CHARS: usize = 200;

/// How long to wait before asking for the input method again after another
/// one held it.
const RETRY_BIND: Duration = Duration::from_secs(3);

/// Linux evdev key codes the grab reacts to.
const KEY_ESC: u32 = 1;
const KEY_BACKSPACE: u32 = 14;
const KEY_ENTER: u32 = 28;
const KEY_KP_ENTER: u32 = 96;
/// Modifiers and lock keys, which never stop listening: the toggle shortcut
/// itself starts with them.
const MODIFIER_KEYS: [u32; 9] = [29, 97, 42, 54, 56, 100, 125, 126, 58];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "otto_dictate=info".into()),
        )
        .init();

    if std::env::args().nth(1).as_deref() == Some("toggle") {
        let mut stream = UnixStream::connect(socket_path())?;
        stream.write_all(b"toggle\n")?;
        return Ok(());
    }

    let engine = Engine::from_env();
    tracing::info!(url = %engine.url, language = %engine.language, "otto-dictate starting");

    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init::<State>(&conn)?;
    let qh = event_queue.handle();

    let shm = Shm::bind(&globals, &qh)?;
    let pool = SlotPool::new(POOL_BYTES, &shm)?;
    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=4, ())?;
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ())?;
    let manager: ZwpInputMethodManagerV2 = globals.bind(&qh, 1..=1, ())?;
    let input_method = manager.get_input_method(&seat, &qh, ());

    let mut event_loop: EventLoop<State> = EventLoop::try_new()?;
    let handle = event_loop.handle();
    WaylandSource::new(conn.clone(), event_queue).insert(handle.clone())?;

    let (toggle_tx, toggle_rx) = channel::channel::<()>();
    listen_for_toggles(toggle_tx)?;
    handle.insert_source(toggle_rx, |event, _, state| {
        if let channel::Event::Msg(()) = event {
            state.toggle();
        }
    })?;

    let (result_tx, result_rx): (Sender<Recognised>, Channel<Recognised>) = channel::channel();
    handle.insert_source(result_rx, |event, _, state| {
        if let channel::Event::Msg(result) = event {
            state.on_recognised(result);
        }
    })?;

    handle.insert_source(Timer::from_duration(IDLE_TICK), |_, _, state| {
        TimeoutAction::ToDuration(state.tick())
    })?;

    let mut state = State {
        qh,
        shm,
        pool,
        buffer: None,
        input_method,
        manager,
        seat,
        retry_at: None,
        compositor,
        popup: None,
        grab: None,
        pending: Field::default(),
        field: Field::default(),
        serial: 0,
        engine,
        results: result_tx,
        listening: None,
        generation: 0,
        bars: Bars::default(),
        balloon: Balloon::default(),
    };

    loop {
        event_loop.dispatch(None, &mut state)?;
    }
}

fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").unwrap_or_else(|| "/tmp".into());
    PathBuf::from(dir).join("otto-dictate.sock")
}

/// Accept `toggle` requests on a Unix socket, each one a message on `tx`.
fn listen_for_toggles(tx: Sender<()>) -> std::io::Result<()> {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            let _ = stream.take(64).read_to_string(&mut line);
            if line.trim() == "toggle" && tx.send(()).is_err() {
                break;
            }
        }
    });
    Ok(())
}

/// A pass that came back from the engine.
struct Recognised {
    /// Which buffer it covered; a pass over a buffer since replaced is dropped.
    generation: u64,
    /// The pass sent on stop, after which everything left is committed.
    is_final: bool,
    words: Result<Vec<Word>, String>,
}

/// The focused text field, as the text-input client describes it.
#[derive(Debug, Default, Clone)]
struct Field {
    active: bool,
    /// The text around the caret, and the caret's byte offset in it.
    surrounding: Option<(String, u32)>,
}

impl Field {
    /// Whether the character before the caret is not a space, so dictated
    /// text needs one in front.
    fn needs_space(&self) -> bool {
        self.surrounding.as_ref().is_some_and(|(text, cursor)| {
            text.get(..*cursor as usize)
                .and_then(|before| before.chars().next_back())
                .is_some_and(|c| !c.is_whitespace())
        })
    }
}

/// One dictation, from toggle on to its final text.
struct Listening {
    /// `None` once stopped and waiting for the final pass.
    capture: Option<Capture>,
    in_flight: bool,
    last_sent: Instant,
    agreement: Agreement,
    /// Settled text, one entry per pass that settled something, so Backspace
    /// can drop the latest.
    chunks: Vec<String>,
    /// Words heard but not settled yet.
    tentative: String,
}

impl Listening {
    /// Add settled `words` to the text.
    fn settle(&mut self, words: &[Word]) {
        let text = join(words, !self.chunks.is_empty());
        if !text.is_empty() {
            self.chunks.push(text);
        }
    }

    fn text(&self) -> String {
        self.chunks.concat()
    }

    /// The end of the settled text, as context for the engine.
    fn prompt(&self) -> String {
        let all = self.text();
        let start = all
            .char_indices()
            .rev()
            .nth(PROMPT_CHARS)
            .map_or(0, |(i, _)| i);
        all[start..].trim().to_string()
    }
}

struct State {
    qh: QueueHandle<State>,
    shm: Shm,
    pool: SlotPool,
    buffer: Option<Buffer>,
    input_method: ZwpInputMethodV2,
    manager: ZwpInputMethodManagerV2,
    seat: wl_seat::WlSeat,
    /// When to ask for the input method again, after it was unavailable.
    retry_at: Option<Instant>,
    compositor: wl_compositor::WlCompositor,
    /// The equaliser's surface, while listening. It is destroyed when the
    /// dictation ends rather than hidden, because Otto keeps showing an
    /// input popup whose buffer is removed.
    popup: Option<(wl_surface::WlSurface, ZwpInputPopupSurfaceV2)>,
    /// The keyboard, while listening.
    grab: Option<ZwpInputMethodKeyboardGrabV2>,
    /// The field as announced, applied on the next `done`.
    pending: Field,
    field: Field,
    /// Number of `done` events so far, which every commit must quote.
    serial: u32,
    engine: Engine,
    results: Sender<Recognised>,
    listening: Option<Listening>,
    /// Bumped whenever the audio buffer is replaced, so passes over the old
    /// one are dropped.
    generation: u64,
    bars: Bars,
    balloon: Balloon,
}

impl State {
    fn toggle(&mut self) {
        if self.listening.is_some() {
            self.stop();
        } else if self.field.active {
            self.start();
        } else {
            tracing::warn!("no text field is focused; nothing to dictate into");
        }
    }

    fn start(&mut self) {
        let capture = match std::env::var_os("OTTO_DICTATE_WAV") {
            Some(path) => match Capture::from_wav(path.as_ref()) {
                Ok(capture) => capture,
                Err(error) => {
                    tracing::error!(%error, "cannot read OTTO_DICTATE_WAV");
                    return;
                }
            },
            None => Capture::start(),
        };
        tracing::info!("start listening");
        self.generation += 1;
        self.bars = Bars::default();
        self.listening = Some(Listening {
            capture: Some(capture),
            in_flight: false,
            last_sent: Instant::now(),
            agreement: Agreement::default(),
            chunks: Vec::new(),
            tentative: String::new(),
        });
        self.grab = Some(self.input_method.grab_keyboard(&self.qh, ()));
        let surface = self.compositor.create_surface(&self.qh, ());
        surface.set_buffer_scale(POPUP_SCALE);
        let popup = self
            .input_method
            .get_input_popup_surface(&surface, &self.qh, ());
        self.popup = Some((surface, popup));
    }

    /// Stop capturing; the final pass commits what is left.
    fn stop(&mut self) {
        let Some(listening) = self.listening.as_mut() else {
            return;
        };
        let Some(capture) = listening.capture.take() else {
            return; // already finishing
        };
        tracing::info!(
            seconds = capture.len() as f32 / SAMPLE_RATE as f32,
            "stop listening"
        );
        listening.in_flight = true;
        let prompt = listening.prompt();
        // A pass still in flight covers a buffer the final one supersedes.
        self.generation += 1;
        self.send(capture.samples(), prompt, true);
        self.release_grab();
    }

    /// Escape: throw the balloon away; the field was never touched.
    fn cancel(&mut self) {
        if self.listening.is_some() {
            tracing::info!("cancel");
            self.generation += 1;
            self.end();
        }
    }

    /// Backspace: drop the last settled phrase and what is not settled yet,
    /// and keep listening from here.
    fn take_back(&mut self) {
        let Some(listening) = self.listening.as_mut() else {
            return;
        };
        let Some(capture) = listening.capture.as_ref() else {
            return;
        };
        capture.trim(capture.len());
        listening.agreement = Agreement::default();
        listening.in_flight = false;
        listening.last_sent = Instant::now();
        listening.tentative.clear();
        if let Some(chunk) = listening.chunks.pop() {
            tracing::info!(%chunk, "take back");
        }
        self.generation += 1;
    }

    /// Transcribe `samples` on a thread of its own.
    fn send(&self, samples: Vec<f32>, prompt: String, is_final: bool) {
        let engine = self.engine.clone();
        let results = self.results.clone();
        let generation = self.generation;
        thread::spawn(move || {
            let started = Instant::now();
            let words = if samples.len() < MIN_SAMPLES {
                Ok(Vec::new())
            } else {
                engine
                    .transcribe(&samples, &prompt, "")
                    .map_err(|e| e.to_string())
            };
            tracing::debug!(
                is_final,
                seconds = samples.len() as f32 / SAMPLE_RATE as f32,
                ms = started.elapsed().as_millis() as u64,
                "pass done"
            );
            let _ = results.send(Recognised {
                generation,
                is_final,
                words,
            });
        });
    }

    fn on_recognised(&mut self, result: Recognised) {
        if result.generation != self.generation {
            return;
        }
        let Some(listening) = self.listening.as_mut() else {
            return;
        };
        listening.in_flight = false;
        let words = result.words.unwrap_or_else(|error| {
            tracing::error!(%error, "transcription failed");
            Vec::new()
        });

        if result.is_final {
            let rest = listening.agreement.finish(words);
            listening.settle(&rest);
            let text = listening.text();
            tracing::info!(%text, "final");
            if !text.is_empty() {
                let text = if self.field.needs_space()
                    && !text.starts_with(|c: char| c.is_ascii_punctuation())
                {
                    format!(" {text}")
                } else {
                    text
                };
                self.input_method.commit_string(text);
                self.input_method.commit(self.serial);
            }
            self.end();
            return;
        }

        let update = listening.agreement.step(words);
        listening.settle(&update.settled);
        listening.tentative = join(&update.tentative, !listening.chunks.is_empty());
        tracing::debug!(settled = %listening.text(), tentative = %listening.tentative, "heard");

        // Cut the audio at the last settled word once the buffer is long, so
        // passes stay short however long you talk.
        let cut = listening.agreement.settled_end();
        if let Some(capture) = listening.capture.as_ref() {
            if capture.len() > TRIM_AFTER && cut > 0.0 {
                capture.trim((cut * SAMPLE_RATE as f32) as usize);
                listening.agreement.trimmed(cut);
                tracing::debug!(seconds = cut, "trimmed");
            }
        }
    }

    /// The dictation is over, one way or another.
    fn end(&mut self) {
        self.listening = None;
        self.release_grab();
        self.close_popup();
    }

    fn release_grab(&mut self) {
        if let Some(grab) = self.grab.take() {
            grab.release();
        }
    }

    fn on_key(&mut self, key: u32) {
        match key {
            KEY_ESC => self.cancel(),
            KEY_BACKSPACE => self.take_back(),
            KEY_ENTER | KEY_KP_ENTER => self.stop(),
            key if MODIFIER_KEYS.contains(&key) => {}
            _ => self.stop(),
        }
    }

    /// One timer tick; returns when the next one is due.
    fn tick(&mut self) -> Duration {
        if self.retry_at.is_some_and(|at| Instant::now() >= at) {
            self.retry_at = None;
            self.input_method.destroy();
            self.input_method = self.manager.get_input_method(&self.seat, &self.qh, ());
        }
        let Some(listening) = self.listening.as_mut() else {
            return IDLE_TICK;
        };
        let recent = listening
            .capture
            .as_ref()
            .map(|c| c.tail(WINDOW))
            .unwrap_or_default();
        let pass_due = listening
            .capture
            .as_ref()
            .is_some_and(|c| c.len() >= MIN_SAMPLES)
            && !listening.in_flight
            && listening.last_sent.elapsed() >= PASS_EVERY;
        if pass_due {
            listening.in_flight = true;
            listening.last_sent = Instant::now();
            let samples = listening
                .capture
                .as_ref()
                .map(Capture::samples)
                .unwrap_or_default();
            let prompt = listening.prompt();
            self.send(samples, prompt, false);
        }

        self.bars.step(&recent);
        self.draw_popup();
        FRAME
    }

    fn draw_popup(&mut self) {
        let Some(listening) = self.listening.as_ref() else {
            return;
        };
        let layout = self.balloon.layout(&listening.text(), &listening.tentative);
        let scale = POPUP_SCALE as f32;
        let w = (layout.width * scale) as i32;
        let h = (layout.height * scale) as i32;
        // Reuse the buffer when it is the right size and the compositor has
        // released it; otherwise take a fresh one from the pool.
        let reusable = self.buffer.as_mut().is_some_and(|b| {
            b.height() == h && b.stride() == w * 4 && b.canvas(&mut self.pool).is_some()
        });
        if !reusable {
            match self
                .pool
                .create_buffer(w, h, w * 4, wl_shm::Format::Argb8888)
            {
                Ok((buffer, _)) => self.buffer = Some(buffer),
                Err(error) => {
                    tracing::warn!(%error, "no buffer for the popup");
                    return;
                }
            }
        }
        let (Some(buffer), Some((surface, _))) = (self.buffer.as_mut(), self.popup.as_ref()) else {
            return;
        };
        let Some(canvas) = buffer.canvas(&mut self.pool) else {
            return;
        };
        self.balloon
            .draw(&layout, self.bars.levels(), canvas, w, h, scale);
        if buffer.attach_to(surface).is_ok() {
            surface.damage_buffer(0, 0, w, h);
            surface.commit();
        }
    }

    fn close_popup(&mut self) {
        if let Some((surface, popup)) = self.popup.take() {
            popup.destroy();
            surface.destroy();
        }
    }

    /// The focused field went away: drop the dictation.
    fn deactivated(&mut self) {
        if self.listening.is_some() {
            tracing::info!("text field lost focus; dictation dropped");
            self.generation += 1;
            self.end();
        }
    }
}

impl Dispatch<ZwpInputMethodV2, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpInputMethodV2,
        event: zwp_input_method_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_input_method_v2::Event::Activate => {
                // Activation resets the field's state.
                state.pending = Field {
                    active: true,
                    surrounding: None,
                };
            }
            zwp_input_method_v2::Event::Deactivate => state.pending.active = false,
            zwp_input_method_v2::Event::SurroundingText { text, cursor, .. } => {
                state.pending.surrounding = Some((text, cursor));
            }
            zwp_input_method_v2::Event::Done => {
                state.serial += 1;
                let was_active = state.field.active;
                state.field = state.pending.clone();
                tracing::debug!(active = state.field.active, serial = state.serial, "done");
                if was_active && !state.field.active {
                    state.deactivated();
                }
            }
            zwp_input_method_v2::Event::Unavailable => {
                // Another input method holds the seat; it may go, so ask
                // again later rather than give up.
                tracing::warn!("another input method is bound; trying again shortly");
                state.end();
                state.field = Field::default();
                state.retry_at = Some(Instant::now() + RETRY_BIND);
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwpInputMethodKeyboardGrabV2, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpInputMethodKeyboardGrabV2,
        event: zwp_input_method_keyboard_grab_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_input_method_keyboard_grab_v2::Event::Key {
            key,
            state: WEnum::Value(wl_keyboard::KeyState::Pressed),
            ..
        } = event
        {
            state.on_key(key);
        }
    }
}

impl Dispatch<ZwpInputPopupSurfaceV2, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpInputPopupSurfaceV2,
        event: zwp_input_popup_surface_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_input_popup_surface_v2::Event::TextInputRectangle {
            x,
            y,
            width,
            height,
        } = event
        {
            tracing::trace!(x, y, width, height, "caret rectangle");
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_shm!(State);
delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: ignore ZwpInputMethodManagerV2);
