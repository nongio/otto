# Window Tiling

**Status:** draft
**Related specs:** [window-decorations](window-decorations.md), [workspaces-multi-output](workspaces-multi-output.md), [window-focus-navigation](window-focus-navigation.md), [context-menus](context-menus.md)

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

## Non-Goals

- Tiling as the default mode. Otto is a floating desktop; a workspace tiles
  because the user asked it to, or because the configuration set the default.
- A layout that overlaps windows, or a "manual" mode where the user positions
  tiles by absolute coordinates. Overlapping windows are what floating mode is.
- Reproducing any specific existing tiler's keybindings verbatim.
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

**Promotion.** A command lifts the focused window out of its container and makes
it a sibling of that container's parent, and the reverse command pushes it back
down into the neighbouring container. Together these reshape nesting without
closing anything.

### Resizing

**Pointer.** The gap between two tiles is a resize handle. Dragging it changes
the shares of the two children on either side of that split and nothing else;
the pointer shows a horizontal or vertical resize cursor over it. Where four
tiles meet at a corner, the corner handle resizes both splits at once. A drag
that would push a window below its minimum size stops there.

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

**Fluid by default.** During an animation a window's *drawn* rectangle follows
the animation curve continuously, whether or not the client has caught up. The
client is asked once for the final size, and until its new buffer arrives its
last frame is drawn scaled into the animated rectangle. When the new buffer
lands the window is drawn at its true resolution again. The user sees a
continuous resize; the client sees one configure.

**Configurable.** The animation has a configurable duration and curve, and can
be reduced to *snap*, where windows are placed at their new cells immediately
with no motion — for a user who wants the tiler to feel instantaneous, and for
the accessibility reduced-motion setting, which forces it regardless of the
tiling configuration. A third setting drives the client's configure size on
every animation frame instead of scaling its last frame: the most faithful
resize, at the cost of reconfiguring every affected client at frame rate, and
therefore not the default for a layout that can move a dozen windows at once.

**Interactive resize is live, not animated.** Dragging a gap resizes
continuously under the pointer, with the affected clients reconfigured as they
acknowledge the previous size rather than on a fixed cadence. There is no
animation to run: the pointer is the animation. On release the layout settles
with no further motion.

**Dragging a window** animates the tree closing behind the detached window and
opening again to accept it, on the same curve.

### Decorations

A tiled window keeps the titlebar Otto draws — it is the move handle, and the
route to the window's controls and menu. In a tiled workspace the bar is drawn
in a compact variant: shorter than the floating one, with the same controls and
title. The window frame keeps its rounded corners and its gap-borne separation
from its neighbours when gaps are on; with gaps off the frame squares off, as it
already does when maximized.

The focused tile is marked with a border in the accent colour. Drop shadows are
not drawn on tiles — nothing overlaps, so there is nothing to cast onto — and
return when the window floats again.

A window that draws its own decorations is tiled the same way and is not given a
bar; it is still moved by dragging whatever it treats as its titlebar, and the
keyboard commands reach it unchanged.

### What clients are told

A tiled window is configured with the exact size of its cell, minus whatever the
titlebar takes. It is told it is tiled on each edge that touches another tile or
the edge of the usable area, so a client can square off the corresponding
corners. It is told it is maximized only when it is alone in the workspace with
gaps off, which is the one case where its rectangle really is the whole usable
area. A window in monocle is told it is maximized.

Windows managed through XWayland tile identically, are configured with the same
rectangles, and carry the same states where the X11 equivalent exists.

### Overview and previews

Exposé, the workspace selector previews, and the app switcher work on a tiled
workspace exactly as on a floating one — a tiled workspace's windows are simply
already spread out. Entering exposé does not disturb the tree, and selecting a
window from it focuses that window in place.

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

**Why the drawn rectangle leads the client.** A layout change can move a dozen
windows at once. Driving every one of their configures at frame rate turns one
keystroke into a resize storm across a dozen clients, and the slowest of them
decides how the animation looks. Scaling the last frame into the animated
rectangle keeps the motion the compositor's business and the size the client's,
which is the only way tiling animations stay smooth with a screenful of windows.

**Why the layout is not expressed as engine layout nodes.** The scene engine can
lay out containers and animate them itself, and a split container maps neatly
onto a flex row. But the compositor needs each window's rectangle *now* — to
configure the client, to map it in the window space, to hit-test input, to
compute exclusive zones and scanout eligibility — and taking it from the engine
would mean reading geometry back out of the render layer on the input path. The
tree is therefore resolved to rectangles by the compositor, and those rectangles
drive the engine's layers and its transitions. The engine animates; it does not
decide.

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

- **Tabbed and stacked containers.** A container whose children occupy the same
  cell, selected by a tab strip, is the natural next thing to want, and Otto's
  compositor-drawn titlebar is well placed to draw the strip. Not specified here;
  the tree must be able to grow a per-container layout kind without a rewrite.
- **Persistence across sessions.** Whether a tiled workspace's tree, and the
  mode itself, should be restored on the next login.
- **Named layouts.** Whether the user should be able to save a tree and apply it
  to a workspace, and whether an application should be able to ask for a slot.
- **Whether the tiling mode belongs in the workspace selector UI** as a visible
  per-workspace indicator, rather than only in a menu and a shortcut.
