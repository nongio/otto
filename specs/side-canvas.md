# Side Canvas

**Status:** draft
**Related specs:** [multi-output](./multi-output.md), [lock-screen](./lock-screen.md),
[plane-scanout](./plane-scanout.md), [settings-app](./settings-app.md),
[stash](./stash.md)

## Summary

The side canvas is a column that slides in over the desktop from the right
edge of an output. Otto owns the column; applications place surfaces in it
through the `otto-canvas-v1` protocol, and Otto stacks them top to bottom at
the column's width. It is a place for small, glanceable client content that is
one swipe away and out of the way the rest of the time.

## Goals

- A client can put any number of surfaces ("items") in the canvas, and remove
  them, without knowing where the canvas is or whether it is on screen.
- Items stack top to bottom by the order their clients set, and in the order
  they were created where that is the same, each at the full column width,
  each as tall as its client's buffer.
- The canvas follows the fingers 1:1 during a swipe from the right edge of the
  touchpad, and settles open or closed with a spring when they lift.
- While the canvas is off screen its items cost nothing: no frame callbacks,
  and a `hidden` event so they can stop work.
- The canvas can also be toggled from the keyboard, and hidden with Escape or
  a click outside it.
- A client can bring the canvas out when it has something new to show,
  without taking the keyboard from the app the user is working in.
- Something dragged from any app can be dropped on an item: a drag resting at
  the right edge opens the canvas, and a drag started beside an open canvas
  does not close it.

## Non-Goals

- **Pushing the desktop aside.** The canvas slides over windows; nothing
  underneath moves or is resized.
- **Scrolling the column.** Items share its height and scroll within
  themselves; the column itself does not scroll (see Open Questions).
- **User-chosen order.** Clients set the order; the user cannot drag items
  around.
- **Per-item widths or client-chosen positions.** The compositor owns layout.
- **Content beyond Sessions.** Otto ships `otto-canvas`, which shows the
  agent sessions (see below); anything else in the canvas is up to clients.

## Behavior

### Protocol

- `otto_canvas_manager_v1.get_canvas_item(id, surface)` gives `surface` the
  canvas item role and adds it to the column with order 0, below every item
  with the same or a lower order. A surface that
  already has another role is a `role` protocol error.
- The compositor answers every new item with `configure(serial, width)`,
  then, from version 5, `max_height(height)`, then `shown` or `hidden`
  according to the canvas's current state. `width` is the configured column
  width in logical points.
- The client acknowledges with `ack_configure` and draws at that width, at a
  height of its choosing; the height is read from the buffer it attaches.
- When the configured width changes, every item receives a new `configure`.
- `dismiss` hides the canvas if it is shown; it is ignored otherwise.
- `set_keyboard_interactivity(none | on_show | never)` (version 2; `never`
  version 3) says when the item takes the keyboard. `none`, the default: when
  pressed on. `on_show`: also when the canvas is shown. `never`: not at all,
  not even when pressed on; the press still reaches the item as a pointer
  event. A value outside the enum, or `never` on a version 2 object, is an
  `invalid_keyboard_interactivity` protocol error.
- `show` (version 3) asks for the canvas to be shown; see Showing and hiding.
- `set_order(order)` (version 3) sets the item's place: lower sits higher,
  ties go by creation, the default is 0. The column is laid out again at
  once, and the first item "in the column" (for `on_show`) follows the new
  order.
- `drag_mime_type(mime_type)`, `drag_started` and `drag_ended` (version 4,
  on the manager) tell every client when a drag and drop operation starts
  and ends anywhere in the session, from any client. Each `drag_mime_type`
  names a type the drag's data is offered as, all of them right before
  `drag_started`; none means the source has not said, so unknown. A manager
  bound while a drag goes on is told about it at once. `drag_ended` follows
  the drop (after `wl_data_device.drop` for the client whose surface took
  it) or the cancel. See Drag and drop.
- `set_content_height(height)` (version 5, on the item) says how tall the
  item's content is in logical points, uncapped. `max_height(height)`
  (version 5, event) is the most the item may be tall; the client draws at
  `min(content, max_height)` and scrolls the rest. See Sharing the height.
- The compositor advertises version 5. A client binds
  `min(advertised, 5)` and sends each request only on a version that has it,
  so a new client still runs against an older compositor.
- Destroying the item (or disconnecting) removes it; the items below move up.
  When the last item goes, a visible canvas is taken down at once.

### Sharing the height

- The height items share is the column's on the output it is on, or would
  open on (under the pointer, else the primary output): the usable area
  (below the top bar, above a dock that does not autohide) less the margin
  above and below, in whole logical points. The gaps between items that
  show are taken off first.
- An item bound at version 5 that has sent `set_content_height` asks for its
  content height. Every other item, including one at version 5 that has not
  said yet, counts as fixed at the height of its buffer (zero, and no gap,
  without one).
- Shares are water-filled in stacking order: an item whose content fits an
  even share of what the fixed items leave keeps its content height; the
  rest split what is left equally, round after round until the shares
  settle. A point left over from the division goes to the topmost of the
  items at the level.
