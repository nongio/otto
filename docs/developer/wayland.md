# Wayland protocols

> What each protocol means to a user is in
> [Open standards](../user/desktop-standards.md#wayland-protocols).
> The freedesktop/XDG file and D-Bus standards around them are in
> [XDG specifications](xdg-specifications.md). This page owns every protocol,
> the `xdg_*` ones included.

Otto follows Smithay's "one big compositor state" architecture: nearly all
protocol state and every handler hangs off a single struct, `Otto<BackendData>`.
Learning where that struct is built, and how Smithay routes requests into it,
makes the rest of the codebase navigable.

## Supported protocols

Versions are the ones the global advertises. Otto's Smithay is the nongio fork
pinned in `Cargo.toml`. **Unsandboxed** means the global is hidden from
clients tagged through `wp_security_context_v1` (`src/sandbox.rs`).

### Core and shells

| Global | v | Availability | Implementation |
|--------|---|--------------|----------------|
| `wl_compositor`, `wl_subcompositor` | 5, 1 | all | `CompositorHandler` in `src/shell/mod.rs` |
| `wl_shm` | 2 | all; the mandatory ARGB8888/XRGB8888 only | `src/state/mod.rs` |
| `wl_seat` | 9 | all | `src/state/seat_handler.rs` |
| `wl_data_device_manager` | 3 | all | `src/state/data_device_handler.rs`, `dnd_grab_handler.rs` |
| `wl_output` | 4 | all, one per output (virtual outputs too) | `src/udev/device.rs`, `src/winit.rs`, `src/x11.rs`, `src/screenshare/mod.rs` |
| `zxdg_output_manager_v1` | 3 | all | `OutputManagerState::new_with_xdg_output` |
| `xdg_wm_base` | 7 | all | `XdgShellHandler` in `src/shell/xdg.rs` |
| `zxdg_decoration_manager_v1`, `org_kde_kwin_server_decoration_manager` | 1, 1 | all | `src/state/xdg_decoration_handler.rs` |
| `xdg_wm_dialog_v1` | 1 | all; the modal hint floats a parentless window when tiling | `XdgDialogHandler` in `src/state/mod.rs`, read in `src/shell/tiling.rs` |
| `xdg_activation_v1` | 1 | all; tokens need a fresh input serial, 10 s | `src/state/xdg_activation_handler.rs` |
| `zxdg_exporter_v2`, `zxdg_importer_v2` | 1 | all | `XdgForeignHandler` in `src/state/mod.rs` |
| `zwlr_layer_shell_v1` | 5 | unsandboxed | `src/shell/layer.rs`, `src/shell/mod.rs` |
| `xwayland_shell_v1`, `zwp_xwayland_keyboard_grab_manager_v1` | 1, 1 | `xwayland` feature (default) | `src/state/xwayland_handler.rs`, `src/shell/x11.rs` |
| `ext_background_effect_manager_v1` | 1 | all; the region only switches blur on, the whole surface frosts | `src/background_effect.rs`, see [specs/background-effect.md](../../specs/background-effect.md) |
| `org_kde_kwin_appmenu_manager` | 2 | all | `src/state/kde_appmenu.rs` |

### Buffers, timing and scaling

| Global | v | Availability | Implementation |
|--------|---|--------------|----------------|
| `zwp_linux_dmabuf_v1` | 6 (3 without feedback, always 3 headless) | per backend | `DmabufHandler` in `src/udev/`, `src/winit.rs`, `src/x11.rs` |
| `wl_drm` | — | per backend (`bind_wl_display`), legacy Mesa EGL | `src/udev/init.rs`, `src/winit.rs`, `src/x11.rs` |
| `wp_linux_drm_syncobj_manager_v1` | 1 | udev, when the GPU supports syncobj eventfd | `src/udev/init.rs`, `src/udev/mod.rs` |
| `wp_drm_lease_device_v1` | 1 | udev, one per DRM node | `src/udev/device.rs`, `src/udev/mod.rs` |
| `wp_presentation` | 2 | all; feedback is sent on udev only | `post_repaint` / `take_presentation_feedback`, `src/udev/render.rs` |
| `wp_fractional_scale_manager_v1` | 1 | all | `src/state/fractional_scale_handler.rs` |
| `wp_viewporter` | 1 | all | `src/state/mod.rs` |
| `wp_cursor_shape_manager_v1` | 2 | all | `src/state/mod.rs` |

### Input

| Global | v | Availability | Implementation |
|--------|---|--------------|----------------|
| `zwp_relative_pointer_manager_v1` | 1 | udev (`HAS_RELATIVE_MOTION`) | `src/input/pointer.rs` |
| `zwp_pointer_gestures_v1` | 3 | udev (`HAS_GESTURES`) | `src/input/gestures.rs` |
| `zwp_pointer_constraints_v1` | 1 | all | `src/state/input_method_handler.rs`, `src/input/pointer.rs` |
| `zwp_tablet_manager_v2` | 1 | all; events from libinput (udev) | `src/input/tablet.rs` |
| `zwp_keyboard_shortcuts_inhibit_manager_v1` | 1 | unsandboxed; every request granted, lock and VT switch still fire | `src/state/mod.rs`, `survives_shortcut_inhibition` in `src/input/keyboard.rs` |
| `zwp_text_input_manager_v3` | 1 | all | `src/state/mod.rs` |
| `zwp_input_method_manager_v2` | 1 | unsandboxed | `src/state/input_method_handler.rs` |
| `zwp_virtual_keyboard_manager_v1` | 1 | unsandboxed | `src/state/mod.rs` |
| `zwlr_virtual_pointer_manager_v1` | 2 | unsandboxed; hand-rolled | `src/state/virtual_pointer.rs` |

### Selection

| Global | v | Availability | Implementation |
|--------|---|--------------|----------------|
| `zwp_primary_selection_device_manager_v1` | 1 | all | `src/state/selection_handler.rs` |
| `zwlr_data_control_manager_v1`, `ext_data_control_manager_v1` | 2, 1 | unsandboxed | `src/state/data_device_handler.rs` |

### Session, windows, capture

| Global | v | Availability | Implementation |
|--------|---|--------------|----------------|
| `ext_session_lock_manager_v1` | 1 | only the locker Otto spawned (`ClientState::is_otto_locker`) | `src/state/session_lock_handler.rs`, `src/lock.rs` |
| `zwp_idle_inhibit_manager_v1` | 1 | all | `src/state/mod.rs`, checked in `src/lock.rs` |
| `wp_security_context_manager_v1` | 1 | all | `src/state/security_context_handler.rs`, `src/sandbox.rs` |
| `ext_foreign_toplevel_list_v1` | 1 | unsandboxed | `src/state/foreign_toplevel_list_handler.rs` |
| `zwlr_foreign_toplevel_manager_v1` | 3 | unsandboxed; hand-rolled over `wayland_protocols_wlr`; `set_rectangle` and the fullscreen output are ignored | `src/state/wlr_foreign_toplevel.rs`, see [foreign-toplevel.md](foreign-toplevel.md) |
| `zwlr_screencopy_manager_v1` | 3 | unsandboxed; shm and dmabuf; `copy_with_damage` copies without `damage` events | `src/state/screencopy.rs` |
| `zwlr_gamma_control_manager_v1` | 1 | unsandboxed; fails without DRM (gamma size 0) | `src/state/gamma_control.rs`, `src/udev/gamma.rs` |

### Otto's own

| Global | v | Implementation |
|--------|---|----------------|
| `otto_surface_style_manager_v1` | 5 | `src/surface_style/`, see [surface-style-protocol.md](surface-style-protocol.md) |
| `otto_dock_manager_v1` | 1 | `src/otto_dock/` |
| `otto_canvas_manager_v1` | 5 | `src/otto_canvas/` |
| `otto_text_cursor_manager_v1` | 1 | `src/text_cursor/`; `unavailable` is a normal answer |

### Not served

`wp_single_pixel_buffer`, `wp_alpha_modifier`, `wp_commit_timing`, `wp_fifo`,
`wp_content_type`, `wp_tearing_control`, `wp_color_management`,
`wp_pointer_warp`, `wl_fixes`, `xdg_system_bell`, `xdg_toplevel_icon`,
`xdg_toplevel_tag`, `xdg_toplevel_drag`, `ext_idle_notify`,
`ext_image_copy_capture` / `ext_image_capture_source`, `ext_workspace`,
`wlr_output_management`, `wlr_output_power_management`. Most exist in the
Smithay fork and need only wiring.

## The big state: `Otto<BackendData>`

`Otto<BackendData>` lives in `src/state/mod.rs` and holds:

- High-level compositor state: workspaces, popups, input state, the scene graph
- Smithay protocol state objects: `CompositorState`, `XdgShellState`,
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
  can actually do (`Backend::HAS_GESTURES`, `HAS_RELATIVE_MOTION`).
- Globals a sandboxed client may not see are created with a filter
  (`new_with_filter`, or the `UnsandboxedOnly` wrapper around a `GlobalDispatch2`).
  The hand-rolled globals check `sandbox::is_sandboxed_client` in their own
  `can_view`.

Backends create their own globals in their entrypoints: `zwp_linux_dmabuf_v1`,
`wl_drm`, `wl_output`, and on udev syncobj and DRM lease.

## How dispatch works

Otto's Smithay fork routes requests through Smithay's `GlobalDispatch2` /
`Dispatch2` traits rather than per-protocol `delegate_*!` macros. The pattern:

1. Otto stores the protocol state object (`xdg_shell_state: XdgShellState`).
2. Otto implements the matching handler trait
   (`impl XdgShellHandler for Otto<BackendData>`).
3. One `smithay::delegate_dispatch2!` in `src/state/mod.rs` generates the
   dispatch glue for every Smithay protocol at once.

Globals Smithay does not ship (virtual pointer, screencopy, gamma, foreign
toplevel management, background effect, Otto's own) are dispatched with
`wayland_server::delegate_dispatch!` / `delegate_global_dispatch!` next to
that line, pointing at their state type.

## Finding a protocol

1. **Find the state**: grep `src/state/mod.rs` for the protocol's
   `…State::new` (or the global's interface name for hand-rolled ones).
2. **Find the handler impl**: `impl<BackendData: Backend> XdgShellHandler for Otto<BackendData>`.
3. **Backend-specific globals** live in `src/udev/`, `src/winit.rs`,
   `src/x11.rs`.

Handlers are split roughly like this:

- `src/state/*.rs`: core protocol handlers (seat, selection, input method,
  fractional scale, foreign toplevel, session lock, screencopy, gamma, …)
- `src/shell/*.rs`: xdg-shell, layer-shell, XWayland, and surface commit plumbing
- `src/{udev,winit,x11}.rs`: backend-specific globals and handler impls

## Protocols with XML in the tree

Otto ships the XML for the protocols nothing else generates bindings for. It
lives in `protocols/`; the compositor and `otto-kit`
(`components/otto-kit/src/protocols/`) both generate from it.

| Protocol | Implementation | What it does |
|----------|----------------|--------------|
| `otto-surface-style-unstable-v1` | `src/surface_style/` | Lets a client style and animate its own surface through the compositor's scene graph: corner radius, shadow, opacity, transforms, batched in transactions. See [surface-style-protocol.md](surface-style-protocol.md). |
| `otto-dock-v1` | `src/otto_dock/` | Lets a client contribute items to the compositor-drawn dock |
| `otto-canvas-v1` | `src/otto_canvas/` | Items on the side canvas |
| `otto-text-cursor-v1` | `src/text_cursor/` | Tells a client where the desktop's text cursor is, so a picker or completion popup can sit beside it |
| `wlr-gamma-control-unstable-v1` | `src/state/gamma_control.rs` | Gamma ramps, for colour temperature tools |
| KDE appmenu | `src/state/kde_appmenu.rs` | Where a Qt/KDE app's menu lives on the bus, for the top bar's global menu |

The first four are Otto's own. Two files are not used:
`protocols/sc-layer-v1.xml` is not implemented (some identifiers still say
`sc_layer`), and `wlr-foreign-toplevel-management-unstable-v1.xml` is kept
for reference while the code uses the `wayland_protocols_wlr` crate.

## What Otto's apps use

`otto-kit` is the client side: core, `xdg_wm_base`, xdg-decoration,
xdg-activation, xdg-foreign, xdg-dialog, xdg-output, fractional scale,
viewporter, cursor shape, text-input-v3, pointer gestures, keyboard shortcuts
inhibit, background effect, KDE appmenu and Otto's own protocols. Beyond it:
layer shell (bar, islands, desk, launcher, lock, greeter, emoji, dictate,
authorize), session lock (`otto-lock`), wlr foreign toplevel (launcher, focus
watcher), wlr virtual pointer (kinetic scrolling, `otto-rdp`), virtual
keyboard (`otto-emoji`, `otto-rdp`), input-method-v2 (`otto-dictate`,
`otto-stash`) and wlr data control (`otto-stash`).
