# Agents

**Status: proof of concept.** The service runs real agents and the launcher
talks to it, but the pieces are moving and the plans in
`components/otto-agentsd/docs/plans/` are ahead of the code. This page describes
what exists today.

Otto can put a coding agent behind the launcher: you press the launcher
shortcut, type a request instead of an app name, and the answer streams back in
the same card. The work is split between two processes that already existed for
other reasons, plus one new one:

| Piece | What it is | Where |
|---|---|---|
| **otto-agentsd** | A headless service that runs agents and publishes their sessions | `components/otto-agentsd/` |
| **otto-launcher** | The client: *ask* mode sends requests, *agents* mode lists sessions | `components/otto-launcher/src/ask.rs` |
| **otto-islands** | Where an agent's permission requests and questions go when no client is watching | `org.otto.Dialog1` |
| **Files "Ask…"** | A files-script that opens the launcher with the selection attached | `components/otto-files/scripts/ask` |

The service draws nothing and the launcher stores nothing. A session belongs to
otto-agentsd and outlives every window that looks at it, which is what makes
closing the launcher mid-answer harmless.

## Two protocols, back to back

```
┌───────────────────┐   AHP over WebSocket    ┌──────────────┐   ACP over stdio   ┌───────┐
│ otto-launcher     │ ──────────────────────▶ │ otto-agentsd │ ─────────────────▶ │ agent │
│ otto-agentsd show │   ws://127.0.0.1:4800   │              │   child process    │       │
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
  (`src/server.rs`, `src/rpc.rs`). It is designed for *several* clients sharing
  one view of a session, which is why the launcher can join a session another
  client started and see the whole conversation.

The spec is vendored read-only under `components/otto-agentsd/spec/upstream/`,
pinned to an upstream release, and `scripts/sync-spec.sh` is the only thing that
may change it. Wire types come from the `ahp-types` crate at exactly the pinned
protocol version; `tests/spec_pin.rs` fails if the two drift apart.

### In the workspace

`components/otto-agentsd` is a member of the root Cargo workspace, like every
other component, and is packaged with Otto as `/usr/bin/otto-agentsd`:

```sh
cargo run -p otto-agentsd -- serve        # ws://127.0.0.1:4800
cargo run -p otto-agentsd -- serve --echo # a built-in agent that repeats the prompt
components/otto-agentsd/scripts/ci.sh     # fmt, clippy, tests, spec drift check
```

### Running it as a service

The packages install a systemd user unit, `otto-agentsd.service`, bound to the
graphical session. It is not enabled by default:

```sh
systemctl --user enable --now otto-agentsd   # start now and at every login
journalctl --user -u otto-agentsd -f         # its log
```

The unit runs with the user manager's environment, not a login shell's. An
agent command found through `~/.local/bin`, nvm or similar needs either a full
path in `agents.toml` or the directory added with
`systemctl --user edit otto-agentsd` (`Environment=PATH=…`).

Sessions are stored in `$XDG_STATE_HOME/otto-agentsd/sessions`. The first start
after the rename moves them over from `otto-ahp/sessions`.

The launcher does **not** depend on that crate. It uses the upstream `ahp` and
`ahp-ws` client crates from crates.io, so the two sides are only coupled
through the protocol.

## Inside otto-agentsd

| Module | Holds |
|---|---|
| `host.rs` | The authoritative state: sessions, chats, subscriptions, questions |
| `server.rs` | The WebSocket listener, one task per connection |
| `rpc.rs` | JSON-RPC framing and error shapes |
| `agent.rs` | The `Backend` trait: what actually runs a turn, plus the echo backend |
| `acp.rs` | The real backend: an ACP agent per session, on stdio |
| `dialog.rs` | Permission and question prompts, and the `org.otto.Dialog1` prompter |
| `elicitation.rs` | ACP form elicitations (Claude's AskUserQuestion) as AHP input requests |
| `config.rs` | `agents.toml`: which agents exist, their models and permissions |
| `store.rs` | Sessions on disk, so they survive a restart |
| `cli.rs` | `otto-agentsd sessions` and `otto-agentsd show`, for the terminal |

**One lock, one sequence.** Every mutation goes through `Host::lock`, which
reduces the action with the same `ahp::reducers` the clients use, stamps it with
the next `serverSeq`, and queues the outgoing envelopes — responses included.
A client therefore sees snapshots, responses and envelopes in an order that
always makes sense, and reaches the same state the host has by replaying the
same reducers.

**The host never talks to agents.** It hands each session's backend a command
channel and an event channel and translates the events into AHP actions
(`agent.rs`). That is what lets the echo backend stand in for a real agent in
tests, with no model credentials anywhere in CI.

**Sessions outlive the process.** `store.rs` writes one JSON file per session
under `$XDG_STATE_HOME/otto-agentsd/sessions/`, every second and once more on the
way out. What is stored is otto-agentsd's own part — the AHP state as clients see
it, plus the agent's own id for the session. The agent keeps its history; on
restart the id is handed back so it can pick that history up when the session is
next asked something.

## The launcher as a client

`components/otto-launcher/src/ask.rs` is the whole client. The launcher's main
loop has no async runtime, so the connection lives on a thread with a
current-thread tokio runtime. The thread sends updates over a channel and wakes
the loop through a socket the launcher already polls — the same `poll_fd` /
`pump` shape the window list uses. It connects as soon as the launcher opens, so
the agent list and the session list are there by the time anyone looks.

| Invocation | Mode |
|---|---|
| `otto-launcher --ask` | Type a request for an agent |
| `otto-launcher --ask --file PATH…` | The same, with files attached to the first request |
| `otto-launcher --agents` | List the service's sessions; Return opens one |
| `otto-launcher --session ID` | Follow an existing session by URI or id prefix |

`otto-ask` and `otto-agents` are aliases, installed as symlinks to
`otto-launcher`: started under either name, the launcher opens in `--ask` or
`--agents` mode, and the rest of the command line works as usual
(`otto-ask --file PATH`).

`OTTO_AGENTS_URL` overrides `ws://127.0.0.1:4800` for both the launcher and the
`otto-agentsd` subcommands.

