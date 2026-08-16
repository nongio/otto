# Wayland protocols

Otto follows Smithay's "one big compositor state" architecture: nearly all
protocol state and every handler hangs off a single struct, `Otto<BackendData>`.
Learning where that struct is built, and how Smithay routes requests into it,
makes the rest of the codebase navigable.

## The big state: `Otto<BackendData>`

`Otto<BackendData>` lives in `src/state/mod.rs` and holds:

- High-level compositor state — workspaces, popups, input state, the scene graph
- Smithay protocol state objects — `CompositorState`, `XdgShellState`,
  `WlrLayerShellState`, `PresentationState`, `ShmState`, …
- Backend-specific data (`BackendData`) for rendering and outputs

So `self.xdg_shell_state` or `self.shm_state` is one of those Smithay objects
stored directly inside `Otto`. There is no separate protocol layer to look for.

## Where it is initialized

Most globals are created in `Otto::init(...)` in `src/state/mod.rs`:

- The Wayland socket and client dispatch source are installed into calloop.
- Smithay protocol states are constructed (`CompositorState::new`,
  `XdgShellState::new`, `PresentationState::new`, …).
- Capability-gated globals are created conditionally, based on what the backend
  can actually do.

Backends create their own globals in their entrypoints — most notably
`zwp_linux_dmabuf_v1`, which is per backend.

## How delegation works

Smithay protocols are wired with `delegate_*` macros. The pattern is always
the same three pieces:

1. Otto stores the protocol state object (`xdg_shell_state: XdgShellState`).
2. Otto implements the matching handler trait
   (`impl XdgShellHandler for Otto<BackendData>`).
3. A `delegate_*` macro is invoked for `Otto<BackendData>`.

The macro generates the dispatch glue, so requests arriving for that global end
up in your handler impl.

## Finding a protocol

When you need to know where protocol X is implemented:

1. **Grep for the delegate macro** — `delegate_xdg_shell!`,
   `delegate_layer_shell!`, `delegate_presentation!`, …
2. **Find the handler impl** — `impl<BackendData: Backend> XdgShellHandler for Otto<BackendData>`
3. **Find where the state is constructed** — usually `Otto::init(...)` in
   `src/state/mod.rs`; backend-specific globals (dmabuf) live in `src/udev/`,
   `src/winit.rs`, `src/x11.rs`.

Handlers are split roughly like this:

- `src/state/*.rs` — core protocol handlers and delegate glue (seat, selection,
  input method, fractional scale, foreign toplevel, session lock, screencopy, …)
- `src/shell/*.rs` — xdg-shell, layer-shell, XWayland, and surface commit plumbing
- `src/{udev,winit,x11}.rs` — backend-specific globals and handler impls

## Common entrypoints

| Protocol | Handler | State + delegation |
|----------|---------|--------------------|
| `wl_compositor` / commits | `CompositorHandler` in `src/shell/mod.rs` | `src/state/mod.rs` |
| `xdg_wm_base` | `XdgShellHandler` in `src/shell/xdg.rs` | `src/state/mod.rs` |
| `zwlr_layer_shell_v1` | `WlrLayerShellHandler` in `src/shell/mod.rs` | `src/state/mod.rs` |
| `wl_seat` | `src/state/seat_handler.rs` | seat wiring in `Otto::init` |
| `wl_data_device_manager` | `src/state/data_device_handler.rs` | |
| primary selection, data control | `src/state/selection_handler.rs` | |
| `wp_presentation` | feedback emitted in `post_repaint` / `take_presentation_feedback` | `src/state/mod.rs` |
| `zwp_linux_dmabuf_v1` | `impl DmabufHandler` per backend | `src/udev/`, `src/winit.rs`, `src/x11.rs` |
| `ext_session_lock_v1` | `src/state/session_lock_handler.rs`, `src/lock.rs` | |
| `zwlr_screencopy_manager_v1` | `src/state/screencopy.rs` | |
| foreign toplevel (both protocols) | `src/state/foreign_toplevel_list_handler.rs`, `src/state/wlr_foreign_toplevel.rs` | see [foreign-toplevel.md](foreign-toplevel.md) |

## Otto's own protocols

Three protocols are Otto-specific. Their XML lives in `protocols/`:

| Protocol | Implementation | What it does |
|----------|----------------|--------------|
| `otto-surface-style-unstable-v1` | `src/surface_style/` | Lets a client style and animate its own surface through the compositor's scene graph — corner radius, shadow, opacity, transforms, batched in transactions. See [sc-layer-protocol-design.md](sc-layer-protocol-design.md) for the design history. |
| `otto-dock-v1` | `src/otto_dock/` | Lets a client contribute items to the compositor-drawn dock |
| `wlr-gamma-control-unstable-v1` | `src/state/gamma_control.rs` | Gamma ramps, used for night shift |

`protocols/sc-layer-v1.xml` is the ancestor of `otto-surface-style` and is no
longer implemented; only stale comments still say `sc_layer`.
