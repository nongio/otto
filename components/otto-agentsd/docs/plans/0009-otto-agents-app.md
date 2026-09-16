# 0009: otto-agents app

**Status:** Superseded by [0011](0011-launcher-ask-v2.md). The launcher's Sessions
(`otto-launcher --agents`) does this job; `otto-agents` is now an alias for it, and the
service is `otto-agentsd`.

## Goal

`otto-agents` is the desktop GUI for agent sessions, and it replaces Otto agents
milestone 3 ("Session Monitor"). Its first version is deliberately simple: it lists the
sessions otto-agents is running, and clicking one opens it in its client, either a
terminal or a web page, depending on configuration.

## Behaviour

- **Connecting.** Connect to otto-agents as an ordinary AHP client, over the Unix socket
  or the token ([0005](0005-authentication.md)):
  1. `initialize` with `initialSubscriptions: ["ahp-root://"]`
  2. `listSessions`
  3. Keep the list live with `root/sessionAdded`, `root/sessionRemoved` and
     `root/sessionSummaryChanged`. No per-session subscription is needed.
- **The list.** Sessions needing input come first, then in progress, then idle, then
  failed or stopped. Each row shows:
  - agent (icon and `displayName` from `RootState.agents`)
  - title
  - folder
  - age (`modifiedAt`)
- **Opening a session.** On click, run the configured opener for that session's agent:
  - **Terminal:** call `x-otto/enterSession` ([0003](0003-acp-agent-backend.md)). The
    service ends its agent process and opens the agent's `enter` command in a terminal,
    so two drivers never run at once. A working session can't be entered, so the app
    offers Cancel first.
  - **Web:** open a URL template in the default browser, e.g. a web AHP client at
    `…/session?uri={uri}`. The session keeps running in the service, and the web client
    attaches as one more AHP client, with no hand-over.
- **Launching.** A notification from the service ("Open in Agents") launches the app
  focused on that session.
- **Disconnecting.** When otto-agents goes away, show a disconnected banner, reconnect
  with backoff, then call `listSessions` again. Root notifications are not replayed
  after a reconnect.

## Configuration

This lives in the same config as [0004](0004-agent-configuration.md), and is published
through `RootState.config` so every client opens sessions the same way.

```toml
[ahp.open]
default = "terminal"                         # terminal | web
web_url = "http://127.0.0.1:4801/session?uri={uri}"

[[agents]]
id = "claude"
# ...
open = "terminal"                            # per-agent override
```

Placeholders:

| Placeholder | Value |
|---|---|
| `{uri}` | Session URI |
| `{session}` | The agent's own session id, from session `_meta` |
| `{folder}` | The session's working directory |

Agents without an `enter` command can only use the web opener.

## Implementation

- **Toolkit.** An otto-kit app in Otto's `components/otto-agents`, built on the shared
  `otto-agents-client` crate ([0007](0007-otto-desktop-integration.md)).
- **Protocol only.** The app uses AHP and nothing else, so it also works against a
  remote otto-agents.
- **Validate the flow first.** A CLI subcommand, `otto-agents sessions`, can check the
  protocol flow before any UI exists.

## Later

Otto milestone 3's full scope:
- a session summary: plan, tool calls, pending question
- actions: prompt, cancel, stop, resume, forget
- answering questions in place, which AHP supports natively

## Testing

- **List:** an end-to-end test against a real otto-agents with the fake ACP agent asserts
  that the list updates on add, remove and summary changes
- **Openers:** placeholder expansion and opener selection are unit-tested, including
  entering being refused for a working session
- **Reconnect:** restarting the server repopulates the list

## Open questions

- Which web AHP client does the `web` opener target? None exists yet. Would one be
  served by otto-agents itself?
- Should the app keep running in the tray, or open only on demand?
