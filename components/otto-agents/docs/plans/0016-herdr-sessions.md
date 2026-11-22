# 0016: Sessions in herdr

**Status:** Draft

## Goal

herdr is the session manager. Every agent session lives in a herdr terminal,
in the agent's own interface. Otto renders what herdr knows about it and sends
your actions back to herdr: a prompt from Ask, an answer to a question, a
cancel. You can work in the terminal or in Otto, on the same session, and
closing every Otto window or restarting otto-agents leaves the agent running.

[herdr](https://herdr.dev) is a terminal workspace manager for coding agents.
Its runtime, herdr-core (`hc`), keeps terminals alive with no window attached,
tells which agent runs in each and what it is doing, and streams those facts as
events.

## Why not ACP

ACP and a real terminal interface can't share one live agent. The ACP adapter
for Claude Code runs Claude through its SDK, with no screen, and starts its own
`claude`; a running TUI accepts no second client. Tried on 2026-11-22: the
adapter answers `initialize` through `hc exec`, but a terminal session and an
ACP session can only take turns on one transcript. Keeping ACP would mean two
kinds of session, one of them without its terminal. With herdr as the manager
there is one kind:

| | Today (ACP) | herdr |
|---|---|---|
| Owner | otto-agents, a child process | herdr-core, a terminal |
| Interface | Otto's chat | the agent's own TUI, and Otto's chat |
| Conversation | ACP updates | the agent's transcript file |
| State | ACP turns | herdr events |
| Questions | ACP elicitation | the `ask` MCP tool |
| Permissions | ACP permission requests | a `PermissionRequest` hook |
| Otto → agent | ACP requests | `hc send`, `hc keys`, `hc interrupt` |
| Survives an otto-agents restart | no | yes |

## Two directions

```
          herdr-core ── hc events, transcript ──▶ otto-agents ── AHP ──▶ Ask, Sessions,
  (agent TUI in a tab) ◀── hc send / keys / run ──           ◀── AHP ──  canvas, Studio
```

- **herdr → Otto: render.** What Otto shows comes from herdr: which sessions
  exist (`hc ls`), their state (`hc events`), and their conversation (the
  transcript of the session herdr runs). Otto keeps no state of its own about
  a session beyond what it needs to map it to AHP.
- **Otto → herdr: events.** What you do in Otto becomes an action on the
  terminal: starting a session is `hc run`, a prompt is `hc send`, Stop is
  `hc interrupt`, closing is `hc kill`. Answers to questions and permissions
  go back to the waiting tool call or hook, not through the screen.

## Decisions

- **otto-agents becomes a projection of herdr.** It stops running agents
  itself. A `herdr` backend (`src/agent.rs` `Backend`) emits the same
  `SessionEvent`s from herdr's data, so the host, the store and every client
  stay as they are. The ACP backend stays only as long as an agent has no
  terminal interface.
- **herdr owns the process.** otto-agents starts the agent with
  `hc run --bg --keep`, never as its own child, and finds it again after a
  restart by label (`otto.session=<id>`). Killing or closing a session goes
  through `hc kill`.
- **herdr shows it where you work.** The terminal carries `herdr.space` and
  `herdr.tab` labels, which is how herdr-app places a terminal in a workspace
  tab. Without them it runs but is listed nowhere.
- **State comes from herdr, not the screen.** `hc events --json --label
  otto.session=<id>` gives `status {app, state, kind, msg}`: `working`, `idle`,
  `done`, `blocked`, `error`, and for `blocked` whether it is a `question`,
  `permission` or `auth`. otto-agents maps these onto the session status
  (`working` → in progress, `blocked` → needs input, `done` → turn ended). It
  does not read the screen itself.
- **The conversation comes from the transcript.** Claude is started with
  `--session-id <uuid>`, so its transcript is a known file,
  `~/.claude/projects/<cwd>/<uuid>.jsonl`. The backend follows it and turns user
  messages, assistant text and tool calls into chat turns. This is per agent:
  Claude first, others when there is a reason.
- **Questions go through an MCP tool, not the screen.** herdr's `blocked` only
  carries a one-line summary of what is on screen, and answering it means
  pressing keys. Instead the Otto plugin gives the agent an `ask` tool
  (below) and the built-in `AskUserQuestion` is removed with
  `--disallowedTools AskUserQuestion`. The question reaches Otto as structured
  input, through the same `InputRequested` path ACP elicitations take today
  (`src/elicitation.rs`).
- **Permissions go through a hook.** `--permission-prompt-tool` only works in
  print mode, not in the TUI. Claude Code's `PermissionRequest` hook does run
  in the TUI, gets the tool name and input as JSON, and can return allow or
  deny. The hook asks otto-agents, which shows the same permission card it
  shows for ACP sessions.
- **Prompts from Otto are typed in.** A prompt sent from Ask goes to the
  terminal with `hc send`. It is the one place this design drives the screen.

## The `ask` tool

`otto-agents mcp --session <id>` is a stdio MCP server the Otto plugin
declares. Like the gateway in [0014](0014-mcp-gateway.md), it is written by hand
on `serde_json`: `initialize`, `tools/list`, `tools/call`. It has one tool:

```json
{
  "name": "ask",
  "description": "Ask the person a question and wait for the answer.",
  "inputSchema": {
    "type": "object",
    "required": ["questions"],
    "properties": {
      "questions": {
        "type": "array",
        "items": {
          "type": "object",
          "required": ["question"],
          "properties": {
            "question": { "type": "string" },
            "header": { "type": "string" },
            "options": {
              "type": "array",
              "items": {
                "type": "object",
                "required": ["label"],
                "properties": {
                  "label": { "type": "string" },
                  "description": { "type": "string" }
                }
              }
            },
            "multiSelect": { "type": "boolean" }
          }
        }
      }
    }
  }
}
```

The shape is `AskUserQuestion`'s, so the agent's habits carry over and the
existing mapping to AHP input requests applies. The call connects to the
service socket, raises the input request on the session's chat, and returns
the answers as the tool result. With no session, or no service, it fails at
once with a message the agent can act on ("ask in the terminal instead").

While the call waits, the TUI shows a running tool. The answer comes from Otto:
the chat card when a client is watching, otto-islands otherwise.

The `ask` tool is not Claude-specific. Any agent with MCP can use it, which is
also how a herdr session of another agent gets structured questions.

## Steps

1. **Session in herdr, state in Otto.** Ask starts sessions with `hc run`,
   labels and `--session-id`; the backend follows `hc events` and publishes the
   session and its status. Sessions lists every agent terminal herdr has, Ask
   shows each as working or waiting. Reattach after an otto-agents restart.
2. **Conversation.** Follow Claude's transcript into chat turns; `session/load`
   style history on first open.
3. **`ask` tool.** `otto-agents mcp`, the plugin's `.mcp.json`, removal of
   `AskUserQuestion` for herdr sessions, and an end-to-end test with a fake MCP
   client.
4. **Permission hook.** `otto-agents hook permission` as the hook command,
   installed by `otto-agents plugins install` for herdr sessions.
5. **Prompts from Otto.** `hc send`, then whatever step 1 learnt about turns.
6. **Taking over a terminal you started.** A Claude session you started in
   herdr yourself shows up in Sessions; needs the session id, which herdr's
   Claude integration reports on `SessionStart`.

Steps 1 and 3 are useful on their own and come first.

## Open questions

- Should Otto's surfaces read herdr directly instead of through otto-agents?
  Keeping AHP in between means no client changes and one place that knows
  transcript formats; going direct removes a process. This plan keeps AHP.
- What happens to existing ACP sessions in the store: shown read-only, or
  reopened in herdr with `claude --resume`?
- `hc send` wraps text sent from inside another hc terminal in
  `<hc from="tN">…</hc>`. Does a send from a process outside hc (the service)
  arrive plain?
- Can `PreToolUse` answer `AskUserQuestion` directly? If it can, the built-in
  could stay and the `ask` tool would only be needed for other agents.
- Is herdr's `blocked` reported for a waiting `ask` call? It should not be: the
  agent is in a tool call, so herdr reports `working`. Otto must show
  "needs input" from its own input request, not from herdr's state.
- The transcript format is Claude's and undocumented. How much of it to read,
  and how to fail when it changes.
- herdr-core is a separate install. Is it a hard dependency of otto-agents, or
  does it fall back to ACP when `hc` is missing?

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
