# Tiling implementation plan

Companion to [`specs/tiling.md`](../../specs/tiling.md), which describes the
behaviour. This document describes how to get there from the code as it stands
today, and what changes when the target audience is people coming from i3 and
sway.

## Who this is for

i3 and sway users switch compositors for one of two reasons: the tree model
they already think in, or the workflow built on top of it — keyboard-only,
config-as-text, scriptable through `i3-msg` / `swaymsg`, a bar that reflects
workspaces. Otto's pitch is that same model with an animated, decorated,
per-workspace desktop around it: a tiled workspace one swipe away from a
floating one, a dock and a top bar that keep working, exposé that already
understands the tree.

That audience changes four things relative to the spec as drafted:

1. **The tree must be i3's tree.** N-ary `splith` / `splitv` containers,
   `focus parent` / `focus child`, and — sooner than the spec's "open
   question" — `tabbed` and `stacked` container layouts. Someone with a
   three-year-old i3 config expects `layout toggle split` to do something.
2. **Named actions, and an i3 preset that is only a config file.** Every
   tiling operation is a builtin action with a name (`FocusLeft`,
   `MoveContainerLeft`, `SplitVertical`, `LayoutTabbed`, …), bound in
   `[keyboard_shortcuts]` exactly like `TileWindowLeft` is today. The i3
   preset is a shipped TOML fragment, `config/presets/i3.toml`, with `Logo`
   as `$mod` and the stock i3 defaults; the user copies it into their config
   and edits lines. No preset key in the config format, no hidden defaults:
   what is bound is what is in the file. The spec's non-goal "reproducing any
   specific existing tiler's keybindings verbatim" softens to "not by
   default".
3. **An i3-syntax command language for scripting.** One parser for
   `focus left`, `move container to workspace 3`,
   `resize grow width 10 px or 5 ppt`, `split v`, `layout tabbed`,
   `floating toggle`, `fullscreen`, `kill`, each resolving to the same named
   action. Used by the D-Bus method, the CLI, and the headless tests, so
   existing `i3-msg` scripts port with a rename. Shortcuts do not use it.
4. **A scriptable surface.** `otto-msg` (CLI) over `org.otto.Shell1` with
   `RunCommand(string)` and `GetTree() -> JSON`. Full sway-ipc socket
   compatibility is a later, optional layer on top of the same two calls.

## What exists today

| Piece | Where | Reuse |
| --- | --- | --- |
| Half-snap zones (left / right / maximize) | `src/workspaces/tiling_overlay.rs`, `TileZone`, `zone_from_pointer` | Overlay view becomes the slot overlay for drag-into-tree; zones stay for floating workspaces |
| Snap apply / restore with animation | `src/shell/xdg.rs` `apply_tile`, `untile`, `animated_client_size` | Extract the per-window "animate to rect, then set states" body into a helper both paths call |
| Per-window snap state | `WindowView { tiled_zone, unmaximised_rect }` in `src/workspaces/window_view/view.rs` | `unmaximised_rect` is the spec's "floating rectangle it had before it was tiled" |
| Usable area | `usable_zone(output)` in `src/shell/mod.rs` | The root container's rectangle, minus the outer gap |
| Placement with transition | `Workspaces::map_window_on_output(.., Some(transition))` | Every cell move goes through this |
| Drag grab with zone detection | `PointerMoveSurfaceGrab` in `src/shell/grabs.rs` | Gains a "workspace tiles" branch: detach + slot overlay instead of edge zones |
| Popup re-anchoring after a move | `reposition_popups_for_window` | Called per relaid-out window |
| Shortcut actions | `src/config/shortcuts.rs` (27 builtins), `src/input/actions.rs` | Add the tiling builtins alongside `TileWindowLeft`; `WorkspaceNum { index }` already shows the parameterised shape `MoveToWorkspace { index }` needs |
| Headless harness | `src/headless.rs` `tile_focused`, `window_tiled_zone`, `window_floating_rect`; `tests/tiling.rs` (11 tests) | Extend with `run_command` and `tree_json` |
| D-Bus | `org.otto.Settings`, `org.otto.Dialog1`, … via `zbus` in `src/settings_service.rs` and the portal | Same pattern for `org.otto.Shell1` |

