# Agent Seats

**Status:** draft  
**Related specs:** security-model.md, pointer-input-focus.md, workspaces-multi-output.md, lock-screen.md, screenshare.md

## Summary

An agent works on the desktop the way a person does, with a pointer and a
keyboard, on a seat of its own. It is given one workspace, a new one of its
own or one the user lends it, and acts there beside the user without moving
their cursor or taking their focus. The workspace is framed in the agent's
colour for as long as the agent holds it, the user can stop the agent at any
moment, and when the agent leaves, the workspace and its windows are the
user's.

## Goals

- An agent's input never moves the user's cursor, raises or activates a
  window, changes the user's keyboard focus or switches their workspace.
- An agent reaches only the windows of the one workspace it was given.
- Stock tools (wlrctl, wtype, ext-image-copy-capture clients) work on an
  agent's connection unchanged.
- Several agents can work at once, each on its own seat, in its own colour.
- The user can see, at any time, which workspace an agent can act on, and
  end it from there.
- The user can work in an agent's windows while it works, and keeps them
  when it leaves.

## Non-Goals

- Agents using Otto's own interface: the dock, exposé, the app switcher,
  the top bar.
- XWayland apps, which know a single seat.
- Arbitrating between an agent and the user typing into the same window.
- Judging which agents to trust. Otto asks the user and enforces the
  answer.
- A pause for all agents, and a top bar indicator; see Open Questions.

## Behavior

### Asking

Agents ask on the session bus, `org.otto.Compositor`. A seat belongs to the
bus connection that asked for it: when that connection leaves the bus, its
seats end.

| Method | What it does | Asks the user |
|---|---|---|
| `RequestAgentSeat(name: s) -> (seat: s, color: s)` | A seat for an agent called `name` | The first time the program asks (security-model.md) |
| `ReleaseAgentSeat() -> b` | Ends every seat the caller holds | No |
| `RequestOwnWorkspace() -> (output: s, x: i, y: i, width: i, height: i, scale: d)` | A new workspace for the seat, or the one it already has | No |
| `ListWorkspaces() -> a(ssb)` | Every workspace, with its output and whether it is shown | No |
| `RequestWorkspace(name: s) -> (output: s, x: i, y: i, width: i, height: i, scale: d)` | One of the user's workspaces, by name, or the one the user is looking at for `""` | Every time |
| `ReleaseOwnWorkspace() -> b` | Ends the seat's workspace, whichever kind | No |
| `ConnectAgent() -> h` | A new Wayland connection that is the agent's; one client each | No |
| `LaunchOnOwnWorkspace(argv: as) -> u` | Starts a program whose window opens on the agent's workspace; returns its pid | No |
| `CaptureWorkspace(workspace: s) -> s` | A PNG of the agent's workspace, for agents that only speak D-Bus | No |

- Every method but the first needs a seat. Launching and capturing also
  need a workspace.
- A name is 1 to 32 characters with no control characters. A name another
  connection holds a seat under is refused; asking again from the holder
  returns the seat it has.
- The returned rectangle is the space the agent's absolute pointer
  coordinates address, in logical pixels, with the output's scale.
- A refusal says why: the program is not allowed, was stopped, is already
  being asked, the user said no, or no dialog could be shown.

### The seat

- A seat is named `agent-<n>` and has a colour from a palette of distinct
  hues, none close to the user's cursor. The same agent name gets the same
  seat name and colour back within an Otto session.
- It has a pointer and a keyboard. Its cursor is the theme's arrow in the
  agent's colour, with the agent's name in a label beside it, drawn under
  the user's cursor and never on the hardware cursor plane.
- The cursor appears with the agent's first motion and only where the
  agent's workspace is the one an output shows. Exposé and the workspace
  selector do not show it.
- After `[agent_cursor] hide_after_ms` (default 5000; 0 never) without
  agent input the cursor fades out over about 200 ms; the next input shows
  it at once at the agent's pointer. It stays while an agent button is held.
- Cursor shapes asked for on an agent's seat are ignored.

### The workspace

Each seat has at most one workspace; asking for another replaces it.

- **Its own.** Added after the last workspace on the primary output and
  named after the agent. The user is not switched to it. The name is not
  saved: it does not come back after a restart.
- **Lent by the user.** Otto asks the user in its dialog each time, naming
  the agent, the workspace and the program. While it is lent, the user's
  programs see the agent's seat as well, so the agent can click and type in
  their windows.
- A seat without a workspace reaches nothing: its pointer moves and its
  cursor is drawn, and that is all.
- Windows on an agent's workspace keep drawing at full rate while the
  workspace is not shown.
- Otto never brings an agent's workspace on screen; the user goes there
  when they want to watch.

### Acting

- The agent's pointer is hit-tested against the windows of its workspace,
  whether or not the workspace is shown. It never reaches anything else:
  not other workspaces, not the bars, the dock or Otto's dialogs.
- A press gives the agent's keyboard to the window under it, without
  raising it or making it look active. The user's keyboard stays where it
  was.
- The agent's keyboard is only ever given to a window on its workspace.
  Keys sent while its focus is elsewhere reach nothing.
- While the session is locked or locking, no agent motion, press, scroll or
  key reaches anything, the lock surface included. Locking takes the
  agent's focus away and unlocking does not give it back.
- The agent's clipboard and primary selection are its seat's, not the
  user's.

### The agent's connection

Everything on a connection from `ConnectAgent` is the agent's, and so is
every client that connects through a security-context listener made on it.
On such a connection:

