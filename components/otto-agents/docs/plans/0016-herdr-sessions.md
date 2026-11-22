# 0016: herdr as the session manager

**Status:** In progress. herdr runs the sessions when it is installed: a
herdr backend inside the AHP service, beside the ACP backend
([0003](0003-acp-agent-backend.md)). Ask, the launcher and every other AHP
client work with it unchanged.

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

## Why not ACP

ACP and a real terminal interface can't share one live agent. The ACP adapter
for Claude Code runs Claude through its SDK, with no screen, and starts its own
`claude`; a running TUI accepts no second client. So under herdr the agent is
its TUI, and otto-agents reads and drives it from outside.

AHP stays. otto-agents serves the herdr sessions to its clients as it serves
ACP ones; only the backend underneath changes.

| | ACP backend | herdr backend |
|---|---|---|
| Owner | otto-agents, a child process over ACP | herdr-core, a terminal |
| Interface | Otto's chat | the agent's own TUI, and Otto's chat |
| Conversation | ACP updates | the agent's transcript |
| State | ACP turns | herdr events |
| Questions | ACP elicitation | the `ask` MCP tool |
| Permissions | ACP permission requests | a `PermissionRequest` hook |
| Otto → agent | ACP requests | `hc send`, `hc interrupt`, `hc kill` |
| Survives restarts | no | yes |

## How it works

```
                       hc events, labels, transcript
  herdr-core  ──────────────────────────────────────────▶  otto-agents ──AHP──▶ Ask,
  (agent TUI in a tab) ◀──────────────────────────────────  (herdr backend)        Sessions, …
                       hc run / send / interrupt / label
```

The backend is `src/herdr.rs`. It runs Claude Code, pi and opencode
(`Harness`), picked by an agent's `command` in `agents.toml`.

- **herdr is the source of truth.** Ask lists what herdr has. The backend
  follows `hc events` over every terminal: a terminal labelled
  `otto.session=<id>`, running or finished, is a session, and when the last
  terminal of one is removed the session leaves Otto too. A Claude Code
  started by hand in herdr is taken up as well: its session id and folder come
  from `~/.claude/sessions/<pid>.json`, and its terminal is labelled as Otto's
  from then on. When that Claude quits and leaves its shell, the labels go.
- **Its own store.** Under herdr the service keeps its sessions in
  `$XDG_STATE_HOME/otto-agents/herdr-sessions` unless `--state-dir` says
  otherwise, apart from the ACP service's. A stored session herdr no longer
  has is hidden, not deleted.
- **herdr owns the process.** Sessions start with `hc run --bg --keep`, carry
  `otto.session`, `otto.agent` and `otto.harness`, and are found again by
  those labels. A finished terminal is a dormant session: opening it in Ask,
  or Ctrl+O, starts it again with the agent's resume command.
- **herdr shows it where you work.** A session's terminal carries
  `herdr.space` and `herdr.tab`, which is how herdr-app places it in a tab.
  The space is the one whose folder is the session's. With none, one is made
  there (`herdr-app do space.new`), named after the agent's `space`, or after
  the folder, capitalised, when that is empty; its first shell tab is closed
  once the session's tab is in, since a space whose only tab closes loses its
  name. If herdr-app cannot make it, the first space takes the terminal.
- **Ctrl+O shows the session in herdr-app.** The session's `otto.terminal`
  command is `otto-agents herdr-show <id>`, with `window = "herdr-app"`. It
  opens herdr-app when its window is closed, gives a terminal without a pane a
  tab (`terminal.keep`), and focuses it (`agent.focus`). A session that is not
  running is opened through the service first, which starts it again.
- **State comes from herdr, not the screen.** `hc events` gives
  `status {app, state, kind, msg}`: `working`, `idle`, `done`, `blocked`,
  `error`. herdr reports it for Claude Code; for pi and opencode the backend
  waits a fixed time before the first prompt and reads turns from the
  transcript alone.
- **The conversation comes from the transcript.** Claude is started with
  `--session-id <uuid>`, so its transcript is a known file,
  `~/.claude/projects/<cwd>/<uuid>.jsonl`. pi's is found by its id under
  `~/.pi/agent/sessions`, opencode's is read from its SQLite database; both
  are turned into Claude's shape, which the rest of the backend reads.
