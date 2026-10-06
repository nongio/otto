# Side canvas

The side canvas is a column that slides in over the desktop from the right
edge of an output. Otto owns the column; clients put surfaces in it through
`otto-canvas-v1`. The behaviour contract is in
[specs/side-canvas.md](../../specs/side-canvas.md); this page covers how it is
built.

## Edge swipe input

On the udev backend a two-finger scroll that starts at the right edge of the
touchpad and moves left opens the canvas, following the fingers. While the
canvas is shown, a two-finger scroll to the right that starts away from that
edge closes it. The code lives in `src/input/edge_swipe/`; it is compiled
only with the `udev` feature, since winit, X11 and headless have no evdev
devices.

### Where the finger positions come from

libinput reports neither finger positions nor where a scroll began, so the
touch origins are read from the kernel's multitouch slot state.

- `evdev::TrackingInterface` wraps smithay's `LibinputSessionInterface`.
  Every `/dev/input` node libinput opens through the session also gets a
  `dup()` stored in `EvdevFds`, keyed by node path; `close_restricted` drops
  it. Otto cannot open the nodes itself because libseat/logind owns them.
- The duplicate shares its open file description with libinput, so it is
  never `read()` (that would steal libinput's events). Only query ioctls
  run on it: `EVIOCGABS` for the `ABS_MT_POSITION_X` range and resolution
  and the `ABS_MT_SLOT` count (cached per device), and `EVIOCGMTSLOTS` for
  each slot's `ABS_MT_TRACKING_ID` and `ABS_MT_POSITION_X`. The ioctl
  numbers are written out by hand from the kernel headers.
- A libinput device is a touchpad when it has the gesture capability; its
  node is `/dev/input/<sysname>`.
- The slots are sampled on every libinput event from the touchpad (motion,
  button, scroll, gestures). `TouchOrigins` records a touch's position the
  first time its tracking id is seen.
- libinput emits nothing until the fingers move, so the first sample can
  come after the fingers have already travelled. The horizontal travel of
  the current scroll, converted from libinput's 1000 dpi units to
  millimetres and capped at 15 mm, is added back to the first sighting.
- When the session is paused the descriptors are revoked, the ioctls fail,
  and no touch counts as an edge start until libinput reopens the device.

### Edge zone

The rightmost touch origin has to be within 12 mm of the right edge and the
second one within 40 mm; further touches, such as a resting thumb, are
ignored. Pads that report no resolution use 10 % and 35 % of the axis range.
The constants are in `src/input/edge_swipe/mod.rs`.

### Claiming a scroll

`EdgeSwipe` is a pure state machine fed one event per finger scroll
(`AxisSource::Finger`), in physical finger terms: the glue flips libinput's
values back when natural scrolling is on, so the setting never changes the
gesture's direction. Deltas are divided by the pointer output's scale, so the
canvas moves in logical points 1:1 with the fingers.

1. At the first event of a scroll, the canvas state picks a candidate:
   reveal when the canvas is hidden, `canvas_available()` and the touches
   started at the edge; dismiss when the canvas is shown and they did not.
   Otherwise the whole scroll passes to clients untouched, with no delay.
2. A candidate's events are held until the fingers have moved 8 points. If
   the travel is at least twice as horizontal as vertical and points the
   right way (left to reveal, right to dismiss) the scroll is claimed:
   `canvas_gesture_begin` plus an update with the travel so far, and the
   held events are dropped. Otherwise the held events are replayed to
   clients in order and the rest of the scroll passes through. A scroll that
   has not covered 8 points after 150 ms, or ends before, is released too.
3. A claimed scroll feeds `canvas_gesture_update` (positive reveals) and is
   swallowed. The axis-stop event (every axis zero: fingers lifted) calls
   `canvas_gesture_end` with the average speed over the last 100 ms, so
   fingers that pause before lifting settle without a fling.
4. A gesture begin or a removed touchpad ends a claimed swipe as cancelled.
   Any other touchpad event arriving while a scroll is held releases the
   held events ahead of it.

The 3-finger workspace swipe, pinch and hold gestures are libinput gesture
events and never reach the state machine as scrolls.

### Known limitations

- The origin correction relies on libinput's scroll units approximating
  1000 dpi; a driver that scales scroll deltas differently shifts the
  effective edge zone by up to the 15 mm cap.
- A scroll is judged once, at its start; it is never re-judged after it has
  passed through or been claimed.
- While the canvas is shown, a rightward scroll dismisses it wherever the
  pointer is, over the column included. Items scroll vertically, so a scroll
  over the column that is not dominantly horizontal is replayed to the item
  under the pointer once the machine has decided (up to 8 points or 150 ms).

## Protocol

`protocols/otto-canvas-v1.xml` has two interfaces:

- `otto_canvas_manager_v1.get_canvas_item(new_id, wl_surface)` gives the
  surface the `otto_canvas_item_v1` role (a `role` error if it has another)
  and adds it to the column at order 0.
- `otto_canvas_item_v1` sends `configure(serial, width)` (width in logical
  points, the column width), then `shown` or `hidden`. Requests are
  `ack_configure`, `dismiss`, `destroy`; since version 2
  `set_keyboard_interactivity(none | on_show)`; since version 3 `show`,
  `set_order(int)` and the `never` interactivity.
- Since version 4 the manager sends `drag_mime_type(string)` for each type a
  starting drag offers, then `drag_started`, and `drag_ended` when it is
  dropped or cancelled.
- Since version 5 the item has `set_content_height(uint)`, its natural
  height in points, and the compositor sends `max_height(uint)`, the most it
  may be tall, after the first configure and whenever its share changes.
  See [Sharing the height](#sharing-the-height).

Otto advertises version 5. otto-kit binds `1..=5`, and each
`CanvasItemSurface` method for a newer request (`set_keyboard_interactivity`,
`show`, `set_order`, `set_content_height`) sends it only when the bound
version has it, returning
false otherwise, so a new client keeps running against an older compositor
(an unknown request would get it disconnected). Clients that speak the wire
directly, like otto-stash, gate the same way on the bound version.

The server side is `src/otto_canvas/`: `protocol.rs` generates the bindings,
`handlers.rs` holds the `GlobalDispatch`/`Dispatch` impls (`CanvasGlobal`),
`allot.rs` shares the column's height, and `mod.rs` the state and
behaviour. The client side is
`otto_kit::surfaces::CanvasItemSurface`, which acks configures, creates its
Skia surface on the first one and forwards every event to an `on_event`
handler. Apps hear of drags through `App::on_canvas_drag_started(ctx,
mime_types)` and `App::on_canvas_drag_ended(ctx)`; `AppData` gathers the
`drag_mime_type` events until `drag_started`.

## Scene

```
overlay_plane_{output}
├── … layer_shell_top, layer_shell_overlay, dock_plane (primary only)
├── canvas_plane_{output}         hidden unless the canvas is on this output
│   └── canvas_column             positioned and clipped by the canvas
│       ├── canvas_slot           one per item, stacked by the canvas
│       │   └── canvas_item       the client's surface layer (surface_layers)
│       └── …
├── workspace_selector_{output}
├── overlay_layer (primary only)
└── popup_overlay (primary only)
```

- Every output gets a `canvas_plane` in its overlay plane, above the
  layer-shell chrome and the dock and right below the workspace selector,
  the OSD and the popups. The selector only shows in exposé, where the
  canvas has faded out. The single
  `canvas_column` is re-parented into the plane of the output under the
  pointer when the canvas comes on screen.
- Exposé fades every `canvas_plane` together with `layer_shell_overlay`,
  following the exposé gesture and its transition. The canvas stays open
  underneath; `canvas_suspended()` keeps it out of pointer, keyboard and edge
  swipe handling until exposé closes and it fades back in.
- `CanvasState::items` is kept sorted by `(order, seq)`, `seq` being a
  creation counter; `canvas_relayout` stacks the slots in that order, so
  `set_order` re-sorts and lays out again at once.
- The item's own layer is registered in `surface_layers`, so the commit path
  (`sync_surface_tree_layers`) owns its size and position. The `canvas_slot`
  above it is the canvas's, which is where the vertical stacking lives, the
  same split the lock screen uses between its shade and the locker's layer.
- `is_overlay_ui_active` counts a visible `canvas_plane`, so the overlay KMS
  plane is pushed while the canvas is on screen. Fullscreen direct scanout is
  refused while the canvas is on screen, and the backdrop-blur interest falls
  back to the whole output, since the column moves and its items may blur.

## Sharing the height

`src/otto_canvas/allot.rs` holds the pure share function,
`allot(available, gap, &[Demand]) -> Vec<u32>`, with its unit tests. A
`Demand` is `Content(h)` for an item that takes part or `Fixed(h)` for one
that does not. It takes the gaps between showing items (height > 0) and the
fixed heights off the column, then water-fills the rest: contents under the
level keep their height, the others get the level, and the points the
division leaves go to the topmost items at the level. `MIN_SHARE` (120 pt,
or the content if less) is what fixed items can never squeeze an item below;
when they leave less, each item gets its minimum and the column overflows,
and a column too short for the minimums alone is water-filled whole.

`Otto::canvas_share_height` builds the demands from `CanvasItem`
(`content_height` from `set_content_height`, otherwise the buffer height from
`item_size_points`), and sends `max_height` to each version 5 item whose
share differs from the `max_height` it was last sent. An item that has not
sent a content height is offered what `allot` would give it wanting the
whole column. The height is `canvas_available_height()`: the column geometry
(`canvas_geometry_on`) of the canvas's output, or of the output under the
pointer or the primary one while hidden, in whole points.

It runs from `canvas_relayout` (every item commit, `set_order`, a removed
item, a width or config change, and `canvas_bring_on_screen` before
`shown`), from `canvas_item_created` between `configure` and `shown`/`hidden`,
from `set_content_height`, and from `layer_zones_changed` when an output's
usable area moves. Stacking still uses buffer heights, so an item that has
not yet resized to its share overflows for a frame rather than being
squeezed by the compositor.

On the client side `CanvasItemSurface::set_content_height` sends only a
changed height, `max_height()` holds the last share
(`CanvasItemEvent::MaxHeight` reaches the handler after it is stored), and
`fit_height(content, fallback_max)` is `min(content, share)`, using
`fallback_max` until a share arrives or below version 5.
`shares_height()` says whether one will come; the Agents panel does not draw
until it has.

## State and animation

`CanvasState` (on `Otto` as `canvas`) holds the items and a phase:
`Hidden`, `Dragging { was_shown }`, `Shown` and `Closing { generation }`.

- The gesture API (`canvas_gesture_begin/update/end`) maps finger travel to a
  0..1 progress, converted through the output's scale into physical pixels.
  Begin reads the column's current (possibly animating) position, so a swipe
  can catch the canvas mid-slide.
- Settling uses `Spring::with_duration_and_bounce(0.5, 0.05)`, the workspace
  swipe spring. lay-rs replaces a spring's initial velocity with the running
  animation's, so the release velocity only decides the direction.
- A close runs the slide with an `on_finish` that records its generation in an
  atomic. `canvas_frame_presented` (called from `lock_frame_presented`, which
  every backend calls per presented frame, and from the headless loop)
  notices it, hides the plane and sends `hidden`. The generation keeps an
  earlier slide's end from finishing a later one.

## Input

- `surface_under` asks `canvas_surface_under` after the lock screen and the
  app switcher; it hit-tests each item's surface tree at its slot position.
- `on_pointer_button` (and the headless synthetic button) run
  `canvas_pointer_button` first: `Consumed` stops the event (a press on the
  column between items; its release is swallowed too), `Item` gives the item the
  keyboard through `KeyboardFocusTarget::CanvasItem`, unless the item's
  `ItemKeyboard` is `Never`, and skips click-to-focus either way. The
  previous focus is restored when the canvas hides.
- `canvas_settle_open(focus)` ends `canvas_show`, a swipe released open and
  a client's `show`. With `focus` (the user's opens) it calls
  `canvas_focus_on_show`: unless the session is locked or an item already
  has the keyboard, the first live item in column order with
  `ItemKeyboard::OnShow` is focused through `canvas_focus_item`, which
  remembers the previous focus for `canvas_restore_focus`.
- `canvas_item_show` (the `show` request) is ignored while shown or
  dragging, while locked and while `canvas_suspended()`. Otherwise it brings
  the canvas on screen as `canvas_show` does, calls `canvas_settle_open(false)`
  and sets `CanvasState::passive` (`canvas_show_passive`, shared with a
  drag resting at the edge). `passive` is cleared by `canvas_show`, a
  gesture begin, a press on an item that takes the keyboard, and any hide.
- A press outside the shown column, passive or not, returns `Pass`, so it
  reaches the window under it and click-to-focus runs as usual; it is
  recorded as `CanvasState::press_outside` (`drag::PressOutside`). A drag
  starting while it is down marks it `dragged`. On the matching release,
  `drag::hide_on_release` hides the canvas only for a plain click released
  outside the column. The release is routed through the canvas before
  `pointer.button`, so it sees the drag still going on; the drop itself
  comes after.
- The keyboard filter asks `canvas_owns_escape()`: the canvas is shown, not
  passive, not suspended, and no canvas item has the keyboard
  (`canvas_item_has_keyboard`). Then Escape becomes
  `KeyAction::CanvasToggle`, which is also a bindable builtin (`CanvasToggle`,
  not bound by default). Otherwise Escape goes to whoever has the keyboard.

## Drag and drop

`src/otto_canvas/drag.rs` holds the decisions as plain functions and types,
unit tested on their own: `at_right_edge` (within the canvas width, `EDGE_FRACTION`),
`EdgeDwell` (fires once per rest of `DWELL`, 250 ms), `hide_when_drag_ends`
and `hide_on_release`. `mod.rs` applies them:

- `WaylandDndGrabHandler::dnd_requested` reads the source's mime types from
  `Source::metadata()` and calls `canvas_drag_started`, which marks a
  pending `press_outside` as dragged, sends every live version 4 manager the
  `drag_mime_type` events and `drag_started`, stores a `DragSession` and
  starts a calloop timer that calls `canvas_drag_tick` every `POLL` (50 ms).
  Bound managers are kept in `CanvasState::managers`; one bound mid-drag is
  told at once.
- `canvas_drag_tick` checks whether the pointer is at the right edge of the
  output under it. When the dwell fires and the canvas is not shown, it opens
  it with `canvas_show_passive` and records `opened_canvas`; if there is no
  item yet the dwell is rearmed, so a client adding one on `drag_started` is
  picked up on the next tick.
- `DndGrabHandler::dropped` (and `cancelled`) call `canvas_drag_ended` with
  the drop target's surface, if any. The timer is removed, and a canvas the
  drag opened hides unless the target's root is a canvas item
  (`canvas_item_root_for`). Then every manager gets `drag_ended`. smithay
  calls `dropped` after the target's `drop`, so the item has the data offer
  by then.
- DnD focus comes from `surface_under`, which includes
  `canvas_surface_under`, so an item's `wl_data_device` gets `enter`/`drop`
  like any other surface.

## Frame callbacks

Canvas items are in no space and no layer map, so `post_repaint` never sees
them. `canvas_frame_presented` sends their frame callbacks while the canvas is
on screen on that output, and nothing while it is hidden. The preferred
fractional scale is set when an item is created and whenever the canvas comes
on screen.

## Configuration

`[canvas]` holds `width`, `margin` and `gap` (logical points). `canvas.width`
is a live setting: `apply_live` calls `canvas_config_changed`, which sends
every item a new configure and lays the column out again.

## Stash card

`components/otto-stash` binds `otto_canvas_manager_v1` at version 3 to 5
when the compositor offers it (it needs `show`, `set_order` and `never`) and
then hosts its "Ask about…" card as a canvas item (`otto-stash/src/canvas.rs`)
instead of the floating overlay balloon (`panel.rs`). `card.rs` holds the
`Card` enum over the two. otto-stash is a plain wayland-client program, not an
otto-kit app, so it speaks the protocol through `otto_kit::protocols` rather
than `CanvasItemSurface`.

The canvas card sets order -100 (above the Agents panel), `never` keyboard
interactivity, and the launcher's frost through `otto-surface-style` without
the balloon's shadow or position. It draws with `balloon.rs` at the
configured width. At version 5 it sends its natural height
(`Layout::natural_height`, the card with every item showing) with
`set_content_height` whenever a layout changes it, counts as configured only once the first `max_height` has
arrived, and lays out again on each new `max_height`, scrolling the items
past it; below version 5 it grows up to 640 points. After an add, or
when the card appears, it sends `show` once the new buffer is committed. It
requests frame callbacks only while `shown`; on `hidden` an item shrinking
away is removed at once. When the stash ends, or Ask holds it, the item is
destroyed and the canvas otherwise stays as it is. A region pick sends
`dismiss` first so the canvas is not in the capture. See
[specs/stash.md](../../specs/stash.md).

At version 4, `invite.rs` adds a drop invitation on `drag_started` when the
drag may carry files (`text/uri-list`, or no types) and there is no card: a
72-point item at the card's order (at version 5 it sends 72 as its content
height, below any minimum share, and ignores `max_height`) with a dashed outline and the
`stash-drop-invite` string, accented while a drag is over it. `drop.rs`
accepts file drops on it as on the card and marks it `dropped`. On
`drag_ended` an invitation nothing was dropped on is destroyed; one that took
files stays until the new card's first buffer is committed (`draw_panel`), or
until the files are read if no card is coming (`settle_invite`), so the
column never empties in between and the canvas does not close under the
drop.

