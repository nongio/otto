# Accessibility

How Otto and otto-kit talk to assistive technologies. The contract is in
[`specs/accessibility.md`](../../specs/accessibility.md); this is the shape of
the implementation.

## The two halves

Accessibility on Linux is AT-SPI: a D-Bus object model, on its own bus, that
applications publish themselves onto and screen readers read. Otto's involvement
is in two unrelated pieces.

**Keys.** A Wayland client cannot read the keyboard, so a screen reader has no
way to receive its own keybindings — the notorious Wayland accessibility gap.
at-spi2-core defines `org.freedesktop.a11y.KeyboardMonitor` for the compositor
to close it, and `src/a11y/keyboard_monitor.rs` implements it, by hand, on the
session connection the other Otto services already use. This is the piece that
makes Orca usable at all.

**Trees.** Everything else is the AT-SPI object model — a dozen interfaces, a
cache protocol and an event vocabulary. That is [AccessKit]'s job: both the
compositor and otto-kit build `accesskit` node trees and hand them to an
`accesskit_unix::Adapter`, which does the D-Bus work.

[AccessKit]: https://accesskit.dev

## Where the pieces are

| | |
|---|---|
| `src/a11y/keyboard_monitor.rs` | the `KeyboardMonitor` interface and the grab table |
| `src/a11y/chrome.rs` | the shell (dock, switcher, workspaces) as an AT-SPI application |
| `src/input/keyboard.rs` | the one call that offers each key to assistive technologies |
| `components/otto-kit/src/focus.rs` | keyboard focus below the window: the traversal order and the ring |
| `components/otto-kit/src/accessibility/tree.rs` | building a window's accessible tree |
| `components/otto-kit/src/accessibility/widgets.rs` | one description per kit widget |
| `components/otto-kit/src/accessibility/adapter.rs` | the per-surface adapter and its mailbox |

## The threading rule

**The grab decision is synchronous, on the compositor thread.** Whether a key
belongs to a screen reader has to be answered while the key is being processed,
in the middle of `keyboard_key_to_action`'s input filter. A D-Bus round trip
there would put the bus in the path of every keystroke in the session.

So the grab table is a plain `Arc<Mutex<..>>` shared with the D-Bus task: the
compositor matches against it directly and pushes outgoing `KeyEvent` signals
onto a channel that the D-Bus task drains and emits. Nothing in the input path
awaits anything.

The AccessKit adapters are the other way round: their handlers are called from
the adapter's own thread, and the trees can only be built where the state is.

- The **shell** keeps a snapshot of what it last drew, refreshed from the
  `Observer<WorkspacesModel>` it registers with `Workspaces`. AccessKit's
  request for an initial tree is answered from that snapshot, on the spot.
- A **kit application** cannot: its state is the UI thread's. So activation
  raises a flag and wakes the run loop, which builds the tree on its next pass
  (`pump_accessibility` in `app_runner/mod.rs`) — AccessKit explicitly allows
  `request_initial_tree` to return `None` and be answered later.

## Identity

Kit node ids come from `FocusId`, the same id the keyboard uses. That is the
point: the focus ring and the accessible tree are two views of one list, so what
Tab moves to and what a screen reader says has focus cannot diverge.

## Nothing is built unless something is listening

`update_if_active` does nothing while no assistive technology is attached, and
the closure that builds the tree is not called. A session with no screen reader
costs one idle D-Bus connection and a map lookup per accessible surface per run
loop pass — no tree building, no per-frame work, nothing in the render path.

## Both halves of the manager, or neither

`org.freedesktop.a11y.Manager` carries two interfaces, and at-spi2-core builds
*one* input device out of both. Serving only `KeyboardMonitor` does not give a
screen reader a keyboard with no mouse review — it gives it no device at all:
Orca calls `PointerLocator.QueryPointer` while constructing the device, and on
an `UnknownInterface` it never asks for a single key grab, then segfaults on the
reply it did not get. Both interfaces live in `src/a11y/`, registered on the one
object in `screenshare/dbus_service.rs`.

Their shapes are not ours to choose, and getting one wrong is fatal rather than
degraded — libatspi parses replies with a fixed format string. When in doubt,
read them out of mutter's binary, which is the implementation at-spi2-core was
written against:

```python
# GDBusInterfaceInfo { ref_count; name*; methods**; signals**; ... }
# GDBusMethodInfo    { ref_count; name*; in_args**; out_args**; ... }
# GDBusArgInfo       { ref_count; name*; signature*; ... }
# Find "org.freedesktop.a11y.PointerLocator" in /usr/lib/libmutter-*.so,
# find the pointer to it in .data.rel.ro, and walk the structs.
```

That is how the contract below was settled, against a guess that would have
crashed Orca a second time:

```
QueryPointer() -> (a{sv} app_data, d rel_x, d rel_y)
PointerPositionChanged()          # no arguments: "it moved, ask again"
```

Otto's `KeyboardMonitor` was checked the same way and matches mutter's exactly,
down to argument names and the `buuuq` of `KeyEvent`.

## Two traps

Both of these cost a live session to find, and neither shows up as a compile
error.

**A repeated node kills the process.** AccessKit panics on a `TreeUpdate` that
names one node twice, and in the compositor that panic takes the whole desktop
down. It is easy to do: the dock's `launchers` and `running_apps` overlap, so an
application that is pinned *and* running was announced twice the moment it
started. Build the dock's list from `DockModel::display_entries`, which is what
the dock draws; and note that one application can legitimately be in both the
dock and the switcher, so the section is part of a node's identity. Both tree
builders now drop a repeat rather than pass it on — `Snapshot::build` and
`A11yTree::push` — because no tree Otto can build should be able to end the
session.

**Never derive a node id by adding to another one.** The shell's workspaces
were numbered `WORKSPACES + 1 + index`, which was fine until a `WINDOWS`
container was added as the next constant — the first workspace's id was then
exactly it, and the session died the moment the overview opened with a
workspace present. Every generated id is now a hash with a per-section salt,
living in the high-bit half of the id space, while the fixed containers keep
small constants; the two cannot meet. `no_tree_can_repeat_a_node` builds every
section at once, which is the shape that caught it.

**`DefaultApp` forwards `App` by hand.** Every kit application is wrapped in
`DefaultApp`, which delegates each trait method to the inner app one method at a
time. A new `App` method that is not added there silently resolves to the trait
default, so the application's implementation is never called and the failure
looks like "the tree is empty" rather than anything to do with delegation. Add
to that impl whenever the trait grows.

Related: an adapter's life is the *window's*, not the render surface's.
`SkiaSurface` is rebuilt on the first configure and on every resize, so tearing
accessibility down in its `Drop` meant the adapter died seconds after the window
opened. It ends in `Window::close` instead.

## Bounds are what make a node findable

A node with no bounds can be read but not *found*: mouse review reads whatever
is under the pointer, and a magnifier follows the focus, and both work by
hit-testing coordinates. The shell's dock icons take their bounds from the
icon's own layer (`DockView::app_icon_bounds`), so magnification is accounted
for by construction; windows in the overview take theirs from the window view's
model. Everything reported is in **logical pixels**, matching the pointer
locator, so the two can be compared — layer geometry is physical and is divided
by the scale on the way out.

`Adapter::set_root_window_bounds` is what makes those coordinates screen
coordinates. The shell sets it to the whole screen at the origin; a kit
application's bounds stay window-relative, which is all AccessKit needs from a
client.

## Testing

```sh
cargo test --lib a11y                 # the grab table and the shell tree
cargo test -p otto-kit --lib focus accessibility
```

`scripts/a11y-keygrab-test.py` holds a connection open and prints the keys it is
sent, which a one-shot `busctl call` cannot do: grabs are dropped when the
client that asked for them disconnects.

**Nothing publishes a tree until an assistive technology is present.** AccessKit
adapters stay dormant while `org.a11y.Status.IsEnabled` is false — which it is
in a session with no screen reader — so Otto and its applications will not
appear on the bus at all. Running a screen reader sets it; to test without one,
set it by hand:

```sh
busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled b true
scripts/a11y-watch.py            # what is on the bus
scripts/a11y-watch.py otto       # the shell's tree
scripts/a11y-watch.py --follow   # focus as it moves
```

GTK applications publish regardless of that flag, which makes it easy to
conclude the bus is fine while every AccessKit application is still invisible.

Without a screen reader, the interface can be exercised directly:

```sh
busctl --user introspect org.freedesktop.a11y.Manager /org/freedesktop/a11y/Manager
# grab Shift_L (keysym 0xFFE1) and nothing else
busctl --user call org.freedesktop.a11y.Manager /org/freedesktop/a11y/Manager \
    org.freedesktop.a11y.KeyboardMonitor SetKeyGrabs 'aua(uu)' 1 65505 0
busctl --user monitor   # KeyEvent should fire, and the key not reach the client
```

`ydotool` is the only injector that tests the real path: `wtype` goes through
`zwp_virtual_keyboard`, which is delivered straight to the focused surface and
never passes the input filter accessibility hooks into, so a grabbed key
injected that way is never seen. `ydotool key 67:1 67:0` (F9) goes through
uinput and libinput like a physical key does.

With one: run Otto on a tty, then `orca -r`. `accerciser` shows both trees —
"Otto" for the shell and one per kit application — and is the quickest way to
see whether a control is described the way it is drawn.

Nested Otto (`--winit`) deliberately exposes none of this, so accessibility work
has to be tested on a real session.