**Every request is queued, never started.** The launcher calls `createSession`,
subscribes to the session and then to its chat, and dispatches
`chat/pendingMessageSet`. It does not wait for a turn. A turn can only begin
once the agent process is up — seconds — and only when the previous turn has
ended, and the service starts each queued request as soon as it can. This is
why the card can close at any moment: by then the service already owns the
request. Subscribing *before* queueing also matters, because it guarantees every
change the request causes arrives after the snapshot it applies to.

**The card becomes a conversation.** Once a request is sent, a log opens above
the field and the field takes follow-ups, which queue on the same chat.
`log.rs` word-wraps the transcript into styled lines and the results
`ScrollPane` scrolls it; `view.rs` draws it. The last line is the status:
*Starting {agent}…*, *Thinking…*, *Working…*, then *Done*, *Cancelled* or
*Failed*. Reasoning is never written into the log — it only shows as
*Thinking…*. Ctrl+C or Cmd+C cancels a running turn; Esc closes the card and
leaves the session running. In the list of sessions (`--agents`), the same keys
stop the highlighted session's turn without opening it.

**Attachments** go as AHP `resource` attachments (a `file://` URI and the file
name), which otto-agentsd passes to the agent as ACP `resource_link` blocks. The
agent reads the files with its own tools. Other attachment kinds are dropped
with a warning.

**Entering a terminal.** When `agents.toml` configures a `terminal` and the
agent has an `enter` command, otto-agentsd publishes the joined command and the
folder in the session's `_meta` as `otto.terminal`. Ctrl+O in the launcher
spawns it in a process group of its own and closes the card, handing the session
to the agent's own interface. Nothing stops a turn from the launcher and one in
the terminal overlapping yet, so don't drive both at once.

