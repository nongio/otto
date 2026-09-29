# Agent Seats

**Status:** draft  
**Related specs:** pointer-input-focus.md, rdp-bridge.md, screenshare.md, workspaces-multi-output.md, lock-screen.md

## Summary

Agents drive Otto the way people do, with a pointer and a keyboard. An agent
seat gives each agent a pointer, keyboard focus and cursor of its own, so it
can work beside the user without moving their cursor or taking their focus.
A workspace grant says where an agent may act: a workspace of its own, the
user's current workspace, or every workspace, with the user's consent. A
workspace under an agent's control is framed in the agent's colour, so the
user always knows which parts of the desktop an agent can touch.

## Goals

- Input synthesized on an agent seat never moves the user's cursor, raises or
  activates a window, or changes the user's keyboard focus.
- An agent's cursor is always visibly different from the user's, and
  different from every other agent's.
- Existing input injectors that drive the user (the RDP bridge) keep doing so,
  unchanged, while agent seats exist.
- Stock automation tools work without modification for the single-agent case.
- Several agents can act at once, each on its own seat.
- An agent can be given a workspace of its own and work there while the user
  works on another.
- An agent acts on the user's workspaces only after the user allows it, and
  the user can end that at any moment.
- Every workspace an agent controls is marked on screen, unmistakably, for as
  long as the agent controls it.
- In the end, an agent cannot reach outside its grant — not with input, and
  not by capturing the screen.

## Non-Goals

- Agents operating Otto's own chrome before the last phase. Exposé comes
  only with an "every workspace" grant, the dock after that; the app
  switcher and topbar are not planned.
- XWayland applications. X11 clients see a single seat.
- Arbitrating between an agent and the user typing into the same window.
- Deciding which agents are trustworthy. Otto asks the user and enforces the
  answer; it does not judge agents itself.

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
  the hardware cursor plane, and only after the agent's first motion. It is
  part of what the screen shows, so screenshots of the output include it.
- The RDP bridge and Otto's test client take the first advertised seat. Tools
  that take the last (`wlrctl`) land on the agent seat.
- Keyboard focus, clipboard and primary selection are per seat: what the agent
  copies is on the agent seat's clipboard, not the user's.
- The agent acts on every workspace — Phase 1 is, in effect, an "every
  workspace" grant with no prompt, no border and no enforcement.

### Phase 1 follow-ups (implemented, except the own-workspace time)

**Idle hiding.** The agent cursor gets out of the way when the agent stops:

- `[agent_cursor] hide_after_ms` (default `5000`; `0` never hides) is how long
  the cursor stays after the agent's last activity.
- On an agent's own workspace (Phase 3) the agent counts as active for as
  long as it holds the workspace: its border stays at full strength until
  the agent releases it, and its cursor uses the longer
  `[agent_cursor] own_workspace_hide_after_ms` (default `30000`). The
  workspace is the agent's, so there is nothing of the user's for the cursor
  to get in the way of.
- Activity is any agent input: pointer motion, button, scroll, or a key from
  a virtual keyboard on the agent seat. Typing keeps the cursor visible, so
  the user can see which window the agent is typing into.
- When the time runs out the cursor fades out over a short animation
  (~200 ms) and is no longer drawn. Nothing else changes: the agent's pointer
  position, pointer focus and keyboard focus stay as they were.
- The next activity shows it again at the agent pointer's current position,
  at full opacity immediately, with no fade-in: a cursor that appears late
  hides the start of what the agent does.
- While an agent button is held (a drag or a text selection in progress),
  the cursor is never hidden, however long the gap between events.
- Hiding and showing cost a redraw only on the outputs the cursor is on;
  a hidden cursor adds no work to a frame.
- The lock screen hides every agent cursor at once, whatever the timer says.

**Lock screen.** While the session is locked or locking, no agent motion,
click, scroll or key reaches any surface, the lock surface included. Locking
takes the agent seat's pointer and keyboard focus away; unlocking does not
give them back — the agent clicks again.

### Phase 2 — a seat per agent

- Agents ask Otto for seats and grants over D-Bus, on
  `org.otto.Compositor`. A seat is tied to the caller's bus name: when that
  name leaves the bus, the seat is removed.