- Every item keeps at least 120 pt, or its content when that is less. When
  the fixed items leave less than that for everyone, each item gets exactly
  its minimum and the column overflows (clipped at the bottom). Only a
  column too short to hold the minimums on its own is shared equally.
- An item at version 5 that has not sent a content height is offered what it
  would get if it wanted the whole column. Every share is at least 1 pt.
- `max_height` is sent only when an item's share changes, to version 5
  items only: after the first configure; when an item's content height
  changes; when items come, go, reorder or commit a buffer of a new height;
  when `canvas.width`, `margin` or `gap` change; when the usable area of an
  output changes; and whenever the column comes on screen, before `shown`.
- Items still stack by the height of the buffers they attach, so a client
  that resizes promptly on `max_height` never overflows the column.

### Showing and hiding

- The canvas can only be shown while at least one item exists. Asking for it
  otherwise does nothing.
- An item's `show` opens the canvas as a toggle does, with the same slide,
  output choice and `shown` events, with these differences:
  - It is ignored while the canvas is shown or following the fingers, while
    the session is locked, and during exposé. It is not remembered: the
    client asks again on its next change. A canvas that is sliding out comes
    back.
  - No item is given the keyboard, `on_show` ones included.
  - The canvas opens *passive*, and stays so until the user acts on it: a
    toggle, a swipe, or a press on an item that takes the keyboard. While
    passive, Escape goes to the app with the keyboard.
  - It stays open until the user hides it, or an item sends `dismiss`.
- It opens on the output under the pointer and stays on that output until it
  is hidden again.
- Shown, the column's right edge sits `margin` points inside the right edge of
  the usable area (the output minus panels and the dock), its top `margin`
  points below the top of the usable area, and it reaches down to `margin`
  points above the bottom of it. Hidden, it is entirely past the right edge
  of the output.
- The column clips what its items draw to its area grown by 48 points on
  every side, so an item's drop shadow (set through its surface style) shows
  while overflow is still cut. Hidden, it is that far further past the right
  edge too, so no shadow shows. Clicks count as inside only on the column
  itself, not on the room around it.
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

- The canvas sits above every window, fullscreen ones included, the
  layer-shell chrome and the dock, and below the workspace selector, popups,
  the app switcher and the lock screen.
- Entering exposé fades the canvas out with the overlay layer; leaving it
  fades the canvas back in. While exposé is up the canvas stays open but takes
  no pointer, keyboard (Escape) or edge swipe input.
- Pointer events over an item go to that item. Hovering beside the canvas
  still reaches the windows there.
- A press on an item gives it the keyboard, unless the item set `never`.
  Either way the press does not move the keyboard anywhere else. When the
  canvas hides, the keyboard goes back to whoever had it before an item took
  it.
- When the canvas settles open (a toggle, or a swipe released open), the
  first item in the column that asked for `on_show` gets the keyboard, unless
  an item already has it. Not while the session is locked. The keyboard goes
  back on hide as for a press.
- A press outside the column while the canvas is shown is not consumed: it
  reaches whatever is under the pointer, which is raised and focused as any
  press would. The canvas hides when that button is released outside the
  column, unless a drag and drop operation started while it was down (the
  press picked something up, to drop on an item); released over the column,
  the canvas stays too. This is the same whether the canvas is passive or
  not. A press on the column between items is consumed, release included,
  without hiding.
- While no canvas item has the keyboard and the canvas is not passive,
  Escape hides a shown canvas and the key does not reach the focused client. While an item has it, Escape goes to
  the item, which decides what it means (Sessions clears its field first,
  then dismisses).
- A two-finger swipe to the right hides a shown canvas wherever the pointer
  is, over the canvas included. A vertical scroll over the canvas goes to the
  item under the pointer.
- The `CanvasToggle` action shows or hides the canvas. It is not bound by
  default.

### Drag and drop

- Every drag and drop operation is announced to every manager (version 4)
  with `drag_mime_type` events and `drag_started`, and its end with
  `drag_ended`, whether it was dropped or cancelled.
- While a drag goes on, the pointer resting within the canvas width of the
  right edge of an output for 250 ms opens the canvas there, as an item's `show`
  does: passive, without moving the keyboard, and not while the session is
  locked or during exposé. It opens only if an item exists by then; a client
  may add one on `drag_started` for this. A rest that finds no item keeps
  trying while the pointer stays at the edge. Leaving the edge and coming
  back starts a new rest.
- Items take the drop through `wl_data_device` like any other surface.
- When the drag ends, a canvas that the drag opened hides again, unless the
  drop landed on an item. A canvas that was already open when the drag
  started stays open, wherever the drop lands.

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

## Agents panel (`otto-canvas`)

