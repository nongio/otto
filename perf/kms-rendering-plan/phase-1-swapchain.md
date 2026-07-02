# Phase 1 — Swapchain infra

**Status**: ✅ done  
**Effort**: 2 days  
**Blocks**: all subsequent phases

## Goal

Replace the single `OnceLock<Dmabuf>` in `SceneDmabufElement` with a proper
`Swapchain<GbmAllocator<DrmDeviceFd>>`, wire VBlank release, and add real
`NodeRef` subtree rendering.

## What was built

### SceneDmabufElement (`src/render_elements/scene_dmabuf_element.rs`)

- **Swapchain**: `Swapchain<GbmAllocator<DrmDeviceFd>>` in `Inner`, 3-slot via
  `GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT`.
- **Slot management**:
  - `current_slot: Option<Slot<GbmBuffer>>` — slot acquired for the current frame.
  - `previous_slot: Option<Slot<GbmBuffer>>` — slot KMS is actively scanning.
    Held for one extra VBlank before release to avoid writing to a buffer
    still being read by the display controller.
- **`ensure_swapchain(gbm, format, render_node)`** — idempotent setup.
- **`render(&mut SkiaRenderer) -> bool`** — acquire slot → clear canvas to
  transparent → translate to subtree origin → `render_node_tree` → `flush_and_submit(SyncCpu::Yes)` → store dmabuf and slot.
- **`submitted()`** — at VBlank: call `swapchain.submitted(previous_slot)`,
  then rotate `current_slot → previous_slot`. Ensures a slot is never
  reacquired while KMS is still scanning it.
- **`underlying_storage()`** — returns `UnderlyingStorage::Dmabuf(&current_dmabuf)`.
- **`opaque: bool`** field; `opaque_regions()` honours it. Background plane set
  to `false` so `scene_element` can serve as GPU fallback without being culled.
- **`position`, `plane_alpha`** exposed as public fields.

### Render path (`src/udev/render.rs`)

- `alloc_plane!` macro: unconditional lazy allocation of all plane elements
  on first render (not gated on `allow_direct_scanout`).
- `push_plane!` macro: calls `el.render(renderer.as_mut())` and pushes only
  on success.
- VBlank: `el.submitted()` called for all plane elements in `frame_finish`.

## Known issues resolved during implementation

- **Canvas clear**: must clear to `(0,0,0,0)` before each render; without it,
  stale content from 2 frames ago (the same swapchain slot) accumulated under
  transparent regions, causing additive double-painting.
- **scene_element double-render**: removed `scene_element` from the multi-plane
  render path. With overlay planes on top, the primary GPU composite also
  rendering the same subtrees caused the dock/overlay to appear doubled.
- **Scanout timing**: `previous_slot` added to hold the scanned buffer one
  extra VBlank. Without it, `submitted()` at VBlank N could release the slot
  KMS had just started scanning, allowing it to be overwritten mid-scanout.
