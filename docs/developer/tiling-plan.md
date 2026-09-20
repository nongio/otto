# Tiling implementation plan

Companion to [`specs/tiling.md`](../../specs/tiling.md), which describes the
behaviour, and to [`shell-dbus-api.md`](shell-dbus-api.md), which documents the
scripting wire. This document is the plan the implementation followed, kept
because most of it is built and the rest is still the intended shape.

> **Status: mostly built.** Phases 0–2b are in the tree: the pure core
> (`src/workspaces/tiling/`), keyboard tiling, pointer resize and drag, the
> decoration variants, the Tiling settings pane, and `org.otto.Shell1` with
> `otto-msg`. Each section below says what is built and what is not. The
> headline gaps are tabbed and stacked containers, monocle, XWayland tiling,
> window rules, an `[animations]` config section, and the sway-ipc socket.
> `docs/user/tiling.md` (*Not there yet*) is the user-facing version of the
> same list.

## Who this is for

i3 and sway users switch compositors for one of two reasons: the tree model
they already think in, or the workflow built on top of it. That workflow is
keyboard-only, config-as-text, scriptable through `i3-msg` / `swaymsg`, with
a bar that reflects workspaces. Otto's pitch is that same model with an
animated, decorated, per-workspace desktop around it: a tiled workspace one
swipe away from a floating one, a dock and a top bar that keep working,
exposé that already understands the tree.

That audience changes four things relative to the spec as drafted:

1. **The tree must be i3's tree.** N-ary `splith` / `splitv` containers,
   `focus parent` / `focus child`, and `tabbed` and `stacked` container
   layouts. Someone with a three-year-old i3 config expects `layout toggle
   split` to do something. *Built:* n-ary split containers
   (`Node::Container { axis, children }`, `src/workspaces/tiling/tree.rs:81`)
   and `focus parent` / `focus child` as commands
   (`TilingState::focused_container`, `src/workspaces/tiling/state.rs:56`).
   *Not built:* tabbed and stacked. `layout tabbed` and `layout stacking`
   parse and then report that they are unsupported.
2. **Named actions, and an i3 preset that is only a config file.** Every
   tiling operation is a builtin action with a name (`FocusLeft`,
   `MoveContainerLeft`, `SplitVertical`, …), bound in `[keyboard_shortcuts]`
   exactly like `TileWindowLeft` is. The spec's non-goal "reproducing any
   specific existing tiler's keybindings verbatim" softens to "not by
   default". *Built:* the named actions
   (`src/config/shortcuts.rs:142`) and an i3-style block, commented out, in
   `otto_config.example.toml` under `[keyboard_shortcuts]`. *Not built:* a
   separate `config/presets/i3.toml` file. There is no `preset =` key and no
   include mechanism; the config loader layers fixed paths only.
3. **An i3-syntax command language for scripting.** One parser resolving each
   command to the same operation the named action performs. Used by the D-Bus
   method, the CLI and the headless tests, so existing `i3-msg` scripts port
   with a rename. Shortcuts do not use it. *Built:*
   `src/workspaces/tiling/command.rs`, dispatched in
   `src/shell/commands.rs`.
4. **A scriptable surface.** `otto-msg` (CLI) over `org.otto.Shell1` with
   `RunCommand` and `GetTree`. *Built:* `src/shell_service.rs` and
   `components/otto-msg/`, documented in
   [`shell-dbus-api.md`](shell-dbus-api.md). Full sway-ipc socket
   compatibility remains a later, optional layer on top of the same calls.

## What it was built on

Tiling shares the half-snap machinery rather than duplicating it:

