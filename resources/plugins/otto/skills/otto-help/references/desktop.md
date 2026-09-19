# Using the desktop

Everything on this page happens on the desktop the person is looking at right
now: a message in the island, an app opening, a window moving. Do not improvise
commands — every one you need is written out below.

## Step 1 — check Otto is running

```sh
busctl --user list | grep -E 'org.otto.(Shell1|Island)'
```

- **`org.otto.Shell1`** → windows, workspaces and monitors can be driven.
- **`org.otto.Island`** → the island can be spoken to.
- **Neither** → say "Otto does not seem to be running, so I cannot do that from
  here", and stop.

## Saying something

Three ways, smallest first. Pick the smallest one that fits.

### A notification

For anything that can be missed. It slides out of the island and goes away by
itself.

```sh
notify-send -a "Otto" "The dock is on the left now"
notify-send -a "Otto" "Backup finished" "Six hundred files, four minutes."
```

The first string is the title, an optional second is the body. `-u low`,
`-u normal` (the default) or `-u critical` sets how loudly it arrives; a
critical one stays until it is dismissed, so keep it for things that genuinely
cannot wait.

**Never pass `-A`.** An action button makes `notify-send` sit there waiting for
a click, and the command never returns.

### An activity in the island

For something that is going on and will end — a job you are running, a file
being fetched. It sits in the island until you take it away.

```sh
# app_id, title, icon, progress, timeout_ms, priority, live
busctl --user call org.otto.Island /org/otto/Island org.otto.Island1 \
    CreateActivity sssdusb "otto" "Tidying the Downloads folder" "otto-files" \
    -- -1 0 "normal" false
```

It prints `t 9` — the `9` is the activity's id, which you need to change or
remove it. Three things to get right:

- The `--` before the progress number. Without it `busctl` reads `-1` as one of
  its own options and refuses.
- `progress` is `-1` for no progress bar, or `0.0` to `1.0`.
- `icon` is an icon name from the theme — `otto-files`, `otto-settings`,
  `folder`, `dialog-information`. An empty or unknown name draws an envelope.

Change the title or the progress as the job moves:

```sh
busctl --user call org.otto.Island /org/otto/Island org.otto.Island1 \
    UpdateActivity tsd 9 "Nearly there" 0.8
```

An empty title leaves the title alone; a negative progress clears the bar.

Take it away when the job ends — nothing else will:

```sh
busctl --user call org.otto.Island /org/otto/Island org.otto.Island1 \
    DismissActivity t 9
```

`priority` is `low`, `normal`, `high` or `critical`. `timeout_ms` and `live`
are part of the call but nothing reads them yet, so pass `0` and `false`.

### A question

Ask with **your own question tool** — `AskUserQuestion` on Claude,
`request_user_input` on Codex. On Otto it already comes out as a panel in the
island, and the answer comes back to you. Do not build a dialog by hand for an
ordinary question.

`org.otto.Dialog1` is the interface behind that panel. It blocks until the
person answers, so call it only when you have no question tool at all:

```sh
# app_id, title, subtitle, body, icon, grant, deny, open, modal, choices
busctl --user call org.otto.Island /org/otto/Dialog org.otto.Dialog1 \
    PresentQuestion ssssssssba\(ssa\(sss\)s\) "otto" "Empty the Trash?" "" \
    "This cannot be undone." "user-trash" "Empty" "Cancel" "" false 0
```

It answers `(u a(ss))`: `0` confirmed, `1` cancelled, `2` ended, `3` the open
button. The trailing `0` is "no choice groups" — a plain confirm.

## Opening an app

Start it detached, or the command sits there until the window is closed:

```sh
setsid otto-files >/dev/null 2>&1 &
```

| To open | Run |
|---|---|
| Files, at home | `setsid otto-files >/dev/null 2>&1 &` |
| Files, at a folder | `setsid otto-files /home/me/Pictures >/dev/null 2>&1 &` |
| The Trash | `setsid otto-files --trash >/dev/null 2>&1 &` |
| The emoji picker | `setsid otto-emoji >/dev/null 2>&1 &` |
| The emoji picker, searching | `setsid otto-emoji heart >/dev/null 2>&1 &` |
| Settings | `setsid otto-settings >/dev/null 2>&1 &` |
| The launcher (apps) | `setsid otto-launcher >/dev/null 2>&1 &` |
| A quick look at a file | `setsid otto-quickview /home/me/a.pdf >/dev/null 2>&1 &` |
| Anything else — a file, a folder, a link | `setsid xdg-open /home/me/notes.txt >/dev/null 2>&1 &` |

The emoji picker types the emoji into whatever window has the keyboard when the
person picks one. `otto-emoji --copy` puts it on the clipboard instead, and
`--type` always types it. Leave it on the default unless they ask.

Settings cannot be opened on a particular page. Say which one to click:
General, Displays, Dock, Keyboard, Trackpad & Mouse, Sound, Power, or Lock &
Login.

## Windows, workspaces and monitors

`otto-msg` drives the compositor. The command words and the JSON are i3's, so
an i3 or sway script ports with a rename.

### Look first

```sh
otto-msg -t get_workspaces -r     # every workspace
otto-msg -t get_outputs -r        # every monitor
otto-msg -t get_tree -r           # every window
```

