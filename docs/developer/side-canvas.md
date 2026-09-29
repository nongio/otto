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
- While the canvas is shown, a scroll that starts with the pointer over the
  column (`canvas_contains_point`) is passed through untouched, so it scrolls
  the item under it and cannot dismiss the canvas.

## Protocol

`protocols/otto-canvas-v1.xml` has two interfaces:

- `otto_canvas_manager_v1.get_canvas_item(new_id, wl_surface)` gives the
  surface the `otto_canvas_item_v1` role (a `role` error if it has another)
  and appends it to the column.
- `otto_canvas_item_v1` sends `configure(serial, width)` (width in logical
  points, the column width), then `shown` or `hidden`. Requests are
  `ack_configure`, `dismiss` and `destroy`.

The server side is `src/otto_canvas/`: `protocol.rs` generates the bindings,
`handlers.rs` holds the `GlobalDispatch`/`Dispatch` impls (`CanvasGlobal`),
and `mod.rs` the state and behaviour. The client side is
`otto_kit::surfaces::CanvasItemSurface`, which acks configures, creates its
Skia surface on the first one and forwards every event to an `on_event`
handler.

## Scene

```
overlay_plane_{output}
├── … dock_plane, overlay_layer (primary only)
├── canvas_plane_{output}         hidden unless the canvas is on this output
│   └── canvas_column             positioned and clipped by the canvas
│       ├── canvas_slot           one per item, stacked by the canvas
│       │   └── canvas_item       the client's surface layer (surface_layers)
│       └── …
└── popup_overlay (primary only)
```

- Every output gets a `canvas_plane` in its overlay plane. On the primary it
  sits above the dock and `overlay_layer` and below the popups. The single
  `canvas_column` is re-parented into the plane of the output under the
  pointer when the canvas comes on screen.
- The item's own layer is registered in `surface_layers`, so the commit path
  (`sync_surface_tree_layers`) owns its size and position. The `canvas_slot`
  above it is the canvas's, which is where the vertical stacking lives, the
  same split the lock screen uses between its shade and the locker's layer.
- `is_overlay_ui_active` counts a visible `canvas_plane`, so the overlay KMS
  plane is pushed while the canvas is on screen. Fullscreen direct scanout is
  refused while the canvas is on screen, and the backdrop-blur interest falls
  back to the whole output, since the column moves and its items may blur.

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
  `canvas_pointer_button` first: `Consumed` stops the event (a press outside
  hides the canvas; its release is swallowed too), `Item` gives the item the
  keyboard through `KeyboardFocusTarget::CanvasItem` and skips
  click-to-focus. The previous focus is restored when the canvas hides.
- The keyboard filter intercepts Escape while the canvas is shown and turns
  it into `KeyAction::CanvasToggle`, which is also a bindable builtin
  (`CanvasToggle`, not bound by default).

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

## Sample client and autostart

`components/otto-canvas` is an otto-kit app that places one 200 pt frosted
panel. Like `otto-bar` and `otto-islands` it is started from `[[exec_once]]`
in the shipped config and installed by the packaging lists.

## Tests

- `cargo test --lib otto_canvas`: the column geometry maths.
- `cargo test --features headless --test side_canvas`: a raw-protocol client
  places an item and checks configure, `shown`/`hidden`, a live width change
  and click-outside dismissal.

## Follow-ups

- Overflow is clipped; the column should scroll.
- Popups of canvas items are not placed yet.