## Where a permission question goes

An agent configured with `permissions = "ask"` has each request routed to
whoever is actually in front of the user. The rule is in `host.rs`:

1. The request opens in the chat as a tool call waiting for confirmation, with
   the agent's own options ("Always Allow", "Allow", "Reject").
2. **A client is subscribed to that chat** — the launcher, usually. The service
   leaves the question alone and waits for `chat/toolCallConfirmed`. The
   launcher shows the question in the log and the options as rows under the
   field, starting on the narrowest allow.
3. **Nobody is subscribed**, or the last watcher closes: `escalate` sends the
   question to otto-islands through `org.otto.Dialog1.PresentQuestion`, the
   same Access-style panel the portal uses, with an extra **Open in Ask** button.
   It is asked non-modal: the user can carry on elsewhere, and the panel shrinks
   into a circle in the island row, still waiting, until clicked open again.
   A plain yes or no picks the narrowest matching option; Open in Ask starts
   `otto-ask --session <uri>` and leaves the question waiting in the chat. An
   older renderer without `PresentQuestion` gets `PresentAccess`, minus that
   button.
4. **The dialog can't be shown** — no session bus, no renderer, an error — and
   the request is **denied**. An agent is never granted something nobody saw.

The first answer wins; a later one is refused, and each question escalates at
most once. The hand-off only runs one way, though: if you open the launcher
while an island dialog is already up and answer there, the dialog stays on
screen with an answer that no longer counts, because `org.otto.Dialog1` has no
way to withdraw a prompt. Giving it one is the obvious next step, and would also
let the islands carry session status rather than questions alone.

## Where an agent's question goes

Agents also ask the user things outright: Claude's AskUserQuestion, or an MCP
server's form. otto-agentsd declares form elicitation in its ACP client
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
- With nobody watching, the question escalates like a permission request. When
  every question is a select — single or multi — the dialog asks them itself
  through `org.otto.Dialog1.PresentQuestions`, with an **Answer** button; several
  questions are asked one page at a time ("2 of 3", **Next**, **Back**), and a
  multi-select question's options are toggles. A question of any other kind
  (free text, a number) makes the whole request one to answer in Ask: the dialog
  then only lists what is being asked, with **Skip** and **Open in Ask**. Each
  group is labelled with the question's own words — for a lone AskUserQuestion
  that is the request's message, since the field carries only the short header —
  and each option's description follows its label after a line break.
- The picks come back as one answer per question: the option chosen for a
  single select, every picked option for a multi select (an empty list when
  none were picked). A renderer too old for `PresentQuestions` is asked the old
  way, with the multi-select questions spelled out instead of asked.
- The service sends only content: who is asking, the questions, the options and
  their descriptions, the multi flag, any message of the agent's own — and
  **Open in Ask**, since nothing else knows there is an Ask to open. The words
  for answering, skipping, paging and the multi-select hint belong to
  otto-islands, which localises them.
- Who is asking is a handle: the agent's provider (the id it is configured
  under, whose own lowercasing stands) as `@claude`, or else a slug of its
  display name (`Code Review Bot` → `@code-review-bot`), or else `@agent`.
- An elicitation's message is shown as context under the first question, in the
  agent's own words, unless it says nothing the questions do not — "Please
  answer the following questions" and its like are dropped rather than
  repeated. Skipping or dismissing declines; a dialog that can't be shown
  leaves the question waiting in the chat.
- Cancelling or ending the turn cancels its open questions, and a restarted
  session drops the ones its earlier agent was waiting on.

## The skills the desktop gives an agent