What does **not** exist: any per-workspace layout state, directional focus,
gap hit-testing, a compact titlebar variant, a general command IPC, or a
scaled-last-frame render path (today `apply_tile` reconfigures the client on
every animation frame; the spec makes that the non-default "faithful" mode).

## Architecture

New module `src/workspaces/tiling/`, kept out of the 7000-line
`workspaces/mod.rs`:

```
tiling/
  tree.rs      pure data + operations, no compositor types
  layout.rs    pure fn (tree, rect, gaps, min sizes) -> Vec<(leaf, rect)>
  apply.rs     relayout: per-window animate + configure, transactions
  command.rs   i3 grammar -> Command enum
  state.rs     per-workspace TilingState, held on WorkspaceView
  focus.rs     directional focus / move over resolved rects
  mod.rs
```

**Division of labour.** `layout.rs` is the truth: it answers "what is this
window's rectangle" synchronously, for the client configure, hit-testing,
scanout eligibility and tests. `apply.rs` turns rectangles into motion and
configures, and owns the one hard fact about animating a tiled layout: a
resized rectangle only has real content once the client has committed a
buffer of that size. The engine can move a layer for free; it cannot resize
its content.

**`tree.rs`.** `Node = Leaf(WindowId) | Container { layout: Split(Axis) | Tabbed | Stacked, children: Vec<(NodeId, f32)> }`.
Operations from the spec, each a method returning what changed: `insert_next_to`,
`remove`, `move_dir`, `swap_dir`, `resize`, `promote` / `demote`,
`set_layout`, `split`, `equalize`, `focus_parent` / `focus_child`, and
`dissolve_single_child_containers` as an invariant restored after every
mutation. Fractions only, never pixels. Fully unit-tested with `cargo test --lib`
before any compositor code touches it.

**`layout.rs`.** Resolves the tree against the usable rectangle in logical
pixels, applying outer/inner gaps, the lone-tile no-gap rule, per-leaf minimum
sizes with the spec's overflow rule, and the tabbed/stacked title-strip height.
Output rects are snapped with `workspaces::utils::snap_extent_px` so tiles land
on the pixel grid at fractional scale (see `rendering.md`; an off-grid layer
origin blurs the whole window).

**`command.rs`.** A hand-written parser for the i3 subset in the table below.
No dependency. Errors carry the offset so `otto-msg` can print them the way
`swaymsg` does.

**`state.rs`.** On `WorkspaceView`, `Arc<RwLock<TilingState>>`:

```
TilingState {
  mode: Floating | Tiling,
  tree: Tree,
  focused: Option<NodeId>,          // may be a container after `focus parent`
  preselect: Option<Axis>,
  floating: HashSet<WindowId>,      // the floating layer: i3's exceptions
  held_slots: HashMap<WindowId, SlotPath>,  // fullscreen / monocle return points
  monocle: Option<WindowId>,
}
```

**`apply.rs` — motion paced by the client.** Moves and swaps, where sizes
do not change, animate the window layers with a lay-rs transition and look
right throughout: the buffer stays valid. Resizes are configured per
animation frame, as half-snap and the MVP already do, and the window is
drawn with whatever buffer the client has committed most recently. A client
that keeps up (GTK, Qt, otto-kit at frame rate) shows real content on every
frame; a slow one lags behind its rectangle for a few frames, which is the
honest state of affairs rather than a stretched or clipped stand-in. No
Taffy mirroring of the tree: it would move the same problem into the engine
and reparent window layers for nothing.

Two refinements on top of the MVP:

- **Transactions for the final frame.** The last configure of a relayout is
  tracked per window; the layout is presented as settled only when every
  affected client has committed the final size, or a short deadline passes.
  This is sway's transaction model applied to the end of the animation, so
  a slow client never leaves a half-applied layout on screen.
- **Configure only what changed.** Leaves whose rectangle is unchanged get
  no configure at all.