| Piece | Where | How it is used |
| --- | --- | --- |
| Half-snap zones (left / right / maximize) | `src/workspaces/tiling_overlay.rs`, `TileZone`, `zone_from_pointer` | The overlay view also shows the drop slot for a drag into the tree; the zones stay for floating workspaces |
| Snap apply / restore with animation | `src/shell/xdg.rs` `apply_tile`, `untile`, `animated_client_size` | The per-window "animate to rect, then set states" body is shared with the relayout path |
| Per-window snap state | `WindowView { tiled_zone, unmaximised_rect }` in `src/workspaces/window_view/view.rs` | `unmaximised_rect` is the spec's "floating rectangle it had before it was tiled" |
| Usable area | `usable_zone(output)` in `src/shell/mod.rs` | The root container's rectangle, minus the outer gap, via `Otto::tiling_area` |
| Placement with transition | `Workspaces::map_window_on_output(.., Some(transition))` | Every cell move goes through this |
| Drag grab with zone detection | `PointerMoveSurfaceGrab` in `src/shell/grabs.rs` | Has a tiling branch: detach plus slot overlay instead of edge zones |
| Popup re-anchoring after a move | `reposition_popups_for_window` | Called per relaid-out window |
| Shortcut actions | `src/config/shortcuts.rs`, `src/input/actions.rs` | The tiling builtins sit alongside `TileWindowLeft` |
| Headless harness | `src/headless.rs`, `tests/tiling.rs` | Extended with `tiling_*` helpers; see *Phases* |
| D-Bus | `org.otto.Settings` etc. via `zbus` | The same pattern serves `org.otto.Shell1` |

## Architecture

The pure core lives in `src/workspaces/tiling/`, kept out of the very large
`workspaces/mod.rs`:

```
tiling/
  tree.rs      pure data + operations, no compositor types
  layout.rs    pure fn (tree, rect, gaps, min sizes) -> Vec<(leaf, rect)>
  command.rs   i3 grammar -> Command enum
  state.rs     per-workspace TilingState, held on WorkspaceView
  splits.rs    pure split arithmetic for edge-drag resize
  drag.rs      pure drop-target resolution for a detach drag
  floating.rs  the floating layer within a tiling workspace
  mod.rs
```

The compositor half (applying a layout, directional focus, the drag grabs)
lives in `src/shell/tiling.rs` and `src/shell/tiling_drag.rs`, because it
needs `Otto`. The plan's `apply.rs` and `focus.rs` became those two files.

**Division of labour.** `layout.rs` is the truth: it answers "what is this
window's rectangle" synchronously, for the client configure, hit-testing,
scanout eligibility and tests. `src/shell/tiling.rs` turns rectangles into
motion and configures, and owns the one hard fact about animating a tiled
layout: a resized rectangle only has real content once the client has
committed a buffer of that size. The engine can move a layer for free; it
cannot resize its content.

**`tree.rs`.** `Node<L> = Leaf(L) | Container { axis: Axis, children: Vec<Child> }`,
where a `Child` is a node id plus a share. Leaves are `ObjectId`s in the
compositor and `&str`s in the tests. Operations, each returning what changed:
`insert_next_to`, `insert_beside`, `insert_at_root_edge`, `remove`,
`move_dir`, `swap_leaves`, `resize`, `set_pair_shares`, `equalize`,
`equalize_container_of`, `equalize_all`, `set_container_axis`. Single-child
containers are dissolved as an invariant after every mutation. Fractions
only, never pixels. Unit-tested with `cargo test --lib`.

*Not built:* a `layout` field on `Container` (tabbed, stacked), and explicit
`promote` / `demote`.

**`layout.rs`.** Resolves the tree against the usable rectangle in logical
pixels, applying outer and inner gaps and the lone-tile no-gap rule. Integer
division remainders go to the last child of each container, so a row of tiles
covers its rectangle exactly.

*Not built:* per-leaf minimum sizes with the spec's overflow rule during
layout resolution, the tabbed/stacked title-strip height, and snapping cell
extents through `workspaces::utils::snap_extent_px`. Minimums are honoured by
the edge drag (`drag.rs` `min_extent`), not by `resolve`. Layout rounds in
logical pixels, and the pixel-grid snap happens per window in
`window_view/render.rs`.

**`command.rs`.** A hand-written parser, no dependency. Errors carry the
offset so `otto-msg` can print them the way `swaymsg` does. It covers more
than the phase-1 subset first drafted: window criteria (`[app_id="…"]`),
`rename workspace`, and an Otto-only `expose`.

