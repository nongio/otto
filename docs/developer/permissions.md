# Sandboxed Clients

What Otto keeps away from sandboxed apps, and why it stops there. The code is
`src/sandbox.rs` and the filters on each global in `src/state/`; the tests are
`tests/sandboxed_globals.rs`, `tests/session_lock.rs` and
`tests/idle_inhibit.rs`.

## The one boundary

Programs running as the user, outside a sandbox, already have the user's
rights: they can edit `config.toml`, read the user's files and debug the
user's other programs. The compositor does not gate them.

A Flatpak app is cut off from all of that, but it talks to the compositor over
the Wayland socket. If Otto offered it the virtual keyboard it could type into
a terminal; with screencopy it could read the screen. So the compositor must
not be a way out of the sandbox.

Flatpak connects its apps through a `wp_security_context_v1` listener.
`is_sandboxed_client` is true for every client that came in that way, and
these globals are never offered to it:

- `zwlr_screencopy_manager_v1`
- `zwlr_virtual_pointer_manager_v1`, `zwp_virtual_keyboard_manager_v1`
- `zwp_input_method_manager_v2`
- `zwlr_data_control_manager_v1`, `ext_data_control_manager_v1`
- `zwlr_foreign_toplevel_manager_v1`, `ext_foreign_toplevel_list_v1`
- `zwlr_layer_shell_v1`, `zwlr_gamma_control_manager_v1`
- `zwp_keyboard_shortcuts_inhibit_manager_v1`
- `wp_security_context_manager_v1`
- Otto's dock and text-cursor protocols

A Flatpak that needs to capture the screen goes through xdg-desktop-portal,
which asks the user.

D-Bus needs nothing from Otto: Flatpak's bus proxy already limits an app to
the portals and the names it declares, so it cannot reach `org.otto.*`.

## Not covered

- **Snap apps** don't use the security context, so on Wayland they are
  treated as ordinary programs.
- **XWayland.** X11 clients can read other X11 windows and type into them.

## The lock screen

These are about the lock being reliable rather than about sandboxes:

- Otto starts the locker on a socketpair and offers
  `ext_session_lock_manager_v1` to it alone. `loginctl lock-session` asks
  Otto to start it.
- The screen blanks as soon as a lock begins, before the locker is up. A
  locker that dies is started again under the standing lock, and an output
  added while locked starts blank.
- Every suspend locks first, behind a logind delay inhibitor
  (`[lock] on_suspend`).
- The lock shortcut and VT switching survive a keyboard-shortcuts inhibitor.
- Popup and input-method grabs are dropped while locked, and a popup grab
  needs the serial of the latest press on that client.
- An idle inhibitor counts only while its surface is on screen.

## Screen capture

- A screencast session answers only the connection that created it, and
  ends when that connection leaves the bus.
- The portal refuses when it cannot show a picker, instead of sharing a
  default output.
- `otto-rdp` listens on localhost unless told otherwise.
