# `org.otto.Shell1` — the scripting D-Bus contract

The D-Bus interface the compositor serves for driving the shell from a script:
an i3-syntax command language, the window tree as JSON, and the two events a
status bar watches.

> **Status**: implemented on both sides — the compositor serves it
> (`src/shell_service.rs`, `src/shell/commands.rs`,
> `src/workspaces/tiling/command.rs`) and `otto-msg`
> (`components/otto-msg/`) consumes it. Behaviour lives in
> [specs/tiling.md](../../specs/tiling.md) and
> [tiling-plan.md](./tiling-plan.md); this is the wire.
>
> This is Otto's answer to `i3-msg` and `swaymsg`. The command grammar and the
> JSON shapes are i3's, so a script written for either ports with a rename.
> A sway-ipc-compatible Unix socket over the same calls is a later, optional
> layer.

Bus name `org.otto.Shell1`, object path `/org/otto/Shell1`, interface
`org.otto.Shell1`. The compositor owns the name, alongside `org.otto.Settings`
and `org.otto.ScreenCast` on the same connection.

## Methods

| Method | Signature | Answer |
| --- | --- | --- |
| `RunCommand` | `(s) → a(bs)` | one `(success, message)` per `;`-separated command |
| `GetTree` | `() → s` | the whole tree, i3's `GET_TREE` node shape |
| `GetWorkspaces` | `() → s` | every workspace, i3's `GET_WORKSPACES` shape |
| `GetOutputs` | `() → s` | every output, i3's `GET_OUTPUTS` shape |

Every method is handed to the compositor thread over the same calloop channel
the settings interface uses, and answered on a `oneshot`, so a call takes
exactly the path a keystroke does. The getters answer with a JSON *string*
rather than a variant tree: that is what i3's IPC does, what `jq` expects, and
what keeps the shapes from being re-encoded at every hop.

### `RunCommand(s) → a(bs)`

Runs a `;`-separated command string. One result per command, in the order they
were given. A command that worked carries an empty message; one that did not
says why, in the words `swaymsg` would print.

A string that does not **parse** abandons the whole string — i3 and sway do the
same rather than run half of it — and comes back as a single failure naming the
character it stumbled on:

```
Unknown/invalid command 'frobnicate' (at character 12)
Invalid focus command (expected left|right|up|down|parent|child) (at character 6)
Otto does not support 'layout tabbed' yet (at character 7)
```

The third shape is the one to watch for: it means the command was understood
and Otto has not built it. Commands that parse and are refused at run time
(nothing focused, the workspace does not tile) come back per-command with
`success = false` instead.

### The command language

| Command | Notes |
| --- | --- |
| `focus left\|right\|up\|down` | never wraps at the edge of a workspace |
| `focus parent\|child` | walks container focus up and back down |
| `move left\|right\|up\|down` | moves the focused tile through the tree |
| `move container to workspace <n>` | also `move to workspace <n>`, `move window to workspace number <n>` |
| `workspace <n\|next\|prev>` | `<n>` is created on demand |
| `split h\|v\|toggle` | arms the axis the next window splits along |
| `layout splith\|splitv\|toggle split` | turns the focused cell's container |
| `resize grow\|shrink width\|height <n> [px\|ppt] [or <n> ppt]` | a bare figure is `ppt`; the `or` fallback is accepted and dropped |
| `floating toggle\|enable\|disable` | move the focused window between the tree and the floating layer |
| `focus mode_toggle\|floating\|tiling` | move focus between the two layers |
| `fullscreen [toggle]` | the same path a client's own request takes |
| `kill` | closes the focused window |
| `tiling toggle\|enable\|disable` | Otto's own: the workspace's mode |
| `gaps inner\|outer <n> [current\|all]` | see below |

**Not implemented yet**, and refused by name rather than ignored:
`layout tabbed`, `layout stacking`, `layout toggle all`, `resize set`,
`split none`,
`fullscreen global`, `move … to output`, per-edge `gaps`, workspaces by name,
`workspace back_and_forth`. Criteria (`[app_id="…"] …`), marks, binding modes,
`scratchpad`, `assign` and `for_window` are not parsed at all.

