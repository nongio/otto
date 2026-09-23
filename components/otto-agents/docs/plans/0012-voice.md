# 0012: Voice (speech to text and text to speech)

**Status:** Draft

## Goal

You can talk to an agent and hear it answer. While you speak, your words appear in the
launcher's field as they are recognised. While the agent speaks, the island shows what
it is saying, with the equaliser from the music island moving to its voice. Both
engines are services you choose in `agents.toml`, the same way agents are.

## Decisions

- **Two pipelines, two owners.**
  - *Speech to text* runs in the **launcher**. It owns the field, the key that starts
    listening and the moment the prompt is sent, so there is no round trip through the
    service while you talk.
  - *Text to speech* runs in **otto-agents**. The service already sees every
    `ChatDelta`, keeps running when the launcher closes, and is the one place that
    knows when a turn ends. The launcher closing never cuts the agent off mid-sentence.
- **One shared crate, `otto-voice`.** Capture, playback, WAV encoding, sentence
  splitting and the provider clients live in a leaf crate that the launcher and the
  service both use. It has no AHP or UI code.
- **Engines are external.** No speech models are linked into Otto. Whisper runs as a
  server (whisper.cpp `whisper-server`, faster-whisper, or any OpenAI-compatible
  endpoint) or as a command. Piper and similar engines run as commands. Models are
  large and fast-moving, and linking whisper.cpp would add a long C++ build to every
  Otto build.
