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
  title and the close, minimize and maximize controls.
- The strip is hit-tested ahead of the client's own surfaces. A press on the
  bar away from the controls starts a window move; a press on a control acts on
  release, as a button does.
- The window's geometry in the layout includes the strip, so mapping,
  maximizing and tiling all account for its height.

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