**`state.rs`.** `TilingState` on `WorkspaceView`:

```
TilingState {
  enabled: bool,                    // is this workspace tiling
  tree: Tree<ObjectId>,
  focused: Option<ObjectId>,        // the tiled layer's focus
  focused_container: Option<NodeId>,// where `focus parent` walked up to
  dirty: bool,                      // a relayout is owed on the next loop turn
  returning: Vec<ObjectId>,         // unminimized windows waiting to rejoin
  floating_focused: Option<ObjectId>,
  preselect: Option<Axis>,
  gaps: Option<Gaps>,               // per-workspace override of [tiling] gaps
}
```

The floating layer itself is not a set on this struct: `floating.rs` derives
it from the workspace's windows and the tree. There is no `monocle` field and
no `held_slots` map. See *Not built* under *Hooks*.

**Motion paced by the client.** Moves and swaps, where sizes do not change,
animate the window layers with a lay-rs transition and look right throughout:
the buffer stays valid. Resizes are configured per animation frame, as
half-snap does, and the window is drawn with whatever buffer the client has
committed most recently. A client that keeps up (GTK, Qt, otto-kit at frame
rate) shows real content on every frame; a slow one lags behind its rectangle
for a few frames, which is the honest state of affairs rather than a
stretched or clipped stand-in. There is no Taffy mirroring of the tree: it
would move the same problem into the engine and reparent window layers for
nothing.

Leaves whose rectangle is unchanged get no configure at all.

*Not built:* sway-style transactions for the final frame: tracking the last
configure per window and presenting the layout as settled only once every
affected client has committed the final size or a deadline passes.

**Applying a layout.** One entry point on `Otto`: `relayout_workspace(output,
animate)` (`src/shell/tiling.rs:184`), with `relayout_workspace_forced` and
`relayout_workspace_with` beneath it. It resolves the tree with `layout.rs`,
animates and configures each leaf whose rect changed, and repositions popups.
Every mutation of the tree ends by calling it, or by setting
`TilingState::dirty` so `flush_tiling_relayout` picks it up on the next
event-loop turn.

**Hooks into existing paths.**

| Event | Floating workspace | Tiling workspace |
| --- | --- | --- |
| `new_toplevel` / first map (`shell/mod.rs` cascade placement) | cascade | if the window is eligible: `insert_next_to(focused)`, relayout |
| unmap, minimize, move to workspace | nothing | `remove`, relayout; the destination workspace inserts |
| focus change | order list | also `tiling_note_focus` |
| `resize_request` / `move_request` on a tiled window | interactive grab | resize refused; move starts the detach drag |
| `recalculate_exclusive_zones`, dock size/edge change, output mode/scale | nothing for windows | `relayout_tiling_workspaces` |
| decoration mode (`xdg_decoration_handler.rs`) | SSD/CSD | SSD gets the configured tile variant; xdg states carry the tiled edges |

*Not built:* `maximize_request` as monocle (maximizing a tile does not hide
its siblings), fullscreen slot holding, and XWayland parity. `is_tileable`
returns false for an X11 window, so it floats (`src/shell/tiling.rs:139`).

**Floating, the i3 way.** A tiling workspace has a floating layer for the
exceptions. A window floats automatically when it has a parent, when it
carries a *modal* `xdg-dialog-v1` hint, when it is not resizable, when it is
fullscreen or minimized, or when it lacks the `Maximize` window-management
capability (`src/shell/tiling.rs:139`). `floating toggle` moves the focused
window between the tree and the floating layer; `focus mode_toggle` moves
focus between the two layers. Floating windows always draw above the tiles
(`restack_floating_above_tiles`), keep their full titlebar and shadow, and
are moved and resized as on a floating workspace.

*Not built:* a `[tiling] float = ["app_id", …]` list, i3's
`for_window … floating enable` equivalent, and a compositor-side built-in
float list of Otto's own dialogs.

