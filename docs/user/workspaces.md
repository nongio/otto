# Workspaces

A workspace is a full-screen page of windows. Otto arranges them in a
horizontal row and slides between them.

## Per-monitor workspaces

**Each monitor has its own independent set of workspaces.** Adding a workspace
on your laptop screen does not add one on the external monitor; switching to
workspace 3 on one screen leaves the other where it is; and the two can have
different numbers of workspaces.

This applies to virtual outputs too — a screenshare or RDP output has its own
workspaces exactly like a physical monitor.

Every monitor always has at least one workspace.

## Switching

| How | What it does |
|-----|--------------|
| `Ctrl+1` … `Ctrl+4` | Jump to workspace 1–4 (the shipped bindings) |
| Three-finger horizontal swipe | Slide between adjacent workspaces |
| Click a preview in the workspace selector | Jump to that workspace |

Keyboard and gesture switching both act on the **focused monitor** — the one
your pointer was last over.

More than four workspaces? Add bindings with a higher `index`:

```toml
"Ctrl+5" = { builtin = "Workspace", index = 4 }
"Ctrl+6" = { builtin = "Workspace", index = 5 }
```

The index is zero-based, so `index = 4` is the fifth workspace. Switching to a
workspace that does not exist does nothing.

### Swipe behaviour

The desktop tracks your fingers as you swipe, so you can see the neighbouring
workspace arrive and reverse out of it. Otto snaps on release based on both
position and velocity — a fast flick advances even from a short movement.

At the first and last workspace the slide meets rubber-band resistance rather
than a hard stop.

## The workspace selector

Open [Exposé](expose-and-switcher.md) (`PageUp`, or three-finger swipe up) and a
strip of live workspace previews appears above the window grid. Each monitor
shows its own strip, listing only its own workspaces.

| Control | Effect |
|---------|--------|
| Click a preview | Switch that monitor to that workspace |
| Click `+` at the end of the strip | Add a workspace to that monitor |
| Hover a preview, click the `×` | Remove that workspace from that monitor |

The previews are live: they show the actual current content of each workspace,
at that monitor's own size and scale, not a stale screenshot.

Adding and removing are animated — a new preview grows in from zero width, a
removed one shrinks away before the workspace actually goes.

### Removing a workspace

- A monitor's last remaining workspace cannot be removed.
- Windows on the removed workspace are **not** closed — they move to that
  monitor's current workspace.
- A workspace holding a fullscreen window with content in it cannot be removed.

## Moving windows between workspaces

Open exposé, then **drag a window preview onto a workspace thumbnail** in the
selector strip. Release, and the window moves to that workspace; the grid
relaws out around the gap it left.

Dropping outside any thumbnail cancels — the preview springs back to where it
came from.

## Moving windows between monitors

Drag a window across the boundary between two monitors and it moves to the
other one, keeping its own workspaces and geometry. While it straddles the
edge only one monitor draws it at a time; a live preview on both is planned but
not implemented.

## Fullscreen and workspaces

Fullscreening a window puts it on a workspace of its own, on the monitor it
already lived on, and scrolls only that monitor to it. Other monitors do not
move. Leaving fullscreen restores the window to its previous size and workspace
and removes the temporary one.

## What is shared and what is not

| Element | Scope |
|---------|-------|
| Workspaces | Per monitor |
| Workspace selector | Per monitor, one strip each |
| Exposé grid | Per monitor, all open together |
| [Dock](dock.md) | Primary monitor only |
| [Top bar](topbar.md) | Primary monitor only |
| [App switcher](expose-and-switcher.md) | One panel; moves to the monitor under the pointer |

## Configuring

Workspaces are not configured in TOML — there is no "number of workspaces"
setting. They are created and removed at runtime through the selector, and each
monitor starts with one.

Wallpaper and background colour are set globally under
[Theming](theming.md).

## Not yet supported

- Naming workspaces
- Dragging a workspace from one monitor to another
- Assigning applications to a workspace by rule
- Persisting the workspace layout across restarts
