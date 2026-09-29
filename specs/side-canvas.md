# Side Canvas

**Status:** draft
**Related specs:** [multi-output](./multi-output.md), [lock-screen](./lock-screen.md),
[plane-scanout](./plane-scanout.md), [settings-app](./settings-app.md)

## Summary

The side canvas is a column that slides in over the desktop from the right
edge of an output. Otto owns the column; applications place surfaces in it
through the `otto-canvas-v1` protocol, and Otto stacks them top to bottom at
the column's width. It is a place for small, glanceable client content that is
one swipe away and out of the way the rest of the time.

## Goals

- A client can put any number of surfaces ("items") in the canvas, and remove
  them, without knowing where the canvas is or whether it is on screen.
- Items stack top to bottom in the order they were created, each at the full
  column width, each as tall as its client's buffer.
- The canvas follows the fingers 1:1 during a swipe from the right edge of the
  touchpad, and settles open or closed with a spring when they lift.
- While the canvas is off screen its items cost nothing: no frame callbacks,
  and a `hidden` event so they can stop work.
- The canvas can also be toggled from the keyboard, and hidden with Escape or
  a click outside it.

## Non-Goals

- **Pushing the desktop aside.** The canvas slides over windows; nothing
  underneath moves or is resized.
- **Scrolling.** Items that do not fit are clipped (see Open Questions).
- **Choosing the order.** Creation order is the only order in this version.
- **Per-item widths or client-chosen positions.** The compositor owns layout.
- **Content.** Otto ships `otto-canvas`, a sample client with one empty
  panel; what belongs in the canvas is up to clients.

## Behavior

### Protocol

- `otto_canvas_manager_v1.get_canvas_item(id, surface)` gives `surface` the
  canvas item role and adds it at the bottom of the column. A surface that
  already has another role is a `role` protocol error.
- The compositor answers every new item with `configure(serial, width)`,
  followed by `shown` or `hidden` according to the canvas's current state.
  `width` is the configured column width in logical points.
- The client acknowledges with `ack_configure` and draws at that width, at a
  height of its choosing; the height is read from the buffer it attaches.
- When the configured width changes, every item receives a new `configure`.
- `dismiss` hides the canvas if it is shown; it is ignored otherwise.
- Destroying the item (or disconnecting) removes it; the items below move up.
  When the last item goes, a visible canvas is taken down at once.

### Showing and hiding

- The canvas can only be shown while at least one item exists. Asking for it
  otherwise does nothing.
- It opens on the output under the pointer and stays on that output until it
  is hidden again.
- Shown, the column's right edge sits `margin` points inside the right edge of
  the usable area (the output minus panels and the dock), its top `margin`
  points below the top of the usable area, and it reaches down to `margin`
  points above the bottom of it. Hidden, it is entirely past the right edge
  of the output.
- Items receive `shown` as soon as the canvas starts coming on screen (the
  first frame of a swipe or a toggle), and `hidden` only once it has finished
  sliding fully off screen.
- Frame callbacks are sent to items only while some part of the canvas may be
  on screen.

### Gesture

- A swipe starts from where the canvas is: at the edge if it was hidden, or at
  its current animated position if it was moving. Updates move it by exactly
  the finger travel, converted to physical pixels at the output's scale, and
  it never goes past fully shown or fully hidden.
- On release, a flick faster than 300 points per second goes the way it was
  moving; otherwise the canvas opens if it is at least half out. A cancelled
  gesture returns to where it started.
- Settling uses the same spring as a workspace swipe.

### Input

- The canvas sits above every window, fullscreen ones included, the dock and
  the layer-shell chrome, and below popups, the app switcher and the lock
  screen.
- Pointer events over an item go to that item. Hovering beside the canvas
  still reaches the windows there.
- A press on an item gives it the keyboard. When the canvas hides, the
  keyboard goes back to whoever had it before an item took it.
- A press outside the column while the canvas is shown hides the canvas and
  is consumed, release included: nothing underneath sees the click. A press
  on the column between items is consumed without hiding.
- Escape hides a shown canvas; the key does not reach the focused client.
- A two-finger scroll over the shown canvas goes to the item under the
  pointer instead of being taken as a dismiss swipe.
- The `CanvasToggle` action shows or hides the canvas. It is not bound by
  default.

### Configuration

`[canvas]` in the config file, all in logical points:

| Key | Default | Meaning |
|-----|---------|---------|
| `width` | 400 | Column width, clamped to 200–1200. Applies live. |
| `margin` | 12 | Space between the column and the edges of the usable area. |
| `gap` | 12 | Space between two items. |

`canvas.width` is in the settings schema as a live setting; `margin` and `gap`
are read the next time the column is laid out.

## Constraints & Edge Cases

- The canvas is drawn in the output's overlay plane, so an output showing the
  canvas keeps that plane up, and a fullscreen window is not scanned out
  directly while the canvas is on screen.
- Items may ask for a background blur. While the canvas is on screen the blur
  backdrop is kept current across the whole output rather than only behind
  the layer-shell chrome.
- A client that commits a buffer before acknowledging the first configure is
  laid out anyway, at its buffer's size.
- Locking the session does not hide the canvas; the lock screen covers it.

## Rationale

- **Compositor-owned layout.** Clients describe content, not placement, so the
  canvas can move between outputs, change width and stack items without any
  client having to cooperate.
- **An overlay, not a strip that pushes workspaces.** Pushing the desktop would
  move or resize every window on each swipe, which is costly and disorienting
  for something meant to be glanced at.
- **`shown` on the first frame, `hidden` on the last.** A client has to draw
  before it is seen, and may stop only once it cannot be.
- **No default shortcut.** Every obvious candidate collides with common
  application bindings; the gesture is the primary way in.

## Open Questions

- Overflow: items that do not fit the usable height are clipped. The column
  should scroll, probably with the two-finger scroll that already passes
  through to items.
- Ordering: should clients be able to ask for a position, or the user reorder
  items by dragging?
- Should the canvas remember the output it was last shown on?
- The canvas does not follow its output being unplugged while shown; it is
  cleaned up the next time it hides.