**Applying a layout.** One entry point on `Otto`, `relayout_workspace(output,
workspace, transition)`: resolve the tree with `layout.rs`, then for each leaf
whose rect changed animate the layer and configure the client through
`apply.rs`, and call `reposition_popups_for_window`. Every mutation of the tree — from a
keystroke, a map, an unmap, a drag drop, a usable-area change — ends by
calling it.

**Hooks into existing paths.**

| Event | Today | With tiling |
| --- | --- | --- |
| `new_toplevel` / first map (`shell/mod.rs` cascade placement) | cascade | if workspace tiles and window is eligible: `insert_next_to(focused)`, relayout |
| unmap, minimize, move to workspace | — | `remove`, relayout; destination workspace inserts |
| focus change (`focus.rs`) | order list | also `state.focused = leaf` |
| `resize_request` / `move_request` on a tiled window | interactive grab | resize refused; move starts the detach drag |
| `maximize_request` | animate to usable zone | monocle: hide siblings, hold slot |
| fullscreen | overlay layer | hold slot, restore into it |
| `recalculate_exclusive_zones`, dock size/edge change, output mode/scale, rotation | nothing for windows | relayout every tiling workspace on that output |
| `TileWindowLeft/Right` | half-snap | in a tiling workspace: `focus left/right` |
| decoration mode (`xdg_decoration_handler.rs`) | SSD/CSD | SSD gets the compact bar; xdg states carry `TiledLeft/Right/Top/Bottom` per touching edge, `Maximized` only for a lone gapless tile or monocle |
| XWayland (`apply_tile_x11`) | same rects | same rects, `_NET_WM_STATE` where an equivalent exists |

**Floating, the i3 way.** A tiling workspace has a floating layer for the
exceptions, exactly as i3 does and as the spec describes. A window floats
automatically when it is a dialog or has a parent, when min == max size or
`is_resizable() == false`, or when it is a utility or splash surface;
`[tiling] float = ["app_id", …]` is the `for_window … floating enable`
equivalent. `floating toggle` moves the focused window between the tree and
the floating layer; `focus mode_toggle` moves focus between the two layers.
Floating windows always draw above the tiles, keep their full titlebar and
shadow, and are moved and resized as on a floating workspace. Nothing in the
tree ever overlaps them.

## Animation configuration

Every tiling animation — a window joining or leaving the tree, a move or
swap, a resize step, a re-fit after the usable area changed, entering or
leaving tiling mode, monocle in and out — is driven by `relayout_workspace`
and takes its transition from one place. That place follows the convention `[workspaces] switch_duration` /
`switch_bounce` already set: a duration in seconds and a bounce, both
clamped, and a duration of `0` means no animation at all — the layer is set
without a transition and the tree lands in one frame. Nothing else is
needed to "disable" tiling animations.

```toml
[tiling]
# Insert, remove, move, swap, equalise, re-fit. 0 = snap.
layout_duration = 0.3
layout_bounce = 0.0
# Entering and leaving tiling mode: every window flies to its cell or back.
mode_duration = 0.4
mode_bounce = 0.1
# Monocle in and out.
monocle_duration = 0.25
# Design mode: cells chasing a dragged handle, splits, swaps, presets.
design_duration = 0.35
design_bounce = 0.25
inner_gap = 8
outer_gap = 8
smart_gaps = true       # a lone tile drops the gaps
decoration = "minimal"  # "minimal" title line, or "none" for a border only
float = []              # app ids that always float
```

The values are read where `relayout_workspace` picks its transition, so they
apply live through the settings machinery like the workspace switch does. A
`None` transition is a first-class case in `apply.rs`, not a very short one:
insert adds the layer at its final size, remove detaches it at once, and the
client is configured with no intermediate frame.

Two things ride on top:

- **Per-invocation skip.** Every named action and command accepts a
  `no_animation` flag (`{ builtin = "FocusLeft", animate = false }` in the
  shortcut config, `--no-animation` on `otto-msg`) so a keybinding can move a window
  or switch the layout instantly while the same operation from the pointer or
  from a script still animates. It is the same distinction the workspace
  switch draws between a keybinding and a trackpad swipe, made explicit per
  binding.
