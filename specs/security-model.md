# Security Model

**Status:** draft  
**Related specs:** agent-seats.md, screenshare.md, portal-access-dialog.md, lock-screen.md, agent-permissions-review.md

## Summary

Which clients get the interfaces that watch the user, act as the user or
control the desktop, and how an AI agent gets a narrow, visible, revocable
share of them. Four rules: a sandboxed app gets none; Otto's own components
get them all; the user's other programs get them by default, or only by
allowlist under `[privacy] strict`; an agent gets the standard protocols on
a connection of its own, scoped to one seat and one workspace.

Otto does not contain programs. A program running unsandboxed as the user
can do everything the user can, and no compositor rule changes that. What
Otto guarantees is that grants come from the person at the keyboard, that
every grant in use is visible, and that an agent given less than the user
still has a standard way to work.

## Goals

- No client but Otto's components, and under `strict` the executables the
  user listed, can capture the screen, inject input on the user's seat,
  read the clipboard without focus or control other apps' windows.
- An agent sees and acts on the desktop through standard Wayland protocols
  on its own connection, scoped to its seat and workspace: stock tools work
  unchanged inside that scope.
- An agent never moves the user's cursor, takes their keyboard focus,
  raises a window on them or switches their workspace.
- Every lasting grant is asked for in a dialog Otto draws, and only Otto's
  own programs can put a question in it.
- A grant in use is shown on screen by Otto, and the user can end it at any
  moment, including with one key no program can fake.

## Non-Goals

- Containing a program that runs unsandboxed as the user. Containment is a
  sandbox's job (Flatpak, an agent's own sandbox, a separate user); a
  sandbox's clients are covered here through security contexts, and so is a
  sandboxed agent's, through the agent's own listener.
- Building or shipping a sandbox for agents.
- Mediating accessibility (AT-SPI), a session-wide bus outside the
  compositor.
- A new protocol or D-Bus API for agents to see or drive the desktop beyond
  asking for a seat, a workspace and a connection.

## Behavior

### Kinds of client

| Kind | How Otto knows |
|---|---|
| Otto component | A socket Otto handed it when starting it (the locker, the polkit agent), or an executable with one of Otto's own names installed where only root can change it, or built beside the running Otto in the user's own directory |
| User program | Otto's public socket (`WAYLAND_DISPLAY`); named by the executable of the process on the other end, pinned by a pidfd so a pid cannot be reused under it |
| Sandboxed app | A `wp_security_context_v1` listener a sandbox engine made (Flatpak does) |
| Agent | A connection Otto made for an agent holding a seat (`ConnectAgent`), or a `wp_security_context_v1` listener the agent made on such a connection |

### The privileged interfaces

| Interface | Component | User program (default) | User program (`strict`) | Sandboxed | Agent |
|---|---|---|---|---|---|
| Screen capture (wlr-screencopy) | yes | yes | allowlist | — | — |
| Screen capture (ext-image-copy-capture, output and toplevel sources) | yes | yes | allowlist | — | its workspace and its windows |
| Virtual pointer and keyboard | user's seat | user's seat | allowlist | — | its own seat only |
| Clipboard without focus (data-control, ext-data-control) | yes | yes | allowlist | — | — |
| Window control (wlr-foreign-toplevel) | all windows | all windows | allowlist | — | its workspace's windows |
| Window list (ext-foreign-toplevel-list) | yes | yes | yes | — | its own windows |
| Input method, layer shell, shortcut inhibition, gamma, Otto's protocols | yes | yes | yes | — | — |
| Security contexts | yes | yes | yes | — | yes (its own clients) |
| Session lock | Otto's locker only | — | — | — | — |

"—" and "allowlist" mean the global is not in the client's registry.
`[privacy] strict` keeps the user's programs off the first four rows;
`[privacy] allow` names executables, by full path, that keep them. Both are
read when Otto starts.

### Otto's bus interfaces

