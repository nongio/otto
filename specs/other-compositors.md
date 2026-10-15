# Otto Apps on Other Compositors

**Status:** draft
**Related specs:** [file-browser.md](./file-browser.md), [peek.md](./peek.md), [file-command-palette.md](./file-command-palette.md), [surface-output-placement.md](./surface-output-placement.md), [background-effect.md](./background-effect.md)

## Summary

Otto's applications, the file browser first, run on other Wayland desktops as
well as on Otto. Where Otto's surface style protocol is missing they lose
decoration, never function: everything still shows where it belongs, clipped
where it should be, and goes away when it is closed.

## Goals

- KDE Plasma (KWin) is the floor. Every feature of the file browser works
  there: list, grid and column views, scrolling, the command palette, Peek and
  the desk.
- On any compositor, a surface an application closes or hides leaves the
  screen, and nothing an application draws reaches outside the area that
  should show it.
- The path taken without the surface style is the path taken on Otto wherever
  the two can be the same, so it is exercised every day rather than only on
  other desktops.

## Non-Goals

- GNOME (Mutter). It has no layer shell, so the desk and Peek cannot run
  there; the browser starts, without them. No special handling is added.
- Blur and frost without the surface style. Materials are solid where the
  compositor does not blur them; the standard background effect protocol is
  used where it exists, and nothing more is done for it.
- Animations the compositor runs on Otto (fades, springs) are not replayed by
  the client elsewhere. Changes land at once.
- Telling assistive technologies where a window is on screen. A Wayland
  client cannot learn that without Otto's protocol; positions stay window-local.

## Behavior

### Tiers

- **Otto.** Everything, with the compositor painting rounded corners,
  shadows, frost and fades, and placing panels against the display.
- **KDE floor.** Requires the core protocols plus `wp_viewporter`,
  `wlr-layer-shell` and `xdg-decoration`. Every feature works; corners are
  square where the compositor would have rounded them, panels have no
  shadow, materials are solid, and changes land without animation.
- **Below the floor.** Whatever starts, starts; nothing is special-cased.

### Scrolling and clipping

- A scrolling list, grid or column shows exactly its viewport. Rows never
  paint over a header, a path bar, a neighbouring column or the desktop
  outside the window.
- A column panned partly out of the file area shows only its visible part; a
  column panned wholly out shows nothing.
- Scrolling moves the content smoothly at the display's rate on every
  compositor, with no repaint per step.
- The scrollbar follows the scroll, and disappears when it fades out.

### Showing and hiding

- Closing the command palette removes its card and its rows from the screen.
- Leaving column view removes every column, divider, status line and the
  docked preview.
- A hidden panel takes no pointer input.

### Placement

- Peek's panel appears centred over the window that opened it. On Otto it is
  centred on the display.
- Moving or resizing a panel takes effect on the next frame, without waiting
  for anything else in the window to change.

## Constraints & Edge Cases

- No compositor clips a subsurface to its parent, so the application clips
  every surface it places itself.
- A crop sent with a buffer it does not fit is a protocol error that
  disconnects the application: every crop must go out with, or after, the
  buffer it was worked out for.
- A subsurface's position belongs to its parent and lands with the parent's
  commit; a move that commits only the child does not move it.
- A surface hidden by opacity alone stays on screen wherever the surface
  style is missing; hiding is unmapping.
- A surface that comes back after being unmapped has no buffer and must be
  painted again.
- Panes nested in a container are clipped to what the container shows by the
  host, which hands each one that rect.

## Rationale

- **KDE as the floor.** A fixed floor keeps degradation to two tiers instead
  of a branch per compositor. KWin has the layer shell the desk and Peek need,
  and the viewporter the clipping needs.
- **One clipping path for every compositor.** Scrolling was already driven a
  frame at a time by the client, so a viewport crop costs Otto nothing and
  keeps the other-desktop path in daily use. The surface style remains for
  what a crop cannot do: rounded clips, shadows, frost, output placement and
  compositor-run fades.
- **No blur fallback.** Solid materials read correctly; a blur emulation would
  be scope with little return.

## Open Questions

- Server-side decorations forced by a compositor (tiling window managers)
  would double the title bar. Reading the negotiated decoration mode is not
  done yet.
- Rounded window corners and a client-drawn shadow for the browser window
  itself on the floor tier.
- The desk on KDE has nothing to start it; Otto's compositor starts it on
  Otto.