**Otto's own dialogs.** The portal file picker is the test case. otto-files
imports the `parent_window` handle through xdg-foreign and parents the picker
to the requesting window; where the compositor or the application does not
offer a foreign handle, it asks for the modal dialog hint instead, which is
what `is_tileable` floats on (`components/otto-files/src/app/lifecycle.rs:868`).

## Animation configuration

**Not built.** Otto animates everywhere: the workspace switch, maximize,
half-snap, minimize, exposé, the dock, tiling. Each grew its own constant or
config key. The intent is that animation timing becomes a general
setting outside `[tiling]`:

```toml
[animations]
scale = 1.0             # multiplies every duration; 0.5 = twice as fast, 0 = snap everywhere
layout = { duration = 0.3, bounce = 0.0 }    # windows moving into place: maximize, snap, tiling relayouts, tiling mode in/out
interactive = { duration = 0.35, bounce = 0.25 }  # anything chasing the pointer: edge and tile drags
switch = { duration = 0.6, bounce = 0.1 }    # workspace scrolling
```

What exists instead is four keys under `[tiling]`: `layout_duration`,
`layout_bounce`, `mode_duration` and `mode_bounce`, each with a `spec(...)`
entry in `src/settings/schema.rs:473`. A duration of 0 passes no transition
to lay-rs: the layer lands in one frame and the client gets one configure.
That is the only "off" switch; there is no separate enable flag.

The rest of the section stands as the plan:

- A family is a duration in seconds plus a bounce; `scale` multiplies the
  duration. `[tiling]` would then carry no duration or bounce keys at all,
  and the existing four would be read as legacy overrides.
- `[accessibility] reduce_motion = true` forces `scale` to 0 and goes out
  over the portal's reduced-motion key so clients see it too. No such key
  exists today.
- The hard-coded `ease_out(0.3)` in maximize, half-snap and restore moves
  onto `layout`. Dock magnification and exposé keep their own tuned springs
  and only honour `scale`.
- Every named action accepts `animate = false` in the shortcut config, and
  `otto-msg` takes `--no-animation`, so a keybinding can be instant while the
  same operation from the pointer or a script animates. Neither is
  implemented.
- Otto Settings shows an *Animations* group in General. The Tiling pane keeps
  decoration, gaps, smart gaps and resize step, as it does today.

## Named actions

**Built** (`src/config/shortcuts.rs:142`), one builtin per operation:

```
TilingToggle                       # workspace mode
FocusLeft/Right/Up/Down, FocusModeToggle
MoveContainerLeft/Right/Up/Down
SplitHorizontal, SplitVertical
ResizeGrowWidth/ShrinkWidth/GrowHeight/ShrinkHeight
EqualizeContainer
FloatingToggle
```

i3's `kill` and `workspace <n>` are served by the general `CloseWindow` and
`Workspace { index }` builtins.

**Not built:** `FocusParent`, `FocusChild` (available as commands only),
`MoveToWorkspace { index }`, `SplitToggle`, the `Layout*` family, a
fullscreen builtin, and a `{ step }` parameter on the resize actions. The
step is the global `[tiling] resize_step`.

The i3-style bindings ship commented out in `otto_config.example.toml` under
`[keyboard_shortcuts]`, with `Logo` as the modifier:

```toml
"Logo+t" = "TilingToggle"
"Logo+h" = "FocusLeft"
"Logo+j" = "FocusDown"
"Logo+Shift+h" = "MoveContainerLeft"
"Logo+b" = "SplitHorizontal"
"Logo+v" = "SplitVertical"
"Logo+e" = "EqualizeContainer"
# …
```

`docs/user/tiling.md` (*Setting up the keys*) documents the block as "copy
this into your config".

## Resizing with the pointer

