# 0014: MCP gateway (desktop D-Bus as agent tools)

**Status:** Idea

## Goal

An agent running under Otto can use the desktop the way you do: read what is
selected in Files, run the commands you'd run from Files' command palette, arrange
windows, read what you collected in the balloon ([0013](0013-gather.md)). What it
does through an app shows up in that app as if you had done it, names the agent
that asked, and asks you first when it is destructive.

Plain file work (copy, move, rename) is not the gateway's job: an agent already has
a shell for that. The gateway covers what only the desktop knows: what you are
looking at, what you selected, and the actions apps offer on top of files. What an
agent does through an app's command also gets that app's undo: a trash through
Files can be put back with Cmd+Z, an `rm` from the shell can't. That is the test
for adding a command to the gateway: the app adds something the shell doesn't
have, such as undo, the selection, or showing you the result.

Apps stay ordinary desktop apps. They offer plain D-Bus interfaces, written for any
caller, and know nothing about agents. One gateway, `otto-mcp`, turns the parts of
those interfaces that agents may use into MCP tools.

## Decisions

- **Apps speak D-Bus, the gateway speaks agents.** No agent-specific interface in
  any app. Files offers `org.otto.Files1` for the balloon, the CLI, scripts and
  other apps alike; the gateway is one more caller.
- **The gateway owns the agent side.** Which methods are tools, their names,
  descriptions, argument docs and hints live in the gateway, one tool list per
  interface (`otto-mcp/tools/files.toml`, …). An app changing its interface means
  updating its tool list, which is the price of keeping apps agent-free.
- **Allow-list.** A method is a tool only if its tool list names it. A new D-Bus
  method never reaches agents by accident.
- **otto-agents injects it.** ACP's `session/new`, `session/load` and
  `session/resume` take `mcpServers`. otto-agents passes a stdio server,
  `otto-mcp --session <id>`, on every session it creates or loads
  (`acp.rs:708` passes none today). Every agent started from Ask or Sessions gets
  the desktop tools, whatever its harness, with nothing installed per harness.
- **One gateway, all apps.** Tools are grouped by app (`files_selection`,
  `files_trash`, `windows_focus`, `gather_items`, …). The gateway lists the apps' tools on the fly
  and sends `notifications/tools/list_changed` when an app appears or goes.
- **No MCP crate.** MCP over stdio is JSON-RPC over newline-delimited JSON:
  `initialize`, `tools/list`, `tools/call`, one notification. It is written by
  hand on `serde_json`, which the workspace already has. D-Bus goes through `zbus`,
  also already there.

## Tool lists

One TOML file per D-Bus interface, embedded in the binary at build time and
overridable from `~/.config/otto/mcp/` for experiments.

```toml
bus_name = "org.otto.Files1"
path = "/org/otto/Files"
interface = "org.otto.Files1"
prefix = "files"

[[tool]]
name = "selection"
method = "Selection"
description = "The items selected in the focused Files window, and the folder it shows."
read_only = true

[[tool]]
name = "select_matching"
method = "SelectMatching"
description = "Select the items in the focused Files window whose names match a pattern, such as *.png."
args = [{ name = "pattern", description = "A glob pattern: *, ? and [..]." }]

[[tool]]
name = "trash"
method = "Trash"
description = "Move items to the Trash. They can be put back from the Trash or with Undo in Files."
destructive = true
args = [{ name = "paths", description = "Absolute paths. Empty: the focused window's selection." }]
```

- **Types come from D-Bus.** The gateway reads the interface's introspection and
  builds each tool's JSON Schema from the method's signature: `s` string, `b`
  boolean, integer types with their ranges, `as` array of strings, `a{sv}` object,
  structs as fixed arrays or named objects when the tool list names the fields. The
  tool list adds only what D-Bus can't say: names for arguments, descriptions,
  hints.
- **Two extras for interfaces shaped like Shell1:** `json = true` on a method whose
  string result is JSON, and `template = "…"` for a tool that fills a fixed command
  string from its arguments instead of exposing a free-form one.
- **Results** go back as MCP structured content, converted the same way, plus a
  short text rendering for harnesses that only read text.
- **Errors.** A D-Bus error becomes a tool error (`isError: true`) with the error
  name and message, so the agent can react. A missing app or a timeout is a tool
  error too, never a gateway crash.
- **Hints** map to MCP tool annotations: `read_only` → `readOnlyHint`,
  `destructive` → `destructiveHint`, `idempotent` → `idempotentHint`.
