# Agent Seats

**Status:** draft  
**Related specs:** pointer-input-focus.md, rdp-bridge.md, screenshare.md, workspaces-multi-output.md

## Summary

Agents drive Otto the way people do, with a pointer and a keyboard. An agent
seat gives each agent a pointer, keyboard focus and cursor of its own, so it
can work beside the user without moving their cursor or taking their focus,
and the user can always see where it is acting. Later, an agent can be bound
to a workspace and work there while the user is on another.

## Goals

- Input synthesized on an agent seat never moves the user's cursor, raises or
  activates a window, or changes the user's keyboard focus.
- An agent's cursor is always visibly different from the user's, and
  different from every other agent's.
- Existing input injectors that drive the user (the RDP bridge) keep doing so,
  unchanged, while agent seats exist.
- Stock automation tools work without modification for the single-agent case.
- Several agents can act at once, each on its own seat.
- An agent can be bound to a workspace and act on it while that workspace is
  not on screen.

## Non-Goals

- Agent identity, authentication or permission to act. Any client allowed to
  create virtual input devices today can use an agent seat; restricting that
  is a separate security design.
- Agents operating Otto's own chrome (dock, exposé, app switcher, topbar).
  The shell's UI has one pointer and it is the user's.
- XWayland applications. X11 clients see a single seat.
- Arbitrating between an agent and the user acting on the same window.

## Behavior

### Phase 1 — one agent seat (implemented, `feat/agent-cursor`)

- With `[agent_cursor] enabled = true`, Otto advertises a second `wl_seat`
  named `agent`, after the user's seat. It has a pointer and a keyboard.
  The setting is read at startup.
- A virtual pointer or virtual keyboard created on the agent seat drives that
  seat only. One created on the user's seat, or with no seat, drives the
  user's, exactly as before.
- Agent pointer motion is clamped to the outputs, like real input, and sends
  enter/leave/motion to the client surface under it. Otto's own UI is not hit
  and does not react to it.
- An agent button press gives the agent seat's keyboard to the window (or
  popup) under the pointer. The window is not raised, and which window looks
  active is unchanged.
- Cursor shapes a client asks for on the agent seat are ignored; the user's
  cursor shape is never changed by them.
- The agent cursor is the theme's default arrow, recoloured with
  `[agent_cursor] color` (`#RRGGBB`, default `#FF9500`): dark pixels take the
  colour, light ones stay light. It is drawn under the user's cursor, never on
  the hardware cursor plane, and only after the agent's first motion.
- The RDP bridge and Otto's test client take the first advertised seat. Tools
  that take the last (`wlrctl`) land on the agent seat.
- Keyboard focus, clipboard and primary selection are per seat: what the agent
  copies is on the agent seat's clipboard, not the user's.

### Phase 2 — a seat per agent

- Agents ask Otto for a seat, and Otto creates one for them, named
  `agent-<n>`, with its own colour from a fixed palette of clearly distinct
  hues (none close to the user's cursor or the accent colour). The request
  returns the seat name and the colour.
- A seat lives as long as the agent's session: it is removed when the agent
  releases it or its requesting connection goes away. Removing a seat ends
  any grab it holds and clears the keyboard focus it gave.
- Next to each agent cursor, a small label shows the agent's name when one was
  given, so the user can tell agents apart at a glance.
- An agent cursor fades out after a configurable idle time and reappears on
  the next motion.
- `enabled = true` keeps its Phase 1 meaning: one static seat named `agent`,
  for stock tools.

### Phase 3 — workspace binding

- An agent seat may be bound to a workspace. The agent's coordinates then
  address that workspace's space: hit-testing resolves against the windows on
  that workspace, whether or not it is shown on an output.
- The agent cursor is drawn only where its workspace is visible: on an output
  showing it, in exposé previews of it, and during workspace switches that
  bring it on screen.
- Windows on a workspace that has a bound agent keep receiving frame callbacks
  at full rate while it is hidden, so their applications keep painting.
- The agent can capture its bound workspace's current image while it is hidden
  (for screenshots it reasons about), at the workspace's output scale.
- When the user switches to a workspace with a bound agent, the agent keeps
  working; the user's input and the agent's go to their own seats.
- A new window an agent's application opens on a bound workspace maps there,
  not on the workspace the user is on, and does not take the user's focus.
- Unbinding, or removing the workspace, returns the seat to output coordinates.

### Phase 4 — polish

- Agent cursors appear in screen recordings and screen shares (configurable).
- Agent cursors follow the cursor theme and size, and animated cursors.
- White-arrow themes are tinted so the body, not the rim, takes the colour.

## Constraints & Edge Cases

- Toolkits pick a default seat. GTK and Qt handle several; Firefox, Chromium
  and Electron are untested and may only listen to one seat. Phase 2 must be
  checked against each before the default setup relies on it.
- Stock `wlrctl` has no seat selection: with more than one agent seat, agents
  need a driver that chooses a seat by its `wl_seat.name`.
- Text input and input methods are tied to the focused surface of a seat. An
  agent typing into a window the user is also typing into sends two
  keystroke streams; nothing arbitrates.
- A hidden workspace has no output. Scale, geometry and capture for it are
  taken from the output it was last shown on (Phase 3).
- The lock screen must stop all agent input: no agent motion, clicks or keys
  reach any surface while the session is locked. **Not yet enforced in
  Phase 1:** an agent keyboard keeps the focus it had when the session
  locked. Fix before the setting is recommended to anyone.
- Drag and drop from an agent seat is not supported until it is designed; a
  drag started there must not interfere with the user's.
- The KMS path has one hardware cursor plane per output, and it belongs to the
  user. Agent cursors are always composited, so they cost a composite when
  they move.

## Rationale

- **A seat, not just a coloured cursor.** Drawing a second cursor while
  agents still drive the user's pointer would only recolour the user's
  cursor. The agent needs its own pointer position and focus, and a seat is
  the Wayland object that carries them.
- **Seats per agent, not per workspace.** A seat is who is acting. Two agents
  on one workspace would fight over a shared seat, and an agent moving between
  workspaces would have to switch. Where an agent acts is a separate property
  of the seat: workspace binding.
- **Advertised after the user's seat.** Clients that take the first seat keep
  talking to the user; tools that take the last land on the agent. That makes
  stock tools work with no changes in the one-agent case.
- **No raise or activation from agent clicks.** Otherwise the user's windows
  would jump around while they work.
- **Opt-in.** A second seat is visible to every client, and seat handling
  varies between applications.

## Open Questions

- Allocation API for Phase 2: a D-Bus method on `org.otto.Compositor`, or a
  small Wayland protocol? D-Bus is simplest; a protocol would tie the seat's
  lifetime to the client connection for free.
- Should anything stop an agent's application from raising itself or taking
  focus on its own (xdg-activation), which would disturb the user?
- Should the user be able to take over an agent seat's pointer, or pause all
  agents with one shortcut?
- Should agents be allowed to use Otto's own UI (dock, exposé)? That needs a
  per-seat pointer in the scene engine.
- How is an agent told the geometry of a hidden workspace it is bound to?
- Is the udev rendering of the agent cursor correct on a real session? Phase 1
  is so far verified in tests and the build, not on screen.
