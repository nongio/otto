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

## Automations

Spec references: `guide/automations.md` and `specification/automation-channel.md`.
These channels are at **stability 1.0 (early development)**, so expect churn, and gate
everything behind the capability.

- **Capability.** Advertise `InitializeResult.automations` (`create`, `schedules`,
  `runCancellation`, `runHistoryLimit`).
- **Catalogue.** `ahp-automations://` holds definitions: `title`, `message`, a
  `session` template (agent and folder), `enabled` and `triggers`.
- **Persistence.** Definitions and run records live in SQLite under
  `$XDG_STATE_HOME/otto-agents/`. Each run is recorded before its session is created.
- **Runs.** `runAutomation { automation, requestId }` is idempotent on `requestId`. Each
  run gets an `ahp-automation-run:/<id>` channel linking its sessions.
- **Scheduler.** One scheduler task evaluates cron triggers in each automation's time
  zone, claims each occurrence atomically, and applies the misfire policy on startup.
- **Event triggers.** These are host-defined and listed by
  `listAutomationTriggerDefinitions`. Start with file-system watches; add Otto desktop
  events later, from `org.otto.Shell1` window and workspace signals.
- **Concurrency cap.** Limit automation-started runs; runs over the cap stay `pending`.

## CLI

A thin AHP client built on the `ahp` crate.

```sh
otto-agents task "summarise today's commits" --agent claude --folder ~/dev/otto   # start and stream
otto-agents task "..." --detach                                                   # print the session URI and exit
otto-agents automation add triage --cron "30 9 * * MON-FRI" --tz Europe/Rome \
  --agent claude --folder ~/dev/otto --message "triage new issues"
otto-agents automation list | run triage | disable triage | runs triage
```

## Dependencies

- [0002](0002-core-protocol-loop.md): sessions and sequencing
- [0003](0003-acp-agent-backend.md): real agents
- [0004](0004-agent-configuration.md): agents and defaults
- [0005](0005-authentication.md): no credentials stored in definitions

## Testing

- **Scheduler:** a fake clock covers cron evaluation, DST gaps and repeated local
  minutes, and both misfire policies
- **Idempotency:** retrying `runAutomation` with the same `requestId` returns the same
  run URI
- **Cancellation race:** the run completes before its cancellation takes effect
- **Restart recovery:** after otto-agents restarts, run records persist and scheduling
  resumes
- **CLI:** end-to-end against a real server with the fake ACP agent

## Open questions

- Is automations' early-development status acceptable, or should v1 ship only one-off
  tasks plus the CLI?
- Where do a task's results go: the transcript only, or also a notification summary?
