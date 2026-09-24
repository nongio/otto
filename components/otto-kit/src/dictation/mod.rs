//! Speech into a text field.
//!
//! A [`Dictation`] listens to the microphone and sends what it hears to a
//! speech-to-text [`Engine`] twice a second. Words two passes agree on are
//! settled and typed into the [`TextInput`] at its caret; the words after them
//! are drawn dimmed, and an equaliser moving with your voice stands in for
//! the caret. Stopping sends the whole clip once more and types what is left.
//!
//! The field only draws; the app decides which keys start, stop and cancel a
//! dictation, and keeps calling [`Dictation::update`] while one runs.
//!
//! ```ignore
//! // A key starts it...
//! self.dictation = Some(Dictation::start(Engine::from_env(), &mut self.input));
//! // ...every frame, and when its poll fd wakes...
//! if let Some(dictation) = self.dictation.as_mut() {
//!     if dictation.update(&mut self.input) == Status::Done {
//!         self.dictation = None;
//!     }
//! }
//! ```

// Rust guideline compliant 2026-02-21

mod agreement;
mod bars;
mod capture;
mod engine;
mod vocabulary;

use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

pub use agreement::{is_marker, Agreement, Update, Word};
pub use bars::{Bars, WINDOW};
pub use capture::{Capture, SAMPLE_RATE};
pub use engine::{Engine, TranscribeError};
pub use vocabulary::Vocabulary;

use crate::components::text_input::{DictationMark, KeyMods, TextInput, TextInputKey};

/// How often the equaliser moves while listening (~24 fps). An app keeps
/// calling [`Dictation::update`] at least this often.
pub const FRAME: Duration = Duration::from_millis(42);
/// How often a pass is sent while you speak.
const PASS_EVERY: Duration = Duration::from_millis(500);
/// Shortest clip worth sending: half a second.
const MIN_SAMPLES: usize = SAMPLE_RATE as usize / 2;
/// Once the buffer is longer than this, it is cut at the last settled word.
/// Shorter buffers keep the settled words as context for the engine.
const TRIM_AFTER: usize = SAMPLE_RATE as usize * 6;
/// Characters of already settled text sent as the prompt of each pass.
const PROMPT_CHARS: usize = 200;
/// Most names sent as hotwords with each pass.
const MAX_HOTWORDS: usize = 100;

/// Where a dictation is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Hearing you.
    Listening,
    /// Stopped; the last pass is on its way.
    Finishing,
    /// Everything is typed; the field is back to itself.
    Done,
}

/// A pass that came back from the engine.
struct Recognised {
    /// Which buffer it covered; a pass over a buffer since replaced is dropped.
    generation: u64,
    is_final: bool,
    words: Result<Vec<Word>, String>,
}

/// One dictation into one field, from start to its last words.
pub struct Dictation {
    engine: Engine,
    /// `None` once stopped.
    capture: Option<Capture>,
    in_flight: bool,
    last_sent: Instant,
    last_frame: Instant,
    agreement: Agreement,
    bars: Bars,
    /// Bumped whenever the buffer is replaced, so older passes are dropped.
    generation: u64,
    results: mpsc::Receiver<Recognised>,
    results_tx: mpsc::Sender<Recognised>,
    wake: UnixStream,
    wake_tx: Arc<UnixStream>,
    /// Where the dictated text starts in the field, and how many bytes of it
    /// were typed so far.
    start: usize,
    typed: usize,
    /// Everything settled so far, as context for the engine.
    heard: String,
    tentative: String,
    status: Status,
    vocabulary: Vocabulary,
}