**Workspaces are created but never removed.** `workspace 7` appends workspaces
until seven exist. Unlike i3, Otto does not drop one when it empties: Otto's
workspaces are named, reorderable and per output, and a renamed workspace
vanishing because its last window closed would break that model.

**Gaps are per session with a per-workspace override.** `gaps inner 4` — or the
explicit `gaps inner 4 current` — overrides the focused workspace alone and
saves that override in `[workspaces.gaps]`, keyed `"<output>:<position>"`
exactly as the workspace names are. `gaps inner 4 all` sets the `[tiling]`
session default and drops every override, so one command undoes a session's
worth of tweaking. `smart_gaps` stays global either way: it is a preference
about how a *lone* tile looks, not a measurement of one workspace. sway's word
order (`gaps inner all set 4`) reads the same.

### `GetTree() → s`

i3's node shape: `root` → one node per output → one per workspace → containers
and windows.

| Key | On | Notes |
| --- | --- | --- |
| `id` | every node | opaque integer, stable within a session, inside 2^53 so `jq` round-trips it |
| `type` | every node | `root`, `output`, `workspace`, `con` |
| `name` | every node | window title, workspace name, output name; `null` for a container |
| `layout` | every node | `splith`, `splitv`, `none` for a window, `output` for an output |
| `orientation` | every node | `horizontal`, `vertical`, `none` |
| `percent` | children | the child's share of its container, `null` at the top |
| `rect` | every node | `{x, y, width, height}` in **logical** pixels |
| `focused` | every node | exactly one window is `true` |
| `focus` | every node | child ids, most recently focused first |
| `nodes` | every node | tiled children |
| `floating_nodes` | workspaces | windows the tree does not hold |
| `urgent` | every node | always `false`; Otto has no urgency hint yet |
| `app_id` | windows | the xdg app id |
| `window_properties` | X11 windows | `{class, instance, title}` |
| `gaps` | workspaces | Otto's own: `{inner, outer}` when the workspace has an override, else `null` |

A container's `rect` is the union of the cells under it — the tree stores
fractions, not rectangles, so a container's extent is what its leaves span.

**A floating workspace lists every window under `floating_nodes`** and has no
`nodes`: nothing is in a tree there. That is also what a tiling workspace does
with the windows the tree refused (dialogs, fixed-size windows), so a script
that reads `floating_nodes` gets the same answer in both modes.

### `GetWorkspaces() → s`

An array of `{num, id, name, visible, focused, urgent, output, rect}`.
`visible` is "this is its output's current workspace"; `focused` narrows that
to the focused output.

### `GetOutputs() → s`

An array of `{name, active, primary, focused, current_workspace, rect}`.
`current_workspace` is a name, as i3 reports it.

## Signals

| Signal | Argument | When |
| --- | --- | --- |
| `WorkspaceChanged` | `s` — i3's `workspace` event as JSON | the current workspace changed |
| `WindowChanged` | `s` — i3's `window` event as JSON | keyboard focus moved to another window |

Both payloads carry a `change` key (`"focus"`) and the subject: `current` for a
workspace, in the `GetWorkspaces` entry shape, and `container` for a window, in
the `GetTree` node shape.

Only the focused window's own node is built for `WindowChanged`, not the whole
tree: focus changes with every click and every step through the app switcher,
and walking every workspace on every output to answer one of them would be a
cost the desktop pays whether or not anything is listening.

This cut emits both signals from the focus paths — `set_keyboard_focus_on_window`
and `set_current_workspace_index`. A workspace scrolled to by a trackpad swipe
does not emit one yet.

## Errors

`org.otto.Shell1.Error.Unavailable` — the compositor is not listening, or did
not answer. Everything else is reported per command inside `RunCommand`'s
reply, not as a D-Bus error: a command string is a batch, and one bad command
must not lose the results of the ones around it.

## Trying it by hand

```sh
busctl --user call org.otto.Shell1 /org/otto/Shell1 org.otto.Shell1 \
    RunCommand s 'focus right; split v'
busctl --user --json=short call org.otto.Shell1 /org/otto/Shell1 \
    org.otto.Shell1 GetTree
```

`otto-msg` is the same calls with the argument handling and output formatting
`swaymsg` has — see [docs/user/scripting.md](../user/scripting.md).