| Interface | Who may call |
|---|---|
| `org.otto.ScreenCast` | the portal backend and the RDP bridge |
| `org.otto.Dialog1` (islands' dialog) | the compositor, the portal backend, otto-agents, Files, islands |
| `org.otto.Compositor.FocusApp` | Otto's interface components |
| `org.otto.Compositor` agent methods | anyone; a seat is asked for, below |
| `org.otto.Shell1`, `org.freedesktop.a11y.KeyboardMonitor` | anyone by default; components and the allowlist under `strict` |
| `org.otto.Settings` protected settings (the locker, the greeter, locking) | anyone, after polkit asks the user |

A caller is named like a Wayland client, by the executable behind its bus
connection. The compositor acts on a consent answer only when it came from
islands.

### Agent sessions

1. A program asks for a seat over D-Bus, naming its agent. The first time a
   program asks, the user is asked in Otto's dialog; the answer is kept for
   that program (xdg-permission-store, listed in Settings › Privacy with a
   switch and Forget). A program the user stopped (below) is refused until
   they log in anew.
2. With a seat, the agent asks for a workspace: a new one of its own,
   named after the agent, that the user is not switched to; or one of the
   user's, by name or the one they are looking at (`ListWorkspaces`,
   `RequestWorkspace`), which the user is asked about each time in Otto's
   dialog. On one of the user's, the agent's cursor is drawn beside theirs
   and it reaches the windows there: for the length of the loan the user's
   programs see the agent's seat too, so it can click and type in their
   windows; the user's cursor and focus stay theirs. One workspace per seat; asking for another moves the seat.
3. The agent asks for a connection (`ConnectAgent`). Everything on it is
   the agent's: it sees the agent's seat and no other, and every other
   client sees the user's seat and not the agent's, but for a workspace the
   user lent it (step 2). A `wp_security_context_v1`
   listener made on it connects more of the agent's clients, with the
   protocol sandboxes speak; they go with the seat.
4. While the seat lasts, its workspace is framed in the agent's colour with
   the agent's name and a Stop chip, and the agent's cursor is drawn in that
   colour wherever its workspace shows.
5. Stop, the agent releasing its seat, or the agent leaving the bus ends the
   session: the seat and its connections go, the cursor goes, the workspace
   and its windows stay for the user. A workspace of the agent's own waits
   for it to come back under the same name; one the user lent it does not. Stop also suspends the program's
   consent for the rest of the login session. The secure attention key
   (Ctrl+Alt+Shift+Esc, delivered by logind) stops every agent at once.

On an agent's connection:

| Protocol | What the agent gets |
|---|---|
| `wl_seat` | Its own seat alone |
| Virtual pointer and keyboard | Input on its own seat whichever seat it names; it reaches only windows on its workspace |
| xdg-shell | Its windows open on its workspace and take its keyboard, never the user's |
| wlr-foreign-toplevel | The windows on its workspace, with titles. Activating gives the agent's keyboard to one; closing closes one; maximize, minimize and fullscreen are ignored |
| xdg-activation | A window a program launched for the agent opens on the agent's workspace; an existing window never moves |
| ext-foreign-toplevel-list | Its own windows, with titles |
| ext-image-copy-capture | An output source shows its workspace, whether or not the user is looking at it, and never the bars, the dialogs or the user's cursor; a toplevel source works for its own windows alone |
| `CaptureWorkspace` (D-Bus) | The same workspace as a PNG, for agents that only speak D-Bus |

Nothing an agent holds changes what the user sees: it never brings a window
or a workspace in front of the user, and there is no dialog to ask for that.
The user goes to the agent's workspace when they want to watch.

### Consent

- Dialogs are drawn by islands, one of Otto's components, and only Otto's
  own programs can put words in them (above).
- A grant given within 600 ms of a dialog appearing is ignored: a click or
  an Enter already on its way does not answer it.
- Answers that last are kept in xdg-permission-store and listed in Settings
  › Privacy. Lending a workspace is asked each time and not kept. A stop
  lasts the login session.
- While the session is locked, no agent input or capture happens.

## Constraints & Edge Cases

