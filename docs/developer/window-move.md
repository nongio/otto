# Window Move

How Otto handles `xdg_toplevel.move` — a client asking the compositor to drag
its window, which is what happens when you grab a title bar.

The shape of the solution is Wayland's standard one: the compositor installs a
**grab** on the seat. While a grab is active, input events stop going to
clients and go to the grab's own handler instead. That is why dragging a window
over another window doesn't make the other window react — nothing downstream
ever sees those events.

## Entry point

`Otto::move_request_xdg` in `src/shell/xdg.rs`. It verifies that the request
comes from the serial of the grab that triggered it (pointer or touch) and that
the target surface is still mapped — clients can and do send stale requests.

A maximized window is unmaximized first, and its initial location is reset to
the pointer or touch position, so the restored geometry appears under the
cursor instead of jumping away from it.

## Pointer drags

A `PointerMoveSurfaceGrab` (`src/shell/grabs.rs`) is installed on the seat's
pointer. Pointer focus is cleared from clients for the duration. Each motion
event goes to `PointerMoveSurfaceGrab::motion`, which computes the delta from
the grab's start point and calls `workspaces.map_window` to reposition the
window; the associated view layers are updated so compositor-side UI stays in
sync with the new position.

## Touch drags

A `TouchMoveSurfaceGrab` (`src/shell/grabs.rs`) does the same through
`workspaces.map_window`, tracking the touch slot that started the grab so a
second finger cannot hijack the drag.

## Release

Both grabs release automatically when the initiating button is released or the
touch slot ends, restoring normal pointer and touch focus.
