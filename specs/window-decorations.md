# Window Decorations

**Status:** stable
**Related specs:** [pointer-input-focus](pointer-input-focus.md), [window-focus-navigation](window-focus-navigation.md), [otto-kit-window-focus](otto-kit-window-focus.md)

## Summary

Otto draws window title bars itself. Clients that negotiate a decoration mode
are told *server-side* unless they explicitly ask to draw their own, and Otto
speaks both decoration protocols in use today so that toolkits which only know
one of them can still be decorated.

## Goals

- A client that negotiates decorations, over either protocol, and expresses no
  preference is decorated by Otto.
- A client that explicitly requests client-side decorations gets them, and Otto
  draws nothing above it.
- The title bar is compositor-owned: dragging it moves the window and its
  controls act on the window regardless of what the client is doing.
- A window Otto decorates can be resized by dragging its edges, without any
  cooperation from the client.
- A window's layout geometry accounts for the title bar when it has one, and
  reclaims that space when it loses it.

## Non-Goals

- Forcing decorations onto clients that asked for client-side ones.
- Decorating clients that never negotiate at all — GTK binds no decoration
  protocol by default, and such windows are left to draw themselves.
- Per-window user override of the negotiated mode (`ToggleDecorations` is a
  developer aid, not a user-facing setting).

## Behavior

**Protocols.** Otto advertises both `zxdg_decoration_manager_v1` and
`org_kde_kwin_server_decoration`. The KDE manager announces `Server` as its
default mode on bind. Which protocol a client picks is its business; both lead
to the same window state.

**Negotiation.**

- A client creates a decoration object without stating a mode → Otto answers
  *server-side* and draws a title bar.
- A client requests a mode → Otto honours it, acknowledges it back to the
  client, and shows or hides its title bar to match.
- A client unsets its mode, or releases its decoration object → Otto's own
  preference applies again, which is *server-side*.
- A client that never creates a decoration object is never decorated by Otto.

**Ordering.** A decoration mode may be negotiated before the surface has an
`xdg_toplevel` — the KDE protocol addresses a bare `wl_surface`, and GTK
requests its mode one message ahead of `get_toplevel`. A mode that arrives with
no window to apply it to must be remembered and applied when that window
appears, and discarded if the surface dies first.

**The title bar.** When a window is server-decorated:

- It carries a title bar strip above the client surface, showing the window
  title and the close and minimize controls. The third control, zoom, is drawn
  only where `show_maximize_button` is on — it is off by default, because a
  double click on the bar zooms the window either way. With it off the group is
  narrower by one dot and one gap, and the point the dot used to occupy hits
  nothing. The setting travels the same way `window_controls_side` does, so it
  applies to a client-drawn otto-kit title bar as well, and changing it takes a
  restart.
- The controls sit at the leading end of the bar by default. The
  `window_controls_side` setting moves them to the trailing end, and they swap
  order when it does, so close stays the outermost one. The screencast
  badge always takes the other end. The setting is read at startup and
  published to the components in the environment, so it applies to a
  client-drawn otto-kit title bar as well as to a server-drawn one, and
  changing it takes a restart.
- The window frame's corners are rounded unless `rounded_corners` is off, in
  which case the bar and the frame square off — as they already do while the
  window is maximized or fullscreen.
- The strip is hit-tested ahead of the client's own surfaces. A press on the
  bar away from the controls starts a window move; a press on a control acts on
  release, as a button does.
- Two presses on the bar away from the controls, within 400 ms and 6 px of each
  other, zoom the window instead: maximized windows are restored, others are
  maximized, and that press starts no move.
- A maximized or tiled window is restored *into* the drag — when the pointer
  has travelled 6 px, not when the button goes down. Restoring at the press
  would make a bare click unmaximize the window, and would spend the first
  press of the double click above: the client is told it is no longer
  maximized, so the second press asks to maximize a window that already
  looks maximized and nothing appears to happen.
- The window's geometry in the layout includes the strip, so mapping,
  maximizing and tiling all account for its height. A resize configures the
  client with the size *under* the bar, so a window does not gain the bar's
  height every time it is dragged.
