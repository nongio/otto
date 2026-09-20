# Window Move

How Otto drags a window: `xdg_toplevel.move`, which a client-decorated window
sends when you grab its title bar, and the equivalent path for the titlebars
Otto draws itself.

The shape of the solution is Wayland's standard one: the compositor installs a
**grab** on the seat. While a grab is active, input events stop going to
clients and go to the grab's own handler instead. That is why dragging a window
over another window doesn't make the other window react — nothing downstream
ever sees those events.

## Entry points

`Otto::move_request_xdg` in `src/shell/xdg.rs` handles the client request. It
checks that the serial belongs to a live pointer or touch grab, that the
surface still maps to a window, and that the grab's focus belongs to the same
client. Clients can and do send stale requests.

Otto defaults to server-side decorations, so most title bars are Otto's own
surface-less input target and no client request is involved. Those go through
`Otto::move_request_ssd` instead, which skips the client-focus check and hands
straight to the same pointer path.

## Pointer drags

`Otto::begin_pointer_move` installs a `PointerMoveSurfaceGrab`
(`src/shell/grabs.rs`) on the seat's pointer with `Focus::Clear`, so no client
has pointer focus for the duration. Each motion event goes to
`PointerMoveSurfaceGrab::motion`, which computes the delta from the drag origin
and calls `workspaces.map_window` to reposition the window. The window's view
layer is moved to match, in the owning output's physical pixels, so
compositor-side UI stays in sync.

A maximized or tiled window is not restored at button-down but *into* the
drag: only once the pointer has travelled `DRAG_THRESHOLD` (6.0 logical
pixels). Restoring at the press would make a plain click on the title bar
unmaximize the window, and would eat the first press of the double click that
zooms it. `Otto::restore_window_for_drag` keeps the point the user grabbed
under the cursor and returns the window's new origin, which the grab then
measures from.

A leaf of a workspace's tiling tree leaves the tree on the same threshold
instead of being restored to a floating rect; the two are mutually exclusive.
While detached the layer is scaled about its origin, so the drag is measured
from the shrunk offset (`shell::scaled_drag_origin`) or the window would slide
out from under the pointer.

While Ctrl is held, the zone under the pointer is previewed through
`workspaces.tiling_overlay`. Ctrl is re-checked at release, because
`active_zone` is only refreshed on motion.

## Touch drags

A `TouchMoveSurfaceGrab` (`src/shell/grabs.rs`) does the same through
`workspaces.map_window`, ignoring any event whose slot is not the one that
started the grab, so a second finger cannot hijack the drag. It keeps the last
touch location, because a touch-up carries none and the tiling drop needs one.

On this path a maximized window is unmaximized in `move_request_xdg` itself,
before the grab is installed. The grab point's position within the maximized
window is kept as a ratio and reapplied to the restored size, so the window
stays under the finger.

## Release

The pointer grab ends when the last button is released, the touch grab when the
starting slot lifts. A window dragged out of a tiling tree is dropped into the
slot the overlay was showing *before* the grab is unset — `unset` cancels a
detach that is still open, which would undo the insert. Otherwise a previewed
snap zone is applied, or the window's popups are repositioned, since they are
not re-unconstrained during the drag. Under XWayland, the window's new root
position is sent to the client either way: an X11 client places its own menus
from the position the server last reported.