**Built.** Gaps are not drag handles. A tile is resized by dragging the
window's own resize edge, exactly as a floating window is: the drag moves
every split that edge lies on, changing the shares of the two children either
side of each and nothing else, stopping at each window's minimum size
(`min_extent`). The boundary snaps to fractions of the pair it divides, and
`Shift` bypasses the snap (`src/shell/tiling_drag.rs:408`). An edge that is
the outside of the tree does not resize and keeps the ordinary arrow cursor.
A window's titlebar drag
detaches it and the slot overlay shows where it would land. This keeps
pointer handling on a tiling workspace identical to a floating one, and
avoids a hidden hit area competing with window edges.

The split arithmetic — where a boundary is, and what a pointer offset does to
the two shares either side of it — is pure and lives in
`src/workspaces/tiling/splits.rs`; the compositor half, which turns a resize
edge into a set of splits and writes the shares into the tree, is in
`src/shell/tiling_drag.rs`. The titlebar drag reuses `TilingOverlayView` to
show the slot before release, and `src/workspaces/tiling/drag.rs` resolves
the drop target and the minimum extent a neighbour refuses to go below.
`tests/tiling_drag.rs` covers both.

## Decorations on tiles

**Built.** Tiles want less chrome than floating windows, and i3 users
disagree on how much less: i3 draws a one-line title bar by default, sway
users very often set `default_border pixel 2` and keep only a coloured
border. So it is a setting, `[tiling] decoration = "normal" | "minimal" |
"none"`, default `minimal`
(`components/otto-kit/src/tile_decoration.rs:20`):

- **normal**: the bar the window wears while it floats: full height, the
  title at its usual size, all three controls. Squared corners and no shadow
  all the same, since a tile abuts its neighbours whatever it wears on top.
- **minimal**: a bar one text line high with the title and a close control,
  no shadow, and corners rounded at half the floating radius
  (`WindowDecoration::MINIMAL_CORNER_RADIUS`, 6pt). 12pt on a 20pt bar
  swallows most of the strip. It keeps the move handle and the window menu,
  which is the reason for keeping a bar at all.
  `WindowDecoration::corner_radius_for` is the one answer for every frame:
  the compositor's bar, otto-kit's window frames and Settings' painted body
  all read it.
- **none**: no bar; the focused tile gets a hairline border in the accent
  colour, the rest a neutral hairline. Moving a tile is then the keyboard's
  job. This is sway's `pixel` border.

Client-side-decorated windows are told they are tiled on every touching edge
and square off on their own; they get no bar under any of the three.
Floating windows in a tiling workspace keep the full floating decoration.

The variant is swapped when the window enters or leaves the tree
(`refresh_tiling_decorations`, `src/shell/tiling.rs:684`), the same path the
maximized (gapless, squared) variant uses.

*Not built:* the title strip tabbed and stacked containers would draw, since
those containers do not exist.

**Otto's own apps.** otto-files, otto-settings, the launcher and Peek draw
their own titlebar through otto-kit's titlebar component, so the
compositor-side variants never reach them. They follow the same setting from
the client side:

- the tiled xdg edge states in the configure tell the app it is tiled; the
  otto-kit titlebar switches to its compact form and rounds (or squares) its
  corners for the variant while any edge is tiled, as GTK does; the
  window's frame is re-rounded to match on the same configure;
- the `minimal` / `none` choice goes out over `org.otto.Settings` like the
  theme and the controls side do, so the app applies it live;
- with `none` the app draws no bar and is moved by the keyboard, like any
  client-decorated window.

One component change in otto-kit covers every app that already implements the
settings callback; islands, Peek and lock still need that hook.

## Settings app and per-workspace settings

**Built.** Both halves persist through the existing settings machinery
(validate → apply → persist → announce), so they are live and reach the
otto-kit apps.

**Global tiling settings in Otto Settings.** A *Tiling* pane,
`components/otto-settings/src/panes/tiling.rs`, discovered from the schema
like the others, with every `[tiling]` key: decoration, inner and outer gap,
smart gaps, resize step, and the layout and mode springs. Each key has a
`spec(...)` entry in `src/settings/schema.rs:424` marked `Live` and an apply
arm in `src/settings/apply.rs` that relays out every tiling workspace (gaps,
decoration) or just stores the value (durations, step).

