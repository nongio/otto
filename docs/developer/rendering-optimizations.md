# Rendering Optimizations

This document covers Otto's rendering performance strategies beyond the core pipeline described in [rendering.md](./rendering.md) and [render_loop.md](./render_loop.md).

## Per-window frame callback throttling

**Source:** `src/state/window_throttle.rs`

The single biggest lever for reducing GPU work is stopping frame callbacks to windows the user can't see. Well-behaved Wayland clients (Chromium, GTK4, Qt6) pause their internal render loops when frame callbacks stop arriving, which saves both compositor-side and client-side GPU work.

### Window states

Every mapped window is classified each frame into one of five states:

| State | Rate | xdg activated | When |
|-------|------|---------------|------|
| **Focused** | Full refresh | yes | Top-of-stack on current workspace, fullscreen, or expose active |
| **Secondary** | ~30 Hz | no | On current workspace, visible, not focused |
| **Occluded** | ~2 Hz | no | On current workspace, fully covered by opaque content above |
| **Minimized** | ~2 Hz | no | Explicitly minimized by the user |
| **HiddenWorkspace** | ~2 Hz | no | Window's workspace is not active on any output |

### Why 2 Hz instead of zero?

Hidden states deliberately trickle callbacks at 2 Hz rather than stopping entirely. Chromium 115+ has an eviction heuristic that discards content buffers when callbacks stop for too long, causing a blank-canvas-on-restore bug. The 2 Hz rate satisfies the heuristic while saving essentially all the work.

### Classification logic

`classify_one()` is a pure function — no Wayland or lay-rs state, easy to unit test:

```
is_minimized?          → Minimized
expose_active?         → Focused (all windows get smooth previews)
is_fullscreen_window?  → Focused
fullscreen_exists?     → Occluded (behind the fullscreen)
is_top_of_stack?       → Focused
in occluded_ids set?   → Occluded
otherwise              → Secondary
```

The `occluded_ids` set is computed by the lay-rs occlusion walk (when available). Currently empty — populated as a future refinement.

### Integration with the render loop

`classify_windows()` runs once per frame and produces a `HashMap<ObjectId, WindowThrottleState>`. The render loop passes each window's throttle duration to Smithay's `Window::send_frame()`, which skips the callback if insufficient time has elapsed.

The `is_activated` flag is sent via `xdg_toplevel.configure`, signaling toolkits to self-throttle on top of the compositor's frame-callback throttling.

## Damage tracking

Otto uses Smithay's `OutputDamageTracker` to render only damaged regions. The render loop skips frame submission entirely when all of these are false:

- `scene_has_damage` — lay-rs scene graph reports changes
- `dnd_needs_draw` — drag-and-drop icon is active
- `cursor_needs_draw` — pointer is in the output
- `has_screencopy` — a screencopy client is waiting for a frame

This means an idle desktop with no animations and no cursor movement submits zero frames.

## Screencopy render forcing

When a screencopy client has a pending frame (`pending_screencopy_frames` is non-empty), `should_draw` is forced true regardless of damage state. This ensures capture tools always get a fresh frame. See [screencopy.md](./screencopy.md) for details.
