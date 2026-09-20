# Agents

**Status: proof of concept.** The service runs real agents and the launcher
talks to it, but the pieces are moving and the plans in
`components/otto-agents/docs/plans/` are ahead of the code. This page describes
what exists today.

Otto can put a coding agent behind the launcher: you press the launcher
shortcut, type a request instead of an app name, and the answer streams back in
the same card. The work is split between two processes that already existed for
other reasons, plus one new one:

| Piece | What it is | Where |
|---|---|---|
| **otto-agents** | A headless service that runs agents and publishes their sessions | `components/otto-agents/` |
| **otto-launcher** | The client: *ask* mode sends requests, *agents* mode lists sessions | `components/otto-launcher/src/ask.rs` |
| **otto-islands** | Where an agent's permission requests and questions go when no client is watching | `org.otto.Dialog1` |
| **Files "Ask…"** | A files-script that opens the launcher with the selection attached | `components/otto-files/scripts/ask` |

The service draws nothing and the launcher stores nothing. A session belongs to
otto-agents and outlives every window that looks at it, which is what makes
closing the launcher mid-answer harmless.

## Two protocols, back to back

```
┌───────────────────┐   AHP over WebSocket    ┌──────────────┐   ACP over stdio   ┌───────┐
│ otto-launcher     │ ──────────────────────▶ │ otto-agents  │ ─────────────────▶ │ agent │
│ otto-agents show  │   $XDG_RUNTIME_DIR/     │              │   child process    │       │
│                   │   otto-agents/          │              │                    │       │
│                   │   agents.sock           │              │                    │       │
└───────────────────┘                         └──────────────┘                    └───────┘
                                                     │ org.otto.Dialog1
                                                     ▼
                                                otto-islands
```

