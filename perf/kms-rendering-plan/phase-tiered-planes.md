# Tiered Plane Assignment

## Goal

Replace the current best-effort plane push with a deterministic tiered system that
guarantees correct Z ordering regardless of hardware plane assignment failures.

## Tiers

```
Tier 1 — Minimum (3 planes):
  Primary:  bg + dock + windows  (all windows except top_window[0])
  Overlay:  top_window[0]        direct client buffer scanout
  Overlay:  overlay_ui           always topmost

Tier 2 — Good (2 + M planes):
  Primary:  bg + dock + windows  (all windows except top_window[0..M])
  Overlay:  top_window[0..M]     M direct client buffer scanouts
  Overlay:  overlay_ui

Tier 3 — Optimal (3 + N planes):
  Primary:  bg + dock
  Overlay:  windows              SceneDmabufElement swapchain
  Overlay:  top_window[0..N]     N direct client buffer scanouts
  Overlay:  overlay_ui
```

**Invariants:**
- `overlay_ui` is always on a hardware overlay plane, never in primary.
- `top_window[0]` is always on a hardware overlay plane (minimum guarantee).
- Primary always contains a complete, correct scene for everything not on a hardware plane.
- Z ordering is always correct: if any plane fails, it falls back to primary which
  already has that content composited in the right order.

---

## Step 1 — KMS grading test

Before implementing any tier rendering logic, we need a reliable way to determine
which tier the current hardware and buffer configuration can achieve. This is the
grading test.

### What it tests

For each tier from highest to lowest, the test asks:
"Can the DRM hardware atomically commit this exact plane configuration?"

It uses `drmModeAtomicCommit(DRM_MODE_ATOMIC_TEST_ONLY)` — no pixels are drawn,
no page flip happens, no display is affected. It is a pure hardware capability probe.

### Test inputs per tier

Each tier requires placeholder framebuffers on each proposed overlay plane:

| Tier | Planes tested |
|------|--------------|
| 3    | primary + windows GBM buf + top_window[0..N] client bufs + overlay_ui GBM buf |
| 2    | primary + top_window[0..M] client bufs + overlay_ui GBM buf |
| 1    | primary + top_window[0] client buf + overlay_ui GBM buf |

For GBM-backed planes (windows, overlay_ui): use the most recently rendered dmabuf
from the swapchain — format/modifier are stable so the test result is also stable.

For client buffer planes (top_window): use the current Wayland buffer from the
client — format/modifier can change when a new window maps or a buffer is reallocated,
so these trigger a re-test.

### Test procedure

```
grade(available_overlays, windows_dmabuf, top_window_bufs[], overlay_ui_dmabuf):
  for tier in [Tier3(N), Tier2(M), Tier1]:
    build atomic request for this tier
    result = drmModeAtomicCommit(TEST_ONLY, request)
    if result == Ok:
      return tier
  return Tier1  // always achievable with our own GBM buffers
```

### Where it runs

The grading test runs in `udev/device.rs` after surface creation, as a new method:

```rust
impl RenderSurface {
    fn grade_tier(
        &self,
        windows_dmabuf: Option<&Dmabuf>,
        top_window_bufs: &[Dmabuf],
        overlay_ui_dmabuf: Option<&Dmabuf>,
        available_overlays: usize,
    ) -> RenderTier
}
```

### Re-test triggers

| Event | Action |
|-------|--------|
| Output hotplug / mode change | Full re-test from Tier 3 |
| `top_window[0]` buffer format or modifier changes | Re-test from Tier 1 up |
| New window maps as top_window candidate | Re-test |
| GBM buffer reallocated (swapchain resize) | Re-test GBM tiers only |

### Smithay API needed

Smithay's `DrmCompositor` currently does not expose a standalone `test_only` commit.
The grading test needs to build and fire a raw `AtomicModeReq` with `TEST_ONLY`.

Options:
- **A** (preferred): Add `test_plane_config(planes: &[(plane::Handle, Dmabuf)]) -> bool`
  to `DrmCompositorSurface` in our Smithay fork. Internally builds and fires the
  `AtomicModeReq` with `TEST_ONLY | NONBLOCK` and returns the result.
- **B** (fallback): Expose `DrmDeviceFd` from the compositor surface and build the
  atomic request directly in Otto, reusing the `PropMapping` already cached in
  `AtomicDrmDevice`.

Option A keeps the KMS details inside Smithay where they belong.

### Result caching

```rust
struct RenderSurface {
    current_tier: RenderTier,
    last_top_window_buffer_id: Option<BufferId>,  // re-test on change
    tier_stable: bool,                             // false → re-grade next frame
}
```

`tier_stable = false` on any re-test trigger. On the next frame, run `grade_tier()`
before pushing any elements, update `current_tier`, set `tier_stable = true`.

---

## Remaining steps (implement after grading test is working)

- Step 2: Per-tier push logic in `render_surface()`
- Step 3: Primary scene content per tier (include/exclude windows from primary render)
- Step 4: Post-frame tier verification via `RenderElementStates`
- Step 5: `draw()` blit for `overlay_ui` (only element that can fail safely)

---

## Plane budget on eDP-1 (Intel i915, 6 overlays)

```
Tier 1: primary + top_window[0] + overlay_ui          = 3 planes
Tier 2: primary + top_window[0..5] + overlay_ui       = 7 planes max (fits: 1+6)
Tier 3: primary + windows + top_window[0..4] + overlay_ui = 7 planes max (fits: 1+6)
```
