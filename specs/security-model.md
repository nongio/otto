# Security Model

**Status:** draft  
**Related specs:** agent-seats.md, screenshare.md, portal-access-dialog.md, lock-screen.md, agent-permissions-review.md

## Summary

Which clients may watch the user, act as the user, or control the desktop,
and how an AI agent gets a narrow, visible, revocable share of that. Otto
does not build sandboxes. It keeps the powerful interfaces for its own
components, gives every agent a scoped view of the standard Wayland
protocols, and makes sure only the person at the keyboard can grant
anything.

## Goals

- No client but Otto's own components, and the programs the user listed,
  can capture the screen, inject input on the user's seat, read the
  clipboard without focus, or control other apps' windows.
- An agent sees and acts on the desktop through standard Wayland protocols
  only, scoped to what the user granted it: stock tools work unchanged
  inside that scope.
- An agent never moves the user's cursor, takes their keyboard focus,
  raises a window or switches their workspace without asking each time.
- Every grant is asked for in a dialog Otto draws, answered by real input
  from the person at the keyboard.
- Every grant in use is shown on screen by Otto, and the user can end it at
  any moment.

## Non-Goals

- Containing a program that runs unsandboxed as the user. It can already
  run every tool the user can, read their files and talk to their sockets;
  no compositor rule changes that. Containment is a sandbox's job (Flatpak,
  an agent's own sandbox, a separate user), and sandboxed clients are
  covered by this model through security-context.
- Building or shipping a sandbox for agents.
- Mediating accessibility (AT-SPI). It is a session-wide bus outside the
  compositor.
- A new protocol or D-Bus API for agents to see or drive the desktop.

## Behavior

### Kinds of client

Otto tells four kinds of Wayland client apart:

| Kind | How it connects |
|---|---|
| Otto component | A socket Otto handed it when starting it, or an executable on the trusted list (below) |
| User program | Otto's public socket (`WAYLAND_DISPLAY`) |
| Sandboxed app | A `wp_security_context_v1` listener (Flatpak and other sandbox engines) |
| Agent | A connection Otto made for an agent holding a seat (below) |

The trusted list is Otto's own programs installed where only root can
change them, and the executables the user lists in `[privacy]
trusted_programs`. A program is named by the executable of the process on
the other end of its socket, when it connects.

### Privileged interfaces

| Interface | Component | User program | Sandboxed | Agent |
|---|---|---|---|---|
| Screen capture (wlr-screencopy, ext-image-copy-capture) | all | — | — | its scope |
| Virtual pointer / keyboard | user's seat | — | — | its own seat |
| Data control (clipboard without focus) | yes | — | — | — |
| Window control (wlr-foreign-toplevel) | all windows | — | — | its scope |
| Window list (ext-foreign-toplevel-list) | all windows | all windows | — | its scope |
| Workspaces (ext-workspace) | all | — | — | its scope |
| Input method | yes | yes | — | — |
| Layer shell, shortcut inhibition, gamma | yes | yes | — | — |
| Session lock | Otto's locker only | — | — | — |
| Otto's private protocols and `org.otto.Shell1` | yes | — | — | — |

"—" means the interface is not offered: it does not appear in the
client's registry, and a D-Bus call is refused. A user program that needs
one is added to the trusted list. Everything else a user program needs
from the desktop goes through the portals (screen sharing, screenshots,
remote desktop), which ask the user.

### Agent sessions

1. A program asks Otto for an agent seat over D-Bus, giving the agent's
   name. The first time a program asks, the user is asked in Otto's dialog;
   the answer is remembered for that program and listed in Settings ›
   Privacy, where it can be switched off or forgotten.
2. With a seat, the agent asks for a scope: a new workspace of its own, or
   one of the user's existing workspaces. An existing workspace is asked
   for in Otto's dialog, naming it. A new one is not: it holds nothing of
   the user's.
3. The agent asks Otto for Wayland connections. Every client on such a
   connection is the agent's: Otto offers it the standard protocols, scoped
   (below), and nothing else privileged.
4. The scope is shown while it lasts: its workspace is framed in the
   agent's colour, with the agent's name and a Stop chip, and the agent's
   cursor is drawn in the same colour.
5. Stop, the agent releasing its seat, or the agent leaving the bus ends the
   session: its connections are closed and its cursor goes. Its workspace
   and windows stay, for the user.

On an agent's connection:

