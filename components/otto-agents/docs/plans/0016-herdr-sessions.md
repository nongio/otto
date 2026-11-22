# 0016: herdr as the session manager

**Status:** Draft. Would supersede the ACP backend and AHP service
([0002](0002-core-protocol-loop.md), [0003](0003-acp-agent-backend.md)) and
change [0011](0011-launcher-ask-v2.md) and [0015](0015-chat-bridges.md).

## Goal

herdr is the session manager. Every agent session lives in a herdr terminal,
in the agent's own interface. Otto's surfaces render what herdr knows about
it and send your actions back to herdr: a prompt from Ask, an answer to a
question, a Stop. You can work in the terminal or in Otto, on the same session,
and closing every Otto window leaves the agent running.

[herdr](https://herdr.dev) is a terminal workspace manager for coding agents.
Its runtime, herdr-core (`hc`), keeps terminals alive with no window attached,
tells which agent runs in each and what it is doing, and streams those facts as
events.

## Why drop ACP and AHP

ACP and a real terminal interface can't share one live agent. The ACP adapter
for Claude Code runs Claude through its SDK, with no screen, and starts its own
`claude`; a running TUI accepts no second client. Keeping ACP would mean two
kinds of session, one of them without its terminal.

AHP exists to give many clients one view of a session otto-agents owns. With
herdr owning the session, herdr-core already is that shared view: every client
reads the same terminals, states and labels, and its events reach them all.
A service in between would only copy herdr's state.

| | Today | With herdr |
|---|---|---|
| Owner | otto-agents, a child process over ACP | herdr-core, a terminal |
| Clients see it through | AHP, from otto-agents | herdr-core, directly |
| Interface | Otto's chat | the agent's own TUI, and Otto's chat |
| Conversation | ACP updates | the agent's transcript file |
| State | ACP turns | herdr events |
| Questions | ACP elicitation | the `ask` MCP tool |
| Permissions | ACP permission requests | a `PermissionRequest` hook |
| Otto → agent | ACP requests | `hc send`, `hc interrupt`, `hc kill` |
| Survives restarts | no | yes |

## Two directions

```
                       hc events, labels, transcript
  herdr-core  ──────────────────────────────────────────▶  Ask, Sessions,
  (agent TUI in a tab) ◀──────────────────────────────────  canvas, Studio, islands
                       hc run / send / interrupt / label
```

- **herdr → Otto: render.** What Otto shows comes from herdr: which sessions
  exist (`hc ls`), their state (`hc events`), what they are waiting for
  (labels, below) and their conversation (the transcript of the session).
- **Otto → herdr: events.** What you do in Otto becomes an action on the
  terminal: starting a session is `hc run`, a prompt is `hc send`, Stop is
  `hc interrupt`, closing is `hc kill`, and an answer is a label write.

## Decisions

- **One client crate.** `otto-herdr` (service layer, replacing
  `otto-agents-client`) talks to herdr-core and gives Otto's surfaces sessions,
  states, pending questions and turns as plain types. It is the only place that
  knows herdr's API and the agents' transcript formats. The launcher, the
  canvas, Studio and otto-islands use it; none of them runs `hc` by hand.
- **No service.** otto-agents stops being a daemon. What remains are commands
  the agent's side runs: `otto-agents mcp` (the `ask` tool),
  `otto-agents hook permission`, and `otto-agents plugins install`.
- **herdr owns the process.** Sessions start with `hc run --bg --keep`, carry
  `otto.session=<id>` and are found again by that label.
- **herdr shows it where you work.** A session started from Otto carries
  `herdr.space` and `herdr.tab` labels, which is how herdr-app places a
  terminal in a workspace tab. Without them it runs but is listed nowhere.
- **State comes from herdr, not the screen.** `hc events` gives
  `status {app, state, kind, msg}`: `working`, `idle`, `done`, `blocked`,
  `error`. Otto shows these as they are; it never reads the screen.
- **The conversation comes from the transcript.** Claude is started with
  `--session-id <uuid>`, so its transcript is a known file,
  `~/.claude/projects/<cwd>/<uuid>.jsonl`, and `otto-herdr` follows it into
  turns: your messages, the agent's text, tool calls. Per agent: Claude first.
- **Questions and permissions wait in herdr labels.** The `ask` tool and the
  permission hook run inside the session's terminal (`HC_TERMINAL_ID` is set).
  Each writes its request as a file under
  `$XDG_RUNTIME_DIR/otto/sessions/<id>/` and sets `otto.pending=<request id>`
  on its terminal. Every Otto surface sees the label change through
  `hc events`, reads the request and can answer it: the answer is written
  beside the request and `otto.pending` is cleared. The waiting tool call or
  hook picks it up. The first answer wins, whether from Ask, a notification
  or otto-islands.
- **Who asks you when no Otto window is open.** otto-islands follows
  `otto.pending` like any client and shows the dialog when no other surface
  is watching that session.
- **Prompts are typed in.** A prompt from Ask goes to the terminal with
  `hc send`. It is the one place this design drives the screen.

## The `ask` tool

`otto-agents mcp` is a stdio MCP server the Otto plugin declares. Like the
gateway in [0014](0014-mcp-gateway.md), it is written by hand on `serde_json`:
`initialize`, `tools/list`, `tools/call`. It has one tool, `ask`, with
`AskUserQuestion`'s input shape (`questions[]` of `question`, `header`,
`options[] {label, description}`, `multiSelect`), so the agent's habits carry
over. Sessions run with `--disallowedTools AskUserQuestion`, and the Otto
agent's instructions name `ask`.