- **`[accessibility] reduce_motion`.** A new global that forces every
  duration above to zero, and the workspace switch's too. It maps to the
  freedesktop `org.freedesktop.appearance` reduced-motion key when that is
  read through the portal, so it reaches clients as well. It does not exist
  today; adding it is part of Phase 1 because the tiling settings are the
  first place a second animation family appears.

## Named actions

Phase-1 set, one builtin per operation, parameterised where i3 takes an
argument:

```
FocusLeft/Right/Up/Down, FocusParent, FocusChild, FocusModeToggle
MoveContainerLeft/Right/Up/Down, MoveToWorkspace { index }
SplitHorizontal, SplitVertical, SplitToggle
LayoutSplitH, LayoutSplitV, LayoutTabbed, LayoutStacking, LayoutToggle
ResizeGrowWidth/ShrinkWidth/GrowHeight/ShrinkHeight { step }
TilingDesignToggle, EqualizeContainer
FloatingToggle, ToggleFullscreen, CloseWindow
TilingToggle                       # workspace mode
```

`config/presets/i3.toml` binds them to the i3 defaults:

```toml
[keyboard_shortcuts]
"Logo+h" = "FocusLeft"
"Logo+j" = "FocusDown"
"Logo+Shift+h" = "MoveContainerLeft"
"Logo+v" = "SplitVertical"
"Logo+b" = "SplitHorizontal"
"Logo+w" = "LayoutTabbed"
"Logo+Shift+Space" = "FloatingToggle"
"Logo+Space" = "FocusModeToggle"
"Logo+f" = "ToggleFullscreen"
"Logo+Shift+q" = "CloseWindow"
"Logo+Shift+3" = { builtin = "MoveToWorkspace", index = 2 }
"Logo+3" = { builtin = "Workspace", index = 2 }
# …
```

The file is documented in `docs/user/keyboard-shortcuts.md` as "copy this
block into your config"; there is no `preset =` key and no include mechanism
(the config loader layers fixed paths only). The example config gains the
same block commented out.

## Design mode

Design mode is the friendly way to shape a tiled workspace. It is for the
person who has never used a tiler and would not learn `split v` or
`resize grow width 10 ppt`: enter it, and the layout's rows and columns
become visible things you can grab, split, drag around and resize, with the
windows running inside their cells the whole time. The keyboard actions and
the command grammar are the power-user path to the same tree; design mode is
the one Otto shows first, and the reason a tiling workspace does not need a
manual.

**Friendly means:**

- *Nothing hidden.* Every handle is drawn, large enough to hit without
  aiming, and lights up on hover with a cursor that says what a drag will do.
  Nothing depends on knowing that a gap is secretly a handle.
- *Always see the result before committing.* Drags show the shares as
  percentages, drops show the slot before release, and a change animates so
  the eye can follow what moved.
- *No wrong states.* A drag stops at a window's minimum size; a split of a
  cell that cannot be split is greyed out; a container never ends up with one
  child. There is no way to make a layout the user then has to repair.
- *Undo.* Every structural edit in design mode goes on the session undo
  stack, `Ctrl+Z` reverts it, the same mechanism Otto Settings uses.
- *Starting points.* An empty workspace in design mode offers a row of
  layout presets — two columns, three columns, main and stack, grid — one
  click applies one as empty slots to drop windows into.
- *Leaves you where you were.* Leaving design mode changes nothing; the
  windows are where the layout put them, focused as before.

**Entering and leaving.** A named action (`TilingDesignToggle`, bound to
`Logo+d` in the preset, since i3's `Logo+d` is a launcher Otto binds
elsewhere), an item in the workspace context menu, or a long press on a gap.
Escape, the action again, or a click on a window's content leaves it. Design
mode is per workspace and ends when the workspace scrolls away.

