# Window Tiling

**Status:** in progress
**Related specs:** [window-decorations](window-decorations.md), [workspaces-multi-output](workspaces-multi-output.md), [window-focus-navigation](window-focus-navigation.md), [context-menus](context-menus.md)

**Implementation status.** The first cut is keyboard-only: a per-workspace
tiling mode, split containers, insertion and removal, directional focus and
move, pre-selection, the keyboard resize step and equalise, and the layout
animation. Not in it yet: the pointer paths (drag-to-detach and design mode),
the floating layer and its toggle, tabbed and stacked containers, the compact
decoration variants — a tile still keeps the full titlebar — and the command
grammar with the interface that runs it.

## Summary

A workspace can be switched from free-floating window placement to *tiling*,
where every window on it is laid out edge to edge with no overlap and no wasted
space. The arrangement is a tree of split containers the user shapes directly —
by keyboard, by dragging a window's titlebar into a slot, or by dragging the gap
between two windows to change their share. Tiling is a property of one workspace
on one output, so a tiled workspace and a floating one coexist a swipe apart.

## Goals

- A workspace can be put into, and taken out of, tiling mode; the mode belongs
  to that workspace on that output and survives switching away and back.
- In a tiled workspace, every non-floating window is visible, non-overlapping,
  and together they fill the usable area exactly.
- The arrangement is a tree: containers split horizontally or vertically and
  nest arbitrarily, so a user can build any layout expressible as recursive
  splits.
- Every operation is available from the keyboard and from the pointer, and the
  two produce the same tree.
- A window that cannot sensibly be tiled — a dialog, a fixed-size window, a
  window the user floats by hand — floats above the tiles instead, and the
  remaining tiles fill the space as if it were not there.
- Layout changes animate, in the same idiom as maximize and half-snap. Windows
  move and resize to their new cells; they never jump.
- The layout is expressed in *fractions*, never in absolute pixels, so a
  workspace re-fits itself to a different output, a different scale, a rotated
  screen, or a changed dock size without losing its shape.
- A window's own minimum size is respected: the layout never configures a window
  smaller than it says it can be.
- Clients are told the truth: a tiled window is told which of its edges abut
  something, so toolkits square off the right corners and hide the right
  affordances.
- Every tiling operation is a named action that can be bound to a key, so a
  workspace can be shaped entirely from the keyboard.
- A preset file ships with Otto binding those actions to the i3 defaults; a
  user copies it into their configuration and edits lines, and nothing in it
  is bound until they do.
- A scripting interface accepts i3's command syntax — `focus left`,
  `move container to workspace 3`, `split v`, `layout tabbed` — resolving to
  the same named actions, so existing scripts port with a rename. The syntax
  is for scripting only; key bindings name actions.

## Non-Goals

- Tiling as the default mode. Otto is a floating desktop; a workspace tiles
  because the user asked it to, or because the configuration set the default.
- A layout that overlaps windows, or a "manual" mode where the user positions
  tiles by absolute coordinates. Overlapping windows are what floating mode is.
- Reproducing any specific existing tiler's keybindings *by default*. The
  bindings are the user's; Otto ships the i3 set as a file to copy in, not as
  a hidden default.
- Delegating layout decisions to an external process. The layout is computed
  inside the compositor. A protocol for an external layout generator is a
  possible future addition and must not be designed out, but is not specified
  here.
- Tiling across outputs. A tree covers one workspace on one output.

## Behavior

### Mode

**Entering and leaving.** A workspace is either *floating* (the default) or
*tiling*. A shortcut, and an item in the workspace's context menu, toggles the
mode of the workspace under focus. On entering tiling mode, every eligible
window already on the workspace joins the tree in its current stacking order,
most recently focused first, and animates into its cell. On leaving, every
window returns to the floating rectangle it had before it was tiled; a window
that has no such rectangle — it was opened while the workspace was tiled — keeps
the geometry its last cell gave it, moved to avoid landing exactly on top of
another window.

**Scope.** The mode, and the tree that goes with it, belong to one workspace on
one output. Moving a window to another workspace removes it from the source
tree and inserts it into the destination's, if that one tiles. A workspace that
moves to a different output keeps its tree and re-fits it to the new usable
area.

**Default.** Configuration may declare that new workspaces start tiled.

**Workspaces on demand.** A command that names a workspace which does not
exist yet creates it: asking for workspace 7 on an output with four appends
workspaces until there is a seventh. A workspace is never taken away again for
becoming empty. Otto's workspaces are named, reorderable and persistent, and
one vanishing under the user because its last window closed would break that;
an empty workspace stays.

