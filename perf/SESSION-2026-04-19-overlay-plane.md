# Session — 2026-04-19 overlay plane scanout

## What we proved

1. **Two-plane composition works**: primary (scene, Smithay swapchain) + overlay (window dmabuf).
2. **No tearing** with this setup — Smithay's swapchain handles primary buffering correctly.
3. **Smithay patches** (`feat/dmabuf-scanout` branch) correctly export window dmabufs to overlay planes.
4. **Overlap rule relaxation** in smithay works: opaque overlay allowed over primary.

## Current wiring (`src/udev/render.rs` scanout branch)

```
Elements (top → bottom):
  cursor elements       → cursor plane
  window surface elems  → overlay plane  (Kind::ScanoutCandidate, render_elements_from_surface_tree)
  SceneElement          → primary plane  (composited into Smithay swapchain, no tearing)
```

Window elements built as:
```rust
render_elements_from_surface_tree(renderer, &*wl_surface, scanout_window_location, scale, 1.0, Kind::ScanoutCandidate)
```

## What was debugged

- **Green screen test** confirmed KMS plane path works (`UnderlyingStorage::Dmabuf` → plane).
- **Tearing on circle**: `SceneDmabufElement` has a single buffer — KMS scans while GPU writes. Not fixable with `SyncCpu::Yes` because there's no second buffer.
- **Root cause of original artifacts** (before this session): transparent clear (`alpha=0.0`) in `SceneDmabufElement::update()` — transparent pixels on primary plane produce undefined hardware output.
- **Correct flush**: `flush_and_submit_surface(surface, SyncCpu::Yes)` not `flush_and_submit()` — context flush doesn't resolve surface FBO ops.

## Next step: scene on primary without GPU compositing

`SceneDmabufElement` needs a swapchain (2-3 GBM buffers) to avoid the single-buffer tearing. Use Smithay's `Swapchain<GbmAllocator<DrmDeviceFd>>`:
1. Allocate swapchain with `GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT`
2. Acquire slot → render scene into it → export as dmabuf → `UnderlyingStorage::Dmabuf`
3. Release slot when KMS signals VBlank (frame_submitted callback)
4. Smithay then composites nothing — the scene dmabuf goes directly to primary plane

This would reduce compositor GPU cost to near-zero for windowed workloads (scene static, only window animating).