Two things make the difference between a key marked `Live` and one that is.
The gap sliders are the session default, so they clear the per-workspace
overrides `gaps <n>` leaves behind; otherwise the slider is dead on the one
workspace the user is watching. A value is also checked against what the
configuration *kept*, not against what was asked for, since the fractions
here are stored as `f32` and 0.05 does not come back as 0.05.
`tests/tiling_settings.rs` drives all of it through the real settings entry
point.

**Per-workspace settings, persisted with the name.** One record per
workspace, keyed `"<output>:<position>"`:

```toml
[workspaces.entries."eDP-1:0"]
name = "Code"
tiling = true          # the mode is restored on login; the tree is not
inner_gap = 0          # optional override; absent = [tiling] default
outer_gap = 0
```

Reading accepts the older `names` and `[workspaces.gaps]` maps and folds them
in; writing emits only `entries` (`src/config/mod.rs:1075`). One writer,
`save_workspace_entry` (`src/config/mod.rs:1282`), covers all three fields,
and `restore_workspace_settings` at the workspace-creation sites restores
name, mode and gaps together. Toggling tiling persists the mode; `gaps …
current` persists the override; `gaps … all` clears every override
(`clear_workspace_gap_overrides`). A workspace that moves position keeps
best-effort semantics, as names do.

**Editing per-workspace settings.** Only where they are already edited: the
name in the workspace selector, the mode with `TilingToggle`, the gaps with
the `gaps` command. Each persists its own field of the record. A Workspaces
pane in Otto Settings, and a `SetWorkspace` call on `org.otto.Shell1` to back
it, are deferred; the record is shaped so they can be added without a
migration. A tiling item in the workspace context menu is also still missing.

## Command language

**Built** in `src/workspaces/tiling/command.rs`, dispatched in
`src/shell/commands.rs`, documented for users in
[`shell-dbus-api.md`](shell-dbus-api.md) and `docs/user/scripting.md`:

```
focus left|right|up|down|parent|child|mode_toggle
focus [app_id="…"]                 # window criteria
move left|right|up|down
move container to workspace <n>
workspace <n|name|next|prev>
rename workspace <n> to <name>
split h|v|toggle
layout splith|splitv|tabbed|stacking|toggle
resize grow|shrink width|height <n> px|ppt
floating toggle|enable|disable
fullscreen
kill
tiling toggle                      # Otto: workspace mode
expose toggle                      # Otto: the overview
gaps inner|outer <n> [current|all]  # current = this workspace's override, all = the default
```

`layout tabbed` and `layout stacking` parse and then report that they are not
supported.

Deferred: `mark` / `[con_mark]` criteria, `mode "resize"` binding modes,
`scratchpad`, `assign`, `for_window`, `exec` (shortcuts already do `run`).

Workspaces by number create on demand: i3 makes workspace 7 when you switch
to it, and `ensure_workspace` (`src/shell/commands.rs:495`) appends
workspaces until `n` exists. i3 also drops a workspace when it empties; Otto
does **not** follow that. Otto's workspaces are persistent: named,
reorderable, per output. A renamed workspace vanishing because its last
window closed would break that model. An empty workspace stays.

## IPC

**Built.** `org.otto.Shell1` on the session bus, served from
`src/shell_service.rs` alongside the other `org.otto.*` interfaces, with
`RunCommand`, `GetTree`, `GetWorkspaces`, `GetOutputs` and the workspace and
window signals a status bar watches. `components/otto-msg` is the CLI:
`otto-msg focus left` and `otto-msg -t get_tree | jq` both work, and
`-t subscribe -m` streams events. The wire contract is
[`shell-dbus-api.md`](shell-dbus-api.md).