### The tree

A tiled workspace holds a tree. Every leaf is a window. Every interior node is a
*container* with an axis — row or column — and an ordered list of children, each
holding a fraction of the container's extent along that axis. Fractions within a
container always sum to one. A container's children are laid out in order, left
to right for a row, top to bottom for a column.

The root container fills the workspace's usable area — the output minus the
dock, the top bar, and any layer-shell exclusive zone — inset by the outer gap.
Between siblings sits the inner gap. Gaps are configurable, and may be set to
zero, in which case tiles share edges exactly.

**Insertion.** A new window is inserted next to the focused window. If a split
direction has been pre-selected (see *Pre-selection*), the focused leaf is
replaced by a new container of that axis holding the old window and the new one.
Otherwise the new window is inserted as a sibling of the focused window when the
parent container's axis matches the shape of the focused window's cell, and
otherwise splits that cell — a cell wider than it is tall splits into a row, a
taller one into a column. The result is that windows opened one after another
fill the screen in a stable, predictable spiral rather than shaving ever-thinner
strips off one side.

The insertion always takes half the focused window's share; the other siblings
are untouched. With no focused window — the workspace was empty — the new window
becomes the root and fills the area.

**Removal.** A window that closes, minimizes, moves to another workspace, or is
floated is removed from the tree. Its share is redistributed to its siblings in
proportion to their existing shares. A container left with one child is
dissolved into its parent, so the tree never keeps a container that splits
nothing.

**Container layouts.** A container lays its children out in one of three
ways. *Split* — the default — gives each child its share of the container's
extent along the axis. *Tabbed* and *stacked* give every child the whole of
the container's cell and show one child at a time, the rest hidden behind a
strip of titles drawn along the top of the cell: one row of tabs side by side
for tabbed, one title per line for stacked. Selecting a title raises that
child. A command sets the focused container's layout and a toggle command
cycles it. From the outside a tabbed or stacked container is one cell:
directional focus and move step over it as a unit, and step between its
children only from within.

**Restoring.** A window that comes back — unminimized, or unfloated — is
inserted at the focused position, not at the one it left. A window that returns
from fullscreen goes back to the exact slot it held, which is kept for it while
it is away.

### Navigation and movement

**Directional focus.** A directional focus command moves focus to the window
whose cell is nearest in that direction, choosing among candidates that overlap
the focused window's span on the perpendicular axis; where several do, the one
nearest the focused window's cursor-side edge wins. Focus never wraps within a
workspace. A directional focus command that finds nothing inside the workspace
moves focus to the adjacent output's focused window, if one lies that way.

**Directional move.** A directional move command moves the focused window
through the tree in that direction: it swaps with the neighbouring sibling when
there is one along the container's axis, and otherwise leaves the container and
is inserted into the grandparent at the corresponding side, splitting it if the
axis does not match. Moving past the last position at the root level splits the
root along the new axis and places the window there. The moved window keeps
focus, and its share travels with it.

**Swap.** A swap command exchanges the focused window with the one in a given
direction, leaving both cells where they are.

**Pre-selection.** A command arms the next insertion to split the focused window
along a chosen axis. While armed, the focused window's cell shows which half the
next window will take. Opening a window, moving one in, or pressing the command
again disarms it.

**Container focus.** Focus normally rests on a window, but it can also rest
on a container. A *focus parent* command moves focus from the focused window
to the container holding it, and again to that container's parent; *focus
child* moves back down into the child it came from. While a container is
focused its whole cell is marked, and the move, resize, layout and close
commands act on the entire subtree — moving a focused container moves
everything inside it. Focusing a window by any means ends container focus.

**Promotion.** A command lifts the focused window out of its container and makes
it a sibling of that container's parent, and the reverse command pushes it back
down into the neighbouring container. Together these reshape nesting without
closing anything.

### Resizing

**Design mode.** Outside design mode the gaps between tiles are not drag
handles: pointer handling on a tiled workspace is the same as on a floating
one, so no hidden hit area competes with a window's edges. A command, an item
in the workspace's context menu, or a long press on a gap enters *design
mode*, where the layout's cells are drawn as panes over the windows and the
gaps between them become visible handles that light up on hover with a cursor
saying what a drag will do. Dragging a bar handle changes the shares of the
two children either side of that split and nothing else; where four cells meet
at a corner, the corner handle drags both splits at once; a drag that would
push a window below its minimum size stops there. A cell can be dragged onto
another to swap with it, or onto a bar handle to be inserted at that split,
and each pane offers splitting the cell and choosing its container's layout.
A split made with nothing to fill it, or a cell whose window closes while
design mode is open, leaves an *empty slot* drawn as a dashed pane: the next
window to open fills the focused empty slot before it splits anything, so a
workspace can be laid out before it is populated. Every named action keeps
working in design mode, and leaving it changes nothing about where the windows
are.