- **Input below the compositor.** A process that can write to `/dev/uinput`
  makes input devices Otto cannot tell from hardware, and can answer Otto's
  dialogs. The model holds on a host where the user's processes cannot: no
  world-writable injector daemon (ydotoold), and the user not in a group or
  ACL that grants uinput. The secure attention key is handled by the kernel
  and logind, below any of that.
- **Identity.** A program is named by its executable, which an unsandboxed
  process can borrow (an interpreter is one program for every script it
  runs). The allowlist and the component names rely on root owning the
  executables; an entry in the user's own directory is trusted at the
  user's word.
- **`strict` breaks tools.** grim, wl-paste in watch mode, clipboard
  managers, wlrctl and taskbars outside Otto stop working until listed in
  `allow`.
- **XWayland.** X clients see and drive each other inside the X server; Otto
  treats the X server as one of its own. X clients cannot reach Wayland
  windows.
- **Windows moved by the user.** A window the user moves onto an agent's
  workspace comes into its scope and is not announced to the agent's window
  lists until it reconnects; one moved out is not withdrawn. The agent's
  input and the workspace capture follow the workspace at once.
- **Capture buffers.** Capture through ext-image-copy-capture takes wl_shm
  buffers; an output capture by the user's programs also takes a dmabuf on
  the udev backend. An agent's "output" shows its workspace, which the
  protocol does not foresee (wayland-protocols !463 proposes a workspace
  source); a capture of an output also deviates for an agent in that the
  output is its workspace's.
- **Toolkits and seats.** An app must listen to the seat it is offered. On
  an agent's connection that is the agent's seat alone, so every toolkit
  binds the right one; a toolkit that only ever uses the first seat is fine.
- **Accessibility.** AT-SPI exposes every app's widget tree to every client
  on the session bus, outside this model.

## Rationale

- **Guarantees, not containment.** Containing a same-user program is
  impossible from the compositor; promising it would be false. Consent from
  the keyboard, visibility and a scoped standard path for agents are what a
  compositor can hold, and they are what agents lack on every other desktop.
- **Default open, with a strict switch, instead of per-program questions.**
  Asking per program looks finer but is not: an answer for `grim` is an
  answer for everything that runs `grim`. KWin dropped its desktop-file
  allowlist as pseudo-security in 2026; sway and Hyprland added user-written
  allowlists. Otto keeps the defaults every desktop has and offers one
  switch and one list for users who want less.
- **Components by socket or by root-owned name.** A name alone is anybody's;
  a socket Otto handed out, or a file only root can change, is not. Building
  beside Otto in the user's own directory is trusted exactly as far as Otto
  itself is: whoever can write there can replace Otto.
- **Standard protocols on the agent's connection.** Agent tools already
  speak them, and scoping them per connection lets stock tools work inside
  the grant without a new API. The agent's connection is the scope, so a
  sandbox around the agent, which hands every client inside the same
  socket, scopes them all.
- **One seat per agent, one workspace per seat.** A seat is what lets an
  agent work beside the user without taking their cursor or focus. One
  workspace keeps "where the agent's input lands" a question with one
  answer.
- **Stop is final for the session.** A stopped agent that could ask again
  at once would train the user to click Allow. The secure attention key is
  the one Stop no program can race or fake.
- **No dialog to come to the front.** It would add a per-action question
  the user learns to click through. The user already has every way to go
  and look.

## Open Questions

- A workspace capture source (wayland-protocols !463) in place of the
  deviation that an agent's output is its workspace, once the protocol
  lands.
- A persistent topbar indicator listing live agent sessions with Stop, for
  an agent whose workspace is not on screen.
- Islands' dialog above every other overlay while it is up, and ignoring
  input that did not come from a device, which needs provenance on input
  events.
- The RemoteDesktop and ScreenCast portals, with libei, as a second door to
  the same agent sessions for tools written for other desktops.
- Should Otto warn, or show in Settings › Privacy, when the user can write
  to `/dev/uinput`? Refusing agent sessions would stop no attacker and break
  Steam and KDE Connect.
- Should a stop be lifted from Settings › Privacy before the user logs in
  again?