**What is shown.** The same overlay the edge snap already shows when a
window is dragged to a screen edge — a translucent white pane with a white
border and rounded corners, fading and sliding in with short ease-out
transitions (`TilingOverlayView`) — but one per cell, so the whole workspace
turns into a grid of those panes drawn over the windows. A cell's pane is its
window's rectangle including the gap; the panes together tile the usable
area exactly, so the layout's rows and columns read off the grid at a glance
with no outlines or dimming needed. The gaps between panes are the handles.
The focused cell's pane carries the accent-coloured border. Nested containers
are not drawn separately: the grid *is* the tree, and dragging a bar that
spans several panes makes the nesting visible when they move together.
Each pane shows a small centred toolbar on hover: split horizontal, split
vertical, layout kind (split / tabbed / stacked), close-cell. An empty slot
is the same pane with a dashed border and a `+`.

**Handles.**

- *Bar handle*: drag along the container's axis to move that split; the two
  neighbouring shares change and nothing else. The bar shows the two shares
  as percentages while dragging. Snaps to halves, thirds and quarters within
  a few pixels; hold `Shift` to bypass snapping. Double-click equalises the
  container.
- *Corner handle*: drags both splits at once.
- *Cell drag*: drag a cell by its body to swap it with the cell it is dropped
  on, or onto a bar handle to insert it at that split. The same slot overlay
  as drag-to-detach shows the result before release.
- *Container drag*: dragging the bar that borders a whole container moves the subtree,
  same rules.
- *Empty cells*: a split made on an empty workspace, or a cell whose window
  closes while in design mode, leaves an empty slot outlined in dashes. The
  next window to map fills the focused empty slot before splitting anything.
  This is how a user lays out a workspace first and populates it after, and
  the base for named layouts later.

**Keyboard in design mode.** Every named action still works, so a keyboard
user gets the visual feedback without giving up their bindings: arrows move
focus between cells, the resize steps apply to the focused cell, `Enter` on
a bar handle equalises. The two paths edit the same tree and can be mixed
mid-session.

**Adjusting cells is animated, a bit bouncy.** Design mode does not follow
the spec's "the pointer is the animation" rule for interactive resize. When a
bar is dragged, the shares update under the pointer but the panes and the
windows beneath them chase it on a spring — the same
`Transition::spring(duration, bounce)` the dock uses for magnification — so
a fast drag lags a touch and overshoots a little before settling on release.
A split, a swap, a preset or an equalise animates on the same spring. It is
what makes the grid feel like a physical thing being pushed around rather
than a wireframe. The spring is `[tiling] design_duration` /
`design_bounce`, defaulting bouncier than the layout spring; `0` snaps like
every other duration. Clients are reconfigured as they ack during the drag
and once more with the final size when the spring settles, so a slow client
never holds the panes back.

**Outside design mode.** Gaps are not drag handles. A window's titlebar drag
still detaches it and the slot overlay still works. This keeps pointer
handling on a tiling workspace identical to a floating one until the user
asks for the editor, and avoids a hidden hit-area competing with window
edges.

**Implementation.** A `TilingDesignView` in `src/workspaces/tiling/design.rs`
that generalises `TilingOverlayView` from one preview pane to a set: the
same layer recipe (30 % white fill, 80 % white 2 px border, 12 pt radius,
0.15 s ease-out move, 0.2 s fade) per cell, plus bar and corner handle layers
in the gaps, parented above the containers and driven from the same shares as
`apply.rs` so the panes animate in step with the windows beneath them. The
pointer path hit-tests the handles before windows (the same hook the dock's resize
handle uses). Drags are a `PointerGrab` that writes shares into the tree and
calls `relayout_workspace` with no transition. The toolbar reuses otto-kit
buttons drawn by the compositor, as the titlebar controls are.

## Decorations on tiles

Tiles want less chrome than floating windows, and i3 users disagree on how
much less: i3 draws a one-line title bar by default, sway users very often
set `default_border pixel 2` and keep only a coloured border. So it is a
setting, `[tiling] decoration = "minimal" | "none"`, default `minimal`:

- **minimal** — a bar one text line high with the title and a close button,
  squared corners, no shadow. It keeps the move handle, the window menu and
  the title, which is the spec's reason for keeping a bar at all.
- **none** — no bar; the focused tile gets a hairline border in the accent
  colour, the rest a neutral hairline. Moving a tile is then design mode or
  the keyboard. This is sway's `pixel` border.