**Keyboard.** A resize command grows or shrinks the focused window along a given
axis by a configurable step, taking from — or giving to — the sibling in that
direction. When the focused window is the only child along that axis, the
command applies to the nearest ancestor that has a sibling along it, so that
every resize command does something as long as more than one window is tiled.

**Reset.** A command redistributes the shares of the focused container, or of
the whole tree, equally.

**Client-driven resize.** A tiled window's request to resize itself is refused:
its cell is its size. A window whose minimum size grows beyond its cell keeps
its minimum and takes the space from its siblings; when even that is impossible,
the window is floated and the user is not left with a broken layout.

### Dragging a window

Dragging a tiled window by its titlebar detaches it: the window follows the
pointer at a reduced size, and the tree behind it closes up as though the window
had been removed. While the drag is over a tiled workspace, an overlay shows the
slot the window would take on release — the half of the hovered tile that the
pointer is nearer to, or the whole of it when the pointer is near its centre,
meaning "swap with this window". Releasing inserts the window there and the
layout animates to accept it. Releasing outside any tile, or on a floating
workspace, drops the window as a floating window at the pointer.

Dragging a *floating* window over a tiled workspace shows the same slot overlay,
so a floating window can be tiled by dropping it in. The existing screen-edge
snap zones remain available on floating workspaces and are not shown on tiled
ones, where every position is already a slot.

### Floating within a tiling workspace

**Automatic.** A window floats rather than tiles when it is a dialog or has a
parent, when it declares a fixed size — its minimum and maximum sizes are equal
— when it is a utility or splash surface, or when it is the kind of window Otto
already refuses to maximize. Configuration may name applications, by app id or
title, that always float.

**By hand.** A command, and a titlebar context-menu item, toggles the focused
window between floating and tiled. A floated window returns to the size it had
before it was tiled, or to a sensible fraction of the workspace if it never had
one, centred on its old cell.

**Stacking.** Floating windows in a tiled workspace always draw above the tiles
and are always focusable; tiles never overlap them. A command cycles focus
between the floating layer and the tiled layer.

### Fullscreen and maximize

A tiled window that goes fullscreen covers the output, and its slot is held for
its return. Maximize in a tiled workspace means *monocle*: the focused window
temporarily fills the whole usable area, hiding the other tiles, and the command
again restores the layout with the tree unchanged. The half-snap tiling commands
have no effect in a tiled workspace beyond focusing the window in that direction.

### Animation

Every discrete layout change — an insertion, a removal, a move, a swap, a reset,
a re-fit after the usable area changed — animates. Windows travel and grow into
their new cells; they never teleport. Animations are interruptible: a second
command arriving mid-flight re-targets the windows already in motion rather than
queueing behind them, so holding down a move command sweeps a window across the
layout smoothly.

**Motion is paced by the client.** A move or a swap, where no size changes,
animates the whole way: the buffer stays valid, so the window travels
smoothly. A resize is different — a rectangle only holds real content once the
client has drawn a buffer of that size — so during a resize the client is
configured with the rectangle of every animation frame and the window is drawn
with the most recent buffer it has committed. A client that keeps up shows
real content throughout; a slow one lags behind its rectangle for a few
frames, which is the honest state of affairs rather than a stretched or
clipped stand-in. Only the windows whose rectangle actually changed are
configured at all.

**Settling.** A layout change is presented as settled only once every affected
client has committed its final size, or a short deadline has passed, so a slow
client never leaves a half-applied layout on screen.

**Configurable.** Each family of layout animation — a layout change, entering
and leaving tiling mode, monocle, a design-mode drag — has a configurable
duration and bounce. A duration of zero means *snap*: the windows are placed
at their new cells in one frame with no motion and each client is configured
once. Snap is a case of its own, not a very short animation. A single key
binding may additionally ask for its action to run without animation, so a
keystroke can move a window instantly while the same operation from the
pointer or from a script still animates. An accessibility *reduce motion*
setting forces every one of these durations, and the workspace switch's, to
zero regardless of the tiling configuration.

**Interactive resize animates too.** Dragging a handle in design mode updates
the shares under the pointer, but the panes and the windows beneath them chase
it on a spring rather than tracking it rigidly, so a fast drag lags a little
and overshoots before settling. Clients are reconfigured as they acknowledge
the previous size during the drag, and once more with the final size when the
spring settles, so a slow client never holds the drag back. That spring has
its own duration, bouncier than the layout one by default; a duration of zero
makes the layout track the pointer exactly.

