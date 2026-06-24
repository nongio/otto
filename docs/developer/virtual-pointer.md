# wlr-virtual-pointer-unstable-v1

Otto implements `zwlr_virtual_pointer_manager_v1`, allowing external tools to synthesize pointer events that feed into the compositor's input pipeline as if they came from a real pointing device.

## Usage

```bash
# Move pointer to absolute position and click
wlrctl pointer move 500 300
wlrctl pointer click left

# Type text (uses the existing virtual-keyboard protocol)
wlrctl keyboard type "hello"

# Focus a specific window
wlrctl toplevel focus firefox
```

Compatible clients: `wlrctl`, `ydotool` (Wayland mode), `wtype`, and any custom automation driver.

### The automation trio

Otto exposes three protocols that together enable full remote control:

| Protocol | Purpose | Implementation |
|----------|---------|----------------|
| `virtual-keyboard-unstable-v1` | Synthesize key events | Smithay delegate |
| `wlr-virtual-pointer-unstable-v1` | Synthesize pointer events | Hand-rolled (`src/state/virtual_pointer.rs`) |
| `wlr-foreign-toplevel-management-unstable-v1` | Enumerate/focus/close windows | Smithay delegate |

## Supported events

All events are accumulated per-frame and flushed on `frame`:

| Request | Behavior |
|---------|----------|
| `motion` | Relative displacement added to current pointer position |
| `motion_absolute` | Normalized `[0, 1]` coordinates mapped to the first output's geometry |
| `button` | Left/right/middle button press/release (standard Linux button codes) |
| `axis` | Scroll amount on vertical/horizontal axis |
| `axis_source` | Sets the source (wheel, finger, etc.) on the pending `AxisFrame` |
| `axis_stop` | Stop notification for an axis (e.g. finger lifted from touchpad) |
| `axis_discrete` | Discrete scroll step (v120 high-resolution) |
| `frame` | Flushes all accumulated events to the pointer |

## Architecture (`src/state/virtual_pointer.rs`)

Smithay does not ship a virtual-pointer delegate, so the `GlobalDispatch`/`Dispatch` plumbing is hand-rolled.

### Per-pointer state

Each `ZwlrVirtualPointerV1` resource holds a `Mutex<VirtualPointerState>` with:
- `pending_motion` — accumulated `(dx, dy)` from `motion` requests
- `pending_absolute` — last `motion_absolute` position (overrides relative motion)
- `pending_buttons` — queued `(button_code, ButtonState)` pairs
- `axis_frame` — Smithay `AxisFrame` being built up across axis/axis_source/axis_stop/axis_discrete

### Frame flush

On `frame`, the accumulated state is committed in order:

1. **Motion** — either absolute (mapped to output geometry) or relative (added to current location). Updates `pointer.motion()`, `layers_engine.pointer_move()`, and `surface_under()` focus.
2. **Buttons** — each queued button dispatched via `pointer.button()`. On press, `focus_window_under_cursor` is called so clicks behave like real ones (raise + focus).
3. **Axis** — the built-up `AxisFrame` sent via `pointer.axis()`.
4. **Pointer frame** — `pointer.frame()` finalizes the sequence.

### Click-to-focus

Real libinput clicks run `focus_window_under_cursor` + `layers_engine.pointer_button_down/up` from `on_pointer_button`. The virtual pointer mirrors this in its frame flush so that `motion_absolute` + `button` correctly focuses and raises the target window.

## Limitations

- **No pointer constraints** — locked/confined pointer regions are not honored for virtual events.
- **No relative-motion reporting** — the `wp_relative_pointer` extension is not notified for synthesized motion.
- **Single seat** — events are injected into the default seat. The `seat` parameter in `create_virtual_pointer` is accepted but ignored.
