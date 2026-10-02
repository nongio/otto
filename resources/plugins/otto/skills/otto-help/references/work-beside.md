# Working on the desktop as an agent

Use this when the job needs you to *use* apps the way a person does: open a
program, look at it, click, type. You get a cursor and a keyboard of your own,
on a workspace of your own, framed in your colour. The person keeps their
cursor, their keyboard and the workspace they are on, and can watch you or
stop you at any time.

Do not use plain `otto-msg` for this. Plain `otto-msg` acts *as the person*:
it moves their focus and switches their workspace under them. Everything here
is `otto-msg agent`.

## The loop

```sh
otto-msg agent start "Claude"            # your name, as the person will see it
otto-msg agent launch gedit --standalone # open what you need, on your workspace
otto-msg agent capture                   # prints the path of a PNG: read it
otto-msg agent click 1532 763            # x y in that PNG's pixels
otto-msg agent type "Hello"
otto-msg agent key Return
otto-msg agent capture                   # look again: did it do what you meant?
otto-msg agent stop                      # done: the workspace is theirs now
```

Always look before you act and look again after: read the PNG `capture`
prints. Coordinates are the pixels of that image, as it is on disk, so take
them from the picture as you see it at its full size.

**The first `start` asks the person** in a dialog whether `otto-msg` may have
an agent cursor. Say so before you run it, and wait: the command returns when
they answer. If they say no, stop and tell them; do not try another way.

## Commands

| Command | What it does |
|---|---|
| `start <name>` | a seat and a new workspace of your own |
| `start <name> --lend [workspace]` | work on one of the person's workspaces instead, by name, or the one they are looking at; they are asked every time |
| `info` | your seat, your workspace and its size in pixels |
| `capture` | a PNG of your workspace; prints its path |
| `windows` | the windows on your workspace, `title  [app_id]` |
| `focus <part of a title or app id>` | give your keyboard to that window |
| `close <part of a title or app id>` | close that window |
| `launch <program> [args]` | start a program; its windows open on your workspace |
| `click <x> <y> [right\|middle]` | click |
| `double-click <x> <y>` | double-click |
| `move <x> <y>` | move your pointer, to hover |
| `drag <x> <y> <to-x> <to-y>` | press, move, release |
| `scroll <x> <y> up\|down [steps]` | scroll with the wheel, 3 steps unless told |
| `type <text>` | type into the window that has your keyboard; `\n` in the text is Return |
| `key <combo>...` | press keys: `Return`, `Tab`, `Escape`, `BackSpace`, `ctrl+s`, `ctrl+shift+t`, `alt+F4`; several in a row |
| `stop` | give everything back |

Key names are xkb's (`Return`, `Page_Down`, `F5`, `Left`), in any case.
Modifiers are `shift`, `ctrl`, `alt` and `super`.

## Things that trip you up

- **Single-instance apps.** Many apps hand a new window to a copy that is
  already running, and that window opens on the person's workspace, not
  yours. Ask for a new instance: `gedit --standalone`,
  `gnome-text-editor --standalone`, `firefox --new-instance -P agent`,
  `chromium --user-data-dir=/tmp/agent-chromium`, `code --new-window` with
  `--user-data-dir`. Check with `windows` that the window is yours.
- **Clicks need a window under them.** Click on a window that `capture`
  shows. Your pointer cannot reach the dock, the top bar, other workspaces
  or Otto's dialogs.
- **Typing goes where your keyboard is.** A new window you launch takes it.
  Otherwise click into the window or `focus` it before you `type`.
- **Things take a moment.** `capture` already waits a little after your last
  input, but an app that is loading may need a second. Capture again rather
  than assume.
- **Run other tools as yourself.** `otto-msg agent info` prints a
  `WAYLAND_DISPLAY`. A program started with it is yours: its windows open on
  your workspace. `launch` already does this.
- **The person can stop you.** If they press Stop on your frame, every
  command fails with "Otto ended the agent's seat". Do not start again on
  your own; ask them first. After a Stop, `otto-msg` is refused until they
  log in again.
- **When you finish, `stop`.** Your workspace and the windows on it stay for
  the person, and they can carry on in them. Tell them what you left there.
