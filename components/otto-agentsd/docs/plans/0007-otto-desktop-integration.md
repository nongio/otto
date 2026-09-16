# 0007: Otto desktop integration

**Status:** Draft

## Decision (2026-09-15)

The agent roles on the Otto desktop are split as follows:

- **otto-agentsd is the agents service.** It takes the service role from
  `../otto/specs/agents.md`.
- **The launcher is the GUI:** Ask and Sessions ([0011](0011-launcher-ask-v2.md)).
- **Every Otto surface is an AHP client** of otto-agentsd.
- **D-Bus is used only outbound.** otto-agentsd calls Otto's existing renderers, the
  notification daemon and the islands; it exposes no D-Bus API of its own.

## Otto's agents milestones, re-homed

| Otto milestone (`specs/agents-milestones.md`) | Now |
|---|---|
| M1: The service (D-Bus `org.otto.Agents1`) | otto-agentsd: [0002](0002-core-protocol-loop.md), [0003](0003-acp-agent-backend.md), [0004](0004-agent-configuration.md) |
| M2: Ask from the launcher | Launcher "ask" mode as an AHP client (spike below) |
| M3: Session monitor | The launcher's Sessions, `otto-launcher --agents` ([0011](0011-launcher-ask-v2.md)) |
| M4: Agents in the islands | Driven by otto-agentsd (below) |
| M5: An Otto guide for agents | [0008](0008-otto-usage-instructions.md) |
| M6: Files automations by agent | Later; builds on 0008 |

**Proposed amendment to Otto's specs:** replace the service section and the D-Bus API
table in `agents.md` with a pointer to otto-agentsd and AHP. The behaviour sections still
apply: states, Enter, questions, notifications, history. Update the milestones to match
the table above.

## How otto-agentsd uses Otto's existing services

- **Notifications.** When a session starts waiting, finishes a long turn or fails, the
  service posts to `org.freedesktop.Notifications`. Waiting and finished sessions offer
  an "Enter" action, and failed ones offer "Open in Agents". This is the service's job,
  as in Otto's spec, because notifications must work with no GUI open.
- **Islands (M4).** For each working or waiting session, the service creates and updates
  an activity through `org.otto.Island1` (`CreateActivity`, `UpdateActivity`,
  `DismissActivity`).
- **Quick permission answers.** Allow / Deny / Enter use Otto's dialog broker
  (`org.otto.Dialog1`, `specs/portal-access-dialog.md`). The broker's response becomes a
  `chat/toolCallConfirmed` dispatched by the service. The first answer wins against any
  AHP client answering at the same time.
  - **Implemented (2026-09-15):** under `permissions = "ask"`, each ACP permission
    request opens in the chat as a tool call waiting for confirmation. A client
    subscribed to the chat answers it with `chat/toolCallConfirmed`. With nobody
    subscribed, or once the last watcher leaves, otto-agentsd asks through
    `org.otto.Dialog1` (otto-islands), naming the agent, the action, the tool call and
    the folder. The first answer wins. An unreachable dialog renderer denies. See
    `src/host.rs` and `src/dialog.rs`, and plan 0011 for the launcher side.
- **Desktop events.** Automation triggers ([0006](0006-task-spawning.md)) come from
  `org.otto.Shell1` signals.
- **Missing services.** otto-agentsd never needs `WAYLAND_DISPLAY`. When a D-Bus service is
  missing (for example outside Otto), that integration is skipped and logged once.

## Running otto-agentsd in the session

- **Service unit.** A systemd user unit, `otto-agentsd.service`, with
  `PartOf=graphical-session.target`, `After=graphical-session.target` and
  `Restart=on-failure`. It follows the pattern of
  `components/xdg-desktop-portal-otto/xdg-desktop-portal-otto.service`.
- **On-demand start.** A socket unit, `otto-agentsd.socket`, on
  `$XDG_RUNTIME_DIR/otto-agentsd/ahp.sock` starts the service when a client first connects,
  replacing the bus activation in Otto's spec. The service may exit when no session is
  live.
- **Without systemd.** `[[exec_once]] otto-agentsd` in Otto's config works, but nothing
  restarts the process if it dies.

## Shared client code for Otto surfaces

The launcher, the islands and any future surface each need the same
connection logic, which could live in a small `otto-agentsd-client` crate on top of `ahp`
(or as an otto-kit module):

- Discover the socket or token ([0005](0005-authentication.md)), then connect and
  `initialize`.
- Offer helpers for "start task", "list sessions" and "enter session".

## Spike: launcher "ask" mode (requested 2026-09-15)

- **Request.** In `../otto-4`, branch `launcher-ask` from the latest `main`. Start
  `otto-launcher` in an "ask" mode where whatever the user types is sent to otto-agentsd.
- **Where it goes.** `components/otto-launcher` has a provider abstraction (`source.rs`)
  with `apps.rs`, `windows.rs` and `calc.rs`. Add an `ask.rs` source, or a launch flag
  such as `otto-launcher --mode ask`.
- **Flow on Return.**
  1. Connect over AHP with `ahp` + `ahp-ws`.
  2. `initialize`
  3. `createSession { provider: default agent, workingDirectories: [last folder] }`
  4. `chat/turnStarted` with the text
  5. Close the launcher. Feedback comes from the service's notification.
- **What works when.**
  - Now: the handshake only.
  - After [0002](0002-core-protocol-loop.md): a full round trip with the stub echo
    agent.
  - After [0003](0003-acp-agent-backend.md): real agents.

## Testing

- **Integrations:** fake `org.freedesktop.Notifications`, `org.otto.Island1` and
  `org.otto.Dialog1` services on a private bus assert what the service posts for
  waiting, finished and failed sessions, and that a broker answer becomes a
  confirmation. CI will need `dbus`.
- **Unit and socket activation:** manual check under a real Otto session.
- **Spike:** manual demo.

## Open questions

- Should otto-agentsd's settings appear in otto-settings through `org.otto.Settings`?
  ([0004](0004-agent-configuration.md))
- Packaging: should otto-agentsd ship with Otto's packages (PKGBUILD, nix) or separately?
- Should the islands move to being AHP clients later, instead of the service pushing
  activities?