impl Dictation {
    /// Start listening into `input`, at its caret. A selection is replaced,
    /// as typing would.
    ///
    /// With `OTTO_DICTATE_WAV` set, that file (16 kHz mono 16-bit) plays in
    /// place of the microphone.
    ///
    /// # Panics
    ///
    /// When the wake-up socket cannot be made.
    pub fn start(engine: Engine, input: &mut TextInput) -> Self {
        let capture = std::env::var_os("OTTO_DICTATE_WAV")
            .and_then(|path| {
                Capture::from_wav(path.as_ref())
                    .inspect_err(|err| tracing::warn!(%err, "cannot read OTTO_DICTATE_WAV"))
                    .ok()
            })
            .unwrap_or_else(Capture::start);
        Self::with_capture(engine, capture, input)
    }

    fn with_capture(engine: Engine, capture: Capture, input: &mut TextInput) -> Self {
        input.state.delete_selection();
        let (wake, wake_tx) = UnixStream::pair().expect("cannot create a wake-up socket");
        let _ = wake.set_nonblocking(true);
        let _ = wake_tx.set_nonblocking(true);
        let (results_tx, results) = mpsc::channel();
        let dictation = Self {
            engine,
            capture: Some(capture),
            in_flight: false,
            last_sent: Instant::now(),
            last_frame: Instant::now(),
            agreement: Agreement::default(),
            bars: Bars::default(),
            generation: 0,
            results,
            results_tx,
            wake,
            wake_tx: Arc::new(wake_tx),
            start: input.state.caret(),
            typed: 0,
            heard: String::new(),
            tentative: String::new(),
            status: Status::Listening,
            vocabulary: Vocabulary::default(),
        };
        dictation.mark(input);
        tracing::info!("dictation started");
        dictation
    }

    /// Names the field expects, such as the items a list offers: what is
    /// heard close to one is written as it, and an engine that takes a
    /// prompt is told about them.
    pub fn set_vocabulary(&mut self, vocabulary: Vocabulary) {
        tracing::info!(hotwords = %vocabulary.hotwords(MAX_HOTWORDS), "dictation vocabulary");
        self.vocabulary = vocabulary;
    }

