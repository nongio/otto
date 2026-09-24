# Dictation engines

otto-dictate sends audio to `http://127.0.0.1:8080/inference` (whisper.cpp's
server API, answered in WebVTT). Two engines can serve it; one runs at a time.

| Engine | Unit | Languages | Notes |
|---|---|---|---|
| Parakeet TDT 0.6B v3 | `otto-stt-parakeet` | 25 European, detected | times each word; the default |
| Whisper small.en | `otto-stt-whisper` | English | honours `OTTO_DICTATE_LANGUAGE` and the prompt |

Both run on the GPU through Vulkan. Switch with either of:

```sh
systemctl --user enable --now otto-stt-parakeet
systemctl --user enable --now otto-stt-whisper
```

Starting one stops the other, and disable the one you left so a reboot
brings back your choice (`systemctl --user disable otto-stt-whisper`).

`install.sh [parakeet|whisper]` builds both servers, fetches the models into
`~/.local/share/otto-voice-poc`, installs otto-dictate into `~/.local/bin`
and starts it with the session through `~/.config/autostart`.

whisper.cpp ships no server for Parakeet, so `parakeet-server/` is one: it is
built inside the whisper.cpp checkout, next to `parakeet-cli`.