- **Checked in CI.** A test loads every embedded tool list against the interface
  XML the app ships (or its zbus introspection) and fails on a missing method, a
  wrong argument count or an unknown type. An app changing its interface breaks
  the gateway's build, not an agent at run time.

## Attribution

Methods that change things take the usual trailing `a{sv} options` argument. A
generic `origin` key names who asked, in words for the user: "Ask: tidy
Downloads". Files puts it in the undo entry and the progress item.

- Any caller can set it: the CLI sets "Terminal", the balloon sets "Gather".
- The gateway sets it from its `--session` id and the session's title, which it
  reads from otto-agents. An agent can't set or change it: the gateway overwrites
  whatever `origin` the arguments carry.
- This is a convention for Otto's interfaces, written down in
  `docs/developer/dbus-conventions.md`, not an agent feature.

## Permissions

- The harness asks before a tool call by its own policy, and destructive and
  read-only hints feed it. otto-agents already turns ACP
  `session/request_permission` into the islands permission request, so the user
  sees one prompt, in Otto's own UI.
- Apps don't prompt for callers. A command's own confirmation (emptying the
  trash) stays as it is for everyone.
- The gateway can override a harness that is too trusting: a tool list may say
  `confirm = true`, and the gateway then asks through otto-agents itself before
  calling. Off by default; for later if a harness ignores hints.
- The session bus is the trust boundary. Anything on it can already call these
  interfaces; the gateway adds no new reach, only a curated view.

## Discovery and activation

- The gateway watches `NameOwnerChanged` for the bus names its tool lists name.
  A tool is listed when its app is on the bus, or can be started by D-Bus
  activation.
- Activation needs the right environment: a bus-activated helper once started
  with a stale `WAYLAND_DISPLAY`. Activated Otto apps take the display from the
  compositor's environment update, not from the systemd user environment at
  login.
- A tool whose app has no window to answer from (Files' selection with no Files
  window) returns an empty result, not an error.

## The CLI

`otto-mcp call files_select_matching --pattern "*.png"` runs one tool
from a terminal, and `otto-mcp list` prints them. Same tool lists, same
conversion, origin "Terminal". It makes the gateway testable by hand and gives
scripts the same curated view agents get.