    /// The socket that becomes readable when a pass comes back.
    pub fn poll_fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }

    /// Where the dictation is.
    pub fn status(&self) -> Status {
        self.status
    }

    /// Move the equaliser, send a pass when one is due and type in what came
    /// back. Never blocks. Call it every [`FRAME`] while it isn't
    /// [`Status::Done`], and when [`Self::poll_fd`] wakes.
    pub fn update(&mut self, input: &mut TextInput) -> Status {
        self.drain_wake();
        while let Ok(result) = self.results.try_recv() {
            self.on_recognised(result, input);
        }
        if self.status == Status::Done {
            return Status::Done;
        }
        if self.last_frame.elapsed() >= FRAME {
            self.last_frame = Instant::now();
            if let Some(capture) = self.capture.as_ref() {
                self.bars.step(&capture.tail(WINDOW));
            }
        }
        let due = self
            .capture
            .as_ref()
            .is_some_and(|c| c.len() >= MIN_SAMPLES)
            && !self.in_flight
            && self.last_sent.elapsed() >= PASS_EVERY;
        if due {
            self.in_flight = true;
            self.last_sent = Instant::now();
            let samples = self
                .capture
                .as_ref()
                .map(Capture::samples)
                .unwrap_or_default();
            self.send(samples, false);
        }
        self.mark(input);
        self.status
    }

    /// Stop listening. The last pass types in what is left; [`Self::update`]
    /// reports [`Status::Done`] once it has.
    pub fn stop(&mut self) {
        let Some(capture) = self.capture.take() else {
            return;
        };
        tracing::info!(
            seconds = capture.len() as f32 / SAMPLE_RATE as f32,
            "dictation stopped"
        );
        self.status = Status::Finishing;
        self.in_flight = true;
        // A pass still in flight covers a buffer the final one supersedes.
        self.generation += 1;
        self.send(capture.samples(), true);
    }

    /// Stop and take out everything this dictation typed.
    pub fn cancel(mut self, input: &mut TextInput) {
        tracing::info!("dictation cancelled");
        self.capture = None;
        let end = (self.start + self.typed).min(input.state.value().len());
        if self.start < end {
            input.state.select_range(self.start..end);
            input.state.delete_selection();
        }
        input.state.dictation = None;
        input.set_value(input.value().to_string());
        input
            .state
            .set_caret(self.start.min(input.value().len()), false);
    }

    /// Show what is pending and the equaliser at the caret, or nothing once
    /// done.
    fn mark(&self, input: &mut TextInput) {
        input.state.dictation = (self.status != Status::Done).then(|| DictationMark {
            pending: self.tentative.clone(),
            levels: if self.capture.is_some() {
                self.bars.levels().to_vec()
            } else {
                // Stopped: the bars rest while the last words arrive.
                vec![0.1; self.bars.levels().len()]
            },
        });
    }

    fn on_recognised(&mut self, result: Recognised, input: &mut TextInput) {
        if result.generation != self.generation {
            return;
        }
        self.in_flight = false;
        let words = result.words.unwrap_or_else(|error| {
            tracing::warn!(%error, "transcription failed");
            Vec::new()
        });
        if result.is_final {
            let rest = self.agreement.finish(words);
            self.type_in(&rest, input);
            self.tentative.clear();
            self.status = Status::Done;
            input.state.dictation = None;
            tracing::info!(text = %self.heard, "dictation done");
            return;
        }
        let update = self.agreement.step(words);
        self.type_in(&update.settled, input);
        self.tentative = self.vocabulary.correct(&join(
            &update.tentative,
            needs_space(input) || !update.settled.is_empty(),
        ));

        // Cut the audio at the last settled word once the buffer is long, so
        // passes stay short however long you talk.
        let cut = self.agreement.settled_end();
        if let Some(capture) = self.capture.as_ref() {
            if capture.len() > TRIM_AFTER && cut > 0.0 {
                capture.trim((cut * SAMPLE_RATE as f32) as usize);
                self.agreement.trimmed(cut);
            }
        }
    }

    /// Type settled `words` at the caret.
    fn type_in(&mut self, words: &[Word], input: &mut TextInput) {
        let text = self.vocabulary.correct(&join(words, needs_space(input)));
        if text.is_empty() {
            return;
        }
        let before = input.value().len();
        input.on_key(TextInputKey::Text(text.clone()), KeyMods::default());
        self.typed += input.value().len().saturating_sub(before);
        self.heard.push_str(&text);
    }

    /// Transcribe `samples` on a thread of its own.
    fn send(&self, samples: Vec<f32>, is_final: bool) {
        let engine = self.engine.clone();
        let results = self.results_tx.clone();
        let wake = self.wake_tx.clone();
        let generation = self.generation;
        let prompt = prompt(&self.vocabulary, &self.heard);
        let hotwords = self.vocabulary.hotwords(MAX_HOTWORDS);
        thread::spawn(move || {
            let words = if samples.len() < MIN_SAMPLES {
                Ok(Vec::new())
            } else {
                engine
                    .transcribe(&samples, &prompt, &hotwords)
                    .map_err(|e| e.to_string())
            };
            if results
                .send(Recognised {
                    generation,
                    is_final,
                    words,
                })
                .is_ok()
            {
                // A full socket already has a wake-up in it.
                let _ = (&*wake).write(&[1]);
            }
        });
    }

    fn drain_wake(&mut self) {
        let mut buffer = [0u8; 64];
        loop {
            match self.wake.read(&mut buffer) {
                Ok(0) => break,
                Ok(_) => continue,
                Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    }
}

/// Whether the character before the caret is not a space, so what is typed
/// next needs one in front.
fn needs_space(input: &TextInput) -> bool {
    input.value()[..input.state.caret()]
        .chars()
        .next_back()
        .is_some_and(|c| !c.is_whitespace())
}

/// `words` as text, with a space in front when it follows other text, unless
/// it starts with punctuation.
pub fn join(words: &[Word], after_text: bool) -> String {
    let raw: String = words.iter().map(|w| w.text.as_str()).collect();
    let text = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let glued = text.starts_with(|c: char| c.is_ascii_punctuation());
    if after_text && !glued && !text.is_empty() {
        format!(" {text}")
    } else {
        text
    }
}

/// Context for the engine: the names expected, then the end of what was
/// settled.
fn prompt(vocabulary: &Vocabulary, heard: &str) -> String {
    let start = heard
        .char_indices()
        .rev()
        .nth(PROMPT_CHARS)
        .map_or(0, |(i, _)| i);
    let said = heard[start..].trim();
    let names = vocabulary.prompt(PROMPT_CHARS);
    match (names.is_empty(), said.is_empty()) {
        (true, _) => said.to_string(),
        (false, true) => format!("{names}."),
        (false, false) => format!("{names}. {said}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::text_input::TextInputStyle;

    fn word(text: &str) -> Word {
        Word {
            text: text.into(),
            start: 0.0,
            end: 0.0,
        }
    }

    /// A dictation that hears nothing: no microphone in tests.
    fn dictate(input: &mut TextInput) -> Dictation {
        Dictation::with_capture(Engine::from_env(), Capture::silent(), input)
    }

    fn field(value: &str) -> TextInput {
        let mut input = TextInput::new(value, TextInputStyle::default());
        input.state.set_focused(true);
        input.state.set_caret(value.len(), false);
        input
    }

    #[test]
    fn words_join_with_single_spaces_and_hug_punctuation() {
        assert_eq!(
            join(&[word(" Hello"), word(" world")], false),
            "Hello world"
        );
        assert_eq!(join(&[word(" again")], true), " again");
        assert_eq!(join(&[word(","), word(" then")], true), ", then");
        assert_eq!(join(&[], true), "");
    }

    #[test]
    fn settled_words_are_typed_at_the_caret_with_a_space_after_text() {
        let mut input = field("Ask");
        let mut dictation = dictate(&mut input);
        dictation.type_in(&[word(" about"), word(" this")], &mut input);
        assert_eq!(input.value(), "Ask about this");
        assert!(
            input.state.dictation.is_some(),
            "the mark stays while listening"
        );
    }

    #[test]
    fn heard_names_are_typed_as_the_vocabulary_writes_them() {
        let mut input = field("");
        let mut dictation = dictate(&mut input);
        dictation.set_vocabulary(Vocabulary::new(["Ghostty", "Firefox"]));
        dictation.type_in(&[word(" ghost"), word(" tea")], &mut input);
        assert_eq!(input.value(), "Ghostty");
        assert_eq!(
            prompt(&dictation.vocabulary, &dictation.heard),
            "Ghostty, Firefox. Ghostty"
        );
    }

    #[test]
    fn cancelling_takes_out_only_what_was_dictated() {
        let mut input = field("Keep this");
        let mut dictation = dictate(&mut input);
        dictation.type_in(&[word(" and"), word(" not"), word(" this")], &mut input);
        assert_eq!(input.value(), "Keep this and not this");
        dictation.cancel(&mut input);
        assert_eq!(input.value(), "Keep this");
        assert_eq!(input.state.caret(), "Keep this".len());
        assert!(input.state.dictation.is_none());
    }

    #[test]
    fn the_last_pass_types_the_rest_and_ends() {
        let mut input = field("");
        let mut dictation = dictate(&mut input);
        dictation.status = Status::Finishing;
        let generation = dictation.generation;
        dictation.on_recognised(
            Recognised {
                generation,
                is_final: true,
                words: Ok(vec![word(" Hello"), word(" there.")]),
            },
            &mut input,
        );
        assert_eq!(input.value(), "Hello there.");
        assert_eq!(dictation.status(), Status::Done);
        assert!(input.state.dictation.is_none());
    }
}