| Protocol | What the agent gets |
|---|---|
| ext-foreign-toplevel-list | The windows on its workspaces, with their titles. No others. |
| wlr-foreign-toplevel | The same windows. Activating one gives the agent's keyboard focus to it, on the agent's seat; the user's focus and the stacking order do not change. Closing works; maximize, minimize and fullscreen are ignored. |
| ext-workspace | Its workspaces. Activating one makes it where the agent's input lands; the user's view does not change. |
| ext-image-copy-capture | Capture sources for its workspaces' outputs and its windows only. Capturing an output gives what its workspace shows, whether or not the user is looking at it. |
| Virtual pointer / keyboard | Input on its own seat, whichever seat it names. Input reaches only windows on its workspaces. |
| xdg-activation | A token from an agent's connection places the *new* window it starts on the agent's workspace. It never moves an existing window. |

An agent acts on what the user sees only by asking each time: activating a
window or workspace outside its scope, or bringing one of its own in front
of the user, raises Otto's dialog. Nothing in its scope lets it change the
user's view.

Agent tools written for other desktops reach the same session through the
RemoteDesktop and ScreenCast portals: the portal dialog offers the same
scopes, and input arrives through libei on the agent's seat.

### Consent

- Otto's dialogs are drawn by Otto, above every client surface. No client
  can draw over them, read them, or answer them.
- They accept real input only: from input devices, not from virtual
  pointers, virtual keyboards or libei.
- A dialog names the program asking and the program that started it, and
  says exactly what is being asked for.
- Answers that last are kept in xdg-permission-store and listed in Settings
  › Privacy. Grants on the user's existing workspaces last for the session
  only.
- While the session is locked, nothing is granted and no agent input or
  capture happens.

## Constraints & Edge Cases

- **Input below the compositor.** A process that can write to `/dev/uinput`
  creates input devices Otto cannot tell from hardware, so it can answer
  Otto's dialogs. The model holds only on a host where the user's processes
  cannot: no world-writable injector daemon (ydotoold), and the user not in
  a group or ACL that grants uinput. Otto warns at startup when the user can
  write to it.
- **Identity.** A program is named by its executable, which an unsandboxed
  process can borrow (an interpreter is one program for every script). The
  trusted list relies on root owning the executables it names; an entry the
  user can write to is trusted at the user's word.
- **Breaking changes.** Tools that relied on the privileged interfaces (grim,
  wl-paste's watch mode, clipboard managers, wlrctl, waybar's taskbar) stop
  working until they are added to the trusted list.
- **XWayland.** X clients can see and drive each other inside the X server;
  Otto treats the X server as one of its own components. X clients cannot
  reach Wayland windows.
- **Multi-seat in toolkits.** An app must listen to more than one seat to
  receive an agent's input. GTK and Qt do; some do not, and an agent cannot
  drive them.
- **Accessibility.** AT-SPI exposes every app's widget tree to every client
  on the session bus. Out of this model's reach; it should be noted to users
  who rely on agents.

## Rationale

- **Guarantees, not containment.** A desktop cannot stop an unsandboxed
  program from doing what its user can. It can guarantee that grants come
  from the person at the keyboard, that they are visible, and that agents
  given less authority still have a way to work. That is the whole model.
- **A trusted list, not prompts, for the privileged interfaces.** Asking per
  program looks finer but is not: an answer for `grim` is an answer for
  every process that runs `grim`. A short list the user writes, of programs
  root owns, says what it means. KWin takes the same approach with its
  `X-KDE-Wayland-Interfaces` desktop-file key.
- **Standard protocols for agents.** Agent tools already speak them, or the
  RemoteDesktop portal with libei; scoping them per connection lets stock
  tools work inside the grant without a new API to learn or maintain.
- **The agent's own seat.** It is what lets an agent work beside the user
  without taking their cursor or focus, and no other desktop offers it.
- **Real input for consent.** If injected input could answer a dialog, an
  agent could grant itself anything.
- **Session-only grants on the user's workspaces.** An agent's own
  workspace holds nothing of the user's; the user's do, and reach into them
  should be asked for afresh.

## Open Questions

- Should the trusted list also be readable from a key in a program's
  desktop file, as KWin does, so packages can declare it?
- Should Otto warn, refuse to start agent sessions, or only document it when
  the user can write to `/dev/uinput`? Closing it removes the Steam
  controller and KDE Connect remote input.
- Does any agent need the user's whole desktop (every workspace) as a scope,
  and if so, how is that shown?
- Which window titles may an agent see in a workspace it is granted that
  also holds the user's windows: all of them, or only windows it started?