Tabbed and stacked containers draw their strip in both settings, since it
is the only way to see the hidden windows. Client-side-decorated windows are
told they are tiled on every touching edge and square off on their own; they
get no bar in either setting. Floating windows in a tiling workspace keep
the full floating decoration.

The variant is chosen per window in `decoration_view.rs` from the
workspace's mode, and swapped when the window enters or leaves the tree —
the same path the maximized (gapless, squared) variant already uses.

**Otto's own apps.** otto-files, otto-settings, the launcher and quick view
draw their own titlebar through otto-kit's titlebar component, so the
compositor-side variants never reach them. They follow the same setting
from the client side:

- the tiled xdg edge states in the configure tell the app it is tiled; the
  otto-kit titlebar switches to its compact form and squares its corners
  while any edge is tiled, as GTK does;
- the `minimal` / `none` choice goes out over `org.otto.Settings` like the
  theme and the controls side already do, so the app applies it live;
- with `none` the app draws no bar and is moved by design mode or the
  keyboard, like any client-decorated window.

One component change in otto-kit covers every app that already implements
the settings callback; islands, quick view and lock still need that hook.

## Command language

The scripting grammar, resolving to the actions above. Phase-1 subset:

```
focus left|right|up|down|parent|child|mode_toggle
move left|right|up|down
move container to workspace <n|name>
workspace <n|name|next|prev>
split h|v|toggle
layout splith|splitv|tabbed|stacking|toggle [split|all]
resize grow|shrink width|height <n> px [or <n> ppt]
resize set width <n> ppt|px [height ...]
floating toggle|enable|disable
fullscreen [toggle]
kill
tiling toggle                      # Otto: workspace mode
gaps inner|outer <n>
```

Deferred: `mark` / `[con_mark]` criteria, `mode "resize"` binding modes,
`scratchpad`, `assign`, `for_window`, `exec` (shortcuts already do `run`).

Workspaces by number need "create on demand": i3 makes workspace 7 when you
switch to it. Otto today has a fixed strip with `Ctrl+1..4`; `workspace <n>`
should append workspaces until `n` exists. i3 also drops a workspace when it
empties; Otto does **not** follow that. Otto's workspaces are persistent —
named, reorderable, per output — and a renamed workspace vanishing because its
last window closed would break that model. An empty workspace stays.

## IPC

`org.otto.Shell1` on the session bus, alongside the existing `org.otto.*`
interfaces:

- `RunCommand(s command) -> a(bs)` — one result per `;`-separated command,
  mirroring `swaymsg`'s reply shape.
- `GetTree() -> s` — JSON with the i3 node shape (`id`, `type`, `layout`,
  `nodes`, `floating_nodes`, `focused`, `rect`, `app_id`, `name`, `window_properties` for X11).
- `GetWorkspaces() -> s`, `GetOutputs() -> s`.
- Signals `WorkspaceChanged`, `WindowChanged`, `ModeChanged` for bars.

`components/otto-msg`: a `-t get_tree` / positional command CLI over that
interface, so `otto-msg focus left` and `otto-msg -t get_tree | jq` work.
A sway-ipc-compatible Unix socket (`$SWAYSOCK`) is a Phase-4 shim over the same
calls; it would let waybar's `sway/workspaces`, `i3-msg`-based scripts and
`autotiling` run as-is. Check separately whether Otto exposes
`ext-workspace-v1` and `wlr-output-management`; waybar's `wlr/workspaces` and
kanshi need them, and both matter to this audience independently of tiling.

## Phases

**Phase 0 — pure core.** `tree.rs`, `layout.rs`, `command.rs` with unit tests
covering every spec rule (insertion by cell shape, share redistribution on
removal, single-child dissolve, move-out-of-container, min-size overflow, lone
tile gaps, tabbed strip height). No compositor changes. Reviewable on its own.