- Agents ask Otto for a seat, and Otto creates one for them, named
  `agent-<n>`, with its own colour from a fixed palette of clearly distinct
  hues (none close to the user's cursor or the accent colour). The request
  carries the agent's display name and returns the seat name and the colour.
- A seat lives as long as the agent's session: it is removed when the agent
  releases it or its requesting connection goes away. Removing a seat ends
  any grab it holds and clears the keyboard focus it gave. Its grants are
  kept for the rest of the Otto session, in case the agent comes back (see
  Workspace grants).
- Each agent seat hides its cursor on its own idle timer.
- Next to each agent cursor, a small label shows the agent's name, so the user
  can tell agents apart at a glance.
- `enabled = true` keeps its Phase 1 meaning: one static seat named `agent`,
  for stock tools.

### Workspace grants

A grant is the set of workspaces an agent seat may act on. There are three
kinds:

| Grant | How it starts | Consent |
|---|---|---|
| **Own workspace** | The agent asks for a new workspace; Otto creates it, named after the agent, and grants it | None: the workspace is new and holds nothing of the user's |
| **Current workspace** | The agent asks for the workspace the user is on | The user allows it in a prompt |
| **Every workspace** | The agent asks for all of them | The user allows it in a prompt that says so plainly |

Rules for every grant:

- Agent pointer input is hit-tested only against windows on granted
  workspaces. A click or motion that would land on anything else reaches no
  surface.
- The agent keyboard is only ever given to a window on a granted workspace.
  If that window moves to a workspace outside the grant, the agent loses its
  keyboard focus.
- A grant ends when the user revokes it, the agent releases it, or the
  workspace is removed. An agent that disconnects and connects again under
  the same name in the same Otto session gets its seat, colour and grants
  back without a new prompt; they are held for it until then. Ending the
  Otto session (logout, restart) ends every grant. An agent-owned workspace that
  ends its grant stays, with its windows, until the user closes it: it now
  belongs to the user.
- Denying a prompt, or leaving it unanswered for 30 s, refuses the grant.
  The agent is told either way.
- The same agent asking again for the same workspace while a prompt is open
  does not open a second prompt.

### The agent border

Every workspace under a grant is framed in the agent's colour — the visible
sign that an agent can act there.

- The border runs along the edges of each output that shows the workspace,
  following the screen's rounded corners when Otto draws them. It is a solid
  line (`[agent_cursor] border_width`, default 3 logical px) with a soft glow
  fading inward over about 12 px.
- It belongs to the workspace, not the output: during a workspace swipe or
  switch it moves with the workspace's content, so swiping from the user's
  workspace to the agent's shows the frame sliding in.
- It is drawn above every window, layer-shell panel and fullscreen surface,
  and below the user's cursor and agent cursors. A client cannot cover it.
- It takes no input and takes no space: the pointer passes through it, and no
  window moves or resizes because of it.
- A small chip at the top centre of the framed output shows the agent's name,
  in its colour, and a **Stop** control. Stop revokes the grant at once. The
  chip responds to the user's pointer only.
- While a window is fullscreen on an output, that output shows no chip; the
  border stays. The chip comes back when the window leaves fullscreen.
- While the agent is active the border is at full strength; after the idle
  time it dims (to about 40 %) but never disappears — it signals the grant,
  not activity. On an agent's own workspace it does not dim at all until the
  agent releases the workspace.
- It fades in over about 150 ms when a grant starts and fades out when it
  ends.
- Several agents on one workspace: the border takes the colour of the agent
  that acted most recently, and the chip lists every agent with its colour.
- An "every workspace" grant frames every workspace on every output.
- Exposé previews and the workspace selector frame the thumbnails of granted
  workspaces in the agent's colour too.
- The lock screen hides every border; they come back on unlock if the grants
  still hold.
- The border is part of what the user's screen shows, so the user's own
  screenshots and recordings include it. Captures an agent takes of its own
  workspace do not, so the agent sees its applications as they are.
- It cannot be switched off while a grant is active: its width can be
  configured, not its presence.

### Phase 3 — agent-owned workspaces

- An agent can ask for a workspace of its own, which starts an own-workspace
  grant and draws its border.
- The agent's coordinates address that workspace's space: hit-testing
  resolves against its windows whether or not it is shown on an output. The
  agent is told the workspace's logical size and scale.
- Windows on a workspace with a grant keep receiving frame callbacks at full
  rate while it is hidden, so their applications keep painting.
- The agent can capture its workspace's current image while it is hidden,
  at the workspace's output scale.
- The agent cursor is drawn only where its workspace is visible: on an output
  showing it, in exposé previews of it, and during switches that bring it on
  screen.
- Applications the agent launches through Otto open on the agent's
  workspace, including new windows of applications that are already running,
  and never take the user's focus.
- An agent's workspace never brings itself on screen: the user goes there
  (by scrolling, the workspace selector or exposé) when they want to watch.
- When the user switches to the agent's workspace, the agent keeps working;
  the user's input and the agent's go to their own seats.

### Phase 4 — current and every-workspace grants

- An agent can ask for the user's current workspace or for every workspace;
  Otto prompts, as in the grants table.
- The prompt names the agent in its colour, says what it is asking for, and
  offers **Allow** and **Deny**. It is Otto's own UI: no client can draw it,
  answer it, or cover it, and an agent seat's input never reaches it.
- A shortcut pauses every agent at once: while paused, no agent input reaches
  any surface, and each border shows a paused state. The same shortcut
  resumes them.
- The topbar shows an indicator whenever any grant is active; opening it lists
  every agent, its grants, and a Stop for each.

### Phase 5 — enforcement

Until this phase, grants hold only for clients that play along: any client
can still create virtual input on the user's seat, or capture the whole
screen.

- Virtual pointers and keyboards may be created only by clients Otto has
  authorized (the RDP bridge, agents holding a seat), and only on the seat
  they were authorized for. Other requests are refused.
- An agent's screen captures are limited to its granted workspaces. Capturing
  anything else, or the whole output, is refused.
- The static `enabled = true` seat of Phase 1 remains an unrestricted,
  explicit opt-in for stock tools, with an "every workspace" border while it
  is in use.

### Phase 6 — Otto's own UI

- An agent holding an "every workspace" grant can use exposé: open it, pick a
  window, and leave it. Exposé shows every workspace, so nothing less than a
  grant on all of them allows it.
- Last of all, the dock: an agent can use it to launch and switch to
  applications. Not needed until everything before it is in place.
- Using either needs a pointer of the agent's own in Otto's scene; until
  then, the user's pointer is the only one Otto's UI answers to.

### Phase 7 — polish

- Agent cursors appear in screen shares and recordings (configurable).
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
  keystroke streams; nothing arbitrates, beyond the pause shortcut.
- A hidden workspace has no output. Scale, geometry and capture for it are
  taken from the output it was last shown on, or the primary output for a new
  one.
- Otto places a new window on the workspace the user is looking at. Putting an
  agent's windows on its own workspace needs the launch to carry the
  workspace (an activation token), because single-instance applications open
  new windows from an existing process.
- A client can draw its own coloured frame and imitate the border. The chip,
  which only Otto draws and which sits above every surface, is the
  authoritative sign; the topbar indicator is the second. During fullscreen
  neither is shown, so the border alone marks the grant, and the pause
  shortcut is the way to stop agents without leaving fullscreen.
- The lock screen stops all agent input (see Phase 1 follow-ups). A button
  an agent holds when the session locks is released to no one: the
  application may see the press without the release.
- Drag and drop from an agent seat is not supported until it is designed; a
  drag started there must not interfere with the user's.
- The KMS path has one hardware cursor plane per output, and it belongs to the
  user. Agent cursors and borders are always composited, so they cost a
  composite when they change.
- Workspaces are independent per output. An own-workspace grant is for one
  workspace on one output; "every workspace" covers all of them on all
  outputs.

## Rationale

- **A seat, not just a coloured cursor.** Drawing a second cursor while
  agents still drive the user's pointer would only recolour the user's
  cursor. The agent needs its own pointer position and focus, and a seat is
  the Wayland object that carries them.
- **Seats per agent, grants per workspace.** A seat is who is acting; a grant
  is where they may act. Two agents on one workspace would fight over a
  shared seat, and an agent moving between workspaces would have to switch.
- **Own workspace first.** It needs no consent and cannot disturb the user's
  work, so it is the safest way to let an agent work in parallel.
- **A border, not only a cursor.** A cursor shows where an agent is; the
  border shows where it may act, including while it is idle and its cursor is
  hidden. Knowing that is what makes a grant something the user can reason
  about.
- **Border on the workspace, not the output.** The grant is to a workspace;
  a frame that stayed on the output during a switch would mark the wrong
  content.
- **No chip over fullscreen windows.** A video, game or presentation would
  have its own controls covered. The border is thin and at the edges, so it
  stays; the chip returns as soon as the window leaves fullscreen.
- **The border cannot be turned off during a grant.** An indicator the user
  can disable is one they can forget they disabled.
- **Advertised after the user's seat.** Clients that take the first seat keep
  talking to the user; tools that take the last land on the agent. That makes
  stock tools work with no changes in the one-agent case.
- **No raise or activation from agent clicks.** Otherwise the user's windows
  would jump around while they work.
- **Agent workspaces stay where they are.** Switching the user's view is a
  disruption; the user already has every way to get there, and the border
  and topbar indicator tell them it exists.
- **Grants survive a reconnect within the session.** Agents crash and
  restart; asking again every time would train the user to click Allow
  without reading. A new Otto session starts with none, so nothing is
  granted that the user did not allow since they logged in. Until Phase 5,
  an agent is recognised by its name alone, which a hostile client can
  borrow — one more reason grants are not a boundary before then.
- **D-Bus for seats and grants.** Otto already serves D-Bus
  (`org.otto.Compositor`, `org.otto.ScreenCast`), agents are processes that
  can reach it without a Wayland connection, and the caller's bus name gives
  a lifetime to tie the seat to.
- **Exposé needs the whole desktop.** It shows and switches between every
  workspace; granting it for less would reveal the rest.
- **A longer idle on the agent's own workspace.** Watching an agent work
  there is the point of going there; a cursor that keeps vanishing between
  steps makes it hard to follow.
- **Enforcement last, but planned.** Grants are useful to cooperative agents
  from Phase 3. They become a security boundary only when virtual input and
  capture are restricted, which is recorded here so no earlier phase is
  mistaken for one.
- **Opt-in.** A second seat is visible to every client, and seat handling
  varies between applications.
- **Rendering verified on hardware.** Phase 1's agent cursor was checked in a
  real `--tty-udev` session: the tint is correct and screenshots include it.

## Open Questions

- Should anything stop an agent's application from raising itself or taking
  focus on its own (xdg-activation), which would disturb the user?
- Should the user be able to take over an agent seat's pointer, beyond
  pausing?
