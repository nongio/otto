# otto-preview

Studio: one file in a window of its own, drawn exactly as Peek draws it over the file list. People see it as Studio; the program, its app id and its bus name are `otto-preview`. One process per session: the first start owns `org.otto.Preview1` and later starts hand their file to it. Beside the document it can host an agent chat, and it fills and marks up PDFs, saving into the file.

- **Layer:** app. Used by nothing (an app). Agents reach its document tools through `otto-preview --mcp`.

## Dependencies
- **Internal:** `otto-peek` (the preview session and its sandboxed decoder), `otto-kit` (window, chrome, drawing), `otto-agents-kit` with `dictation` and `otto-agents-client` (the chat and its log, session URIs), `otto-media-kit` without default features (playback; the GStreamer worker is the separate `otto-media-worker` binary), `otto-pdf` (form filling and annotations).
- **External:** owns `org.otto.Preview1`; calls the desktop portal's `SaveFile` for Save As. PDFium, which `otto-pdf` loads from `libpdfium.so` when a PDF is first edited. A running `otto-agents` for the chat; its sessions carry `_meta.otto.mcpServers` naming this program with `--mcp`. The Peek decoders' external programs.

## Develop
- `cargo run -p otto-preview -- ~/Pictures/photo.jpg`. `--chat` opens the chat beside it; `--session URI` carries on an agent session (and implies `--chat`). Against a dev compositor: `cargo run -- --winit`, then `WAYLAND_DISPLAY=wayland-1 cargo run -p otto-preview -- FILE`.
- A running Studio owns `org.otto.Preview1` and a new start only hands it the file. Quit the running one first to test a new build.
- It is launched from its desktop entry (`resources/otto-preview.desktop`, `Exec=otto-preview %f`), by `xdg-open`, and by the agent UIs when a session is about a file.
- `otto-preview --mcp` is the MCP server an agent starts over stdio, with the document in `OTTO_PREVIEW_DOC`. It holds no state and forwards each tool to the running Studio over `org.otto.Preview1`.
- `RUST_LOG` filters logging (default `info`).
- Tests: `cargo test -p otto-preview`. Ignored tests render PNGs to look at (`OTTO_RENDER_OUT`); the chat one needs a running `otto-agents` and `OTTO_RENDER_SESSION`.

## Where things are
- `src/main.rs` holds the arguments, the `--mcp` branch and the hand-off to a running instance; `src/instance.rs` is `org.otto.Preview1`.
- `src/app.rs` is the window and its input; `src/viewer.rs` and `src/content.rs` are what it shows around and under the Peek session.
- The chat and its session `_meta` are in `src/chat.rs`; the agent's document tools in `src/mcp.rs` and `src/pdf_tools.rs`.
- PDF editing centres on `src/edit_queue.rs`. Copies kept while an agent works go under `$XDG_STATE_HOME/otto-preview/versions/` (`src/versions.rs`).

## Rules
- `otto_peek::run_worker_if_requested()` stays the first call in `main`, before `--mcp` and before any thread.
- `--mcp` never touches the display and keeps no state; every tool goes through the running instance.
- `otto-agents` only starts MCP servers given as an absolute path to a program named `otto-*` (`components/otto-agents/src/agent.rs`). `session_meta` in `src/chat.rs` uses `current_exe()` for that reason.
- `otto.app` and `otto.subject` in a session's `_meta` route it back here from any session list (`components/otto-agents-kit/src/opener.rs`). Keep both sides in step.
- Every PDF edit, undo, redo and save runs through `src/edit_queue.rs`, one at a time. `otto-pdf` owns PDFium on one thread.
- The menus, the keys and the shortcuts sheet read `src/commands.rs`. Add a command there, not in one of them.

## See also
- [Spec](../../specs/preview-app.md), [Peek spec](../../specs/peek.md).
- [File previews](../../docs/developer/file-previews.md), [Agents](../../docs/developer/agents.md).
- [otto-peek](../otto-peek/AGENTS.md), [otto-pdf](../otto-pdf/AGENTS.md), [otto-agents-kit](../otto-agents-kit/AGENTS.md).
