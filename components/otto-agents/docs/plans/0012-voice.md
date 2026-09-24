# 0012: Voice (speech to text and text to speech)

**Status:** Draft

## Goal

You can talk to Otto and hear an agent answer.

**First target: speech to text in the launcher.** You hold the mic key and speak.
While you listen, an equaliser inside the field moves with your voice, and the words
appear in the field live as they are recognised. You can edit the text, then send it.
You pick the recognition engine in Otto's own config, and you can change it without a
restart.

After that, the agent answers aloud. While it speaks, the island shows what it is
saying, with the same bars moving to its voice.

## Decisions

- **Two pipelines, two owners.**
  - *Speech to text* runs in the **launcher**. It owns the field, the key that starts
    listening and the moment the prompt is sent, so there is no round trip through a
    service while you talk. Dictation is a desktop feature, not an agent feature: it
    fills the field whatever the field is for.
  - *Text to speech* runs in **otto-agents**. The service already sees every
    `ChatDelta`, keeps running when the launcher closes, and is the one place that
    knows when a turn ends. The launcher closing never cuts the agent off mid-sentence.
- **The STT engine is Otto config.** It lives in `config.toml` under
  `[speech_to_text]`, with entries in the settings schema (`src/settings/schema.rs`),
  so `org.otto.Settings` serves it and announces changes with `Changed`. The launcher
  reads it over that interface and follows it live, so switching engines needs no
  restart. `agents.toml` keeps only what the agents service owns (`[tts]`).
- **One shared crate, `otto-voice`.** Capture, playback, WAV encoding, the level
  meter, sentence splitting and the provider clients live in a leaf crate that the
  launcher, islands and the service all use. It has no AHP, Wayland or Skia code.
- **Bars are shared UI.** `BarAnimator` and `draw_bars` move from `otto-islands` into
  otto-kit, so the launcher's field and the islands draw the same bars.
- **Engines are external.** No speech models are linked into Otto. Whisper runs as a
  server (whisper.cpp `whisper-server`, faster-whisper, or any OpenAI-compatible
  endpoint) or as a command. Piper and similar engines run as commands. Models are
  large and change quickly, and linking whisper.cpp would add a long C++ build to
  every Otto build.
- **Parakeet is the default engine.** The dictation proof of concept tried both on
  an Iris Xe through Vulkan. Parakeet TDT 0.6B v3 is more accurate than Whisper
  small, detects the language itself and times each word, at the same speed. It
  answers whisper.cpp's `/inference` API through a small server
  (`components/otto-dictate/engines/parakeet-server`), because whisper.cpp ships
  none for it, so the `whisper` provider covers both.
- **Whisper sync, fed from PipeWire.** Whisper is synchronous: a whole clip goes in
  and text comes out, with no streaming protocol. Live text comes from a loop in
  `otto-voice` (`WhisperSync`) that owns the whole path from the mic to the text:
  - A PipeWire capture stream (16 kHz mono f32) writes into a ring buffer holding the
    utterance so far.
  - About once a second while you speak, the loop encodes the buffer as WAV and sends
    it to the recogniser. Only one request is in flight: if the engine is slower than
    a second, the next pass waits and takes the longer clip.
  - Each answer is a partial result for the launcher. On release, the loop sends the
    whole clip one last time, and that answer is final.
  - A clip longer than the engine handles well (30 s for Whisper) is cut at the last
    quiet stretch. The part before the cut is final and later passes start after it.
  - This is what whisper.cpp's `stream` does with SDL; here the audio comes from
    PipeWire, so Otto chooses the source and the engine stays a plain server or
    command.
- **One capture stream while listening.** The same PipeWire stream drives the
  equaliser: `buffer_level` on each buffer feeds the bars. There is no second stream
  just for the meter.
