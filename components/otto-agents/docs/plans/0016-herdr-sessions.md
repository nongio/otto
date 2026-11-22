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
  The space is the one whose folder is the session's. With none, one is
  made there (`herdr-app do space.new`, its first shell tab closed), named
  after the agent's `space` or else the folder; if herdr-app cannot make
  it, the first space takes it. Ctrl+O gives a terminal
  without a pane a tab (`terminal.keep`) before showing it.
- **State comes from herdr, not the screen.** `hc events` gives
  `status {app, state, kind, msg}`: `working`, `idle`, `done`, `blocked`,
  `error`. Otto shows these as they are; it never reads the screen.
- **The conversation comes from the transcript.** Claude is started with
  `--session-id <uuid>`, so its transcript is a known file,
  `~/.claude/projects/<cwd>/<uuid>.jsonl`, and `otto-herdr` follows it into
  turns: your messages, the agent's text, tool calls. Per agent: Claude first.
- **Questions and permissions wait in herdr labels.** The `ask` tool and the
  permission hook run inside the session's terminal (`HC_TERMINAL_ID` is set).
  Each writes its request as JSON in an `otto.ask` label on its own terminal.
  Every Otto surface sees it through `hc events` and can answer by writing
  `otto.answer` with the request's id; the waiting tool call or hook picks it
  up and clears both labels. The first answer wins, whether from Ask, a
  notification or otto-islands.
- **Who asks you when no Otto window is open.** otto-islands follows
  `otto.ask` like any client and shows the dialog when no other surface
  is watching that session.
- **Prompts are typed in.** A prompt from Ask goes to the terminal with
  `hc send`. It is the one place this design drives the screen.
- **Files travel as paths.** Attachments become `@/path` mentions in the
  prompt, and the agent reads them itself, pictures included. Something with
  no path (a pasted picture, a screenshot) is saved under
  `$XDG_RUNTIME_DIR/otto/sessions/<id>/` first, a folder the session is allowed
  to read without asking. A session on another machine gets the file with
  `hc put`, which says where it landed. Unlike ACP, nothing is sent inline.

## The `ask` tool

`otto-agents mcp` is a stdio MCP server the Otto plugin declares. Like the
gateway in [0014](0014-mcp-gateway.md), it is written by hand on `serde_json`:
`initialize`, `tools/list`, `tools/call`. It has one tool, `ask`, with
`AskUserQuestion`'s input shape (`questions[]` of `question`, `header`,
`options[] {label, description}`, `multiSelect`), so the agent's habits carry
over. Sessions run with `--disallowedTools AskUserQuestion`, and the Otto
agent's instructions name `ask`.

While the call waits, the TUI shows a running tool and herdr reports
`working`. Otto shows "needs input" from `otto.ask`, not from herdr's
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
3. **`ask` tool and pending labels.** `otto-agents mcp`, the `otto.ask` and `otto.answer` labels,
   answering from Ask and otto-islands.
4. **Conversation.** Follow Claude's transcript into turns.
5. **Permission hook.** `otto-agents hook permission` through the same
   pending mechanism.
6. **Retire the service.** Remove AHP, ACP, the store and the facade once
   Ask, Sessions and Studio no longer use them.

## Open questions

- herdr-core's socket API: documented and stable enough to build on, or does
  `otto-herdr` run `hc --json` and parse its output?
- Any terminal can write any other terminal's labels, so a session can
  answer another session's question. Acceptable on one person's machine; worth
  a check per request (a nonce only Otto knows) if it matters.
- Can `PreToolUse` answer `AskUserQuestion` directly? If it can, the built-in
  could stay for Claude.
- The transcript format is Claude's and undocumented: how much to read, and how
  to fail when it changes.
- Background Claude jobs (`claude --bg`, watched with `claude agents`) are not
  herdr terminals: each is its own process with a note in
  `~/.claude/sessions/<pid>.json` (`kind: "bg"`). Show them in Ask as
  read-only rows read from the transcript, or keep Ask to herdr's terminals?
  A prompt from Ask has no reliable way in: `claude agents` takes typing for
  whichever job is selected.
- Deleting a session Otto took up from a shell tab (Claude started by hand)
  kills the shell and closes the tab. Stop following it, or end only Claude,
  instead?
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
- The `ask` round trip works: a Claude session in herdr with `ask-mcp.py`
  (`--mcp-config`, `--disallowedTools AskUserQuestion`) called `ask`, the
  question appeared as `otto.ask` JSON, an `otto.answer` label written from
  outside answered it, and the agent went on with the answer.
- Labels hold at least 4 KB, a terminal can label itself, and an empty value
  removes a label.
- `hc send` from a process outside hc arrives plain; only sends from inside an
  hc terminal get the `<hc from="tN">` wrapper.
- The Otto agent's `tools` list (`plugins/otto/agents/otto.md`) filters out MCP
  tools: it needs `mcp__otto__ask` added, or the agent falls back to asking in
  plain text.
