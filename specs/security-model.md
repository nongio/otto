# Security Model

**Status:** draft  
**Related specs:** agent-seats.md, screenshare.md, portal-access-dialog.md, lock-screen.md

## Summary

Which programs may watch the screen, act as the user or control the
desktop, and how an AI agent gets a narrow, visible and revocable share of
that. Otto cannot contain a program that runs as the user; what it
guarantees is that every grant comes from the person at the keyboard, that
every grant in use is on screen, and that an agent given less than the user
still has a standard way to work.

## Goals

- Only Otto's own programs, and the user's programs unless the user turns
  that off, can capture the screen, type or point as the user, read the
  clipboard without focus, or control other programs' windows.
- A sandboxed app gets none of that from the compositor; it asks through
  the portals.
- An agent sees and acts on the desktop through standard Wayland protocols,
  so stock tools work, but only within one workspace and on a seat of its
  own.
- An agent never moves the user's cursor, takes their keyboard focus,
  raises a window in front of them or switches their workspace.
- Every lasting grant is asked for in a dialog only Otto's programs can
  word, and every grant in use is shown on screen until it ends.
- The user can end any agent's grant at any moment, and every agent's at
  once with a key no program can send.

## Non-Goals

- Containing a program that runs as the user outside a sandbox. That is a
  sandbox's job (Flatpak, an agent's own sandbox, another user account).
- Building or shipping a sandbox for agents.
- Accessibility (AT-SPI), a session-wide bus that exposes every app's
  contents outside the compositor.
- A new protocol for agents to see or drive the desktop. Agents use the
  standard ones; Otto adds only a way to ask for a seat, a workspace and a
  connection.

## Behavior

### Kinds of client

Otto sorts every Wayland client into one of five kinds when it connects.

| Kind | How Otto knows |
|---|---|
| Otto component | Otto started it on a socket of its own (the locker, the password panel), or its executable has one of Otto's names and only root can change the file, or it is installed beside the running Otto |
| User program | It connected to Otto's public socket; it is named by the executable on the other end, held so the process cannot be swapped under the name |
| Sandboxed app | It connected through a security-context listener a sandbox engine made (Flatpak does) |
| Agent client | It connected on a socket Otto made for an agent that holds a seat, or through a security-context listener such a client made |
| Handed-over client | It was an agent client and the agent's seat has ended |

XWayland is treated as one of Otto's own: X clients see and drive each
other inside the X server, and cannot reach Wayland windows.

### Privileged interfaces

A "—" means the interface is not in the client's registry at all.

| Interface | Component | User program | User program, `strict` | Sandboxed | Agent client | Handed over |
|---|---|---|---|---|---|---|
| Screen capture (wlr-screencopy) | yes | yes | if listed | — | — | — |
| Screen capture (ext-image-copy-capture) | yes | yes | if listed | — | its workspace, its windows | — |
| Virtual pointer and keyboard | user's seat | user's seat | if listed | — | its own seat only | — |
| Clipboard without focus (data-control) | yes | yes | if listed | — | — | — |
| Window control (wlr-foreign-toplevel) | every window | every window | if listed | — | its workspace's windows | — |
| Window list (ext-foreign-toplevel-list) | every window | every window | every window | — | its own windows | none |
| Input method, layer shell, gamma, shortcut inhibition | yes | yes | yes | — | — | — |
| Otto's dock and text-cursor protocols | yes | yes | yes | — | — | — |
| Security contexts | yes | yes | yes | — | yes, for more of its clients | yes, as a plain sandbox |
| Session lock | Otto's locker only | — | — | — | — | — |
| `wl_seat` | user's | user's | user's | user's | its own, then the user's | the user's |

- `[privacy] strict = true` keeps the user's programs off the first five
  rows; `[privacy] allow` lists executables, by full path, that keep them.
  Both are read when Otto starts.
- Interfaces that only style a client's own surfaces are offered to
  everyone.
- A handed-over client keeps what it had bound, but none of it reaches a
  window or the screen any more: its window lists announce nothing, its
  window requests are ignored and its captures stop.
- While the session is locked, no capture produces a frame and no agent
  input reaches anything.

### Otto's bus interfaces

| Interface | Who may call |
|---|---|
| `org.otto.ScreenCast` | Otto's portal backend and its RDP bridge |
| `org.otto.Dialog1` (the islands dialog) | the compositor, the portal backend, otto-agents, Files and islands |
| `org.otto.Compositor` focus requests | Otto's bar, launcher, islands and Settings |
| `org.otto.Compositor` agent methods | anyone; a seat is asked for (agent-seats.md) |
| `org.otto.Shell1` methods, the keyboard monitor for assistive technology | anyone; under `strict`, components and listed programs only |
| `org.otto.Settings` protected settings (the locker, the login screen, locking, the lid and power button) | anyone, after the user types their password |

A caller is named by the executable behind its bus connection, as a Wayland
client is.

### Consent

- Dialogs are drawn by islands, one of Otto's components, and only Otto's
  own programs can put words in them. The compositor acts on an answer only
  when it came from islands.
- A grant given within 600 ms of a dialog appearing is ignored, so a click
  or an Enter already on its way does not answer it.
- A program is asked once whether it may have agent seats; the answer is
  kept and listed in Settings › Privacy › Agents, with a switch and Forget.
  Turning a program off, or forgetting it, ends the seats it holds.