## Sessions client and autostart

`components/otto-canvas` is an otto-kit app that places one frosted item: the
agent sessions list. Like `otto-bar` and `otto-islands` it is started from
`[[exec_once]]` in the shipped config and installed by the packaging lists.

It shares its pieces with the launcher's agents mode through
`components/otto-agents-kit`, which both depend on:

- `otto_agents_kit::item` has the row model (`Item`, `Activity`, `Origin`)
  and `rank`, which the launcher's sources also use. It and `rows` live in
  `otto_kit::components::item_list` — the settings sidebar's search lists its
  matches with them too — and are re-exported here under their old names.
- `otto_agents_kit::sessions` has the session rows (`session_items`, used by
  `Ask::session_rows` too) and `SessionFeed`, a background connection that
  lists agents and sessions and lists again on `root/sessionAdded`,
  `root/sessionRemoved` and `root/sessionSummaryChanged`. The canvas holds a
  feed only between `shown` and `hidden`, so a hidden canvas has no
  connection.
- `otto_agents_kit::rows::paint_item_rows` paints rows exactly as the
  launcher's list does (`Palette::paint_rows` calls it at the card width).
  `rows` also has `field_style`, the divider and highlight colours, `ROW_H`
  and `MAX_ROWS`.
- `otto_agents_kit::keys` has the keys the launcher's field and list share:
  `list_step` (Down/Up, Ctrl+N/P, Tab/Shift+Tab, Page Down/Up), `wrap`,
  `edit_field` (Ctrl+U/A/C/X/V/W, then otto-kit's `key_for`) and
  `copy_to_clipboard`. The launcher's `on_key_event` and the canvas both go
  through it; each checks its own keys (Enter, Escape, Ctrl+L, Right) first.
- `SessionFeed::items(source, query)` filters with `session_items`, as
  `Ask::session_rows` does. Rows carry the session's index in the feed, so the
  canvas maps a filtered row back to its URI.

The item draws immediate-mode into its one surface: the heading band with the
Ask button, the launcher's `TextInput` with `field_style` at 16 pt, a hairline
(`rows::divider_color`), then the rows, with otto-kit's `ScrollView` for the
scroll physics and scrollbar. Scroll animation steps only when
`CanvasItemSurface::frame_in_flight` is false. The field's caret shows while
`CanvasItemSurface::has_keyboard` is true and blinks on the idle timeout.
The item asks for `on_show` when it is created. Opening a row spawns
`otto-launcher --session <URI>`, the Ask button and Ctrl+L spawn
`otto-launcher --ask`, and Enter with no row spawns
`otto-launcher --ask -- <text>` (the launcher takes everything after `--` as
its query). Each is reaped on a thread and followed by `dismiss`.

## Tests

- `cargo test --lib otto_canvas`: the column geometry maths.
- `cargo test --features headless --test side_canvas`: a raw-protocol client
  places an item and checks configure, `shown`/`hidden`, a live width change,
  click-outside dismissal, and that an `on_show` item gets `wl_keyboard.enter`
  on every show and the window that had the keyboard gets it back on hide.
  Version 3: `show` opens the canvas passive without moving the keyboard (and
  a later user open does focus the `on_show` item), a click outside a shown
  canvas passes through and hides it on release, a click on a `never` item
  leaves the keyboard with the window, and `set_order` restacks two items and
  ties go back to creation order. Version 4, with `TestClient::start_drag`
  from a window: a drag started beside an open canvas keeps it open, a drag
  resting at the right edge opens it passive, managers get the mime types,
  `drag_started` and `drag_ended`, a drop elsewhere hides it again, and a
  drop on an item keeps it.
- `cargo test --lib otto_canvas`: also the drag decisions in `drag.rs`, and
  the height shares in `allot.rs` (content that fits, even splits over
  several rounds, fixed items, the minimum, tiny columns).
- Version 5, `items_share_the_column_height`: an item hears `max_height`
  between `configure` and `hidden`; a tall and a short item get shares that
  fit the column, the short one its content; a version 4 item with a buffer
  counts as fixed and is told nothing.
- `cargo test -p otto-agents-kit keys`: the shared list keys.

## Follow-ups

- Items below version 5, or more items than the minimums fit, can still
  overflow the column, which is clipped.
- Output scale or mode changes reach the shares only through the usable
  area or the next show.
- Popups of canvas items are not placed yet.
