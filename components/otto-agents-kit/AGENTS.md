# otto-agents-kit

The shared parts of Otto's agent UIs: the feed of `otto-agents` sessions and their rows, the keys that walk a list and edit its field, and the Ask chat with its log. Each host links it, keeps its own window and field, and talks to `otto-agents` over AHP through it.

How it works: [Agents](../../docs/developer/agents.md).

- **Layer:** kit. Used by `otto-launcher`, `otto-canvas` and `otto-preview`.

## Dependencies
- **Internal:** `otto-kit` (drawing, text input, the item list rows), `otto-md-kit` (Markdown in the chat log), `otto-agents-client` (socket, URIs, the AHP connection).
- **External:** `ahp` and `ahp-types`, pinned to the same version as `otto-agents-client`; `tokio` on the connection thread. The `dictation` feature pulls in `otto-kit/dictation`, which brings the microphone and the speech engine's client. The launcher and Studio turn it on; the canvas leaves it off.

## Develop
- Tests: `cargo test -p otto-agents-kit`; add `--features dictation` to build `src/dictation.rs` too.
- To see it live, run a host against a running `otto-agents`, e.g. `cargo run -p otto-launcher -- --ask`. `OTTO_AGENTS_URL` points the feed and the chat at another service.

## Where things are
- `src/link.rs` is the connection thread. A host's loop is not async, so `Link` runs the AHP client on its own thread and wakes the loop through `Link::poll_fd`.
- `src/sessions.rs` is the session rows and the `SessionFeed`; `src/opener.rs` decides where a picked session opens.
- The Ask conversation is in `src/chat/`, its log (laid out at the host's width, painted a band at a time) in `src/log/`.
- `src/keys.rs` and `src/dictation.rs` are the list keys and Ctrl+D dictation into a host's field.

## Rules
- `item` and `rows` are re-exports of `otto_kit::components::item_list`, which the settings sidebar search uses too. Change the rows there.
- Keys and dictation live here so the launcher, the canvas and Studio behave the same. Do not copy them into a host.
- `opener.rs` routes a session by `otto.app` and `otto.subject` in its `_meta`; `components/otto-preview/src/chat.rs` writes them. Keep both sides in step.

## See also
- [Agents](../../docs/developer/agents.md), [Side canvas](../../docs/developer/side-canvas.md).
- Specs: [dictation](../../specs/dictation.md), [preview app](../../specs/preview-app.md).
- [otto-agents](../otto-agents/AGENTS.md), [otto-agents-client](../otto-agents-client/AGENTS.md), [otto-kit](../otto-kit/AGENTS.md).