`-r` gives one line, which is what you want in a pipe. `get_tree` is the whole
nested tree, not a list — to answer "what is open?" pipe it through `jq`.

**The open windows**, one per line, the focused one marked:

```sh
otto-msg -t get_tree -r | jq -r '.. | objects
    | select(.layout == "none" and .name != null)
    | "\(.app_id // .window_properties.class // "?")\t\(.name)\(if .focused then "  <- focused" else "" end)"'
```

Three things about the tree decide that expression, and getting any of them
wrong loses windows:

- A **window is a leaf**: `layout` is `"none"`. Everything else — the root,
  an output, a workspace, a split — is a container.
- A **floating window hangs off `floating_nodes`**, not `nodes`, which is why
  the recursive `..` is there rather than a walk down `nodes`.
- An **X11 window has no `app_id`**; it carries `window_properties.class`
  instead. Without that fallback it does not appear at all.

**What is focused**, on its own:

```sh
otto-msg -t get_tree -r | jq -r '.. | objects
    | select(.focused == true and .layout == "none") | .name'
```

There is no other way to list windows. `wlrctl`, `lswt`, `swaymsg` and the
rest are not Otto's and may not be installed; if you find yourself reaching
for one, the answer is the command above.

**To focus one particular window**, put a criteria in front of `focus`:

```sh
otto-msg '[app_id="google-chrome"] focus'
otto-msg '[title="Inbox"] focus'
otto-msg '[app_id="foot" title="build"] focus'
```

The match is a **case-insensitive substring**, so `chrome` finds
`google-chrome`. `class` and `instance` are accepted as the X11 spellings of
`app_id`. Both fields together must match. It switches workspace to reach the
window, and unlike the directional `focus` it does not need a tiling
workspace. Several matches take the first — narrow the criteria to reach the
others. Nothing matching is an error, not a silent no-op.

This is the only way to focus a window by name. `wlrctl`, `lswt` and
`swaymsg` are not Otto's and may not be installed.

### Then move things

```sh
otto-msg 'move container to workspace 3; workspace 3'
```

| Command | What it does |
|---|---|
| `[app_id="…"] focus` | focus that window wherever it is, switching workspace to reach it |
| `focus left\|right\|up\|down` | move focus to the neighbouring tile |
| `focus parent` / `focus child` | out to the surrounding container, and back in |
| `focus mode_toggle\|floating\|tiling` | between the floating windows and the tiled ones |
| `move left\|right\|up\|down` | move the focused tile through the layout |
| `move container to workspace <n>` | send the focused window to workspace `<n>` |
| `workspace <n\|next\|prev>` | switch workspace; `<n>` is made if it is not there |
| `rename workspace [<n>] to <name>` | name the focused workspace, or number `<n>`, making it if it is not there |
| `split h\|v\|toggle` | which way the *next* window splits the focused cell |
| `layout splith\|splitv\|toggle split` | turn the container the focused cell is in |
| `resize grow\|shrink width\|height <n> [px\|ppt]` | resize the focused tile |
| `floating toggle\|enable\|disable` | float the focused window, or put it back |
| `fullscreen` | fullscreen the focused window |
| `kill` | close the focused window |
| `tiling toggle\|enable\|disable` | tiling on this workspace |
| `expose [show\|hide\|toggle]` | the window overview; bare `expose` toggles |
| `gaps inner\|outer <n> [current\|all]` | the gaps |

Most of these need a **tiling workspace**: `otto-msg tiling enable` first. On a
floating workspace they say so rather than doing something surprising.

Workspaces are made and never destroyed — `otto-msg workspace 7` leaves seven
of them. Sorting a pile of windows into named workspaces is its own page:
[sort-desktop.md](sort-desktop.md). Gaps are per workspace unless the command ends in `all`.

`layout tabbed`, `layout stacking`, `resize set` and moving a window to another
monitor are understood but not built yet, and say so. Criteria
marks and `for_window` rules are not parsed at all. A criteria
(`[app_id="…"]`) is read, but only in front of `focus`; on anything else it
says so rather than acting on the focused window instead.

## Rules

1. **Do not close, kill or move the person's windows unless they asked.**
   `kill` closes whatever has the keyboard, and unsaved work goes with it.
2. **Do not type into the focused window.** `otto-emoji` without `--copy` types
   where the person is working; open it and let them pick.
3. **One message per thing that happened.** An activity you created is yours to
   dismiss when it is over.
4. **Start GUI apps detached.** `setsid … &`, or the command never returns.
5. **Never pass `notify-send -A`.** It waits for a click that never comes.
6. **Do not run `otto --probe`.** It takes over the session.
7. **Do not invent commands.** If it is not on this page, it does not exist.

## How to write

- Short sentences. British spelling: colour, behaviour, minimise.
- Say what happened, plainly: "Files is open at your Pictures folder."
- No exclamation marks. No emoji. Do not congratulate anyone.
- Do not compare Otto to other desktops.

## Documentation to share

| Topic | Link |
|---|---|
| Scripting Otto with `otto-msg` | https://nongio.github.io/otto/scripting/ |
| Keyboard shortcuts | https://nongio.github.io/otto/keyboard-shortcuts/ |
| All the user guides | https://nongio.github.io/otto/ |
