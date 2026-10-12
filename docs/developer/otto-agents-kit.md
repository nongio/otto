# otto-agents-kit

The shared parts of Otto's agent UIs: the feed of `otto-agents` sessions and their rows, the keys that walk a list and edit its field, and the Ask chat with its log. It is a library: each host links it, keeps its own window and field, and talks to `otto-agents` over AHP through it.

It sits in the **kit** layer (see [Architecture Rules](architecture.md)) and is used by `otto-launcher`, `otto-canvas` and `otto-preview`.

## Dependencies
- **Internal:** `otto-kit` (drawing, text input, the item list rows), `otto-md-kit` (Markdown in the chat log), `otto-agents-client` (socket, URIs, the AHP connection).
- **External:** `ahp` and `ahp-types`, pinned to the same version as `otto-agents-client`; `tokio` on the connection thread. The `dictation` feature pulls in `otto-kit/dictation`, which brings the microphone and the speech engine's client. The launcher and Studio turn it on; the canvas leaves it off.

## Develop
- Tests: `cargo test -p otto-agents-kit`; add `--features dictation` to build `src/dictation.rs` too.
- To see it live, run a host against a running `otto-agents`, e.g. `cargo run -p otto-launcher -- --ask`. `OTTO_AGENTS_URL` points the feed and the chat at another service.

## Code map
- `src/link.rs`: the connection thread. A host's loop is not async, so `Link` runs the AHP client on a thread of its own and wakes the loop through `Link::poll_fd`.
- `src/sessions.rs`: the session rows and `SessionFeed`, which lists again when the service announces a change.
- `src/chat/`: the Ask conversation, its transcript and its input.
- `src/log/`: the chat log, laid out at the host's width, painted a band at a time, with selection.
- `src/keys.rs`: the keys a field and its list answer to.
- `src/opener.rs`: where a picked session opens (Ask, Studio, or the terminal it is in).
- `src/dictation.rs`: Ctrl+D dictation into a host's field (feature `dictation`).

## Rules
- `item` and `rows` are re-exports of `otto_kit::components::item_list`, which the settings sidebar search uses too. Change the rows there.
- Keys and dictation live here so the launcher, the canvas and Studio behave the same. Do not copy them into a host.
- `opener.rs` routes a session by `otto.app` and `otto.subject` in its `_meta`; `components/otto-preview/src/chat.rs` writes them. Keep both sides in step.

## See also
- [Agents](agents.md), [Side canvas](side-canvas.md), [otto-agents](otto-agents.md), [otto-kit](otto-kit.md).
- Specs: [dictation](../../specs/dictation.md), [preview app](../../specs/preview-app.md).
