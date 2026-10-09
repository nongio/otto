# 0006: Spawning tasks to agents

**Status:** Draft

## Goal

Hand work to an agent in three ways:

1. **Fire and watch:** start a task now and follow it live.
2. **Fire and forget:** start a task from the launcher, a shortcut or a script, and get
   notified when it needs input or finishes.
3. **Recurring or event-driven:** tasks run on a schedule or when something happens.

Every task is an otto-agents session, so all of them show up in every client: the
launcher's Sessions, the islands and editors.

## One-off tasks

- **The AHP sequence.** Spec references are under `spec/upstream/`.
  1. `createSession { channel, provider, workingDirectories }`
  2. Wait for `session/ready`.
  3. Start the first turn, either with `createChat { initialMessage }` or by
     dispatching `chat/turnStarted` on the default chat.

  `createSession` itself has no initial-message field.
- **Entry points:**
  - Otto's launcher "ask" mode ([0007](0007-otto-desktop-integration.md))
  - the `otto-agents task` CLI
  - any AHP client
- **Attention state.** `SessionSummary.status` carries `InputNeeded` and `Error`, and
  `SessionState.inputNeeded` lists pending questions. A watcher doesn't need to
  subscribe to every chat.
- **Notifications** come from the service (0007), so they arrive even when no GUI is
  open.

## Scheduled tasks

No code. Reminders and scheduled work are recipes in the otto-help skill
(`resources/plugins/otto/skills/otto-help/references/later.md`): an agent writes
systemd user units named `otto-later-<name>`, which run `notify-send` for a nudge or
the agent's own headless command for work. systemd already provides calendar
expressions, catch-up after the machine was off (`Persistent=true`), persistence, logs,
and commands to list, run and remove, so the person can see and debug every schedule
with standard tools.

Known limit: a scheduled run of work happens outside otto-agents, so it does not show
up in Sessions. Its result arrives as a notification and a file.

### Not now: AHP automations

`guide/automations.md` and `specification/automation-channel.md` define a host-side
scheduler, SQLite persistence, run channels, cancellation and event triggers, at
stability 1.0 (early development). Adopt them only if the recipes fall short, for
example if scheduled runs need to be sessions, or if AHP clients need to browse and
edit schedules. A recipe already holds what a definition needs: a name, a calendar
expression, an agent, a folder and a message.

## CLI

A thin AHP client built on the `ahp` crate.

```sh
otto-agents task "summarise today's commits" --agent claude --folder ~/dev/otto   # start and stream
otto-agents task "..." --detach                                                   # print the session URI and exit
```

## Dependencies

- [0002](0002-core-protocol-loop.md): sessions and sequencing
- [0003](0003-acp-agent-backend.md): real agents
- [0004](0004-agent-configuration.md): agents and defaults
- [0005](0005-authentication.md): no credentials stored in tasks

## Testing

- **CLI:** end-to-end against a real server with the fake ACP agent

## Open questions

- Where do a task's results go: the transcript only, or also a notification summary?