- **ACP** ([Agent Client Protocol](https://agentclientprotocol.com)) is what the
  agent speaks. Each session runs its agent as a child process on stdio
  (`src/acp.rs`). Claude Code through its ACP adapter is the default, and uses
  your existing login.
- **AHP** ([Agent Host Protocol](https://microsoft.github.io/agent-host-protocol/))
  is what clients speak: JSON-RPC 2.0, one message per WebSocket text frame
  (`src/server.rs`, `src/rpc.rs`). It is a coordination layer over ACP: the
  host owns the one ACP connection and gives many clients a synchronized view
  of it, as a state snapshot plus an ordered stream of actions. That is why the
  launcher can join a session another client started and see the whole
  conversation.

### The transport

The service listens on a Unix socket, `$XDG_RUNTIME_DIR/otto-agents/agents.sock`
(`/run/user/<uid>` when the variable is unset), created 0600 in a directory of
mode 0700. Anything that can reach the service can drive the agents with the
user's own permissions, so the socket's owner is the whole access check; there
is no authentication inside the protocol. A socket file left by an earlier
run is replaced at start, unless a service still answers on it.

`--listen` (`OTTO_AGENTS_LISTEN`) moves it: a socket path (absolute, or after
`unix:`) or `host:port`. TCP is for development and the tests, which bind
ephemeral loopback ports; the service logs a warning when it listens that way,
because a port on `127.0.0.1` is open to every process on the machine. With
no runtime directory at all the service falls back to `127.0.0.1:4800`, with
the same warning.

Clients take `--url` (`OTTO_AGENTS_URL`): `unix:///path/to/agents.sock` (the
default is the socket above), a bare absolute path, or `ws://host:port`. The
WebSocket handshake runs the same over both, with `localhost` as the nominal
host name on a socket. `src/client.rs` has the service's own connect helper;
the launcher carries the same few lines, since it does not depend on the
service crate.

The spec is vendored read-only under `components/otto-agents/spec/upstream/`,
pinned to an upstream release, and `scripts/sync-spec.sh` is the only thing that
may change it. Wire types come from the `ahp-types` crate at exactly the pinned
protocol version; `tests/spec_pin.rs` fails if the two drift apart.

### In the workspace

`components/otto-agents` is a member of the root Cargo workspace, like every
other component, and is packaged with Otto as `/usr/bin/otto-agents`:

```sh
cargo run -p otto-agents -- serve        # on $XDG_RUNTIME_DIR/otto-agents/agents.sock
cargo run -p otto-agents -- serve --echo # a built-in agent that repeats the prompt
cargo run -p otto-agents -- serve --listen 127.0.0.1:4800   # TCP, for development
components/otto-agents/scripts/ci.sh     # fmt, clippy, tests, spec drift check
```

### Running it as a service

The packages install a systemd user unit, `otto-agents.service`, bound to the
graphical session. It is not enabled by default:

```sh
systemctl --user enable --now otto-agents   # start now and at every login
journalctl --user -u otto-agents -f         # its log
```

The unit runs with the user manager's environment, not a login shell's. An
agent command found through `~/.local/bin`, nvm or similar needs either a full
path in `agents.toml` or the directory added with
`systemctl --user edit otto-agents` (`Environment=PATH=…`).

The unit is hardened: `NoNewPrivileges`, `PrivateTmp`, `RestrictSUIDSGID` and
`ProtectSystem=strict` with `ReadWritePaths=%h %t`, so the service and the
agents it runs write under the home and runtime directories and nowhere else.
`ProtectHome` stays off because the home directory is where agents work.

Sessions are stored in `$XDG_STATE_HOME/otto-agents/sessions`. They are the
person's conversations: the directory is created 0700 and each record 0600,
and a store written with wider permissions is tightened to the same on the next
start.

The launcher does **not** depend on that crate. It uses the upstream `ahp` and
`ahp-ws` client crates from crates.io, so the two sides are only coupled
through the protocol.

## Inside otto-agents

| Module | Holds |
|---|---|
| `host.rs` | The authoritative state: sessions, chats, subscriptions, questions |
| `server.rs` | The WebSocket listener, one task per connection |
| `rpc.rs` | JSON-RPC framing and error shapes |
| `agent.rs` | The `Backend` trait: what actually runs a turn, plus the echo backend |
| `acp.rs` | The real backend: an ACP agent per session, on stdio |
| `dialog.rs` | Permission and question prompts, and the `org.otto.Dialog1` prompter |
| `elicitation.rs` | ACP form elicitations (Claude's AskUserQuestion) as AHP input requests |
| `images.rs` | Pictures agents send, written to a cache so they travel as URIs |
| `config.rs` | `agents.toml`: which agents exist, their models and permissions |
| `store.rs` | Sessions on disk, so they survive a restart |
| `cli.rs` | `otto-agents sessions`, `show`, `new` and `enter`, for the terminal |

**One lock, one sequence.** Every mutation goes through `Host::lock`, which
reduces the action with the same `ahp::reducers` the clients use, stamps it with
the next `serverSeq`, and queues the outgoing envelopes, responses included.
A client therefore sees snapshots, responses and envelopes in an order that
always makes sense, and reaches the same state the host has by replaying the
same reducers.

**The host never talks to agents.** It hands each session's backend a command
channel and an event channel and translates the events into AHP actions
(`agent.rs`). That is what lets the echo backend stand in for a real agent in
tests, with no model credentials anywhere in CI.

**Sessions outlive the process.** `store.rs` writes one JSON file per session
under `$XDG_STATE_HOME/otto-agents/sessions/`, every second and once more on the
way out. What is stored is otto-agents' own part: the session's and its chat's
AHP state, minus the turns, plus the agent's own id for the session, in
`agentSession`. The turns are the agent's. The record keeps only a `written`
flag saying it has some. On restart the id is handed back so the agent can pick
that history up, and what it replays becomes the chat again. See below.

## A picture an agent sends

ACP carries a picture in the message: `ContentBlock::Image` with the bytes
base64-encoded and a media type beside them. AHP carries *state*, not payloads
(a chat snapshot is meant to be small enough to hand a client whole), so the
two do not meet directly.

**A picture comes from a tool, not from the model.** This is the part worth
knowing before reading the rest: a language model does not emit an image. It
calls a tool that returns one (Claude reading a JPEG, a browser tool taking a
screenshot), so the picture arrives in that call's `content`, as
`ToolCallContent::Content` wrapping a `ContentBlock::Image`. ACP also allows an
image in `agent_message_chunk`, and otto-agents handles that too, but in practice
nothing sends one. Both paths end in the same `SessionEvent::MessageImage`, so a
picture from a tool takes its place in the *answer* rather than being folded into
the tool call's line: the log draws a tool call as one dimmed line, and the
agent's next words usually describe what the picture shows.

A tool call is updated more than once and may repeat its content on each update,
so each session remembers the `(tool_call_id, path)` pairs it has already sent
and shows a picture once. Because files are named after their contents, that
comparison is exact and costs nothing.

`images.rs` bridges them. The bytes are decoded and written under
`$XDG_CACHE_HOME/otto/agents/images`, and what reaches the chat is a
`contentRef` response part: the file's `file://` URI, its size and its media
type. The socket stays small, the picture is a file any client can open, and a
client that does not draw pictures ignores a part it does not know.

Three consequences worth knowing:

- **The file is named after its contents.** `<label>-<digest>.<extension>`,
  where the digest is FNV-1a over the bytes. The same picture sent twice (or
  replayed out of the agent's own history on `session/load`) lands on the file
  already there rather than on a second copy.
- **The label is in the name because there is nowhere else.** A `contentRef` has
  a URI, a size and a media type, and no field for a caption, so what the agent
  called the picture goes into the file name and the client reads it back out.
- **The cache is disposable.** It is trimmed to 128 MiB, oldest first, so a
  conversation left open for a week does not fill the disk. A client must
  therefore cope with a picture whose file has gone; the launcher draws its name
  in its place.

A picture the agent points at rather than sends (`ContentBlock::ResourceLink`
to a `file://` path) is not copied at all. The chat points at the file where it
lies, provided it exists and is a picture.

Only bitmaps are taken: PNG, JPEG, GIF, WebP, BMP, AVIF and HEIF. SVG is
refused, because what draws these files decodes bitmaps and a picture that never
appears is worse than its name in its place. A picture in an agent's *reasoning*
is dropped: the log folds thinking into a single line, with nowhere to put one.
Every picture a tool returns is shown, including one the agent only looked at on
the way to an answer — it did look at it, and hiding that would be the less
honest choice.

## The agent's history is the session

**The agent's history is the truth, and the host's chat is a projection of it,
rebuilt whenever opening the session starts its agent.** The agent keeps the
history (for Claude, `~/.claude/projects/<cwd-slug>/<agent-session>.jsonl`)
and the host's `ChatState` is built from what the agent replays. The only thing
linking a stored record to that history is its `agentSession` field. So a
session touched outside the host comes back whole the next time it is opened in
Ask. The plainest way to touch one is Ctrl/Cmd+O in the launcher, which hands
it to the agent's own interface in a terminal (`claude --resume {session}`).

The store keeps no turns, so a chat is empty until its agent has replayed them.
Subscribing to a chat is what opens it: when the session's agent is not running
and has an `agentSession` to take up, the host starts it right there (`load_for`
in `host.rs`), and the turns follow once the agent is up, which with an
`npx …@latest` agent can be a while after the card opens. An agent already
running has already replayed its history, and the chat it is writing stands as
it is; nothing is loaded again.

Opening a session that has an `agentSession` therefore prefers ACP
`session/load` over `session/resume` (`open_session` in `acp.rs`).
`session/load` replays the whole history back as `session/update`
notifications; `session/resume` returns nothing at all. No turn is running to
claim those replayed updates, so a `Replay` collects them while the load is in
flight, and the session reports them as `SessionEvent::HistoryLoaded`.

The host then applies `chat/truncated`, clearing every turn, followed by
`chat/turnsLoaded` with the turns it rebuilt (`rebuild_chat`, `history_turn`).
Both are ordinary actions on the ordinary stream, so every connected client
reconciles the same way it takes any other change, with no special case
anywhere.

The replay carries no turn markers, only `user_message_chunk`,
`agent_message_chunk`, `agent_thought_chunk`, `tool_call` and
`tool_call_update`, so a turn is taken to be everything from one user message
up to the next. A replayed tool call is recorded as completed, carrying its
title and whether it succeeded, which is all the agent says about it.

Loading costs no more than resuming. Both spawn the same agent process
and both have it read its own history; neither sends anything to the model, so
neither spends tokens. What differs is that only `load` hands the history back.

`session/load` is also the method that is actually in the spec, gated by the
agent's `loadSession` capability; `session/resume` and `session/close` are both
marked UNSTABLE in the ACP schema: "not part of the spec yet, and may be
removed or changed at any point". Every agent tried against the service so far
(claude, opencode, pi and hermes) advertises `loadSession: true`, so
`resume` stays only as a fallback for an agent that cannot load.

## The launcher as a client

`components/otto-launcher/src/ask.rs` is the whole client. The launcher's main
loop has no async runtime, so the connection lives on a thread with a
current-thread tokio runtime. The thread sends updates over a channel and wakes
the loop through a socket the launcher already polls, the same `poll_fd` /
`pump` shape the window list uses. It connects as soon as the launcher opens, so
the agent list and the session list are there by the time anyone looks.

| Invocation | Mode |
|---|---|
| `otto-launcher --ask` | Type a request for an agent |
| `otto-launcher --ask --file PATH…` | The same, with files attached to the first request |
| `otto-launcher --agents` | List the service's sessions; Return opens one |
| `otto-launcher --session ID` | Follow an existing session by URI or id prefix |

`otto-ask` is an alias, installed as a symlink to `otto-launcher`: started
under that name the launcher opens in `--ask` mode, and the rest of the command
line works as usual (`otto-ask --file PATH`). The session list has no alias of
its own. It is `otto-launcher --agents`, so that `otto-agents` stays free for
the service.

`OTTO_AGENTS_URL` overrides the default socket for both the launcher and the
`otto-agents` subcommands: `unix:///path` or `ws://host:port`.

**The launcher queues a request; it never waits for it.** It calls
`createSession`, subscribes to the session and then to its chat, and dispatches
`chat/pendingMessageSet`. It does not wait for a turn. A turn can only begin
once the agent process is up (seconds), and only when the previous turn has
ended, and the service starts each queued request as soon as it can. This is
why the card can close at any moment: by then the service already owns the
request. Subscribing *before* queueing also matters, because it guarantees every
change the request causes arrives after the snapshot it applies to.

**The card becomes a conversation.** Once a request is sent, a log opens above
the field and the field takes follow-ups, which queue on the same chat.
`log.rs` word-wraps the transcript into styled lines and the results
`ScrollPane` scrolls it; `view.rs` draws it. An answer is a list of `Said`
(Markdown and pictures in the order they arrived), so a picture keeps its place
in the answer rather than collecting at the end; `view.rs` decodes each picture
once into raster pixels and caches it, because the log is laid out again on
every chunk that lands and each pass asks every picture how large it is. The last line is the status:
*Starting {agent}…*, *Thinking…*, *Working…*, then *Done*, *Cancelled* or
*Failed*. Reasoning is never written into the log. It only shows as
*Thinking…*. Ctrl+C or Cmd+C cancels a running turn; Esc closes the card and
leaves the session running. In the list of sessions (`--agents`), the same keys
stop the highlighted session's turn without opening it, and Ctrl+Backspace or
Cmd+Backspace removes it for good: the launcher calls AHP `disposeSession`,
otto-agents stops the agent, drops the session and deletes its record, and
`root/sessionRemoved` tells every client. The agent's own history is left
alone.

**Attachments** go as AHP `resource` attachments (a `file://` URI and the file
name), which otto-agents passes to the agent as ACP `resource_link` blocks. The
agent reads the files with its own tools. Other attachment kinds are dropped
with a warning.

**Entering a terminal.** When the agent has an `enter` command, otto-agents
publishes it, the folder and the agent's id for the session in the session's
`_meta` as `otto.terminal`: `enter` is the command on its own, and `command`
the same wrapped in the configured `terminal`, or `null` when there is none.
Ctrl+O in the launcher spawns `command` in a process group of its own, handing
the session to the agent's own interface. From the list of sessions the
launcher then goes at once; from an open conversation the card lingers a moment
on the handover line before it closes, so the log says where the session went.

A harness enters a session it has never written differently from one it has:
Claude takes `--session-id <id>` for the first and `--resume <id>` for the
second, and refuses the wrong one either way. So an agent has two commands
(`enter` and `enter_new`), and the service publishes whichever fits, swapping
`enter_new` for `enter` as soon as the session has a history. Any one of three
things makes it written: the agent has been given a turn, a replayed history
came back on `session/load`, or the session has been handed to a terminal,
whose turns this service never sees. The turns themselves are not stored, so
the flag is, and it outlives a restart.

**From a terminal.** `otto-agents new [agent]` creates a session, with the
agent named by its id or the name it is shown under, in the folder the command
was run from. It waits for the agent to give it an id, hands it over the same
way Ctrl+O does, and `exec`s the enter command in place: the terminal becomes the
agent's own interface. `otto-agents enter [session]` does the same for a
session that is already there. Both are the other direction of the same
handover, and what is said in them is in Ask the next time the session is
opened.

A session has one terminal. `{session}` in `enter`, `enter_new` and `terminal`
is always the agent's own id for the session, its `agentSession`, not the AHP
URI the clients and `otto-agents enter` use. The launcher looks for a process
that names that id on its command line and has a controlling terminal (the
agent otto-agents runs names the id too, but talks over pipes), and when it
finds one, Ctrl+O brings that terminal's window to the front rather than opening
another. Picking the session from the list does the same, in place of opening
the conversation. The window is found by the id in its app id or title, which
is what `--class=otto.agent.s{session}` in the `terminal` command is for;
without it the terminal is still not opened twice, but cannot be raised.
`{title}` is filled in by the launcher with what the window is called (*Ask:
@Claude: will it rain today?*), which is what the dock shows.

Entering the terminal hands the session over. The launcher asks the service to
`releaseSession`: otto-agents stops its own agent as soon as it is idle (a
turn under way, and anything queued behind it, finishes first), so the
terminal's agent is the only one writing the history. That is what the card's
last line says while it lingers (*Carrying on in the terminal…*, or *Finishing
this turn, then…*). What is said in the terminal lands in the agent's history,
and the next time the session is opened in Ask the agent is started again and
replays it. See
[the agent's history is the session](#the-agents-history-is-the-session).

## Where a permission question goes

An agent configured with `permissions = "ask"` has each request routed to
whoever is actually in front of the user. The rule is in `host.rs`:

1. The request opens in the chat as a tool call waiting for confirmation, with
   the agent's own options ("Always Allow", "Allow", "Reject") in the agent's
   order. The service picks which one to start on in `dialog::default_option`,
   the one place that reasons about option kinds: the narrowest allow ("once"
   before "always"), or the narrowest reject when the agent marked the request
   `defaultToNo`. It publishes that as `_meta.otto.defaultOption` on
   `chat/toolCallReady`, since AHP has no field for it. What the tool would
   touch goes with it: the first ACP `location` and the `rawInput` as
   `tool_input` (inline JSON, `{"path", "line", "rawInput"}`), and any `diff`
   content as `edits` (`[{"path", "oldText", "newText"}]`). The adapters'
   `_meta.permission` hints are honoured: `title` replaces the composed
   "{agent} wants to …" line, `description` becomes a body line before the
   folder, `defaultToNo` flips the default.
2. **A client is subscribed to that chat**, the launcher usually. The service
   leaves the question alone and waits for `chat/toolCallConfirmed`. The
   launcher shows the question in the log (the path and the edit as `-`/`+`
   lines under it, cut to a dozen lines) and the options as rows under the
   field, starting on the service's default and only falling back to its own
   narrowest-allow pick when the service sent none. The wait has a limit: after
   `WATCHED_GRACE` (20 s) unanswered, the question goes to the dialog as
   well, so a subscriber that shows nothing cannot hold it. Whichever answers
   first counts.
3. **Nobody is subscribed**, or the last watcher closes: `escalate` sends the
   question to otto-islands through `org.otto.Dialog1.PresentQuestions`, the
   same Access-style panel the portal uses, with an extra **Open in Ask** button.
   It is asked non-modal: the user can carry on elsewhere, and the panel shrinks
   into a circle in the island row, still waiting, until clicked open again.
   A plain yes or no picks the narrowest matching option; Open in Ask starts
   `otto-ask --session <uri>` and leaves the question waiting in the chat.
   Older renderers are tried in turn: one without `PresentQuestions` gets
   `PresentQuestion`, with any multi-select questions spelled out in the body
   rather than asked; one without that either gets `PresentAccess`, minus the
   Open in Ask button, and only for a request that carries no questions.
4. **The dialog can't be shown** (no session bus, no renderer, an error), and
   the request is **denied**. An agent is never granted something nobody saw.

The first answer wins; a later one is refused, and each question escalates at
most once. Cancelling the turn (`chat/turnCancelled` from a client, or the
agent ending it) cancels the questions it left open: the ACP request is
answered with the `cancelled` outcome, not a refusal, so the agent is not told
the person said no. A reply channel that is dropped rather than answered still
denies. The dialog's own words (the tool-kind phrases, "in {folder}", "Open in
Ask") come from the Fluent catalogue under `agents-permission-*`, loaded by
the service's own small `i18n` module from `LC_ALL`/`LC_MESSAGES`/`LANG`, since
the sentence is composed here even though otto-islands draws it. The hand-off only runs one way, though: if you open the launcher
while an island dialog is already up and answer there, the dialog stays on
screen with an answer that no longer counts, because `org.otto.Dialog1` has no
way to withdraw a prompt. Giving it one is the obvious next step, and would also
let the islands carry session status rather than questions alone.

**Modes first.** An agent's modes (Claude's Manual, Accept edits, Plan and
Auto; Codex's read-only, agent and full access) carry its own sandboxing,
and are the recommended way to loosen or tighten what it may do: set `mode` in
`agents.toml` for every new session, or switch with Shift+Tab in Ask. The
permission questions above are what the agent still asks in the mode it is
in. `permissions = "allow"` and `"deny"` stay as the coarse switch on this
side, answering every question the same way without a look.

## Where an agent's question goes

Agents also ask the user things outright: Claude's AskUserQuestion, or an MCP
server's form. otto-agents declares form elicitation in its ACP client
capabilities (that is what turns AskUserQuestion on), and `elicitation.rs` turns
each `elicitation/create` into an AHP input request:

- Each field of the requested schema becomes a question keyed by its name:
  enums as single or multi selects, strings, numbers, integers and booleans as
  their own kinds. AskUserQuestion's free-text "Other" field, marked in its
  `_meta`, is folded into its select as free-form input.
- The request opens in the chat's active turn (`chat/inputRequested`) and is
  mirrored into the session's `inputNeeded`, so a client watching only the
  session sees it. A URL-mode elicitation, or one outside a turn, is declined.
- Clients answer with `chat/inputAnswerChanged` and `chat/inputCompleted`. The
  host checks each answer fits its question and that an accept answers every
  required one; the answers go back to the agent under the field names.
- With nobody watching, the question escalates like a permission request. The
  dialog asks every select question, single or multi, itself, through
  `org.otto.Dialog1.PresentQuestions`, with an **Answer** button; several
  questions are asked one page at a time ("2 of 3", **Next**, **Back**), and a
  multi-select question's options are toggles. A question of another kind (free
  text, a number) is left out when nothing turns on it. Codex offers a note
  beside its choices, and a note nobody has to write is no reason to send the
  choices elsewhere; the whole form is in Ask for anyone who wants the rest of
  it. A **required** question of such a kind is another matter, and makes the
  request one to answer in Ask: the dialog then only lists what is being asked,
  with **Skip** and **Open in Ask**. Each
  group is labelled with the question's own words (for a lone AskUserQuestion
  that is the request's message, since the field carries only the short
  header), and each option's description follows its label after a line break.
- The picks come back as one answer per question: the option chosen for a
  single select, every picked option for a multi select (an empty list when
  none were picked). A renderer too old for `PresentQuestions` is asked the old
  way, with the multi-select questions spelled out instead of asked.
- The service sends only content: who is asking, the questions, the options and
  their descriptions, the multi flag, any message of the agent's own, and
  **Open in Ask**, since nothing else knows there is an Ask to open. The words
  for answering, skipping, paging and the multi-select hint belong to
  otto-islands, which localises them.
- Who is asking is a handle: the agent's provider (the id it is configured
  under, whose own lowercasing stands) as `@claude`, or else a slug of its
  display name (`Code Review Bot` → `@code-review-bot`), or else `@agent`.
- An elicitation's message is shown as context under the first question, in the
  agent's own words, unless it says nothing the questions do not. "Please
  answer the following questions" and its like are dropped rather than
  repeated. Skipping or dismissing declines; a dialog that can't be shown
  leaves the question waiting in the chat.
- Cancelling or ending the turn cancels its open questions, and a restarted
  session drops the ones its earlier agent was waiting on.

## The skills the desktop gives an agent

Otto ships skills of its own, covering how to configure the desktop, how to
use it (opening an app, saying something through the island, driving windows
with `otto-msg`) and how to write a Files command. They are installed as a
plugin directory, `/usr/share/otto/plugins/otto/`: a `.claude-plugin/plugin.json` and a `skills/`
tree with one `SKILL.md` each, the [Open Plugins](https://open-plugins.com/)
shape. A person's own go under `$XDG_DATA_HOME/otto/plugins/`, which is searched
first, so a plugin of the same name shadows the packaged one.
`OTTO_AGENTS_PLUGINS` replaces the search path entirely, which is how one is tried
out without installing it; it is a development knob, since it makes any
directory a trusted plugin, and the service logs a warning when it is set.
`src/skills.rs` reads them once, at startup.

The skills reach an agent by one of two routes, picked per agent with
`skills` in `agents.toml`. The setting is off unless set: an agent is given
nothing in its session, and an agent whose answers should owe nothing to Otto
needs no configuration at all.

- **Claude loads them as a plugin**: `skills = "claude"`, the recommended
  setting for Claude and what the built-in fallback agent uses. The service
  hands the plugin directories to claude-agent-acp in the session's
  `_meta.claudeCode.options.plugins`, and Claude loads them as its own skills:
  `/otto-help` invokes it, and each skill's `allowed-tools` frontmatter is Claude's
  to honour when it runs. The same `_meta` carries `systemPrompt.append`, a
  short preamble (where Claude is, that the skills are loaded as the `otto`
  plugin and how one is invoked, and when to reach for one), which the adapter
  appends to its `claude_code` preset rather than replacing it, so Claude's own
  safety text stays. The adapter reads both on `session/new`, `session/load`
  and `session/resume`, so a session taken up again gets them too.
- **Every other agent reads them from disk**: the setting for everyone
  else, and what the default expects. `otto-agents
  plugins install` links each skill into `~/.agents/skills/<name>`, the
  directory the Agent Skills convention names and Copilot, Codex and Claude
  read (`--dir` picks another, for an agent that looks elsewhere; Hermes is
  told about it with `skills.external_dirs` in its config). The links
  point at the skill's own directory, so a package upgrade reaches them;
  anything already at a link's path that is not that link is left alone and
  said so. `otto-agents plugins status` shows the plugins and skills found and
  whether each is linked. The session carries nothing. (`skills` is an alias for
  `plugins`, so `skills install` and `skills status` work too.)

There is no third route: `skills` takes `false` or `"claude"`, and anything
else (`skills = true` among them) is refused with a message naming the two
routes above (`config.rs`).

Whatever the route, what was found goes **to the clients** as AHP
customizations. Each plugin publishes as a `PluginCustomization` with its
skills as `SkillCustomization` children, on `AgentInfo.customizations` and
again on each session's `SessionState.customizations`, so a client can show
exactly what a session was given. They are read-only: they are the desktop's,
and this host has no way to write into `/usr/share`. Toggling one off is not
supported yet, and the host says so rather than accepting a toggle that would
change nothing. An agent with `skills` off publishes none, so what a client
shows is what the agent was given rather than a catalogue.

The launcher reads that list to complete skill names: type `/otto-h` in ask mode
and the rest of the name appears in grey, with Tab to take it. See
[`specs/launcher.md`](../../specs/launcher.md).

### The plugin's agent

Beside its skills the `otto` plugin carries an agent, `agents/otto.md`: a
Markdown file in the shape Claude Code plugins use: YAML frontmatter (`name`,
`description`, `tools`, `skills`, optionally `model`) and, as the body, the
system prompt of a helper that answers questions about the desktop, changes
settings on request and does things on the desktop itself (opening an app,
telling the person something through the island, moving a window), working
from the `otto-help` skill. `src/skills.rs` reads the frontmatter and the body
at startup, next to the skills; a plugin directory with an `agents/` file and no `skills/` is still a plugin. The
agent is published to clients as an `AgentCustomization` child of the plugin
(its `description`, `model` and `tools` from the frontmatter; `tools` absent
when the file lists none), after the skill children.

With `skills = "claude"` Claude Code loads a plugin's `agents/` itself from
the directory it was handed, the `systemPrompt.append` preamble says the
agent is there, and `agent = "otto"` in `agents.toml` runs the session as
it. Every other harness gets the same file rendered for it, below.

### The Otto agent on every harness

One file is maintained, `resources/plugins/otto/agents/otto.md`, and
`otto-agents plugins install` renders it for each harness that cannot load
it as it is (`src/vendors.rs`). The body, the instructions, is
byte-identical in every rendering; what differs is the frontmatter, a
heading derived from the frontmatter's `name` and `description` where a
plain file needs one, and where the file goes. Selection is per launch, from
`agents.toml`, so the harness in a terminal is not turned into Otto. The
table, as checked against the versions installed when this was written:

| Harness | Version checked | File written | Dialect | Selected by |
|---|---|---|---|---|
| Claude Code | claude-agent-acp 0.79 | none: reads the plugin's own file | Claude plugin agent | `agent = "otto"` → `--agent otto:otto` |
| OpenCode | 1.18.31 | `~/.config/opencode/agents/otto.md` | frontmatter `description`, `mode: primary` | `env = { OPENCODE_CONFIG_CONTENT = '{"default_agent":"otto"}' }` |
| Hermes | 0.16.0 | `~/.hermes/profiles/otto/SOUL.md` | plain: the identity slot of its prompt | `args = ["-p", "otto", "acp"]` (the profile is named after the agent) |
| Codex | codex-acp 1.12.0 | `~/.local/share/otto/agents/codex/otto.md` and `otto.json` | plain, plus a `CODEX_CONFIG` object | `env = { CODEX_CONFIG = "{file:~/.local/share/otto/agents/codex/otto.json}" }` |
| pi | 0.85.1, pi-acp 0.0.33 | `~/.local/share/otto/agents/pi/otto.md` and `otto-pi`, a wrapper | plain | `env = { PI_ACP_PI_COMMAND = "~/.local/share/otto/agents/pi/otto-pi" }` (an absolute path) |

What was verified, and what was not:

- **OpenCode** has no `--agent` on `opencode acp`; `OPENCODE_CONFIG_CONTENT`
  is a final config merge for that process alone, and `default_agent` is
  what its ACP server starts a session as. Its `tools` frontmatter is
  deprecated and can only deny, so it is not rendered. Verified with
  `opencode run --agent otto` and with the env var.
- **Hermes** makes `~/.hermes/profiles/<profile>/` its home under `-p`, and
  `SOUL.md` there replaces only the identity paragraph of its prompt;
  `hermes acp` builds the prompt the same way `hermes chat` does, and the
  `agent.system_prompt` config key is ignored by the ACP adapter. The
  default profile's `~/.hermes/SOUL.md` is never touched, and a missing
  profile is reported with the `hermes profile create` line, not created.
  Verified with `hermes -p otto chat -Q -q`.
- **Codex** appends `developer_instructions`; `model_instructions_file`
  takes a path but replaces the prompt, and `experimental_instructions_file`
  is gone. codex-acp 1.12 takes its whole session config as one JSON object
  in `CODEX_CONFIG`, so the installer renders that object beside the
  Markdown, and the service expands `{file:<path>}` in an agent's `args` and
  `env` when it starts the agent (`config::expand_args`, `config::expand_env`;
  `~` is the home directory), so the text is read from the rendered file and
  never copied into `agents.toml`. The adapter package is
  `@agentclientprotocol/codex-acp`: the archived `@zed-industries` one takes
  `-c key=value` instead and bundles a Codex the API now refuses. Verified
  over codex-acp's stdio.
- **pi**: pi-acp spawns `pi --mode rpc` and drops its own arguments, but
  passes its environment on, and `PI_ACP_PI_COMMAND` names the executable it
  spawns. pi's `--append-system-prompt <file>` adds a file to the default
  prompt (`SYSTEM.md` would replace it), so the wrapper adds that flag and
  hands over to `pi`. A flag on the command line makes pi skip its
  `~/.pi/agent/APPEND_SYSTEM.md`, for that process. Verified with `pi -p
  --append-system-prompt`; the wrapper under `--mode rpc` was checked
  through the service.

Every rendered file carries a marker, `rendered by otto-agents from
<source>`. A file without it is somebody else's and is left alone in both
directions, with a line saying so; a file with it is rewritten only when the
rendering changed, so `plugins install` after an upgrade writes nothing when
nothing moved. A harness whose own directory is not there (`~/.codex`,
`~/.pi`, `~/.config/opencode`, the Hermes profile) is skipped and said so.
`--only <harness>` renders for one harness and skips the skills; `--home
<dir>` places everything under another root, for trying it out.
`plugins status` shows the same files as `current`, `stale` or `not there`.

### What the service does not mediate

The service advertises no `fs/*` or `terminal/*` client capability, so an agent
never asks it to read or write a file or run a command on its behalf. Every
file and shell operation an agent performs runs inside the agent, under the
agent's own permission engine and whatever sandboxing its mode carries. What
the service sees of it is the ACP `session/request_permission` exchange, and
the island dialog that answers one is advisory: it tells the agent which of the
options the agent itself offered the person picked, and nothing stops an agent
that ignores the answer. Trust in what an agent can touch is trust in the
agent's own gate, not in this dialog.

## Configuration

`~/.config/otto/agents.toml`, after `/etc/otto/agents.toml`; the last file that
lists agents wins. With no file at all, `claude` runs through
`npx -y @agentclientprotocol/claude-agent-acp@0.79.0`: a pinned adapter, so
the agent an unconfigured desktop starts is a known one rather than whatever
the registry serves that day.

```toml
terminal = ["ghostty", "--class=otto.agent.s{session}", "--title={title}", "--working-directory={cwd}", "-e"]

[[agents]]
id = "claude"
name = "Claude"
description = "Anthropic's coding agent"
command = "claude-agent-acp"
model = "haiku"                          # optional; the agent's own name for it
mode = "acceptEdits"                     # optional; one of the agent's own mode ids
permissions = "ask"                      # or "allow"; "deny" is the default
folder = "~/dev"                         # optional; where this agent's sessions start
enter = ["claude", "--resume", "{session}"]
enter_new = ["claude", "--session-id", "{session}"] # how a session with no history yet is entered
skills = "claude"                        # "claude" or false (the default)
agent = "otto"                           # optional; a plugin agent to run as (needs skills = "claude")
colour = "orange"                        # optional; `color` works too
config = { collaboration_mode = "plan" } # optional; the agent's own session options
```

`folder` is the folder an agent's sessions start in when the client names none,
published to clients as `_meta.otto.folders.<id>` on the root state. It is the
reach the agent is given (everything under it is readable, and `permissions`
only covers what the agent stops to ask about), so it is worth setting narrowly,
and leaving unset for an agent that only changes desktop settings. Ask falls
back to a scratch folder, `$XDG_STATE_HOME/otto/ask`, rather than the home
folder; `createSession` itself refuses a request with no folder at all.

`agent` names an agent file shipped in a plugin (`agents/<name>.md`, see
below) and runs this entry as that agent, the way `claude --agent
plugin:name` does: the session opens with that flag in
`_meta.claudeCode.options.extraArgs`, and Claude reads the file itself and
honours its instructions, `tools`, `skills`, `model` and `allowed-tools`. It
rides the Claude route, so it needs `skills = "claude"`; a name no plugin has
is logged and the entry runs as itself. The Otto preamble still goes in the
system prompt alongside it.

The agent file's `allowed-tools` is where a standing permission belongs. A
skill's `allowed-tools` only covers the turns the skill is loaded for, so the
otto agent asking about a settings change it made two turns ago, once it is
running the command straight from what it already read, is the shape to
expect. The same rule on the agent file holds for the whole session, and the
`otto` agent carries the settings bus there: `busctl --user list` and calls to
`org.otto.Settings`, and nothing else.

`agent = "otto"` also gives the agent a face. A dialog from the desktop's own
helper (a permission request, or a question nobody is watching the chat to
answer) is Otto speaking to the person rather than a third-party agent asking
for something, so it wears Otto's icon instead of the tool glyph the dialog
would otherwise carry (`system-run` for a permission, `dialog-question` for a
question). The name sent is `otto-files`: the icon theme has one Otto mark
installed and that is it, and otto-islands resolves a dialog's icon by theme
name. `dialog::agent_icon` is the whole rule, `Backend::icon` carries it to the
host beside `colour` and `folder`, and an agent that is not running as `otto`
gets `None` and keeps the tool glyph.

`colour` names the frosted material the agent's surfaces wear: the Ask card
while it is a request to, or a conversation with, that agent. It is one of
otto-kit's `Frosted` materials, in lower case: `red`, `orange`, `amber`,
`yellow`, `lime`, `green`, `teal`, `cyan`, `blue`, `indigo`, `violet` or
`magenta`; anything else is a configuration error that lists them. An agent
without one leaves its surfaces on the desktop's plain material. The service
publishes the colours in the root state's `_meta`, as `otto.colours`, a map
from agent id to name (`AgentInfo` has no field for it).

`mode` is the agent's mode id every new session starts in: `acceptEdits` for
Claude, say, or `agent` for Codex. Modes are the agent's own permission and
sandboxing presets, and its own list is what counts: the service reads it
from the `session/new` response and only sends `session/set_mode` for an id
that is on it; an unknown one is logged and ignored, and a session taken up
again keeps whatever mode it was last in, here or in a terminal. The modes are
published in the session's `_meta` as `otto.modes` (`current`, and `available`
as `{id, name, description}` in the agent's order), kept in the session's
record, and a client switches them with the `setMode` request
(`{session, modeId}`; `SESSION_NOT_FOUND` or `INVALID_PARAMS` for an id the
agent does not offer). The agent's own answer moves `current`, whichever side
asked, so a mode changed from a terminal shows up here too.

`config` carries the agent's own session configuration options, by its own
ids, and the service sets them with `session/set_config_option` on every
session once the model is settled. They are the way to a setting ACP has no
field for. Codex needs one: its `request_user_input` tool exists in the `plan`
collaboration mode only, and that mode is an option rather than an ACP mode,
so `config = { collaboration_mode = "plan" }` is what lets Codex ask a
question instead of guessing in prose. An option the agent refuses is logged
and the session carries on without it — an option is a preference, not a
requirement.

The first agent listed is the default, which is how clients and `createSession`
tell which one to use when none is asked for.

## Testing

`components/otto-agents/docs/testing.md` has the full table. The shape of it:

- The echo backend and an in-memory fake ACP agent keep every automated test
  free of model credentials.
- `tests/handshake.rs` drives a real server over WebSocket with the upstream
  `ahp` client, so passing means interoperability rather than agreement with
  ourselves.
- `tests/spec_pin.rs` and `scripts/sync-spec.sh --check` catch spec drift.
- The launcher's live tests are `#[ignore]`d and run against
  `otto-agents serve --echo`.

## Read next

- [`components/otto-agents/docs/plans/`](../../components/otto-agents/docs/plans/README.md)
  covers one plan per milestone, with status. [0010](../../components/otto-agents/docs/plans/0010-poc.md)
  is the proof of concept, [0011](../../components/otto-agents/docs/plans/0011-launcher-ask-v2.md)
  the launcher's ask mode.
- [`specs/launcher.md`](../../specs/launcher.md): the launcher's behavioural contract.
- [`specs/portal-access-dialog.md`](../../specs/portal-access-dialog.md): the dialog
  contract the permission prompts reuse.