- **Questions and permissions wait in herdr labels.** The `ask` tool and the
  permission hook run inside the session's terminal (`HC_TERMINAL_ID` is set).
  Each writes its request as JSON in a label on its own terminal (`otto.ask`,
  `otto.permission`), and waits for the answer label (`otto.answer`,
  `otto.permission.answer`) the backend writes back from Ask or a dialog.
- **Prompts are typed in.** A prompt from Ask goes to the terminal with
  `hc send`, only while the agent is the program in front. It is the one place
  this design drives the screen.
- **Files travel as paths.** Attachments become `@/path` mentions in the
  prompt, and the agent reads them itself.

## Configuration

`backend` in `agents.toml` picks what runs the sessions: `"herdr"` or
`"acp"`. Unset, it is herdr when `hc` is on `PATH`, else ACP. Settings ›
Agents › Sessions run in sets it; `serve --herdr` and `serve --acp` override
it.

Under herdr, every agent whose command runs Claude Code, pi or opencode
(directly, or as the ACP adapter `npx` starts) is offered; others, such as
Gemini or Codex, are left out. `space` names the herdr-app space made for an
agent's sessions; unset, it is named after the folder. The default agent,
which an agent with `id = "claude"` replaces, comes from the environment
while this is a prototype:

| Variable | What it sets |
|---|---|
| `OTTO_HERDR_SPACE` | the name of a space made for the default agent |
| `OTTO_HERDR_MODEL` | its model |
| `OTTO_HERDR_AGENT` | the plugin agent it runs as: `otto` unless set, `none` for plain Claude Code |
| `OTTO_HERDR_PERMISSION_MODE` | Claude's `--permission-mode` for every session, such as `auto` or `manual` |
| `OTTO_HERDR_ASK_MCP` | the command line of the `ask` MCP server, in place of `otto-agents herdr-ask` |
| `OTTO_HERDR_PERMISSION_HOOK` | the command line of the `PermissionRequest` hook, in place of `otto-agents herdr-permission` |

The `ask` server and the permission hook are this binary's hidden
`herdr-ask` and `herdr-permission` commands (`src/herdr_tools.rs`), which
Claude sessions are started with.

## The `ask` tool

A stdio MCP server with one tool, `ask`, with `AskUserQuestion`'s input shape
(`questions[]` of `question`, `header`, `options[] {label, description}`,
`multiSelect`), so the agent's habits carry over. Sessions run with
`--disallowedTools AskUserQuestion`, and a profile that lists
`AskUserQuestion` gets `mcp__otto__ask` in its place.

While the call waits, the TUI shows a running tool and herdr reports
`working`. Otto shows "needs input" from `otto.ask`, not from herdr's state.

## Left to do

- Retire the ACP backend once herdr covers Studio and the clients of
  `otto-agents acp`, and herdr-core is packaged.
- The harnesses below.

## Open questions

- Label trust: any herdr terminal can write any other terminal's labels. A
  session can answer another session's question, approve another session's
  tool (`otto.permission.answer`), or raise a fake `otto.permission` or
  `otto.ask` that Otto shows. Acceptable on one person's machine; a nonce only
  Otto knows would close it.
- `OTTO_HERDR_ASK_MCP` and `OTTO_HERDR_PERMISSION_HOOK`, when set, are split
  on whitespace: a path with a space breaks them.
- Startup asks `hc` synchronously, once per stored session, whether herdr
  still has it and whether it runs.
- `otto-agents new` and `otto-agents enter` under `--herdr` focus herdr-app
  rather than entering the session in the current terminal.
- herdr-core's socket API: documented and stable enough to build on, or keep
  running `hc --json` and parsing its output?
- Can `PreToolUse` answer `AskUserQuestion` directly? If it can, the built-in
  could stay for Claude.
- The transcript formats are the agents' own and undocumented: how much to
  read, and how to fail when they change.
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

## More harnesses

Claude Code, pi and opencode run in herdr. Gemini and Codex were researched on
2026-11-22 and set aside: there is no budget for their models now. Each needs
the same four things: start or resume a session by an id Otto knows, find its
transcript, read it as Claude's (prompts, text, tool calls and results, the
end of a turn), and take a pasted prompt. Suggested order: Gemini, then Codex.

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
- herdr reports Codex's status, as it does Claude's (not pi's).

### opencode (1.19)

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