- Claude Code 2.1.330 has `PermissionRequest`, `Elicitation` and
  `ElicitationResult` hook events.

## More harnesses (deferred)

Researched on 2026-11-22 and set aside: there is no budget for these models
now. Claude Code, pi and opencode run in herdr today. Each harness below needs the same
four things: start or resume a session by an id Otto knows, find its
transcript, read it as Claude's (prompts, text, tool calls and results, the
end of a turn), and take a pasted prompt. Suggested order: Gemini, Codex,
opencode.

### Gemini CLI (`gemini` 0.61)

- **Id:** `--session-id <uuid>` makes a session, and refuses an id that
  exists; `--resume <uuid>` takes it up again. The backend picks by whether
  the session is new. `--skip-trust` avoids the folder trust dialog.
- **Transcript:** `~/.gemini/tmp/<project>/chats/session-<time>-<id8>.jsonl`
  (`~/.gemini/projects.json` maps folders to project names). Match on the
  first line's `sessionId`, not the name: two sessions made in one minute with
  the same id prefix overwrote each other. Appended, but a record is written
  again under the same `id` as it grows, so readers keep the last version.
  Records: header `{sessionId, projectHash, startTime, kind}`; `{"$set": …}`
  metadata (skip); prompts `{id, type: "user", content: [{text}]}`; replies
  `{id, type: "gemini", content, thoughts, toolCalls: [{id, name, args,
  result, status, resultDisplay}]}`; a `user` record holding a
  `functionResponse` repeats a tool's result (skip).
- **End of turn:** no marker. Infer it (a `gemini` record with every tool call
  settled and no prompt after it), or have an `AfterAgent` hook write one.
- **Model:** `-m`. The default was overloaded (503) and `gemini-2.5-flash` is
  closed to new users; `gemini-3.5-flash-lite` answered on the API key in
  `~/.gemini/settings.json`.
- **Instructions:** no append flag. `GEMINI_SYSTEM_MD` replaces the whole
  prompt; a `GEMINI.md` adds context. **Approval:** `--approval-mode
  default|auto_edit|yolo|plan`.
- **Paste:** Enter within 40 ms of a bracketed paste becomes a newline, so
  `hc send` must wait before Enter.

### Codex CLI (`codex` 0.156)

- **Id:** none to choose; Codex makes a UUIDv7. Find it in the new
  `~/.codex/sessions/YYYY/MM/DD/rollout-<time>-<uuid>.jsonl` (first line
  `session_meta.payload.id`) or `~/.codex/session_index.jsonl`; the file may
  appear only with the first prompt. `codex resume <uuid>` takes it up again.
  `--no-daemon` keeps the TUI off the shared app server.
- **Transcript:** `{timestamp, type, payload}` per line. Prompts are
  `event_msg/user_message`, not `response_item` user messages (those carry
  injected context). Replies: `response_item message role:assistant
  content[{type: output_text}]`. Tools: `function_call` / `custom_tool_call`
  and their `*_output` by `call_id`. Reasoning has a readable `summary` only.
- **End of turn:** `event_msg/task_complete {turn_id, last_agent_message}`,
  opened by `task_started`. An interrupted turn's record is unconfirmed.
- **Model and auth:** `-m`; logged in with ChatGPT (`gpt-5.6-terra` in
  `~/.codex/config.toml`), so it spends the plan's limits. No free model but
  `--oss` with ollama. **Approval:** `-a`, `-s`. **Instructions:** `-c
  developer_instructions=…` (unconfirmed).
- **Paste:** an Enter right after a paste may become a newline: wait first.
- herdr reports Codex's status, as it does Claude's and pi's.

### opencode (1.19): done

An agent with `command = "opencode"` runs it in herdr (`Harness::OpenCode`).

- **Id:** Otto makes the id (`ses_` and 26 hex characters) and the session,
  by `opencode import` of an empty export run in the session's folder
  (opencode takes the folder and project from where import runs; `slug`,
  `projectID`, `version` and `time` are required). Then `opencode -s <id> -m
  <model>`, with `--auto` when Otto's permission mode is `auto`.
- **Transcript:** read with the `sqlite3` CLI from
  `~/.local/share/opencode/opencode.db` (messages joined with their parts);
  `opencode export` takes two seconds of CPU a call, too much to poll. The
  write-ahead log's size and time say when to read again. Each settled piece
  is an entry of its own: a prompt, a finished text part, a tool call and its
  result, and the end of the turn.
- **End of turn:** an assistant message completed with any `finish` but
  `tool-calls`, or with an `error`.
- **Left:** the agent profile's prompt (an agent in
  `OPENCODE_CONFIG_CONTENT`); herdr reports no status for opencode, so there
  is no working dot and only the 5 s wait before the first prompt; a request
  opencode keeps retrying (the free quota's "retrying in 11h") leaves the turn
  open in Ask; opencode updates itself and exits on a launch now and then
  (1.18 to 1.19 ended one session at start); hand-started opencode is not
  taken up. The free models share one quota, spent again today.