- **Audio through PipeWire.** The `pipewire` crate is already a workspace dependency.
  Capture asks for 16 kHz mono (Whisper's input). Playback is a stream with a fixed
  node name, `otto-agents-voice`, and `media.role = Communication`, so ducking and
  routing work like any other voice app, and islands can find the node by name.
  - The island's `LevelMeter` captures only the monitor of the default sink today. It
    opens its stream only while it is active, so the sound card can still suspend. It
    gains a target so the voice island can follow a named node.
  - Speech goes out through the default sink as well, so the music island's bars
    move while the agent talks. See Open questions.
- **HTTP through `ureq`**, which the music island already uses for album art. No
  `reqwest`.
- **Secrets never go in config files** (0005). A hosted engine names the environment
  variable that holds its key with `api_key_env`, or uses the credential store once
  0005 lands.
- **The island shows the reply, not the prompt.** What you say goes in the field,
  where you can correct it before sending. What the agent says goes in the island,
  because the launcher may be closed by then.

## Configuration

### Speech to text: Otto's `config.toml`

```toml
[speech_to_text]
provider = "whisper"               # whisper | openai | command
url = "http://127.0.0.1:8080/inference"
language = "system"                # system | auto | an ISO 639-1 code: "en", "it", …
live = true                        # text appears while you speak
send_on_release = false            # true: release the key and the prompt goes
```

A hosted or OpenAI-compatible engine:

```toml
[speech_to_text]
provider = "openai"
url = "https://api.openai.com/v1/audio/transcriptions"
model = "whisper-1"
api_key_env = "OPENAI_API_KEY"
```

A command reads a WAV on stdin and prints the text:

```toml
[speech_to_text]
provider = "command"
command = ["whisper-cli", "-m", "~/.local/share/whisper/ggml-base.en.bin", "-nt", "-f", "-"]
```

- Leaving the table out turns dictation off: no mic button, and the key does nothing.
- `language` is the language you speak:
  - `system`, the default, takes the first of Otto's `locales` (`it_IT` → `it`).
  - `auto` lets the engine detect it on each pass. This is slower, and a short
    partial clip can be detected wrongly, so the final pass may change language.
  - A code such as `en` fixes it.
  - Every provider gets it: whisper.cpp's `language` form field, OpenAI's `language`
    field, and `{language}` substituted in a command's arguments.
  - Changing it takes effect on the next dictation, like every other key.
- Every key is a schema entry (`speech_to_text.provider`, `speech_to_text.url`, …),
  applied live. The schema honesty test covers them like the rest.
- `api_key_env` holds a variable name, never a key. The launcher reads the variable
  from its own environment.
- `otto_config.example.toml` gains the table, commented out.

### Text to speech: `agents.toml`

```toml
[tts]
provider = "piper"                 # piper | openai | command
command = ["piper", "--model", "~/.local/share/piper/en_GB-alba-medium.onnx", "--output-raw"]
sample_rate = 22050
speak = "voice"                    # voice | always | never
```

- `ConfigFile` is `deny_unknown_fields`, so `[tts]` is added to it explicitly.
- `speak = "voice"` speaks a reply only when the prompt was spoken. That is the
  default, so typing never makes the desktop talk.
- An agent may override the voice with `voice = "…"` in its `[[agents]]` entry.
  Later, not in the first cut.
- `otto-agents doctor` checks the engine: the command exists or the server answers,
  and a one-word synthesis works.

### Providers

- `whisper` posts a WAV to whisper.cpp's `/inference`.
- `openai` posts to `/v1/audio/transcriptions` or `/v1/audio/speech`.
- `command` writes WAV to stdin and reads text from stdout (STT), or writes text to
  stdin and reads raw PCM from stdout (TTS).

## Flow

1. **Listening.** Hold the mic key in the launcher, or click the mic at the end of the
   field. A global shortcut opens the launcher already listening. It is a new
   `KeyAction`, so it needs arms in `actions.rs` and both `input_handler.rs` paths.
   - The mic glyph at the trailing end of the field becomes the equaliser. The bars
     move with the capture level at about 24 fps.
   - With `live = true`, recognised text appears at the caret as it arrives, dimmed
     until it is final. Text already in the field stays where it is.
   - Speaking while the agent talks stops the playback (barge-in): the launcher
     dispatches a stop to the service first.
2. **Release.** The final pass replaces the dimmed text, the bars settle back into the
   mic glyph, and the field is an ordinary field again: edit it, then press Return.
   With `send_on_release = true` it is sent at once. A prompt sent to an agent is
   tagged as spoken, so the service knows to answer aloud.
3. **Speaking.** The service buffers `ChatDelta` text for the turn, strips markdown,
   drops code blocks, tool output and reasoning, and cuts it into sentences. It
   synthesises sentence n+1 while sentence n plays, so the first words come after one
   sentence, not after the whole answer.
4. **The island.** A live activity, app id `org.otto.agents.voice`, opens with the
   first sentence and shows it as a caption. The bars follow the `otto-agents-voice`
   node. The caption moves on sentence by sentence. Tapping the island stops
   speaking; the activity closes a moment after `ChatTurnComplete` and the last
   sentence.

## Milestones

Milestones 2 to 4 are the first target. Each one merges on its own.

### 1. Land the music island

Implemented on the `otto-voice` branch, on top of `main` (commits `4cc0cc6f` and
`57c9516c`). It replaces the older `feat/island-music` branch, which polls
`playerctl` and `wpctl` and is not carried forward. What is there:

- `otto-islands/src/audio_viz.rs`:
  - `LevelMeter::new()` and `set_active(bool)`. The meter captures the monitor of the
    default sink and follows the default when it changes. The stream exists only
    while the meter is active. `buffer_level(bytes, channels)` is the pure maths.
  - `BarAnimator::step(level, seed)`: per-bar sines scaled by the level, with phases
    seeded from the track title.
  - `draw_bars(canvas, rect, levels, colour, BarStyle::{Mini, Compact(n), Large})`.
- `otto-islands/src/mpris.rs` follows players over D-Bus (zbus): `NameOwnerChanged`,
  `PropertiesChanged` and `Seeked`, plus a once-a-second position read while a track
  plays. The transport controls and `Raise` are MPRIS calls. Album art loads off the
  render thread.
- `music.rs` owns `MusicMonitor` (playback, meter, bars) and `MusicActivityRenderer`.
  An island counts as music by its `app_id` (`org.otto.music`), not by being
  `ActivitySource::Internal`.
- `main.rs`: each island has an optional `Visualiser`, a child subsurface redrawn at
  ~24 fps by `redraw_music` while a track plays and the island is on screen. The
  rest of the island is retained.
- `focus_watcher` lives in otto-kit (`otto_kit::utils::focus_watcher`) and can
  activate a window as well as read the focused one.
- `specs/dynamic-island.md` and `specs/notification-island.md` are synced.
- 24 unit tests across `audio_viz`, `mpris` and `music`.

Left before it merges:

- A PR for the music island on its own, ahead of any voice work.
- The manual pass from `specs/dynamic-island.md` on a live session.
- Review `focus_watcher`: it opens a second Wayland connection and keeps global
  state.

### 2. `otto-voice` crate (capture and recognition) and shared bars

- New crate `components/otto-voice`:
  - `AudioSource` trait. The PipeWire source captures 16 kHz mono f32 from the
    default source and hands out buffers. The test source reads a WAV file or a
    sample vector.
  - `LevelMeter` and `buffer_level` move here from `otto-islands`, with a target:
    `Target::{DefaultSinkMonitor, DefaultSource, NodeName(String)}`.
  - WAV encoding by hand (a 44-byte header, no crate).
  - `Recogniser` trait with the `whisper`, `openai` and `command` providers, run off
    the caller's thread with a timeout. Each request carries the language.
  - `WhisperSync`: the capture → ring buffer → recogniser loop, with one request in
    flight, partial and final results, and the cut at a quiet stretch. It takes an
    `AudioSource` and a `Recogniser`, so it runs in tests on a WAV file and a
    scripted recogniser.
  - `SttConfig`, the parsed `[speech_to_text]` table, shared by the compositor
    (which validates it) and the launcher (which uses it).
- `BarAnimator`, `BarStyle` and `draw_bars` move to otto-kit. `BarAnimator` takes a
  plain seed, so a mic with no track title still gets its own phases.
- `otto-islands` switches to both, with no change in behaviour. Its tests move with
  the code.

### 3. The engine in Otto's config

- `[speech_to_text]` in `Config` (`src/config/mod.rs`), with its schema entries, all
  applied live.
- `org.otto.Settings` serves them through `Get` and `GetAll` and emits `Changed` when
  they change. The launcher reads them at start and follows `Changed`.
- `otto_config.example.toml`, and `docs/user/` configuration reference.
- A Voice section in otto-settings is optional here. The schema makes it cheap, and
  it can follow once the first target works.

### 4. Speech to text in the launcher

- **Listening state machine** (`otto-launcher/src/voice.rs`): idle → listening →
  finishing → idle. It owns the capture stream, the rolling clip, the in-flight
  recogniser request and the partial text. A newer partial result replaces an older
  one, and a result that arrives after release is dropped unless it is the final one.
- **The equaliser in the field.** The field is a lay-rs layer (`Palette::field`). The
  bars get their own child layer at the field's trailing edge, where the mic glyph
  sits, drawn with `draw_bars` in `BarStyle::Compact`. Only that small layer is
  redrawn at ~24 fps, while the text layer stays retained. `TextInputStyle` reserves
  trailing padding so text never runs under the bars.
- **Live text in `TextInput`.** otto-kit's field gains a pending range: text drawn
  dimmed at the caret, not part of `value()`, replaced wholesale by each partial and
  committed by the final pass (`set_pending(text)`, `commit_pending()`,
  `clear_pending()`). Typing during listening keeps what you typed and moves the
  pending text after it. After a commit, the launcher refilters the list as it would
  after typing.
- **Keys and pointer.** The mic key held in the field, the mic glyph as a click
  target, and the global `KeyAction` that opens the launcher already listening.
- **Errors.** An unreachable engine, a failing command or a missing API key variable
  shows as the field's placeholder and the bars stop. Nothing blocks the field.
- **Spoken prompts.** A prompt sent to an agent after dictation carries
  `_meta: { "otto.voice": true }` on its `Message` in `ChatPendingMessageSet`. AHP's
  `Message._meta` is meant for this kind of host context, so no extension field is
  needed. Nothing reads it until milestone 5.
- **Docs.** `specs/voice.md` from the template, covering dictation, and a user page
  on running whisper.cpp's server, added to `website/build-docs.sh`.

### 5. Text to speech in otto-agents

- `AudioSink`, the `Synthesiser` trait and its providers, the sentence splitter and
  the markdown stripper go into `otto-voice`.
- `[tts]` in `agents.toml`, checked by `doctor`.
- A per-session speaker that folds chat actions, splits sentences, synthesises ahead
  and plays in order.
- Stop and barge-in as an otto extension action, dispatched by the launcher and by
  the island.
- One speaker at a time across sessions: a new spoken turn stops the old one.

### 6. The voice island

- `UpdateActivityBody(id, body)` on `org.otto.Island1`. Today `UpdateActivity` can
  change only the title and progress.
- The caption shows the current sentence (it fits the three-line body limit, so no
  scrolling).
- Generalise the visualiser, which is music-only today:
  - The meter and `BarAnimator` move out of `MusicMonitor` into the `Visualiser`, one
    per island, so two islands can animate at once from different sources. Music
    keeps the default sink monitor; the voice island uses
    `NodeName("otto-agents-voice")`.
  - `Visualiser` is created for any island that asks for one, not on
    `app_id == org.otto.music`. The frame loop in `redraw_music` becomes a
    visualiser loop, and the bar layout comes from the island's renderer, not from
    `MusicActivityRenderer::eq_layout`.
- Tap to stop: islands calls the service's stop action.

### 7. Documentation

- Extend `specs/voice.md` to the spoken answer; update `specs/dynamic-island.md`.
- `docs/developer/agents.md` (the `[tts]` table) and a user page section on piper.

## Testing

CI has no microphone, no speakers, no models and no network, so every automated test
runs against fakes. Real engines are exercised by an opt-in round trip and a manual
pass on a live session.

**Seams that make this possible.** `otto-voice` puts three things behind traits:
- `AudioSource`: capture. PipeWire in production, a WAV file or a sample vector in
  tests.
- `AudioSink`: playback. PipeWire in production, a recording sink in tests that keeps
  every buffer with a timestamp.
- `Recogniser` / `Synthesiser`: the engines. The fakes are a `command` provider
  backed by a shell script, and an HTTP stub on `127.0.0.1:0` written with
  `std::net::TcpListener` (no crate). The stub answers `/inference`,
  `/v1/audio/transcriptions` and `/v1/audio/speech`, and records the requests.

### Layers

| Layer | Lives in | What it proves |
|---|---|---|
| Unit: audio | `otto-voice` | The WAV header is byte-correct for 16 kHz mono (compared against a fixture). Resampling and f32 to s16 conversion keep level within tolerance. `buffer_level` on silence, sines and multi-channel buffers (moved from islands) |
| Unit: text | `otto-voice` | The sentence splitter handles abbreviations, decimals, lists, URLs and a sentence split across two `ChatDelta`s. The markdown stripper drops code blocks, links' targets, tables and tool output |
| Providers | `otto-voice/tests/providers.rs` | Each provider against the stub or script: the request carries the right fields, audio format and language, errors and timeouts surface as errors, a slow engine never blocks the caller |
| Whisper sync | `otto-voice` | With a WAV source and a scripted recogniser: a pass about once a second, never two requests in flight, a slow engine gets the longer clip next, release gives exactly one final result, a long clip is cut at a quiet stretch and the text on both sides survives |
| Otto config | `src/config`, `src/settings/schema.rs` | `[speech_to_text]` parses, a missing table turns dictation off, a bad provider or language is rejected, `system` resolves from `locales`, every key has a schema entry applied live, `Changed` fires on a change |
| Bars | otto-kit | `BarAnimator` deterministic per seed, settling to a shimmer on silence, rising with level (moved from islands). `draw_bars` pixel check with fixed levels for each style |
| Launcher | `otto-launcher` `#[cfg(test)]` | The listening state machine with a scripted recogniser and a WAV source: partial replaces partial, the final pass replaces it, a late partial is dropped, typing during listening is kept, `send_on_release` sends once, an engine error reaches the placeholder, a config change swaps the engine, barge-in dispatches stop before capture starts |
| Field bars | `otto-launcher` headless | While listening, a frame redraws the bars layer and not the text layer; the bars stay inside the trailing padding; the glyph comes back on release |
| Text field | `otto-kit` `text_input` | The pending range: drawn dimmed, never part of `value()`, replaced by the next partial, committed by the final pass, kept after typed text; caret behaviour unchanged |
| Config: TTS | `otto-agents` `config.rs` | `[tts]` parses, unknown keys fail, a missing table turns speech off, `api_key_env` never lands in `RootState.config` |
| Speaker | `otto-agents/tests/voice.rs` | Against the echo backend with a fake synthesiser and a recording sink: a spoken prompt is answered aloud, a typed one is not (`speak = "voice"`), sentences play in order, sentence n+1 is synthesised before sentence n finishes, the first sentence plays before `ChatTurnComplete`, code is never spoken, stop and barge-in cut playback within one buffer, a new spoken turn stops the old one |
| Doctor | `otto-agents/tests/voice.rs` | `doctor` reports a missing command, an unreachable server and a working synthesis |
| Islands | `otto-islands` `#[cfg(test)]` | `UpdateActivityBody` changes the body and triggers a redraw; a voice activity gets a visualiser without being music; it renders its caption and bars; two visualisers animate from different levels |
| Round trip (opt-in) | `otto-voice/tests/round_trip.rs`, `OTTO_VOICE_E2E=1` | Real engines: a fixed WAV is transcribed and the words match after normalising. Once TTS exists, the synthesiser speaks a sentence and the recogniser hears it back. This is how to judge a new engine or voice |

All of these except the round trip run in `scripts/ci.sh` and in CI's existing
`cargo test --lib --workspace` and `cargo test -p otto-agents` steps. Any test asserting
English wording calls `i18n::pin_source_locale()` first.

### PipeWire

The PipeWire paths (`AudioSource`, `AudioSink`, `LevelMeter`) get one opt-in test,
`OTTO_VOICE_PW=1`. It starts a private PipeWire and WirePlumber on a temporary runtime
dir with a null sink and a null source. It feeds a tone to the source and checks that
`AudioSource` captures it at 16 kHz mono. It plays a tone through `AudioSink` on the
`otto-agents-voice` node and checks that `LevelMeter` with `Target::NodeName` sees it.
It never touches the user's session audio.

### Manual pass

On a live session, before each milestone merges:
- Hold the key and speak: the bars move with your voice, words appear dimmed as you
  speak, and the final text replaces them on release. Edit and send.
- Type a few words, then dictate: the typed words stay and the dictation follows.
- Switch `speech_to_text.provider` while the launcher is open: the next dictation
  uses the new engine.
- Stop the engine: the field says so, nothing hangs.
- An answer with code and a list: code is skipped, the island caption follows the
  speech, the bars move with the voice.
- Talk over the agent: it stops at once. Tap the island: it stops.
- Close the launcher mid-answer: speech carries on.
- Play music at the same time: the voice ducks it, and the music island and the voice
  island don't fight for the same place.

`docs/testing.md` gains the new layers when milestone 2 lands.

## Open questions

- **TTS config.** Speech to text is Otto config; should `[tts]` follow it into
  `config.toml` so both engines are set in one place, with otto-agents reading it
  over `org.otto.Settings`?
- **Mic in use.** Should a small island mark the microphone as live while the launcher
  listens, for privacy? It would reuse `LevelMeter` with `Target::DefaultSource`.
- **Where the island gets its stop.** Islands calling otto-agents over AHP adds an
  AHP client to islands. The alternative is a small D-Bus method on the service, as
  otto-agents already speaks D-Bus to islands for questions. Islands now keeps a
  zbus session connection for MPRIS, so the D-Bus method is the cheaper choice.
- **Music bars during speech.** Speech plays through the default sink, so the music
  island's meter picks it up. Options: leave it, have the music meter skip the
  `otto-agents-voice` node, or hide the music bars while the voice island is up.
- **Wake word.** Out of scope; push-to-talk only.