**Phase 1 — keyboard tiling.** `TilingState` on `WorkspaceView`; mode toggle
(shortcut + workspace context-menu item); insert/remove on map/unmap/minimize/
move; `relayout_workspace` with the extracted animation helper; xdg tiled
states; auto-float rules and the floating layer; `[tiling]` durations with
`0` = snap, per-action `animate = false`, and `[accessibility] reduce_motion`;
the named actions and `config/presets/i3.toml`; focus
directions, moves, splits, layouts (tabbed/stacked rendered as a title strip on
the compact bar), resize step, float toggle, monocle, fullscreen slot holding;
`workspace <n>` create-on-demand. Headless tests in `tests/tiling_tree.rs`
drive everything through `run_command` and assert on `tree_json` and
geometries, the way `tests/tiling.rs` does for half-snap.

**Phase 2 — pointer and chrome.** Design mode: the pane grid, bar and corner
handles with `ew-resize`/`ns-resize`/`nwse-resize` cursor shapes, live
reconfigure on ack, cell toolbar, empty slots, cell and container drag;
drag-to-detach with the reused `TilingOverlayView` showing
the slot; dropping a floating window into a tree; the minimal and none decoration variants;
accent focus border; no shadow on tiles; usable-area re-fit on dock/layer-shell/
mode changes; XWayland parity; workspace-selector mode indicator.

**Phase 3 — scriptability and depth.** `org.otto.Shell1` + `otto-msg`;
`GetTree`; marks and criteria; binding modes (`mode "resize"`); scratchpad;
per-app `assign`/`for_window` rules; persistence of mode and tree across
restart (spec open question — recommend yes for mode, tree best-effort by
app_id).

**Phase 4 — compatibility.** Sway-ipc socket shim if there is demand.
The spec's scaled-last-frame animation is dropped: there is no honest way
to fill a resized rectangle before the client has drawn it. Per-frame
configure plus end-of-animation transactions is the model.

## Open: exposé on a tiled workspace

Deliberately unresolved. The spec says exposé works on a tiled workspace
exactly as on a floating one, but that is untested against the tree, and a
tiled workspace is already an overview of itself. Questions to settle once
Phase 1 is on screen:

- whether exposé should spread tiles at all, or only pull the floating layer
  and the tabbed/stacked hidden windows out where they can be seen;
- what happens to the containers' layers while exposé reparents or mirrors
  windows (`pre_expose_order` and the mirror path assume `windows_layer`
  children);
- whether selecting a window in exposé should also move it in the tree, or
  only focus it;
- how the workspace-selector previews render a tree (they replicate
  `wallpaper_group` plus windows today).

Revisit after Phase 1; nothing in Phases 1–2 depends on the answer.

## Spec deltas to make

Update `specs/tiling.md` when Phase 1 starts:

- Non-goals: keybinding parity becomes "not the default"; add the named
  actions, the shipped i3 preset file, and the command grammar as goals.
- Promote tabbed/stacked from open question to behaviour.
- Add `focus parent` / `focus child` and container focus.
- Replace the "Resizing / Pointer" paragraph with design mode: gaps are not
  handles outside it; add empty slots; interactive resize animates on a
  spring rather than tracking the pointer rigidly.
- Add the command language and IPC sections.
- Add workspace create-on-demand (touches `workspaces-multi-output.md`).
- Animation: replace "fluid by default" (scaled last frame) with per-frame
  configure paced by the client, plus transactions for the settled frame.

## Risks

- **Configure storms.** Per-frame configures across N clients on every
  layout change is exactly what the spec warns about. Mitigate in Phase 1 by
  configuring only leaves whose rect changed and by throttling configures to
  acked ones (the interactive-resize rule), not by fixed cadence.
- **Scanout.** A lone tile or monocle must still promote to a plane
  (`specs/plane-scanout.md`); the tiled xdg states must not change the
  promotion gate. Verify on tty with `/tmp/otto-dump-planes`.
- **lay-rs cancelled changes.** Re-targeting a running transition has bitten
  exposé before; interruptible relayout leans on that fix and needs a headless
  regression that issues two moves within one animation.
- **Fractional scale.** Every cell rect goes through `snap_extent_px`; a test
  at scale 1.5 asserting integer physical origins belongs in Phase 0.
- **`workspaces/mod.rs` size.** All new logic lives in `workspaces/tiling/`;
  `mod.rs` only gains the calls into it.