**Not built.** A sway-ipc-compatible Unix socket (`$SWAYSOCK`) over the same
calls; it would let waybar's `sway/workspaces`, `i3-msg`-based scripts and
`autotiling` run as-is. Whether Otto exposes `ext-workspace-v1` and
`wlr-output-management` (which waybar's `wlr/workspaces` and kanshi need)
is a separate question worth checking, and matters to this audience
independently of tiling.

## Phases

**Phase 0: pure core.** *Done.* `tree.rs`, `layout.rs`, `command.rs` with
unit tests run by `cargo test --lib`. Minimum sizes and their overflow rule
are not implemented.

**Phase 1: keyboard tiling.** *Done.* `TilingState` on `WorkspaceView`; the
mode toggle; insert and remove on map, unmap, minimize and workspace move;
`relayout_workspace`; xdg tiled states; the auto-float rules and the floating
layer; the `[tiling]` springs with `0` = snap; the named actions and the
example-config block; focus directions, moves, splits, resize step, float
toggle; `workspace <n>` create-on-demand. `tests/tiling_tree.rs` and
`tests/tiling_scripting.rs` drive it through the headless harness the way
`tests/tiling.rs` does for half-snap.

*Left over from Phase 1:* the workspace context-menu toggle, tabbed and
stacked layouts, monocle, fullscreen slot holding, and per-action
`animate = false` with `[accessibility] reduce_motion`.

**Phase 2: pointer and chrome.** *Done.* Edge-drag resize with the
`ew-resize` / `ns-resize` cursor shapes and live reconfigure on ack;
drag-to-detach with `TilingOverlayView` showing the slot; dropping a floating
window into a tree; the minimal and none decoration variants; the accent
focus border; no shadow on tiles; usable-area re-fit on dock, layer-shell and
mode changes.

*Left over from Phase 2:* XWayland parity (X11 windows float), the
workspace-selector mode indicator, and dropping a tile outside the layout
leaving it floating where it was released.

**Phase 2b: settings.** *Done.* The Tiling pane, and the per-workspace
record with name, mode and gaps persisted together. The Workspaces pane is
still deferred. See *Settings app and per-workspace settings*.

**Phase 3: scriptability and depth.** *Partly done.* `org.otto.Shell1`,
`otto-msg` and `GetTree` are built, along with window criteria on `focus`.
Still to do: marks, binding modes (`mode "resize"`), scratchpad, per-app
`assign` / `for_window` rules, and persistence of the tree across restart
(the mode already persists; the tree is best-effort by app_id at most).

**Phase 4: compatibility.** Sway-ipc socket shim if there is demand.
The spec's scaled-last-frame animation is dropped: there is no honest way
to fill a resized rectangle before the client has drawn it. Per-frame
configure plus end-of-animation transactions is the model.

## Open: exposé on a tiled workspace

Partly settled. `expose toggle` is a command, and exposé runs on a tiling
workspace, but the questions below are still open:

- whether exposé should spread tiles at all, or only pull the floating layer
  and any hidden windows out where they can be seen;
- what happens to the containers' layers while exposé reparents or mirrors
  windows (`pre_expose_order` and the mirror path assume `windows_layer`
  children);
- whether selecting a window in exposé should also move it in the tree, or
  only focus it;
- how the workspace-selector previews render a tree (they replicate
  `wallpaper_group` plus windows).

## Risks

- **Configure storms.** Per-frame configures across N clients on every layout
  change is exactly what the spec warns about. Only leaves whose rect changed
  are configured, and configures are throttled to acked ones (the
  interactive-resize rule), not to a fixed cadence.
- **Scanout.** A lone tile must still promote to a plane
  (`specs/plane-scanout.md`); the tiled xdg states must not change the
  promotion gate. Verify on tty with `/tmp/otto-dump-planes`.
- **lay-rs cancelled changes.** Re-targeting a running transition has bitten
  exposé; interruptible relayout leans on that fix and wants a headless
  regression that issues two moves within one animation.
- **Fractional scale.** Layout rounds in logical pixels and the pixel-grid
  snap happens per window in `window_view/render.rs`. A test at scale 1.5
  asserting integer physical origins is still missing.
- **`workspaces/mod.rs` size.** All the pure logic lives in
  `workspaces/tiling/` and the compositor half in `shell/tiling*.rs`;
  `mod.rs` only gains the calls into them.