| Protocol | What the agent gets |
|---|---|
| `wl_seat` | Its own seat first, then the user's; no other agent's |
| Virtual pointer and keyboard | Input on its own seat, whichever seat it names |
| xdg-shell | Its windows open on its workspace and take its keyboard, not the user's; a window maximized as it opens stays there |
| xdg-activation | A window it presents gets the agent's keyboard and nothing more; nothing comes forward for the user |
| wlr-foreign-toplevel | The windows on its workspace, with titles. Activate gives the agent's keyboard to one, close closes one; the rest is ignored |
| ext-foreign-toplevel-list | The windows it opened, with titles |
| ext-image-copy-capture | An output source shows its workspace, shown or not, without the frame, the bars or anyone's cursor. A window source works for its own windows on its workspace |

A program started with `LaunchOnOwnWorkspace` is the user's program, not an
agent client; only where its window opens follows the agent.

### Capturing

- `CaptureWorkspace` takes the workspace's id, its name (any case) or `""`
  for the caller's own, and must name the caller's workspace.
- The PNG is the wallpaper and the windows at the output's resolution,
  without the frame, the bars and the dock, written to a directory only the
  user can read.
- It is refused while the session is locked.

### The frame

Every workspace an agent holds is framed in its colour.

- A line `[agent_cursor] border_width` logical pixels wide (default 3)
  along the edges of the output showing the workspace, following the
  screen's rounded corners, with a glow fading inward over about 12 px.
- It belongs to the workspace: it slides in and out with it during a swipe
  or switch.
- It is drawn above the workspace's windows and below the top bar, the dock
  and overlays. It takes no input and no space.
- A chip at the top centre shows the agent's name in its colour and a
  **Stop** control. Only the user's real pointer can press Stop.
- While a window on the workspace is fullscreen, the chip is hidden and the
  line stays.
- It fades in over about 150 ms when the agent gets the workspace and goes
  at once when it ends.
- Exposé keeps the frame on the workspace, without the chip; Stop is not
  offered there.
- Thumbnails in exposé and the workspace selector show a solid ring in the
  agent's colour along the edge, the same whether or not the workspace is
  selected, with a badge holding the agent's arrow on the top-left corner.
- The user's own screenshots and recordings include the frame; the agent's
  captures of its workspace do not.
- Its width can be changed; it cannot be turned off.

### Ending

A seat ends when the agent releases it, when its bus connection goes, when
the user presses Stop, when the user turns the program off or forgets it in
Settings › Privacy, or with the secure attention key. When it ends:

- Its cursor, its frame and its access go.
- Its workspace stays, with its windows, unframed. A workspace Otto made
  for the agent loses the agent's name; a lent one was the user's all
  along.
- The agent's windows stay too, for the user. They go on working on the
  user's seat, but what the agent's clients had bound reaches nothing any
  more: no window lists, no window control, no capture, no input.
- The same agent asking again later starts over: it asks for a workspace
  anew, and after a Stop its program is refused until the user's next
  session.

## Constraints & Edge Cases

- **Toolkits and seats.** On an agent's connection the agent's seat comes
  first, so toolkits that only use the first seat take the agent's input,
  and the user's seat is what keeps the window working once the agent has
  gone. On a lent workspace, the user's programs already running hear of
  the agent's seat only when the loan starts: Qt and plain libwayland apps
  (foot) take it, so the agent can type in them; GTK 3 and 4 apps ignore a
  seat that appears after they started, so the agent sees them but cannot
  type in them. Chromium did not take the agent's clicks on a lent
  workspace when tried.
- **A hidden workspace has no output.** Its scale and size are those of the
  output it was last shown on, or the primary output for a new one.
- **Single-instance apps.** A new window from a program that is already
  running opens where the program puts it; only a launch through Otto
  carries the agent's workspace with it.
- **Two agents on one workspace.** The frame and chip show the agent that
  got the workspace last.
- **A client can imitate the frame.** The chip, drawn by Otto, is the sign
  to trust. Over a fullscreen window there is no chip, so the secure
  attention key is the way to stop agents without leaving fullscreen.
- **A button held at lock.** It is released to no one: the app may see the
  press without the release.
- **Drag and drop** from an agent's seat is not supported.
- **Composited cursors.** Agent cursors and frames are composited, never on
  the hardware cursor plane, so a change costs a redraw.

## Rationale

- **A seat, not a coloured cursor.** The agent needs its own pointer
  position and focus; a seat is the Wayland object that carries them.
- **One workspace per seat.** "Where can the agent's input land" stays a
  question with one answer, and the frame shows it.
- **Its own workspace first.** It needs no question and cannot disturb the
  user's work. Lending one of the user's is asked every time, because what
  is on it changes.
- **A frame, not only a cursor.** The cursor shows where an agent is; the
  frame shows where it may act, idle or not.
- **The frame on the workspace, not the output.** The grant is to a
  workspace; a frame that stayed on the output during a switch would mark
  the wrong content.
- **No chip over fullscreen.** It would cover a video's or a game's own
  controls; the line stays at the edges.
- **No raise or activation from agent input.** Otherwise the user's windows
  would jump around while they work.
- **Agent workspaces stay where they are.** Switching the user's view is a
  disruption; they have every way to get there.
- **Nothing kept for a reconnect.** A grant held for an agent that left
  would outlive the frame that shows it. Asking again is one call.
- **The workspace and windows go to the user.** Ending an agent should not
  throw work away, and should not leave its reach behind.
- **D-Bus for asking.** Agents can reach it without a Wayland connection,
  and the bus connection gives the seat a lifetime.

## Open Questions

- A shortcut that pauses every agent, and a top bar indicator listing live
  agents with Stop.
- Dimming the frame while its agent is idle, and fading it out when it
  ends.
- The chip listing every agent when several share a workspace.
- Should the user be able to take over an agent's pointer?
- Agent cursors in screen shares and recordings, following the cursor theme
  and size.