**Dragging a window** animates the tree closing behind the detached window and
opening again to accept it, on the same curve.

### Decorations

How much chrome a tile keeps is a setting, because users disagree about it.
With *minimal*, the default, a tile gets a bar one text line high with the
title and a close button, squared corners and no shadow: it is still the move
handle and the route to the window's controls and menu. With *none* a tile
gets no bar at all, and moving it is design mode or the keyboard. Tabbed and
stacked containers draw their title strip under both settings, since it is the
only way to see the windows they hide. The window frame keeps its rounded
corners and its gap-borne separation from its neighbours when gaps are on;
with gaps off the frame squares off, as it already does when maximized.

The focused tile is marked with a border in the accent colour — a hairline one
under *none*, where the other tiles get a neutral hairline. Drop shadows are
not drawn on tiles — nothing overlaps, so there is nothing to cast onto — and
return when the window floats again.

A window that draws its own decorations is tiled the same way and is given no
bar under either setting; it is still moved by dragging whatever it treats as
its titlebar, and the keyboard commands reach it unchanged. Because it is told
which of its edges are tiled, it can square the corresponding corners and
compact its own titlebar. Otto's own applications, which draw their titlebars
themselves, follow the same setting from the client side: under *minimal* the
titlebar takes its compact form, and under *none* they draw no bar and are
moved by design mode or the keyboard.

### What clients are told

A tiled window is configured with the exact size of its cell, minus whatever the
titlebar takes. It is told it is tiled on each edge that touches another tile or
the edge of the usable area, so a client can square off the corresponding
corners. It is told it is maximized only when it is alone in the workspace with
gaps off, which is the one case where its rectangle really is the whole usable
area. A window in monocle is told it is maximized.

Windows managed through XWayland tile identically, are configured with the same
rectangles, and carry the same states where the X11 equivalent exists.

### Scripting

Every tiling operation is reachable by name from a key binding, and by i3's
command syntax from a script: `focus left|right|up|down|parent|child`,
`move …`, `move container to workspace <n>`, `workspace <n>`, `split h|v`,
`layout splith|splitv|tabbed|stacking|toggle`,
`resize grow|shrink width|height <n>`, `floating toggle`, `fullscreen`,
`kill`, `gaps inner|outer <n>`. The same interface reports the tree, the workspaces and the outputs in
i3's shape, so a bar or a script written against i3 or sway keeps working
after a rename. Key bindings never use that syntax: a binding names an action.

### Overview and previews

Exposé, the workspace selector previews, and the app switcher work on a tiled
workspace as far as this spec goes exactly as on a floating one — a tiled
workspace's windows are simply already spread out. Entering exposé does not
disturb the tree, and selecting a window from it focuses that window in place.
What exposé should add to a workspace that is already an overview of itself is
an open question below.

## Constraints & Edge Cases

- **Empty workspace.** A tiling workspace with no windows shows the wallpaper.
  The tree is empty, not a container of zero children.
- **One window.** A single tile fills the usable area inside the outer gap. When
  configured for it, a lone tile drops the gaps entirely and fills the area, so
  a single window does not look inset for no reason.
- **Usable area changes.** A change to the dock's size or edge, a layer-shell
  surface claiming or releasing an exclusive zone, an output mode or scale
  change, or a rotation re-fits the tree to the new area in one animation. Since
  shares are fractions, nothing is lost.
- **Minimum sizes.** When the sum of the minimum sizes along a container's axis
  exceeds the space available, the container gives each child its minimum and
  overflows past its own bounds rather than configuring any window below its
  minimum; the overflow is clipped at the usable area. This is the failure the
  user must see, not a silently broken layout.
- **Very many windows.** There is no cap on the number of tiles. Once cells fall
  below a usable size the user is expected to reach for another workspace; the
  layout does not start stacking on its own.
- **Windows that never map.** A window that is inserted and then dies before it
  ever draws is removed with no visible animation.
- **Popups.** A popup anchored to a window that moves is repositioned against
  the window's new rectangle, as it already is for half-snap and maximize.
- **Direct scanout.** A tiled workspace with one full-area tile, or a monocle
  tile, must remain eligible for scanout on the same terms as a maximized
  floating window.
- **Session lock and screensaver.** Locking does not disturb any tree.
- **Multiple outputs.** Two workspaces on two outputs may both tile, with
  independent trees. A window dragged across the boundary leaves one tree and
  enters the other.

## Rationale