Otto ships skills of its own — how to configure the desktop, how to write a
Files command — and installs them as a plugin directory,
`/usr/share/otto/plugins/otto/`: a `.claude-plugin/plugin.json` and a `skills/`
tree with one `SKILL.md` each, the [Open Plugins](https://open-plugins.com/)
shape. A person's own go under `$XDG_DATA_HOME/otto/plugins/`, which is searched
first, so a plugin of the same name shadows the packaged one.
`OTTO_AGENTS_PLUGINS` replaces the search path entirely, which is how one is tried
out without installing it. `src/skills.rs` reads them once, at startup.

What is found goes two ways:

- **To the agent**, as a briefing prepended to the first prompt of a new
  session. ACP has no field for skills and none of the agents we run read
  Otto's directory on their own, so the briefing is a list: one line per skill
  with its name, its frontmatter description, and the absolute path of its
  `SKILL.md`. The agent reads a file only when a request matches its
  description — which is what keeps the cost a few hundred bytes however long
  the skills grow — and a request opening with `/<name>` names one outright.
  A session taken up again is not briefed twice: it is already in the history
  the agent keeps.
- **To the clients**, as AHP customizations. Each plugin publishes as a
  `PluginCustomization` with its skills as `SkillCustomization` children, on
  `AgentInfo.customizations` and again on each session's
  `SessionState.customizations`, so a client can show exactly what a session
  was given. They are read-only: they are the desktop's, and this host has no
  way to write into `/usr/share`. Toggling one off is not supported yet, and
  the host says so rather than accepting a toggle that would change nothing.

The launcher reads that list to complete skill names: type `/conf` in ask mode
and the rest of the name appears in grey, with Tab to take it — see
[`specs/launcher.md`](../../specs/launcher.md).

With `skills = "claude"` the service does not brief the agent. It hands the
plugin directories to claude-agent-acp in the session's
`_meta.claudeCode.options.plugins`, and Claude loads them as its own skills:
`/otto` invokes it, and each skill's `allowed-tools` frontmatter is
Claude's to honour when it runs. The service never approves anything itself.

An agent configured with `skills = false` is told nothing and publishes no
customizations, so what a client shows is what the agent was given rather than
a catalogue.

## Configuration

`~/.config/otto/agents.toml`, after `/etc/otto/agents.toml`; the last file that
lists agents wins. With no file at all, `claude` runs through
`npx @agentclientprotocol/claude-agent-acp`.

```toml
terminal = ["ghostty", "--working-directory={cwd}", "-e"]

[[agents]]
id = "claude"
name = "Claude"
description = "Anthropic's coding agent"
command = "claude-agent-acp"
model = "haiku"                          # optional; the agent's own name for it
permissions = "ask"                      # or "allow", "deny"
enter = ["claude", "--resume", "{session}"]
skills = "claude"                        # true (briefing, the default), false, or "claude"
```

The first agent listed is the default, which is how clients and `createSession`
tell which one to use when none is asked for.

## Testing

`components/otto-agentsd/docs/testing.md` has the full table. The shape of it:

- The echo backend and an in-memory fake ACP agent keep every automated test
  free of model credentials.
- `tests/handshake.rs` drives a real server over WebSocket with the upstream
  `ahp` client, so passing means interoperability rather than agreement with
  ourselves.
- `tests/spec_pin.rs` and `scripts/sync-spec.sh --check` catch spec drift.
- The launcher's live tests are `#[ignore]`d and run against
  `otto-agentsd serve --echo`.

## Read next

- [`components/otto-agentsd/docs/plans/`](../../components/otto-agentsd/docs/plans/README.md)
  — one plan per milestone, with status. [0010](../../components/otto-agentsd/docs/plans/0010-poc.md)
  is the proof of concept, [0011](../../components/otto-agentsd/docs/plans/0011-launcher-ask-v2.md)
  the launcher's ask mode.
- [`specs/launcher.md`](../../specs/launcher.md) — the launcher's behavioural contract.
- [`specs/portal-access-dialog.md`](../../specs/portal-access-dialog.md) — the dialog
  contract the permission prompts reuse.
