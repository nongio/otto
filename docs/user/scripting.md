# Scripting Otto

`otto-msg` drives the desktop from a script or a terminal: it runs window
commands, prints the window tree as JSON, and follows what the compositor is
doing. If you have written anything for i3 or sway, this is `i3-msg` and
`swaymsg` under another name — the command words and the JSON shapes are the
same ones.

```sh
otto-msg focus right
otto-msg -t get_tree
```

Behind it is a D-Bus interface, `org.otto.Shell1`, so anything that can make a
D-Bus call can do the same without the CLI — see
[shell-dbus-api.md](../developer/shell-dbus-api.md) for the wire.

## Running commands

Anything not consumed by an option is the command. Several commands go in one
string, separated by `;`.

| Command | What it does |
|---------|--------------|
| `focus left\|right\|up\|down` | move focus to the neighbouring tile; never wraps |
| `focus parent` / `focus child` | move focus up to the surrounding container, and back down |
| `move left\|right\|up\|down` | move the focused tile through the tree |
| `move container to workspace <n>` | send the focused window to workspace `<n>`, creating it if needed |
| `workspace <n\|next\|prev>` | switch workspace; `<n>` is created if it does not exist |
| `split h\|v\|toggle` | decide which way the *next* window splits the focused cell |
| `layout splith\|splitv\|toggle split` | turn the container the focused cell sits in |
| `resize grow\|shrink width\|height <n> [px\|ppt]` | resize the focused tile; a bare number means percent |
| `fullscreen [toggle]` | fullscreen the focused window |
| `kill` | close the focused window |
| `tiling toggle\|enable\|disable` | turn the current workspace's tiling on or off |
| `gaps inner\|outer <n> [current\|all]` | set the gaps |

Most of these need a **tiling workspace** — `tiling enable` first, or bind
`TilingToggle` to a key. On a floating workspace they say so rather than doing
something surprising.

**Workspaces are created, never destroyed.** `otto-msg workspace 7` gives you
seven workspaces. Unlike i3, Otto does not delete one when its last window
closes: Otto's workspaces are named, reorderable and per monitor, and one
vanishing under you would lose that.

**Gaps are per workspace unless you say otherwise.** `otto-msg gaps inner 0`
closes the gaps on the workspace you are looking at and remembers that for next
session; `otto-msg gaps inner 8 all` sets the default for the session and
forgets every per-workspace tweak.

Some i3 commands are understood but not built yet, and say so instead of
quietly doing nothing: `floating toggle`, `layout tabbed`, `layout stacking`,
`resize set`, `focus mode_toggle`, and moving a window to another output.
Criteria (`[app_id="…"]`), marks, binding modes and `for_window` rules are not
parsed at all.

## Reading what is on screen

```sh
otto-msg -t get_tree          # every window, in i3's node shape
otto-msg -t get_workspaces    # every workspace
otto-msg -t get_outputs       # every monitor
```

Output is pretty-printed; `-r` gives one line, which is what you want in a
pipe. `-q` prints nothing at all and leaves only the exit status, which is `1`
if any command failed.

## Following along

```sh
otto-msg -m -t subscribe '["workspace","window"]'
```

prints one JSON event per line as the focused window or workspace changes —
what a status bar reads. Without `-m` it prints the first event and exits, so a
script can wait for one thing to happen. `workspace` and `window` are the only
events so far.

## Three things to try

**What is focused, right now.**

```sh
otto-msg -t get_tree | jq -r '.. | objects | select(.focused == true and .layout == "none") | .name'
```

**Send the browser to workspace 3 and follow it.**

```sh
otto-msg 'move container to workspace 3; workspace 3'
```

**Port an i3 script.** A typical i3 keybinding script is `i3-msg` plus a
command string; the rename is the whole job:

```sh
# i3
i3-msg 'workspace 2; append_layout ~/.config/i3/work.json'

# Otto — the command half ports as it stands; `append_layout` does not
# exist yet, so the layout is built with splits instead.
otto-msg 'workspace 2; split v'
```

Where a script reads the tree, `.nodes`, `.floating_nodes`, `.focused`,
`.app_id` and `.rect` mean what they mean in i3, so `jq` filters carry over
unchanged. Two differences to watch for: a workspace that is not tiling lists
everything under `floating_nodes`, and Otto adds a `gaps` key to each workspace
node holding that workspace's override (or `null`).

## Binding it to a key

`otto-msg` is not the way to bind a key — the compositor has named actions for
every one of these commands, and going out through D-Bus and back for a
keystroke is slower and can fail. Bind the action instead, in
`[keyboard_shortcuts]`; see
[keyboard-shortcuts.md](keyboard-shortcuts.md). Keep `otto-msg` for scripts,
for a status bar, and for the terminal.
