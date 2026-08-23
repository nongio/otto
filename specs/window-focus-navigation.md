# Window Focus Navigation

**Status:** draft  
**Related specs:** pointer-input-focus, workspaces-multi-output

## Summary

Defines which window becomes focused when the user navigates between windows
without pointing at them: closing the focused window, committing the app
switcher, and cycling through the windows of one application. These paths must
work when an application's windows are spread across several workspaces.

## Goals

- Focus never disappears silently: closing the focused window hands focus on.
- Navigating to an application lands on the window of that application the user
  used last, wherever it lives.
- Cycling the windows of an application visits every one of its windows in turn,
  including those on other workspaces.
- Whenever the window that gains focus is on another workspace, the owning
  output scrolls to that workspace so the window is actually visible.

## Non-Goals

- Pointer-driven focus (see pointer-input-focus).
- Focus policy on workspace switch, expose close, or window minimise/unminimise.
- Ordering of the *applications* inside expose or the dock.

## Behavior

### Focus order

- Otto tracks the order in which windows were last focused, across all
  workspaces and outputs. A window becoming focused — including a newly mapped
  one — makes it the most recent; a window that closes leaves the order.

### Closing a window

- When the focused window closes, focus moves to the topmost non-minimised
  window of the focused output's current workspace. Activated state, the
  window's active styling and keyboard focus all move together.
- If that workspace has no such window, keyboard focus is cleared.

### Switching to an application

- Committing the app switcher on an application, or clicking its dock icon,
  raises that application's windows and focuses the one it most recently
  focused, even when that window is on another workspace — the owning output
  then scrolls to it.
- The app switcher lists applications most-recently-used first, ordered by the
  focus order above, so a single next-and-release flips between the last two
  applications regardless of which workspaces their windows are on.

### Cycling the windows of an application

- The cycle-windows action steps through the focused application's non-minimised
  windows in a stable order — by output, then workspace, then stacking position
  within that workspace — starting from the window currently focused, and wraps
  around at either end.
- Each step raises the window it lands on, focuses it, and scrolls its output to
  its workspace.
- The order does not depend on which workspace is current, so repeated steps
  visit every window of the application exactly once per lap.

## Constraints & Edge Cases

- Whether the closed window held focus must be read *before* it is unmapped:
  afterwards the focus target is already gone. Focus held by a layer surface or
  popup at that moment is left alone, and a locked session never has focus moved.
- Per-workspace stacking order cannot answer "which window of this app did I use
  last?" — every workspace has its own topmost window — so the focus order is
  tracked globally rather than derived from stacking.
- Minimised windows are skipped by cycling and by the app-switcher target. An
  application whose only window is minimised is unminimised instead.
- A fullscreen self-managing X11 window keeps focus across these paths (see
  the focus-out rationale in the XWayland handling).

## Rationale

- Selecting a target by position in the workspace model was wrong for
  applications with windows on several workspaces: that model lists the current
  workspace's windows last, so the target always resolved to the window already
  on screen — the switcher appeared to do nothing and cycling kept returning to
  the same window.
- Cycling uses a stable geometric order rather than the recency order so that
  repeated presses walk through all windows instead of oscillating between the
  two most recent ones; the app switcher uses recency because it is a
  "go back to what I was doing" gesture.

## Open Questions

- Whether closing a window should instead restore focus to the most recently
  focused *surviving* window, which may be on another workspace, rather than to
  the top of the current workspace.