**Why a tree of n-ary containers, rather than binary splits or a scrollable
strip.** Containers that hold more than two children make the common operations
behave the way users expect: three windows in a row resize against each other
without a hidden nesting order deciding which two move together, and removing
one of the three does not leave a lopsided binary chain behind. A scrollable
column model — windows never resized, the workspace scrolling sideways past
them — was considered and rejected: Otto's workspaces already scroll
horizontally as one plane, and a second horizontal scroll axis inside a
workspace would compete with that gesture and with exposé for the same input.

**Why per workspace and per output.** Otto's workspaces are already independent
per output, and the desk metaphor holds: one desk is laid out in a grid because
that is the work happening there, the next is a scatter of floating windows.
Making tiling a global mode would force the choice on work that does not want
it.

**Why fractions rather than pixels.** Every hard case — hotplug, rotation, a
dock that grows, a workspace dragged to a second screen, a scale change — is
free if the layout never stores a pixel. It also makes the layout comparable
across outputs, which is what lets a workspace move.

**Why motion is paced by the client.** An earlier draft had the drawn
rectangle lead the client: one configure for the final size, the last frame
scaled into the animated rectangle until the new buffer arrived. That was
dropped. There is no honest way to fill a resized rectangle before the client
has drawn it, and a scaled stand-in reads as a stretched, blurry window at
exactly the moment the user is watching it move. Configuring each frame, and
drawing whatever the client has committed, keeps every pixel true; the cost —
a configure storm across a screenful of clients — is paid down by configuring
only the windows whose rectangle changed and by holding the settled frame
until they have all acknowledged it.

**Why the layout is not expressed as engine layout nodes.** The scene engine can
lay out containers and animate them itself, and a split container maps neatly
onto a flex row. But the compositor needs each window's rectangle *now* — to
configure the client, to map it in the window space, to hit-test input, to
compute exclusive zones and scanout eligibility — and taking it from the engine
would mean reading geometry back out of the render layer on the input path. The
tree is therefore resolved to rectangles by the compositor, and those rectangles
drive the engine's layers and its transitions. The engine animates; it does not
decide.

**Why tabbed and stacked containers are specified, not deferred.** They were
an open question, and are now behaviour. Someone arriving with a years-old i3
configuration expects `layout toggle split` to do something, and a tree that
cannot hold a per-container layout kind has to be rewritten to gain one later.
Otto's compositor-drawn titlebar is already the right thing to draw the strip
with.

**Why the compositor computes the layout and the engine only animates.** The
tree is resolved to rectangles by the compositor, which needs each window's
rectangle synchronously in any case; those rectangles are then handed to the
scene engine, which animates positions and sizes towards them and decides
nothing. The layout is never expressed as engine layout nodes, so there is no
second source of truth to read geometry back out of on the input path.

**Why keep decorations.** The titlebar is compositor-owned in Otto and is the
window's move handle; dropping it in tiling mode would cost drag-to-rearrange,
the window controls, and the window menu, and would make the tiled and floating
halves of the desktop feel like two different systems. Making it compact instead
buys back the vertical space that motivates hiding it.

**Why the layout is computed in the compositor.** Layout has to happen inside
the same frame as the animation, the exclusive-zone recalculation, the popup
repositioning, and the decorated-versus-client rectangle conversion. A layout
process outside the compositor would sit in the middle of that path on every
map, unmap and resize. The layout is nonetheless specified as a pure function
from a tree and a rectangle to a set of rectangles, so that a future protocol
letting an external client answer that question is a substitution rather than a
redesign.

**Why insertion follows the shape of the cell.** Splitting a cell along its
longer axis keeps cells near square as a workspace fills up, which is what makes
a layout usable without the user directing every split. The pre-selection
command exists for when they want to.

## Open Questions

- **Persistence across sessions.** Whether a tiled workspace's tree, and the
  mode itself, should be restored on the next login.
- **Named layouts.** Whether the user should be able to save a tree and apply it
  to a workspace, and whether an application should be able to ask for a slot.
- **Whether the tiling mode belongs in the workspace selector UI** as a visible
  per-workspace indicator, rather than only in a menu and a shortcut.
- **Exposé on a tiled workspace.** A tiled workspace is already an overview of
  itself, so what exposé should add to it is unsettled:
  - whether exposé should spread the tiles at all, or only pull out the
    floating layer and the windows a tabbed or stacked container hides;
  - what becomes of the containers while exposé rearranges or mirrors the
    windows it shows;
  - whether selecting a window in exposé should move it in the tree as well as
    focus it;
  - how a workspace-selector preview should render a tree.
