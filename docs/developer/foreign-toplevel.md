## Foreign Toplevel Management

"Foreign toplevel" protocols are how a compositor lets *other* clients see and
control its window list — taskbars, launchers, window switchers, screen-share
pickers. Otto implements two of them, because the ecosystem is split between
the modern read-only standard and the older wlroots one that most existing
tools actually use.

| Protocol | Otto's support | What it offers |
|----------|----------------|----------------|
| [`ext-foreign-toplevel-list-v1`](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/ext-foreign-toplevel-list/ext-foreign-toplevel-list-v1.xml) | Smithay's implementation, complete | Read-only list: title, app_id, stable identifier |
| [`wlr-foreign-toplevel-management-unstable-v1`](https://gitlab.freedesktop.org/wlroots/wlr-protocols/-/blob/master/unstable/wlr-foreign-toplevel-management-unstable-v1.xml) | Custom implementation | List **plus** control: activate, close, minimize, maximize, fullscreen |

The `ext` protocol's identifier is also what Otto's screenshare API uses to
name a window — `RecordWindow`'s `window-id` is an
`ext-foreign-toplevel-list-v1` identifier. See [screenshare.md](screenshare.md).

### One window, two handles

Rather than duplicating bookkeeping, `src/state/foreign_toplevel_shared.rs`
wraps both protocol handles in one struct and fans every update out to both:

```rust
pub struct ForeignToplevelHandles {
    pub ext: Option<ExtHandle>,
    pub wlr: Option<WlrForeignToplevelHandle>,
}
```

Methods: `send_title`, `send_app_id`, `send_state`, `send_output_enter`,
`send_output_leave`, `send_done` (ext only — it batches), `send_closed`.
Callers never pick a protocol.

Handles are stored in `Otto::foreign_toplevels`, a `HashMap` keyed by the
surface's `ObjectId`.

### Lifecycle

**Creation** — when an xdg toplevel is mapped (`src/shell/xdg.rs`), Otto pulls
the app_id and title, creates one handle per protocol, wraps them, stores them,
and immediately sends the window's initial state.

```rust
let ext_handle = self.foreign_toplevel_list_state.new_toplevel::<Self>(&app_id, &title);
let wlr_handle = self.wlr_foreign_toplevel_state.new_toplevel::<Self>(&display_handle, &app_id, &title);
self.foreign_toplevels.insert(surface_id, ForeignToplevelHandles::new(ext_handle, wlr_handle, output));
```

**Updates** — title and app_id changes are broadcast from `src/shell/mod.rs`,
after checking the value actually changed so idle clients aren't spammed.
Focus changes call `Otto::send_foreign_toplevel_state`, which reads the
window's current minimized/maximized/fullscreen flags and sends the whole state
set.

**Destruction** — on unmap, the handle is removed and `send_closed()` notifies
every connected taskbar.

### Control requests (wlr protocol)

All of these are implemented in `src/state/wlr_foreign_toplevel.rs` and route
into the same code paths the compositor's own UI uses:

| Request | Maps to |
|---------|---------|
| `Activate` | `Otto::activate_window` |
| `Close` | `toplevel.send_close()`, or `X11Surface::close()` for XWayland windows |
| `SetMinimized` / `UnsetMinimized` | `Workspaces::minimize_window` / `unminimize_window` (plus focus and scanout demotion) |
| `SetMaximized` / `UnsetMaximized` | the `XdgShellHandler` maximize/unmaximize requests |
| `SetFullscreen` / `UnsetFullscreen` | the `XdgShellHandler` fullscreen/unfullscreen requests |
| `SetRectangle` | ignored — a minimize-animation hint, not required by the protocol |

### Protocol state (wlr)

The custom implementation keeps two levels of state:

- **`WlrForeignToplevelManagerState`** — the list of bound manager instances,
  one per connected client. Creating a toplevel broadcasts it to all of them.
- **`WlrToplevelData`** — per-window: app_id, title, current state, and one
  protocol resource per manager instance, in an `Arc<Mutex<_>>`.

When a new manager binds, every existing window is replayed to it — including
its current state and output — so a taskbar started after the fact sees a
correct list immediately.

### Known gaps

- **State events are only sent on focus change and on map.** A window minimized
  or maximized by some other route does not push a fresh `state` event, so an
  external taskbar can show a stale flag until focus next moves.
- **Output tracking is set once.** `output_enter` is sent when the handle is
  created; moving a window to another monitor does not currently emit
  `output_leave` / `output_enter`.
- **No parent tracking** — transient/child window relationships are not exposed.

### Testing

```sh
# terminal 1
cargo run -- --winit

# terminal 2 — a wlr-protocol consumer
WAYLAND_DISPLAY=wayland-1 rofi -modi window -show window
WAYLAND_DISPLAY=wayland-1 waybar
```

Selecting a window in `rofi` should focus it; close buttons in a taskbar should
close it.

### Related

- [Dock Design](dock-design.md) — the built-in, compositor-drawn dock
- [Wayland Protocols](wayland.md) — how handlers are wired in general
- [Smithay `foreign_toplevel_list`](https://smithay.github.io/smithay/smithay/wayland/foreign_toplevel_list/)
