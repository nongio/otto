# 0003: Agent backend (ACP)

**Status:** Draft

## History

1. **First draft.** otto-ahp spawned ACP agents itself.
2. **Brief detour.** The plan was revised to bridge to the `otto-agents` D-Bus service
   specified in `../otto/specs/agents.md`.
3. **Settled on 2026-09-15.** **otto-ahp is the agent service.** It takes the role that
   Otto's spec gives `otto-agents`, and speaks AHP instead of `org.otto.Agents1`. The
   name `otto-agents` now belongs to the GUI app ([0009](0009-otto-agents-app.md)).

## Goal

otto-ahp runs ACP agents as its child processes. It keeps their sessions alive whether
or not any client is attached, and presents them as AHP sessions to every client: Otto's
launcher, the otto-agents app, the islands, editors and the CLI.

```
Otto surfaces (launcher, otto-agents app, islands) ─┐
External clients (editors, CLI, other devices) ─────┤ AHP
                                                    ▼
                         otto-ahp ── sessions, history, sequencing, auth, automations
                                                    │ ACP over stdio
                                                    ▼
                                agents (Claude Code, Goose, …)
```

## Behaviour carried over from Otto's agents spec

`../otto/specs/agents.md` already specifies the service's behaviour well. We keep the
behaviour and replace its D-Bus API with AHP:

- **Unavailable agents are listed.** An agent whose command is missing appears as
  unavailable rather than being left out ([0004](0004-agent-configuration.md)).
- **Sessions outlive clients.** History is persisted under
  `$XDG_STATE_HOME/otto-ahp/sessions/`. After a restart, every session is listed as
  stopped and can be resumed when the agent supports loading sessions (ACP
  `session/load`).
- **Enter.** A session can be handed to the agent's own interface in a terminal, using
  the agent's configured `enter` command. The service ends its agent process first, so
  only one driver ever runs. This is exposed as the extension command
  `x-otto/enterSession`. Entering a working session is refused, and the session returns
  to stopped when the terminal exits.
  - **POC (2026-09-15):** the joined `terminal` + `enter` command is published in the
    session's `_meta` as `otto.terminal`, and the launcher spawns it on Ctrl+O
    ([0011](0011-launcher-ask-v2.md)). The command, ending the agent process and the
    refusal are still to do.
- **Questions.**
  - The first answer wins, which AHP's tool-call confirmation already enforces.
  - An unanswered question waits; nothing is auto-denied.
  - "Don't ask again" lasts only for the session.
- **Crashes.** An agent that exits unexpectedly marks its session failed, with its last
  stderr lines as the reason, and withdraws any pending question.
- **No display needed.** The service never connects to the compositor.

AHP session lifecycle has no "entered" or "stopped" state, so those are carried in
session `_meta` as `otto.entered` and `otto.stopped`.

## Mapping: ACP ⇄ AHP (to verify against both specs)

| ACP | Direction | AHP |
|---|---|---|
| `session/new { cwd, mcpServers }` | service → agent | On `createSession`: `lifecycle: creating`, then `session/ready` or `session/creationFailed` |
| `session/prompt` | service → agent | Triggered by `chat/turnStarted` |
| `session/update` `agent_message_chunk` | agent → service | `chat/responsePart`, then `chat/delta` |
| `session/update` `agent_thought_chunk` | agent → service | `chat/reasoning` |
| `session/update` `tool_call` / `tool_call_update` | agent → service | `chat/toolCallStart` / `chat/toolCallReady` / `chat/toolCallComplete` |
| `session/update` `plan` | agent → service | No AHP equivalent (see open questions) |
| `session/request_permission` | agent → service | Tool call pending confirmation, plus `session/inputNeededSet`. The winning `chat/toolCallConfirmed` picks the option. |
| `session/cancel` | service → agent | Triggered by `chat/turnCancelled` |
| `session/prompt` result `stopReason` | agent → service | `chat/turnComplete`, `chat/error`, or cancelled turn |
| `session/load` | service → agent | Resuming a stopped session |
| `fs/read_text_file`, `fs/write_text_file` | agent → service | Served by the service, scoped to the session's working directory |
| `terminal/*` | agent → service | Not advertised in v1 |

## Constraints

- **One process per session.** Each session gets its own agent process, which is
  simplest; sharing a process per agent can come later.
- **One folder and one chat per session** in v1. Neither `multipleWorkingDirectories`
  nor `multipleChats` is advertised.
- **Children die with the service.** Agent processes run in the service's process
  group and are killed when it exits. On the next start their sessions are stopped.
- **No sandbox.** Agents run as the user, as Otto's spec states.
- **Unknown updates.** ACP updates the service doesn't recognise are logged and skipped.

## Implementation

A `crates/otto-ahp-acp` crate built on the
[`agent-client-protocol`](https://crates.io/crates/agent-client-protocol) crate. It
implements the backend trait from [0002](0002-core-protocol-loop.md).

## Testing

A fake ACP agent binary replays scripted updates over stdio. It covers:
- every row of the mapping table
- permissions answered, left pending, and raced by two clients
- cancel, agent crash, and service restart followed by resume
- enter, using a fake `enter` command

These tests need no model credentials.

## Otto impact

The service section and the D-Bus API in `../otto/specs/agents.md` are replaced by
otto-ahp and AHP. The behaviour sections still apply. See
[0007](0007-otto-desktop-integration.md).

## Open questions

- **Plans:** where do `plan` updates go, as a markdown response part or in `_meta`?
- **Retention:** how long is history kept? (Also open in Otto's spec.)
- **Terminal:** which terminal opens an entered session?
