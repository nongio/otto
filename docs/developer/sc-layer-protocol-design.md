# Surface Style Protocol — design and history

> **Status: superseded.** This document was the design exploration for a
> client-facing animation protocol, originally sketched as `sc_layer_shell`.
> What shipped is **`otto-surface-style-unstable-v1`**:
> XML in [`protocols/otto-surface-style-unstable-v1.xml`](../../protocols/otto-surface-style-unstable-v1.xml),
> implementation in `src/surface_style/`. The XML is the authority for the
> current interface; this page explains *why* it looks the way it does.
> `protocols/sc-layer-v1.xml` is the dead ancestor — only stale code comments
> still say `sc_layer`.

## The idea

Otto already runs a full animation engine, `lay-rs`, on the compositor side:
springs, bezier timing, transforms, shadows, blur, batched transactions. Every
Wayland client sitting next to it has to reimplement all of that itself, in its
own process, and then push the result across the socket one buffer at a time.

The protocol hands the engine to clients. A client says *"move to (100, 200)
and fade to 0.5, over 300 ms, ease-out"* and stops thinking about it. The
compositor runs the animation at display refresh, on the GPU, whether or not
the client is scheduled.

The analogy is CSS transitions versus animating with `setInterval`. You are not
describing each frame; you are describing the destination and the feel, and
something below you handles the frames. That comparison is not accidental —
the model is Core Animation's, and several names (`contents_gravity`,
`anchor_point`, `masks_to_bounds`) come straight from it.

## Design principles

1. **Declarative.** Clients state the end state and the timing; the compositor
   executes.
2. **Transaction-based.** Property changes are grouped so they animate together
   and land atomically.
3. **Server-side.** Animation state lives in the compositor. No per-frame IPC,
   and a busy or blocked client does not stutter its own animation.
4. **Implicit.** Changes made inside a transaction animate by default.
5. **Spring physics.** Springs are interruptible mid-flight with the current
   velocity carried over, which is what makes gesture-driven motion feel right.

## How it maps

| Client object | Compositor state | lay-rs |
|---------------|------------------|--------|
| `otto_surface_style_v1` | `SurfaceStyle` | `Layer` |
| `otto_style_transaction_v1` | `StyleTransaction` | `TransactionRef` |
| `otto_timing_function_v1` | timing function state | `TimingFunction` / `Spring` |

## What shipped

Four interfaces, all in the XML:

**`otto_surface_style_manager_v1`** — the global. `get_surface_style` attaches
a style object to a `wl_surface`; `begin_transaction` and
`create_timing_function` make the other two.

**`otto_surface_style_v1`** — the properties. Geometry: `set_position`,
`set_z_position`, `set_size`, `set_scale`, `set_rotation`, `set_anchor_point`,
`set_transform`. Appearance: `set_opacity`, `set_background_color`,
`set_corner_radius`, `set_border`, `set_shadow`, `set_blend_mode`.
Layout and clipping: `set_hidden`, `set_masks_to_bounds`,
`set_contents_gravity` (how the surface buffer fills the layer — resize,
aspect-fit, aspect-fill), `set_z_order` (whether the style renders above or
below the surface's own content). Plus `cancel_animation` and
`cancel_all_animations`.

**`otto_style_transaction_v1`** — `set_duration`, `set_delay`,
`set_timing_function`, `enable_completion_event`, `commit`, and a `completed`
event.

**`otto_timing_function_v1`** — `set_preset` (linear, ease-in, ease-out,
ease-in-out), `set_bezier` for a custom cubic curve, and two ways to specify a
spring: `set_spring` (duration, bounce, initial velocity) or
`set_spring_stiffness_damping` for direct physical parameters.

## Usage shape

```c
// Group the changes, give them a feel, commit.
otto_style_transaction_v1 *tx = otto_surface_style_manager_v1_begin_transaction(mgr);
otto_style_transaction_v1_set_duration(tx, wl_fixed_from_double(0.3));

otto_timing_function_v1 *ease = otto_surface_style_manager_v1_create_timing_function(mgr);
otto_timing_function_v1_set_preset(ease, OTTO_TIMING_FUNCTION_V1_PRESET_EASE_OUT);
otto_style_transaction_v1_set_timing_function(tx, ease);

otto_surface_style_v1_set_position(style, wl_fixed_from_int(100), wl_fixed_from_int(200));
otto_surface_style_v1_set_opacity(style, wl_fixed_from_double(0.5));

otto_style_transaction_v1_commit(tx);
```

The gesture case is the one that justifies springs. Set the position directly
while the finger is down — no transaction, no animation — then on release,
commit a spring seeded with the gesture's own velocity so the motion continues
rather than restarting:

```c
otto_timing_function_v1_set_spring(spring,
    wl_fixed_from_double(0.3),      // duration
    wl_fixed_from_double(0.1),      // bounce
    wl_fixed_from_double(velocity)); // carried from the gesture
```

## Compositor side

On commit, the transaction's timing is converted to a lay-rs `TimingFunction`,
the pending property changes are turned into layer changes, and both are handed
to the engine:

```rust
let animation = engine.add_animation_from_transition(&transition, true);
let transactions = engine.schedule_changes(&changes, animation);

if tx.wants_completion {
    if let Some(tr) = transactions.first() {
        tr.on_finish(move |_, _| tx_object.completed(), true);
    }
}
```

See `src/surface_style/handlers/transactions.rs`.

## Not built

These were part of the original exploration and were not implemented. They are
recorded here so the reasoning is not re-derived:

- **Compositing filters** (blur, brightness, contrast, saturation as a
  client-settable filter object).
- **Keyframe animations** — multi-stop value tracks with per-segment timing.
- **Animation groups** — choreographing several transactions with relative
  start times.
- **Gesture recognizers** — binding a compositor-side gesture directly to a
  layer property, with `progress`/`velocity` events. Clients currently drive
  this themselves from ordinary pointer/touch events, which is more code but
  keeps the protocol small.

## References

- [`protocols/otto-surface-style-unstable-v1.xml`](../../protocols/otto-surface-style-unstable-v1.xml) — the interface
- `src/surface_style/` — the implementation
- [lay-rs](https://github.com/nongio/layers) — the engine being exposed
- `src/workspaces/mod.rs` — the compositor's own spring animation usage