- The bar is 34 logical points tall, and on a fractional scale that is not a
  whole number of physical pixels (34 x 1.75 = 59.5). The painted bar and the
  offset the client's content starts at are both rounded onto the pixel grid,
  and rounded from the same value, so the bar's bottom edge is a crisp hairline
  and the client's surfaces still begin exactly where it ends. See
  [rendering.md](../docs/developer/rendering.md#sizes-not-just-origins).

**Resize borders.** A server-decorated client has no frame of its own to grab,
and never asks the compositor to resize it — so Otto offers the border itself:

- A strip along the window's own edges, a few points wide, is hit-tested ahead
  of both the titlebar and the client's surfaces. The corners take both of
  their edges, so a press in a top corner resizes rather than moves.
- The pointer over the strip shows the resize cursor for the edge it would
  grab, and a press there starts a compositor resize grab.
- Maximized, fullscreen and minimized windows have no border: there is no free
  size to drag. Neither do windows too small to have an interior left.
- Clients that draw their own decoration keep their own affordances and get no
  border from Otto.

**Zoom.** Maximizing and restoring animate the window's size, but the
`maximized` state itself is sent on the *first* configure of that animation,
not the last: a client that draws its own decoration decides from that state
whether its own zoom control maximizes or restores, and a client that never
hears it asks to be maximized over and over. A window that is already
maximized ignores a further maximize request — honouring it would overwrite
the geometry it has to restore to with the maximized one, and unmaximizing
would then appear to do nothing. A window that asked to be maximized before it
ever had a size of its own restores to a default rect (two thirds of the
usable zone, centred) rather than to an empty one.

**Popups across a geometry change.** A popup is placed once, against the
parent window as it stood when the popup mapped. Moving or resizing the window
afterwards moves that parent out from under it, and the popup — which keeps its
offset inside the parent — would ride off the screen edge. Every change to a
window's geometry re-runs the unconstrain pass over its popups and configures
the clients with the corrected geometry, so an open menu stays on the output its
window landed on: maximizing and unmaximizing, tiling and untiling, going
fullscreen and back, dragging a tiled window loose, and the interactive move and
resize grabs.

The two grabs do it when the button is released, not on every motion event: the
pass configures every popup the window owns, and a drag is many events. Popups
follow their window on screen throughout regardless, since their position is
derived from the window's each frame — what the pass corrects is a popup that
has ended up over an edge.

A client whose positioner is not reactive cannot be reconfigured and keeps the
placement it committed; repositioning it is its own to request.

**Fixed-size windows.** A client that pins its minimum and maximum size to the
same non-zero value — Files' Get Info panel, say — is asking for one size and
no other, so it has no maximized and no tiled form: a maximize or tile request
is ignored, and the window keeps the size it asked for. Its zoom control is
drawn gray, reveals no glyph on hover and does not light up under a press, the
way a control the window does not support is drawn anywhere else. Honouring the
request instead would configure the client to a size it will not draw, leaving
its fixed layout stranded in the corner of an empty surface.

**Client-drawn title bars.** A client that decorates itself owns these
gestures too, so otto-kit offers them from `Window`: a press on the title bar's
drag area starts a compositor move, and two such presses within 400 ms and 6 px
of each other maximize the window, or restore it if it already is — the same
thresholds the server-side bar uses. Otto's own applications (Files, Settings)
go through it, so a client-decorated window answers a double click on its bar
the way a server-decorated one does.

**Activation.** A client that draws its own decoration needs to know when it
has the focus, so every mapped toplevel's `activated` state follows the
keyboard: the focused window has it, every other window on every workspace and
every output does not, and the configure carrying the change is sent to the
windows losing it as much as to the one gaining it. Keyboard focus on a *popup*
counts as focus on the window that put the popup up — a window must not dim
itself the moment one of its own menus opens. What a client *does* with the
state is its own business; for otto-kit's own windows, see
[otto-kit-window-focus](otto-kit-window-focus.md).

**Toggling.** The `ToggleDecorations` action flips the mode of every window
that has negotiated one, and windows that have not are left alone.

## Constraints & Edge Cases

- Answering a mode is not the same as the client obeying it: a client may
  ignore the mode event and keep drawing its own title bar. Otto must not
  interpret its own answer as proof of the client's behavior.
- Acknowledging a requested mode before acting on it matters — clients wait for
  the mode event before deciding whether to draw their own bar.
- Showing or hiding the bar changes window geometry, so it must re-run layout,
  not just repaint.
- The compositor-owned strip must not steal input from layer-shell surfaces or
  the dock stacked above the window.

## Rationale

Server-side is the default because a desktop where every application invents
its own title bar has no consistent window controls and no consistent way to
grab a window. Clients that insist on client-side decorations are honoured
anyway: fighting a toolkit that draws its own bar produces two title bars, which
is worse than either choice alone.

Both protocols are implemented because neither one covers the field.
`xdg-decoration` is the standard, and Qt/KDE clients use it; GTK never binds it
at all, and the GTK applications that do offer a server-side mode look only for
`org_kde_kwin_server_decoration`. Supporting only the modern protocol would
leave those applications permanently client-decorated no matter what their
configuration said.

## Open Questions

- Whether the default should be configurable per-app, for clients whose
  self-drawn decorations are a poor fit for the desktop.
