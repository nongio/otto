# otto-preview

Studio: one file in a window of its own, drawn exactly as Peek draws it over the file list. People see it as Studio; the program, its app id and its bus name are `otto-preview`. It is a Wayland client, one process per session: the first start owns `org.otto.Preview1` and later starts hand their file to it. Beside the document it can host an agent chat, and it fills and marks up PDFs, saving into the file.

It sits in the **app** layer (see [Architecture Rules](architecture.md)) and is used by nothing: it is an app. Agents reach its document tools through `otto-preview --mcp`.

## Dependencies
- **Internal:** `otto-peek` (the preview session and the sandboxed decoder under it), `otto-kit` (window, chrome, drawing), `otto-agents-kit` with `dictation` and `otto-agents-client` (the chat and its log, session URIs), `otto-media-kit` without default features (playback; the GStreamer worker is the separate `otto-media-worker` binary), `otto-pdf` (form filling and annotations).
- **External:**
  - D-Bus: owns `org.otto.Preview1`; calls the desktop portal's `SaveFile` for Save As.
  - PDFium, which `otto-pdf` loads from `libpdfium.so` when a PDF is first edited.
  - A running `otto-agents` for the chat. Its sessions carry `_meta.otto.mcpServers` naming this program with `--mcp`.
  - The Peek decoders' external programs (see [otto-peek](otto-peek.md)).

## Develop
- `cargo run -p otto-preview -- ~/Pictures/photo.jpg`. `--chat` opens the chat beside it; `--session URI` carries on an agent session (and implies `--chat`). Against a dev compositor: `cargo run -- --winit`, then `WAYLAND_DISPLAY=wayland-1 cargo run -p otto-preview -- FILE`.
- If a Studio is already running in the session it owns `org.otto.Preview1`, and a new start only hands it the file. Quit the running one first to test a new build.
- It is launched from its desktop entry ([resources/otto-preview.desktop](../../resources/otto-preview.desktop), `Exec=otto-preview %f`), by `xdg-open`, and by the agent UIs when a session is about a file.
- `otto-preview --mcp` is the MCP server an agent starts, over stdio, with the document in `OTTO_PREVIEW_DOC`. It holds no state and forwards each tool to the running Studio over `org.otto.Preview1`.
- `RUST_LOG` filters logging (default `info`).
- Tests: `cargo test -p otto-preview`. Ignored tests render PNGs to look at (`OTTO_RENDER_OUT`); the chat one needs a running `otto-agents` and `OTTO_RENDER_SESSION`.

## Code map
- `src/main.rs`: arguments, the `--mcp` branch, the hand-off to a running instance.
- `src/instance.rs`: `org.otto.Preview1`, for later starts and for the MCP tools.
- `src/app.rs`: the window, its input and the work it sends off the UI thread. `src/viewer.rs`: what the window shows around the Peek session. `src/content.rs`: the preview under the chrome.
- `src/chrome.rs`, `src/sidebar.rs` (page thumbnails), `src/commands.rs` (one list for menus, keys and the shortcuts sheet in `src/shortcuts.rs`).
- `src/chat.rs`: the chat beside the document and the session `_meta` it creates. `src/mcp.rs`, `src/pdf_tools.rs`: the agent's document tools. `src/marks.rs`: marks drawn by the person and by the agent.
- `src/editing.rs`, `src/edit_queue.rs`, `src/signature.rs`, `src/confirm.rs`, `src/save_as.rs`: PDF editing, its one queue of edits, signatures, unsaved-change questions, Save As.
- `src/versions.rs`: copies of the file while an agent works on it, under `$XDG_STATE_HOME/otto-preview/versions/`.

## Rules
- `otto_peek::run_worker_if_requested()` stays the first call in `main`, before `--mcp` and before any thread.
- `--mcp` never touches the display and keeps no state; every tool goes through the running instance.
- `otto-agents` only starts MCP servers given as an absolute path to a program named `otto-*` (`components/otto-agents/src/agent.rs`). `session_meta` in `src/chat.rs` uses `current_exe()` for that reason.
- `otto.app` and `otto.subject` in a session's `_meta` route it back here from any session list (`components/otto-agents-kit/src/opener.rs`). Keep both sides in step.
- Every PDF edit, undo, redo and save runs through `src/edit_queue.rs`, one at a time. `otto-pdf` owns PDFium on one thread.
- The menus, the keys and the shortcuts sheet read `src/commands.rs`. Add a command there, not in one of them.

## See also
- [Spec](../../specs/preview-app.md), [Peek spec](../../specs/peek.md).
- [File previews](file-previews.md), [otto-peek](otto-peek.md), [otto-pdf](otto-pdf.md), [otto-agents-kit](otto-agents-kit.md), [Agents](agents.md).