- `otto-canvas` places one item: a heading ("Agents"), a prompt field ("Ask
  an agent…"), a hairline, and the agent sessions otto-agents has, one row per session, most recently
  changed first.
  The rows are the launcher's agents-mode rows (`otto-launcher --agents`):
  title (or "Untitled session"), `@agent · status · folder`, and the activity
  dot.
- The item wears the frosted popup material with the desktop corner radius
  and hairline border, drawn by the compositor through `otto-surface-style`.
- Its content height is the heading, the field and one row per session,
  sent with `set_content_height`. It is drawn at the smaller of that and its
  `max_height` (480 pt below version 5); past that the rows scroll (wheel,
  touchpad with momentum, scrollbar drag). The surface is resized whenever
  the row count or the share changes the height. At version 5 it does not
  draw until the first `max_height` has arrived.
- On `shown` it connects to the agent service and lists the sessions; while
  shown it lists them again whenever the service announces a session added,
  removed or changed on the root channel. On `hidden` it disconnects and does
  no work until the next `shown`. Rows from the last listing stay up until the
  new one arrives.
- The field is a prompt for a new request, not a search: typing does not
  change the rows, which always show every session.
- With no sessions it says "No agent sessions yet"; with the service not
  running, "The agent service is not running".
- The item asks for `on_show`, so the field has the keyboard as soon as the
  canvas opens and shows its caret while it does. On a version 1 compositor a
  press on the item gives it the keyboard.
- Whenever there are rows, one is highlighted (the first by default). The row
  under the pointer is highlighted too.
- Keys, as in the launcher's agents mode (`otto_agents_kit::keys` is shared):
  - Down, Ctrl+N, Tab: next row; Up, Ctrl+P, Shift+Tab: previous row; Page
    Down / Page Up: eight rows. All wrap around, and scroll the highlight into
    view.
  - Enter with text typed: send it as a new request. Enter with nothing
    typed: open the highlighted session.
  - Right with nothing typed: open the highlighted session.
  - Ctrl+L or Cmd+L: send what is typed, or open an empty request.
  - Escape: clear the field; with the field empty, `dismiss`.
  - Everything else edits the field as the launcher's does: the shared
    otto-kit field keys, plus Ctrl+U (clear), Ctrl+A (select all),
    Ctrl+C/X/V (clipboard) and Ctrl+W (delete a word).
- Opening a session (click, Enter with nothing typed, Right) runs
  `otto-launcher --session <session URI>`, which opens the launcher card in
  ask mode on that session, then sends `dismiss` on the item. A session
  Preview started about a file (`_meta.otto.app` is `otto-preview`) runs
  `otto-preview --session <session URI> <file>` instead: the file, with the
  conversation beside it.
- Sending a request (Enter or Ctrl+L with text typed) runs
  `otto-launcher --ask --send -- <text>`, so the launcher opens with the
  request already sent and the conversation running, then sends `dismiss`.
  Ctrl+L with nothing typed runs `otto-launcher --ask`.
- The field is cleared when the canvas hides.

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
- **Escape belongs to the item with the keyboard.** An item with a field has
  a better first use for Escape (clearing it) than hiding the canvas, and
  only the item knows whether it has one.
- **Keyboard on show is version 2.** A new client must keep working against
  a compositor that only knows version 1, where an unknown request would
  disconnect it. `show`, `set_order` and `never` are version 3 for the same
  reason.
- **A client's `show` leaves the keyboard alone.** It is for news (something
  was stashed), not for input. Taking the keyboard would drop the app's
  selection and caret, which is what the user was about to act on.
- **Clicks outside go through.** The first click in an app beside the canvas
  should reach the app, and a drag from the app has to be able to end on an
  item. Hiding on release rather than on press, and not at all when the
  press started a drag, is what makes that drop possible. A plain click
  outside still closes the canvas.
- **Rest at the edge to open.** A drag cannot swipe or press a shortcut, and
  the right edge is where the canvas lives. The pause keeps a drag on its way
  to an output further right from opening it.
- **Drags are announced to everyone.** A client with nothing in the canvas
  may still want the drop (the stash starts one); it cannot add an item for
  it unless it knows a drag is on. The mime types let it add one only for
  what it can take.
- **`show` is ignored while locked or in exposé.** A lock hides the desktop
  for a reason, and exposé owns the screen; queueing the request would slide
  the canvas in over whatever the user unlocks or picks, long after the news.
  Clients show again on their next change.
- **Items share the height rather than cap themselves.** A client cannot
  know how tall the column is or what else is in it, so any cap it picks is
  wrong somewhere: too short on a tall screen, too tall beside another item.
  The compositor knows both. Water-filling lets short items keep all of
  their content and gives the room that is left to the ones that scroll.
- **`never` rather than a stash-only rule.** An item that must never steal
  the keyboard says so on the wire; the compositor needs no knowledge of
  which client it is.
- **The Agents panel's field is a prompt, not a search.** The panel lists
  only recent sessions in a narrow column, so narrowing them saved little,
  while a separate Ask button made starting a request two steps. Typing a
  request and pressing Enter sends it; the launcher opens on the running
  conversation instead of asking for a second Enter.

## Open Questions

- Overflow: items bound below version 5, or so many items that the minimums
  do not fit, can still overflow the column, which is clipped. Should the
  column scroll then?
- Ordering: should the user be able to reorder items by dragging?
- Should the canvas remember the output it was last shown on?
- The canvas does not follow its output being unplugged while shown; it is
  cleaned up the next time it hides.