- Lending one of the user's workspaces to an agent is asked each time and
  never kept.

### Ending agents

- Stop on an agent's frame ends that agent's seat and refuses the program
  further seats until the user's next session. Only the user's real pointer
  can press Stop.
- The secure attention key (Ctrl+Alt+Shift+Esc, delivered by logind, not by
  Otto) ends every agent's seat at once, and is beyond the reach of any
  program in the session.

## Constraints & Edge Cases

- **Input below the compositor.** A process that can write to `/dev/uinput`
  makes input devices Otto cannot tell from hardware, and can answer Otto's
  dialogs. The model holds only on a system where the user's processes
  cannot: no world-writable injector daemon (ydotoold), and the user not in
  a group or ACL that grants uinput. The secure attention key comes from the
  kernel and logind, below any of that.
- **Names are borrowed.** An executable's name can be borrowed by any
  unsandboxed process, and an interpreter is one program for every script it
  runs. Component names are trusted because root owns the files; a program
  installed beside a development build of Otto in the user's own directory
  is trusted exactly as far as that Otto is.
- **Seat consent names programs differently.** A program asking for a seat
  is named by its Flatpak id, else its desktop entry, else the file name of
  its executable, so every script under one interpreter shares one answer.
- **`strict` breaks tools.** grim, `wl-paste --watch`, clipboard managers,
  wlrctl and third-party taskbars stop working until listed in `allow`.
- **The window list is open.** Under `strict` too, every unsandboxed program
  sees every window's title and app id, through ext-foreign-toplevel-list
  and through `org.otto.Shell1` signals.
- **Capture buffers.** ext-image-copy-capture takes shared-memory buffers;
  an output capture by the user's programs also takes dmabufs on real
  hardware. An agent's "output" is its workspace, which the protocol does
  not foresee.
- **The secure attention key needs a real session.** It is heard only when
  Otto runs on the hardware, not nested and not as the login screen, and
  only if logind is set up to deliver it.

### Not yet as specified

Places where Otto today does less than this model asks:

- A program launched for an agent on its workspace is an ordinary user
  program, with the user's interfaces, not an agent client.
- A window that activates with an agent's launch token moves to the agent's
  workspace even if the user already had it open, and launch tokens do not
  expire.
- A window an agent opens before it has a workspace opens on the user's and
  takes the user's keyboard.
- A virtual pointer on the user's seat, which every user program has by
  default, can click a consent dialog once it is armed.
- A stop lasts until Otto restarts, not until the user logs out.
- No limits on how many seats, workspaces, connections, launches or
  captures an agent may ask for; captures are not deleted.
- An agent back under the same name gets the seat name of its earlier
  session, and with it that session's windows in its own window list.
- Agent names may hold invisible and direction-changing characters, and
  the seventh agent repeats the first one's colour.
- `org.otto.Shell1` can move windows between workspaces, and so into or
  out of an agent's, and is open to every user program unless `strict`.

## Rationale

- **Guarantees, not containment.** Containing a program that runs as the
  user is impossible from the compositor; promising it would be false.
  Consent from the keyboard, visibility and a scoped standard path for
  agents are what a compositor can hold.
- **Open by default, with one strict switch, rather than per-program
  questions.** An answer for `grim` is an answer for everything that runs
  `grim`, so per-program questions look finer than they are. KWin dropped
  its desktop-file allowlist as pseudo-security in 2026; sway and Hyprland
  added allowlists the user writes. Otto keeps the defaults every desktop
  has and offers one switch and one list.
- **Components by socket or by root-owned file.** A name alone is anyone's;
  a socket Otto handed out, or a file only root can change, is not.
- **Standard protocols on the agent's own connection.** Agent tools already
  speak them, and scoping them per connection lets stock tools work inside
  the grant with no new API. A sandbox around the agent that hands every
  client inside the same socket scopes them all.
- **Agent clients see the user's seat too.** So the user can click and type
  in the agent's windows while it works, and those windows go on working
  for the user once the agent has gone. The agent's seat comes first, so a
  toolkit that uses only the first seat takes the agent's input.
- **Handed over, not closed.** Closing an agent's windows when it leaves
  would throw away the user's half of the work; leaving them with the
  agent's reach would leave the agent's reach behind. They stay, on the
  user's seat, with no reach of their own.
- **Stop is final for the session.** An agent that could ask again at once
  would train the user to click Allow. The secure attention key is the one
  Stop no program can race or fake.
- **No dialog for coming to the front.** It would be a per-action question
  the user learns to click through; the user already has every way to go
  and look.

## Open Questions

- A workspace capture source (wayland-protocols !463) in place of the
  agent's output standing for its workspace, once the protocol lands.
- A topbar indicator listing live agent sessions with Stop, for agents
  whose workspace is not on screen, and a Stop that works over a fullscreen
  window.
- Accepting only input from real devices in consent dialogs, which needs
  to know where an input event came from.
- The RemoteDesktop and ScreenCast portals, with libei, as a second door to
  the same agent sessions for tools written for other desktops.
- Should Otto warn, in Settings › Privacy, when the user can write to
  `/dev/uinput`? Refusing agent sessions would stop no attacker and would
  break Steam and KDE Connect.
- Should a stop be lifted from Settings › Privacy before the user logs in
  again?
