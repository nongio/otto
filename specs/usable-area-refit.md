# Usable Area Refit

**Status:** stable  
**Related specs:** [tiling](tiling.md), [topbar](topbar.md), [surface-output-placement](surface-output-placement.md)

## Summary

A maximized or half-snapped window fills a share of the output's usable area:
the output minus the dock and any layer-shell exclusive zones. When that area
changes, those windows move and resize to fill it again.

## Goals

- Moving the dock to another edge, resizing it, or turning autohide on or off
  refits every maximized and half-snapped window.
- A layer-shell surface that claims, changes or releases an exclusive zone
  refits the maximized and half-snapped windows on its output.
- A refit keeps the window's zone: a maximized window stays maximized, a left
  half stays the left half.

## Non-Goals

- Floating windows are not moved, even when the dock now covers part of them.
- Fullscreen windows are not affected; they ignore the usable area.
- Tiling workspaces follow their own rules (see [tiling](tiling.md),
  *Usable area changes*), though they are laid out again on the same triggers.

## Behavior

- The trigger is a change to the reserved area, not to a particular setting.
  Whatever changes the band the dock reserves (its edge, its thickness, whether
  it autohides) or an exclusive zone counts, whether it came from Settings, a
  dock drag, a config reload or a panel.
- Dock changes are applied once the dock has come to rest. The dock animates to
  its new shape, and windows sized to a band it is only passing through would
  have to move twice.
- A layer-shell change is applied as soon as the surface commits it.
- Each refitted window animates to its new rectangle, the same way maximize
  does, and is configured with its new size.
- Windows on workspaces that are not showing are refitted too, in place: they
  stay on their workspace and keep their place in the stack.
- A window that already fills its zone is left alone.

## Constraints & Edge Cases

- The rectangle a window returns to on unmaximize or untile is never touched by
  a refit.
- Popups of a refitted window are repositioned against its new rectangle.
- A layer surface torn down with its output does not trigger a refit on that
  output.
- The dock is only drawn on the primary output, so only that output's windows
  make room for it.

## Rationale

- Keying on the reserved area rather than on each dock setting keeps one path
  for every source of change, including ones added later.
- Waiting for the dock to rest avoids a double move, at the cost of windows
  following a fraction of a second after the dock.

## Open Questions

None.