Harnesses started outside otto-agents don't get the injected server. `otto-agents
plugins` registers `otto-mcp` in each harness's own MCP config, next to the skills
it already installs, for those.

## First interfaces

The pattern already exists: the compositor serves `org.otto.Shell1`
(`docs/developer/shell-dbus-api.md`) and `otto-msg` is a thin CLI over it. The
gateway is another client of the same kind, for agents.

0. **`org.otto.Shell1`** (exists, served by the compositor). The first tool list,
   since it needs no app work:
   - `GetTree`, `GetWorkspaces`, `GetOutputs` as read-only tools. They answer a
     JSON string (`s`), so the tool list marks the result `json = true` and the
     gateway passes it through as structured content rather than a string.
   - `RunCommand` takes i3's free-form command language, so one tool would have
     one permission level for `focus` and `kill` alike. The tool list instead
     defines narrow tools from templates, each with its own hints:
     `windows_focus(con_id)` → `[con_id={con_id}] focus`, `windows_move_to_workspace`,
     `windows_close` (destructive), `workspaces_switch`. Arguments are validated
     against the schema and quoted before substitution, so an agent can't inject
     a second command.
   - `otto-msg` stays the terminal CLI for Shell1; `otto-mcp call` is for testing
     tool lists.
1. **`org.otto.Files1`** (new, in otto-files next to `org.otto.FilePicker1`).
   A few of the palette's commands, not file manipulation in general: copying,
   moving and renaming an agent does with its shell. Each method runs the same
   code as picking the command in the palette (`command.rs` ids in brackets), with
   no palette open:

   | Method | Palette command | Tool |
   |---|---|---|
   | `Selection() → (folder, paths)`, `SelectionChanged` signal | (the window's selection) | `files_selection`, read-only |
   | `SelectMatching(pattern)` | Select Matching (`select_matching`) | `files_select_matching` |
   | `Trash(paths, options)` | Move to Trash (`trash`) | `files_trash`, destructive |
   | `OpenLocation(path, options)` | Go to Path (`go_to_path`) | `files_open_location` |
   | `SearchRecent(query) → items` | Recent (`recent`) + Search (`search`) | `files_search_recent`, read-only |
| `UndoHistory() → entries` | (the undo history) | `files_undo_history`, read-only |
| `Undo(options)` | Undo (`undo`) | `files_undo` |

   - **Selection** reads the focused Files window: the folder it shows and the
     selected paths. `SelectMatching` changes that selection the way the palette
     does, so the agent can say "the PNGs are selected" and you can see them.
   - **Trash** goes through Files' trash path, so it lands in the undo history
     with `origin`, and Put Back works. Empty `paths` means the selection. It is
     the only destructive tool.
   - **OpenLocation** navigates the focused Files window, or opens one if none is
     open. Files also answers `org.freedesktop.FileManager1.ShowItems`, so "show
     in folder" in other apps lands in the same place.
   - **SearchRecent** returns recent files (path, name, when last used) matching
     the query, for the agent to use. It does not change any window.
   - **Undo** undoes the most recent entry in Files' undo history, the same as
     Cmd+Z. `UndoHistory` lists the entries (title, `origin`, when) so the agent
     can see what it is about to undo. With an `origin` in `options`, `Undo` acts
     only if the latest entry has that origin, and fails otherwise. The gateway
     always passes the session's origin, so an agent can take back its own
     actions ("undo that") but never one of yours.
2. **The gathering service** from 0013: `Items`, `AddText`, `AddFile`. An agent
   can read what you collected and add to it.
3. Later, whatever the desktop already has on the bus: notifications, the
   screenshot and region picker, settings (`org.otto.Settings`), windows.

## Milestones

1. **Gateway core.** Stdio MCP server, tool-list loading, introspection-to-schema
   conversion, JSON-string results, command templates, `tools/list` and
   `tools/call` against a fake D-Bus service, the CLI. Ships with the Shell1 tool
   list, so agents can see and arrange windows from the first milestone.
2. **Injected into sessions.** otto-agents passes `otto-mcp --session <id>` in
   `mcpServers`; `otto-agents doctor` checks the gateway starts and lists tools.
3. **`org.otto.Files1`, read side.** `Selection`, `SelectionChanged`,
   `SearchRecent`, and their tool list.
4. **`org.otto.Files1`, actions.** `SelectMatching`, `OpenLocation`, `ShowItems`,
   `Trash` with `origin` in the undo entry, `UndoHistory` and `Undo` limited to
   the caller's own entries.
5. **The gathering service's tools,** once 0013 has the service.
6. **Plugins.** `otto-agents plugins` registers the gateway with harnesses run on
   their own.

## Testing

- **Gateway, unit:** schema from signatures for every D-Bus type, JSON to D-Bus and
  back, hints to annotations, tool-list validation errors.
- **Gateway, integration:** a private `dbus-daemon --session` in a temp dir, a
  small zbus test service implementing a fake interface, and the gateway driven
  over stdio: list, call, error, app appears and disappears
  (`tools/list_changed`), activation, `origin` overwritten.
- **Tool lists vs apps:** the CI check above, per embedded tool list.
- **otto-agents:** against the echo backend, a new, loaded and resumed session all
  carry the gateway in `mcpServers` with the session id.
- **Files:** `org.otto.Files1` against a temp folder in the existing headless
  harness: each method has the same effect as its palette command, `Trash` lands
  in the undo history with its origin, `SearchRecent` reads a seeded recent list.
- **Manual:** in Ask, "select the screenshots in Downloads and trash them":
  Files opens at Downloads, the PNGs get selected, one permission prompt for the
  trash, and Cmd+Z in Files puts them back with the session named in the undo
  entry. Then trash them again and say "undo that": the agent's `files_undo`
  puts them back; after you rename a file by hand, the same request fails
  because the latest entry is yours.

## Open questions

- **Tool names.** `files_trash` vs `files.trash`: MCP allows dots, some harnesses
  don't. Pick the safest common form.
- **More palette commands later.** The command seam is plain data and scripts in
  `~/.config/otto/files-scripts/` plug into it, so a generic `Commands` / `Run`
  pair could expose all of them, scripts included. Left out for now: the short
  list above covers what an agent can't do from a shell.
- **Per-agent reach.** Should `agents.toml` restrict which apps' tools an agent
  gets (a coding agent may not need Files)?
- **Streaming progress.** A long trash (thousands of items): return at once with a job
  id, or use MCP progress notifications while the call runs.
- **Where the tool lists live.** In the gateway (decided here) or shipped by each
  app as a data file the gateway reads, still agent-free in the app's code.