- **Whisper sync.** Recognition is synchronous: a whole clip goes in and text comes
  out, with no streaming protocol. Live text in the field comes from re-sending the
  rolling clip about once a second while you speak (as whisper.cpp's `stream` does).
  The final pass on release replaces the partial text.
- **Audio through PipeWire.** The `pipewire` crate is already a workspace dependency.
  Capture asks for 16 kHz mono (Whisper's input). Playback is a stream with a fixed
  node name, `otto-agents-voice`, and `media.role = Communication`, so ducking and
  routing work like any other voice app, and islands can find the node by name.
- **HTTP through `ureq`**, which the music island already brings in. No `reqwest`.
- **Secrets never go in `agents.toml`** (0005). A hosted engine names its key with
  `api_key_env`, or uses the credential store once 0005 lands.
- **The island shows the reply, not the prompt.** What you say goes in the field,
  where you can correct it before sending. What the agent says goes in the island,
  because the launcher may be closed by then.

## Configuration

New top-level tables in `agents.toml`. `ConfigFile` is `deny_unknown_fields`, so both
are added to it explicitly. Either table can be left out, which turns that half off.

```toml
[stt]
provider = "whisper"               # whisper | openai | command
url = "http://127.0.0.1:8080/inference"
language = "auto"
live = true                        # partial text while you speak
send_on_release = false            # true: release the key and the prompt goes

[tts]
provider = "piper"                 # piper | openai | command
command = ["piper", "--model", "~/.local/share/piper/en_GB-alba-medium.onnx", "--output-raw"]
sample_rate = 22050
speak = "voice"                    # voice | always | never
```

A hosted or OpenAI-compatible engine (OpenAI, Kokoro-FastAPI, LocalAI, a remote
faster-whisper):

```toml
[tts]
provider = "openai"
url = "http://127.0.0.1:8880/v1/audio/speech"
model = "kokoro"
voice = "af_bella"
api_key_env = "OPENAI_API_KEY"     # optional
```

- `whisper` posts a WAV to whisper.cpp's `/inference`; `openai` posts to
  `/v1/audio/transcriptions` or `/v1/audio/speech`; `command` writes WAV to stdin and
  reads text from stdout (STT), or writes text to stdin and reads raw PCM from stdout
  (TTS).
- `speak = "voice"` speaks a reply only when the prompt was spoken. That is the
  default, so typing never makes the desktop talk.
- An agent may override the voice with `voice = "…"` in its `[[agents]]` entry, so
  two agents sound different. Later, not in the first cut.
- The `[stt]` and `[tts]` values are published in `RootState.config` like
  `default_agent`, so the launcher reads the STT settings over AHP instead of parsing
  the file itself.
- `otto-agents doctor` checks each engine: the server answers, the command exists,
  a one-word round trip works.

## Flow

1. **Listening.** Hold the mic key in the launcher (or click the mic in the field). A
   global shortcut opens the launcher already listening; it is a new `KeyAction`, so
   it needs arms in `actions.rs` and both `input_handler.rs` paths.
   - The field shows *Listening…* and a small level meter from the capture samples.
   - With `live = true`, recognised text fills the field as it arrives, dimmed until
     it is final.
   - Speaking while the agent talks stops the playback (barge-in): the launcher
     dispatches a stop to the service first.
2. **Release.** The final pass replaces the partial text, and the field is an ordinary
   field again: edit it, then Return. With `send_on_release = true` it is sent at once.
   The message is tagged as spoken, so the service knows to answer aloud.
3. **Speaking.** The service buffers `ChatDelta` text for the turn, strips markdown,
   drops code blocks, tool output and reasoning, and cuts it into sentences. It
   synthesises sentence n+1 while sentence n plays, so the first words come after one
   sentence, not after the whole answer.
4. **The island.** A live activity, app id `org.otto.agents.voice`, opens with the
   first sentence and shows it as a caption. The equaliser follows the
   `otto-agents-voice` node. The caption moves on sentence by sentence. Tapping the
   island stops speaking; the activity closes a moment after `ChatTurnComplete` and
   the last sentence.

## Milestones

### 1. Land the music island

`feat/music-island-on-main` is 148 commits behind `main` and conflicts in 17 hunks of
`otto-islands/src/main.rs`, so it is re-applied by hand rather than merged. On the way
in:

- Extract `otto-islands/src/audio_viz.rs` out of `music.rs`:
  - `LevelMeter::spawn(target)`, with `Target::{DefaultSinkMonitor, DefaultSource,
    NodeName(String)}`. The capture loop is already generic apart from the hard-coded
    `MEDIA_ROLE=Music` and `STREAM_CAPTURE_SINK`.
  - `BarAnimator::step(level, active, seed)`, the per-bar phases and easing now in
    `MusicMonitor::tick`.
  - `draw_bars(canvas, rect, levels, colour)`, from `draw_equalizer_*`.
- The equaliser subsurface and its 24 fps pacing in `main.rs` key on "this island has
  a visualiser", not on `IslandKind::Music`.
- Music detection stops treating every `ActivitySource::Internal` activity as music.
- Review `focus_watcher` (a second Wayland connection and global state) before it goes
  into otto-kit.
- Sync `specs/dynamic-island.md` and `specs/notification-island.md`.

### 2. `otto-voice` crate and configuration

- Capture (16 kHz mono f32), playback (named node), WAV encoding by hand (a 44-byte
  header, no crate), the sentence splitter and markdown stripper, the three provider
  kinds for each direction.
- `[stt]` / `[tts]` in `ConfigFile`, published in `RootState.config`, checked by
  `doctor`.
- The `AudioSource`, `AudioSink`, `Recogniser` and `Synthesiser` traits, with their
  fakes, so every later milestone can be tested without audio or models (see
  Testing).

### 3. Speech to text in the launcher

- Mic key and mic button, the level meter, partial and final text in `TextInput`
  (`set_value` then `refilter`).
- A dimmed "not final yet" range in otto-kit's `TextInput`, if the field has no way
  to show one today.
- Tag spoken prompts on `ChatPendingMessageSet` with `_meta: { "otto.voice": true }` on
  the `Message`. AHP's `Message._meta` is meant for exactly this kind of host context,
  so no extension field is needed.

### 4. Text to speech in otto-agents

- A per-session speaker that folds chat actions, splits sentences, synthesises ahead
  and plays in order.
- Stop and barge-in as an otto extension action, dispatched by the launcher and by
  the island.
- One speaker at a time across sessions: a new spoken turn stops the old one.

### 5. The voice island

- `UpdateActivityBody(id, body)` on `org.otto.Island1`. Today `UpdateActivity` can
  change only the title and progress.
- The caption shows the current sentence (it fits the three-line body limit, so no
  scrolling).
- The equaliser from milestone 1 with `Target::NodeName("otto-agents-voice")`.
- Tap to stop: islands calls the service's stop action.

### 6. Documentation

- `specs/voice.md` from the template; update `specs/dynamic-island.md`.
- `docs/developer/agents.md` (the `[stt]` / `[tts]` tables) and a user page on
  setting up whisper and piper, added to `website/build-docs.sh`.
- An otto-settings pane is out of scope, as it is for agents in general (0004's open
  question).

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
| Unit: text | `otto-voice` `#[cfg(test)]` | The sentence splitter handles abbreviations, decimals, lists, URLs and a sentence split across two `ChatDelta`s. The markdown stripper drops code blocks, links' targets, tables and tool output |
| Unit: audio | `otto-voice` | The WAV header is byte-correct for 16 kHz mono (compared against a fixture). Resampling and f32 to s16 conversion keep level within tolerance |
| Unit: config | `otto-agents` `config.rs` | `[stt]` / `[tts]` parse, unknown keys fail, a missing table turns that half off, `api_key_env` never lands in `RootState.config` |
| Providers | `otto-voice/tests/providers.rs` | Each provider against the stub or script: the request carries the right fields and audio format, errors and timeouts surface as errors, a slow engine never blocks the caller |
| Speaker | `otto-agents/tests/voice.rs` | Against the echo backend with a fake synthesiser and a recording sink: a spoken prompt is answered aloud, a typed one is not (`speak = "voice"`), sentences play in order, sentence n+1 is synthesised before sentence n finishes, the first sentence plays before `ChatTurnComplete`, code is never spoken, stop and barge-in cut playback within one buffer, a new spoken turn stops the old one |
| Doctor | `otto-agents/tests/voice.rs` | `doctor` reports a missing command, an unreachable server and a working round trip |
| Launcher | `otto-launcher` `#[cfg(test)]` | The listening state machine with a scripted recogniser: partial text replaces partial text, the final pass replaces it, typing during listening is kept, `send_on_release` sends once, barge-in dispatches stop before capture starts |
| Text field | `otto-kit` `text_input` | The not-final range: shown dimmed, cleared on the final pass, never part of `value()` until final, caret behaviour unchanged |
| Level meter | `otto-islands` `audio_viz` | `LevelMeter`'s maths on synthetic sines and silence, `BarAnimator` deterministic for a fixed seed and settling to rest on silence |
| Islands | `otto-islands` `#[cfg(test)]` | `UpdateActivityBody` changes the body and triggers a redraw; music detection ignores non-music `Internal` activities; a voice activity renders its caption and bars (pixel check with fixed levels, as the dock tests do) |
| Round trip (opt-in) | `otto-voice/tests/round_trip.rs`, `OTTO_VOICE_E2E=1` | Real engines: the synthesiser speaks a fixed sentence, the recogniser transcribes it, and the words match after normalising. One test checks both halves and the config, and it is how to judge a new engine or voice |

All of these except the round trip run in `scripts/ci.sh` and in CI's existing
`cargo test --lib --workspace` and `cargo test -p otto-agents` steps. Any test asserting
English wording calls `i18n::pin_source_locale()` first.

### PipeWire

The PipeWire paths (`AudioSource`, `AudioSink`, the islands `LevelMeter`) get one
opt-in test, `OTTO_VOICE_PW=1`. It starts a private PipeWire and WirePlumber on a
temporary runtime dir with a null sink, plays a tone through `AudioSink` on the
`otto-agents-voice` node, and checks that `LevelMeter` with `Target::NodeName` sees
it. It never touches the user's session audio.

### Manual pass

On a live session, before each milestone merges:
- Hold the key, speak, check the partial and final text; edit and send.
- An answer with code and a list: code is skipped, the island caption follows the
  speech, the bars move with the voice.
- Talk over the agent: it stops at once. Tap the island: it stops.
- Close the launcher mid-answer: speech carries on.
- Stop the engines: the field and the island say so, nothing hangs.
- Play music at the same time: the voice ducks it, and the music island and the voice
  island don't fight for the same place.

`docs/testing.md` gains the new layers when milestone 2 lands.

## Open questions

- **Where the island gets its stop.** Islands calling otto-agents over AHP adds an
  AHP client to islands. The alternative is a small D-Bus method on the service, as
  otto-agents already speaks D-Bus to islands for questions.
- **Mic in use.** Should a small island mark the microphone as live while the launcher
  listens, for privacy? It would reuse `LevelMeter` with `Target::DefaultSource`.
- **Wake word.** Out of scope; push-to-talk only.