While the call waits, the TUI shows a running tool and herdr reports
`working`. Otto shows "needs input" from `otto.pending`, not from herdr's
state. Outside herdr, or with no Otto session, the tool fails at once with a
message the agent can act on: "ask in the terminal instead".

Any agent with MCP can use it, which is how agents other than Claude get
structured questions too.

## What changes elsewhere

- **Ask and Sessions** render from `otto-herdr` instead of AHP. Starting a
  request starts a terminal; the chat card is a view of it.
- **Studio** starts its `studio` agent the same way; its preview MCP is
  unchanged.
- **Chat bridges** ([0015](0015-chat-bridges.md)) lose `otto-agents acp`.
  They would need a herdr-aware bridge, or `hc` on the bridge's side.
- **Agents without a terminal interface** have no place. Today that is none of
  the configured ones (Claude, opencode, pi, codex all have one).
- **Stored sessions** reopen in herdr with the agent's resume command
  (`claude --resume <id>`); the AHP store is read once and retired.

## Steps

1. **`otto-herdr` and Sessions.** List herdr's agent terminals and their
   states; open one in herdr. No conversation yet.
2. **Start from Ask.** `hc run` with labels and `--session-id`; prompts with
   `hc send`; Stop with `hc interrupt`.
3. **`ask` tool and pending labels.** `otto-agents mcp`, the request files,
   answering from Ask and otto-islands.
4. **Conversation.** Follow Claude's transcript into turns.
5. **Permission hook.** `otto-agents hook permission` through the same
   pending mechanism.
6. **Retire the service.** Remove AHP, ACP, the store and the facade once
   Ask, Sessions and Studio no longer use them.

## Open questions

- herdr-core's socket API: documented and stable enough to build on, or does
  `otto-herdr` run `hc --json` and parse its output?
- `hc send` wraps text sent from inside another hc terminal in
  `<hc from="tN">…</hc>`. Does a send from outside hc arrive plain?
- Label size and write rate limits, and whether a label write from inside the
  terminal is allowed for its own terminal only.
- Can `PreToolUse` answer `AskUserQuestion` directly? If it can, the built-in
  could stay for Claude.
- The transcript format is Claude's and undocumented: how much to read, and how
  to fail when it changes.
- herdr-core becomes a hard dependency of Otto's agent features. Packaging:
  depend on it, or bundle a pinned build as Sheprd does.

## Verified on 2026-11-22

- `hc run --bg --label herdr.space=… --label herdr.tab=…` places a terminal in
  a herdr-app tab.
- A Claude TUI started under hc reports `status {app: "claude-code", state}`,
  `idle` → `done` over a turn, and for `AskUserQuestion`
  `{state: "blocked", kind: "question", msg: "answer: <question> (<options>)"}`.
- Programs report their own status with OSC 7501:
  `ESC ] 7501 ; state=working:app=otto BEL`.
- Claude Code 2.1.330 has `PermissionRequest`, `Elicitation` and
  `ElicitationResult` hook events.
