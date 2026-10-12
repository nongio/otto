# otto-dictate

A proof of concept: dictation as a Wayland input method. It runs for the whole session as the seat's input method and waits. `otto-dictate toggle` starts listening in the focused text field; a balloon under the caret shows an equaliser and the words heard so far, and when listening stops the whole text goes into the field as one commit. Speech is recognised by a local speech-to-text server, not in this process.

It sits in the **app** layer (see [Architecture Rules](architecture.md)) and is used by nothing: it is an app.

## Dependencies

- **Internal:** `otto-kit` with its `dictation` feature, for microphone capture (PipeWire), the engine client (`ureq`), word agreement and the equaliser bars; also Skia text and the XDG runtime dir. The launcher reaches the same code through `otto-agents-kit`.
- **External:**
  - `zwp_input_method_v2`, with its keyboard grab and input popup surface, from the compositor (through `wayland-protocols-misc`). It works in any app that supports text input.
  - A speech-to-text server answering whisper.cpp's `/inference` API on `http://127.0.0.1:8080/inference`. The `engines/` directory holds three: Parakeet, Whisper and CrispASR, each a systemd user unit, one at a time.
  - Settings come from `~/.config/otto/dictation.toml`, written by the Settings app's Dictation pane and read again at each start.

## Develop

- Against a dev compositor: `cargo run -- --winit`, then `WAYLAND_DISPLAY=wayland-1 cargo run -p otto-dictate`, and in another shell `cargo run -p otto-dictate -- toggle`. A speech server must be running.
- For real it is not in the packages. `engines/install.sh [parakeet|whisper|crispasr]` builds or fetches the engine, downloads the models to `~/.local/share/otto-voice-poc`, installs `otto-dictate` to `~/.local/bin` and starts it with the session through `~/.config/autostart`. Restart it by killing it and running `otto-dictate` again.
- Switch engines with `systemctl --user enable --now otto-stt-<engine>`; see the engines README.
- Environment overrides for `dictation.toml`: `OTTO_DICTATE_URL`, `OTTO_DICTATE_LANGUAGE`, `OTTO_DICTATE_HOTWORDS_BOOST`. `OTTO_DICTATE_WAV` plays a 16 kHz mono WAV in place of the microphone.
- Logs: `RUST_LOG=otto_dictate=debug` (the default is `otto_dictate=info`). Tests: `cargo test -p otto-dictate`.

## Code map

- `src/main.rs` is the input method, the toggle socket, the listening loop and the commit.
- `src/balloon.rs` draws the balloon under the caret.
- `engines/` holds `install.sh`, the systemd user units, the autostart entry and the Parakeet server source built into whisper.cpp.

## Rules

- The toggle socket is `otto-dictate.sock` in the user's runtime directory, with no `/tmp` fallback: any local user could reach a socket there. Without a runtime directory the program exits with an error.
- Each toggle connection gets one second to send one short line (`toggle`). Keep it that way so a silent client cannot block the next.
- The field is not touched while listening; text is committed once, when listening stops.
- While listening the keyboard is grabbed and no key reaches the app. Escape discards, Backspace drops the last settled phrase, any other key stops.
- `dictation.toml` and its keys are shared with the launcher and the Settings app. Keep them in step with the spec.

## See also

- [Dictation spec](../../specs/dictation.md) and the [engines README](../../components/otto-dictate/engines/README.md).
- [Settings app spec](../../specs/settings-app.md) for the Dictation pane.
